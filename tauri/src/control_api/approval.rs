//! Desktop approval gate for control-plane writes.
//!
//! Every request that mutates state is classified as L1 (reversible) or L2
//! (destructive / trust boundary). Depending on the persisted policy it is
//! allowed, denied, or parked here while the desktop UI asks the user. The
//! HTTP handler awaits the same oneshot the modal resolves, so the CLI gets a
//! real 200/403/408 instead of polling.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use super::error::{ApiError, ApiResult};
use super::settings::{ControlApiApprovalMode, ControlApiSettings};

pub const APPROVAL_REQUEST_EVENT: &str = "control-api-approval-request";
pub const APPROVAL_EXPIRED_EVENT: &str = "control-api-approval-expired";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Read-only query.
    L0,
    /// Reversible write.
    L1,
    /// Destructive or trust-boundary write.
    L2,
}

impl RiskLevel {
    pub fn label(self) -> &'static str {
        match self {
            RiskLevel::L0 => "read",
            RiskLevel::L1 => "standard_write",
            RiskLevel::L2 => "high_risk",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalInfo {
    pub request_id: String,
    pub action: String,
    pub summary: String,
    pub risk: RiskLevel,
    pub target: Option<String>,
    pub timeout_secs: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
    Ask,
}

/// Pure policy evaluation, separated so it is testable without a runtime.
pub fn decide(
    mode: ControlApiApprovalMode,
    risk: RiskLevel,
    auto_approve_local_plugin_reload: bool,
    is_local_plugin_reload: bool,
) -> ApprovalDecision {
    if risk == RiskLevel::L0 {
        return ApprovalDecision::Allow;
    }
    match mode {
        ControlApiApprovalMode::ReadOnly => ApprovalDecision::Deny,
        ControlApiApprovalMode::AutoApprove => ApprovalDecision::Allow,
        ControlApiApprovalMode::AskHighRisk => {
            if risk == RiskLevel::L2 {
                ApprovalDecision::Ask
            } else {
                ApprovalDecision::Allow
            }
        }
        ControlApiApprovalMode::AlwaysAsk => {
            // The local-reload fast path only covers L1. A local plugin cannot
            // normally reach L2, but the explicit risk check keeps this policy
            // safe if a future local source is classified as destructive.
            if auto_approve_local_plugin_reload && is_local_plugin_reload && risk == RiskLevel::L1 {
                ApprovalDecision::Allow
            } else {
                ApprovalDecision::Ask
            }
        }
    }
}

struct PendingApproval {
    info: ApprovalInfo,
    sender: oneshot::Sender<bool>,
}

#[derive(Default)]
pub struct ApprovalHub {
    pending: Mutex<HashMap<String, PendingApproval>>,
    next_id: AtomicU64,
}

impl ApprovalHub {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_request_id(&self) -> String {
        let sequence = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        format!("approval-{}-{sequence}", std::process::id())
    }

    pub fn pending_infos(&self) -> Vec<ApprovalInfo> {
        let mut items = self
            .pending
            .lock()
            .map(|guard| {
                guard
                    .values()
                    .map(|pending| pending.info.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        items
    }

    /// Registers a pending approval and returns the request id + receiver.
    pub fn register(&self, info: ApprovalInfo) -> (String, oneshot::Receiver<bool>) {
        let request_id = info.request_id.clone();
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut guard) = self.pending.lock() {
            guard.insert(request_id.clone(), PendingApproval { info, sender });
        }
        (request_id, receiver)
    }

    /// Resolves a pending request. Returns false when the id is unknown (for
    /// example after a timeout), so the command can report a stale click.
    pub fn resolve(&self, request_id: &str, approved: bool) -> bool {
        let pending = self
            .pending
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(request_id));
        match pending {
            Some(pending) => pending.sender.send(approved).is_ok(),
            None => false,
        }
    }

    fn remove(&self, request_id: &str) -> Option<ApprovalInfo> {
        self.pending
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(request_id))
            .map(|pending| pending.info)
    }

    /// Rejects every parked approval and returns their infos so the caller can
    /// broadcast `control-api-approval-expired` and close the desktop modals.
    pub fn cancel_all(&self) -> Vec<ApprovalInfo> {
        let pending = self
            .pending
            .lock()
            .map(|mut guard| guard.drain().map(|(_, pending)| pending).collect::<Vec<_>>())
            .unwrap_or_default();
        pending
            .into_iter()
            .map(|pending| {
                let info = pending.info.clone();
                let _ = pending.sender.send(false);
                info
            })
            .collect()
    }
}

/// Inputs needed by the approval gate. Keeping this small lets the route
/// layer own the app/db references and the hub stay runtime-free.
pub struct ApprovalContext<'a> {
    pub app: &'a AppHandle,
    pub hub: &'a ApprovalHub,
    pub settings: ControlApiSettings,
}

pub struct ApprovalRequestSpec {
    pub action: String,
    pub summary: String,
    pub target: Option<String>,
    pub risk: RiskLevel,
    pub is_local_plugin_reload: bool,
}

/// Runs the policy and, when required, waits for the desktop modal.
pub async fn require_approval(
    context: &ApprovalContext<'_>,
    spec: ApprovalRequestSpec,
) -> ApiResult<()> {
    match decide(
        context.settings.approval_mode,
        spec.risk,
        context.settings.auto_approve_local_plugin_reload,
        spec.is_local_plugin_reload,
    ) {
        ApprovalDecision::Allow => Ok(()),
        ApprovalDecision::Deny => Err(ApiError::read_only(
            "The control API is in read-only mode; enable writes in Settings > Agent Integration",
        )),
        ApprovalDecision::Ask => {
            let timeout_secs = context.settings.approval_timeout_secs;
            let info = ApprovalInfo {
                request_id: context.hub.next_request_id(),
                action: spec.action,
                summary: spec.summary,
                risk: spec.risk,
                target: spec.target,
                timeout_secs,
                created_at: chrono::Local::now().to_rfc3339(),
            };
            let (request_id, receiver) = context.hub.register(info.clone());
            focus_main_window(context.app);
            let _ = context.app.emit(APPROVAL_REQUEST_EVENT, &info);

            match tokio::time::timeout(Duration::from_secs(timeout_secs), receiver).await {
                Ok(Ok(true)) => Ok(()),
                Ok(Ok(false)) | Ok(Err(_)) => Err(ApiError::approval_denied()),
                Err(_) => {
                    // Drop the parked sender first, then tell the UI which card
                    // is no longer valid.
                    if let Some(info) = context.hub.remove(&request_id) {
                        let _ = context.app.emit(
                            APPROVAL_EXPIRED_EVENT,
                            json!({"requestId": info.request_id, "reason": "timeout"}),
                        );
                    }
                    Err(ApiError::approval_timeout())
                }
            }
        }
    }
}

/// Shows and focuses the main window; rebuilds it first in lightweight mode.
/// The rebuild path is required by the app-wide lightweight-mode contract:
/// `get_webview_window("main")` returns `None` while the window is destroyed.
pub fn focus_main_window(app: &AppHandle) {
    if crate::lightweight::is_lightweight_mode() {
        if let Err(error) = crate::lightweight::exit_lightweight_mode(app) {
            log::warn!("Failed to rebuild the main window for a control API approval: {error}");
            return;
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_mode_denies_every_write() {
        assert_eq!(
            decide(ControlApiApprovalMode::ReadOnly, RiskLevel::L1, true, true),
            ApprovalDecision::Deny
        );
        assert_eq!(
            decide(
                ControlApiApprovalMode::ReadOnly,
                RiskLevel::L2,
                false,
                false
            ),
            ApprovalDecision::Deny
        );
        assert_eq!(
            decide(
                ControlApiApprovalMode::ReadOnly,
                RiskLevel::L0,
                false,
                false
            ),
            ApprovalDecision::Allow
        );
    }

    #[test]
    fn ask_high_risk_only_asks_for_l2() {
        assert_eq!(
            decide(
                ControlApiApprovalMode::AskHighRisk,
                RiskLevel::L1,
                false,
                false
            ),
            ApprovalDecision::Allow
        );
        assert_eq!(
            decide(
                ControlApiApprovalMode::AskHighRisk,
                RiskLevel::L2,
                false,
                false
            ),
            ApprovalDecision::Ask
        );
    }

    #[test]
    fn always_ask_can_fast_path_local_plugin_reloads() {
        assert_eq!(
            decide(ControlApiApprovalMode::AlwaysAsk, RiskLevel::L1, true, true),
            ApprovalDecision::Allow
        );
        assert_eq!(
            decide(
                ControlApiApprovalMode::AlwaysAsk,
                RiskLevel::L1,
                true,
                false
            ),
            ApprovalDecision::Ask
        );
        assert_eq!(
            decide(ControlApiApprovalMode::AlwaysAsk, RiskLevel::L2, true, true),
            ApprovalDecision::Ask
        );
    }

    #[test]
    fn auto_approve_allows_writes_but_never_bypasses_l0_policy() {
        assert_eq!(
            decide(
                ControlApiApprovalMode::AutoApprove,
                RiskLevel::L2,
                false,
                false
            ),
            ApprovalDecision::Allow
        );
        assert_eq!(
            decide(
                ControlApiApprovalMode::AutoApprove,
                RiskLevel::L0,
                false,
                false
            ),
            ApprovalDecision::Allow
        );
    }

    #[test]
    fn hub_resolves_exactly_once_and_reports_stale_ids() {
        let hub = ApprovalHub::new();
        let info = ApprovalInfo {
            request_id: "req-1".to_string(),
            action: "test".to_string(),
            summary: "summary".to_string(),
            risk: RiskLevel::L1,
            target: None,
            timeout_secs: 60,
            created_at: "now".to_string(),
        };
        let (id, mut receiver) = hub.register(info);
        assert_eq!(id, "req-1");
        assert_eq!(hub.pending_infos().len(), 1);
        assert!(hub.resolve("req-1", true));
        assert_eq!(receiver.try_recv(), Ok(true));
        assert!(!hub.resolve("req-1", false));
        assert!(hub.pending_infos().is_empty());
    }
}
