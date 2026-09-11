//! `aitoolbox://` deep-link provider import.
//!
//! Architecture (adapted from cc-switch):
//! 1. The `tauri-plugin-deep-link` plugin unifies all three URL entry points
//!    (macOS `RunEvent::Opened`, Win/Linux cold-start argv, Win/Linux
//!    second-instance argv forwarded by `tauri-plugin-single-instance`'s
//!    `deep-link` cargo feature) into a single `deep-link://new-url` event.
//! 2. [`install_deeplink_handlers`] subscribes via `on_open_url`; every URL
//!    routes through [`handle_deeplink_url`], which **parses only** — before the
//!    frontend listener is ready it stores the latest parsed request in a
//!    pending slot, and it always emits `deep-link-import` as a best-effort live
//!    event. No DB write happens here.
//! 3. The frontend confirmation dialog calls [`import_from_deeplink_unified`]
//!    after the user confirms, which is the only place that writes.
//! 4. Cold-start race: after the frontend listener is attached it calls
//!    [`mark_deeplink_frontend_ready`] to atomically mark the listener ready and
//!    drain the pending slot.

mod importer;
pub mod parser;
pub mod portable;
mod provider;
mod source;
mod utils;

use std::sync::Mutex;

use log::warn;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

pub use importer::*;
pub use parser::{DeepLinkError, DeepLinkErrorPayload, DeepLinkImportRequest};
pub use source::*;

use crate::db::SqliteDbState;

#[derive(Default)]
struct DeepLinkQueue {
    pending: Option<DeepLinkImportRequest>,
    frontend_ready: bool,
}

/// Pending deep-link request that arrived before the frontend listener mounted.
/// Latest-wins until [`mark_deeplink_frontend_ready`] drains it; after that,
/// hot links are delivered only through the live Tauri event.
#[derive(Default)]
pub struct DeepLinkState {
    queue: Mutex<DeepLinkQueue>,
}

impl DeepLinkState {
    pub(crate) fn mark_frontend_not_ready(&self) -> bool {
        self.queue
            .lock()
            .map(|mut guard| std::mem::replace(&mut guard.frontend_ready, false))
            .unwrap_or(false)
    }

    fn store_if_frontend_not_ready(&self, request: DeepLinkImportRequest) {
        if let Ok(mut guard) = self.queue.lock() {
            if !guard.frontend_ready {
                guard.pending = Some(request);
            }
        }
    }

    /// Mark the frontend listener as ready and drain the cold-start pending slot.
    pub(crate) fn mark_frontend_ready(&self) -> Option<DeepLinkImportRequest> {
        self.queue.lock().ok().and_then(|mut guard| {
            guard.frontend_ready = true;
            guard.pending.take()
        })
    }
}

/// Single funnel for every incoming `aitoolbox://` URL. Parses the URL, queues
/// the request only while the frontend listener is not ready, emits
/// `deep-link-import` (success) or `deep-link-error` (failure). Returns `true`
/// if the URL was a recognized deep link (regardless of parse success).
pub fn handle_deeplink_url(app: &AppHandle, url: &str, focus_window: bool) -> bool {
    if !url.starts_with("aitoolbox://") {
        return false;
    }

    match parser::parse_deeplink_url(url) {
        Ok(request) => {
            if let Some(state) = app.try_state::<DeepLinkState>() {
                state.store_if_frontend_not_ready(request.clone());
            }
            let _ = app.emit("deep-link-import", request);
            if focus_window {
                focus_main_window(app);
            }
            true
        }
        Err(error) => {
            let payload = DeepLinkErrorPayload {
                url: utils::redact_url_for_log(url),
                error: error.to_string(),
            };
            let _ = app.emit("deep-link-error", payload);
            if focus_window {
                focus_main_window(app);
            }
            true
        }
    }
}

/// Show and focus the main window (mirror of the existing single-instance
/// callback behavior). Rebuilds the window first when the app is in
/// lightweight mode (main window destroyed). No-op if the window doesn't exist.
fn focus_main_window(app: &AppHandle) {
    if crate::lightweight::is_lightweight_mode() {
        if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
            warn!("Failed to exit lightweight mode on deeplink: {e}");
        }
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        {
            use tauri::ActivationPolicy;
            let _ = app.set_activation_policy(ActivationPolicy::Regular);
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Register the `on_open_url` handler. Must be called from `setup()` after the
/// `deep-link` plugin is registered and after [`DeepLinkState`] is managed.
pub fn install_deeplink_handlers(app: &AppHandle) {
    let app_handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        // First matching `aitoolbox://` URL wins.
        for url in event.urls() {
            if handle_deeplink_url(&app_handle, url.as_str(), true) {
                break;
            }
        }
    });
}

/// Frontend-facing command: called after the frontend has attached the
/// `deep-link-import` listener. Returns the latest cold-start request, if any.
#[tauri::command]
pub fn mark_deeplink_frontend_ready(
    state: tauri::State<'_, DeepLinkState>,
) -> Option<DeepLinkImportRequest> {
    state.mark_frontend_ready()
}

/// Frontend-facing command: import a provider after the user confirms in the
/// dialog. The only place that writes to the DB. Dispatches by `resource`
/// (only `provider` in v1) and then by `app` to the per-tool builder.
#[tauri::command]
pub async fn import_from_deeplink_unified<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: AppHandle<R>,
    request: DeepLinkImportRequest,
    conflict_policy: Option<ImportConflictPolicy>,
) -> Result<DeepLinkImportResult, String> {
    if request.resource != parser::SUPPORTED_RESOURCE {
        return Err(format!(
            "deep-link: unsupported resource '{}'; only 'provider' is supported",
            request.resource
        ));
    }
    if !parser::SUPPORTED_APPS.contains(&request.app.as_str()) {
        return Err(format!("deep-link: unsupported app '{}'", request.app));
    }
    build_and_create_provider(state, &app, &request, conflict_policy.unwrap_or_default()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request(name: &str) -> DeepLinkImportRequest {
        parser::parse_deeplink_url(&format!(
            "aitoolbox://v1/import?resource=provider&app=codex&name={name}&category=custom"
        ))
        .expect("sample URL should parse")
    }

    #[test]
    fn pending_queue_stores_and_takes() {
        let state = DeepLinkState::default();
        assert!(
            state.mark_frontend_ready().is_none(),
            "fresh queue must be empty"
        );

        let state = DeepLinkState::default();
        state.store_if_frontend_not_ready(sample_request("First"));
        let taken = state.mark_frontend_ready();
        assert_eq!(taken.as_ref().unwrap().name, "First");
        assert!(
            state.mark_frontend_ready().is_none(),
            "marking ready must drain the queue"
        );
    }

    #[test]
    fn pending_queue_is_latest_wins() {
        let state = DeepLinkState::default();
        state.store_if_frontend_not_ready(sample_request("First"));
        state.store_if_frontend_not_ready(sample_request("Second"));
        state.store_if_frontend_not_ready(sample_request("Third"));

        // Only the most recent survives; v1 uses latest-wins semantics.
        let taken = state.mark_frontend_ready();
        assert_eq!(taken.as_ref().unwrap().name, "Third");
        assert!(state.mark_frontend_ready().is_none());
    }

    #[test]
    fn pending_queue_mark_ready_is_idempotent_when_empty() {
        let state = DeepLinkState::default();
        state.store_if_frontend_not_ready(sample_request("X"));
        let _ = state.mark_frontend_ready();
        // Repeated drains on an empty queue stay None (no panic, no stale value).
        assert!(state.mark_frontend_ready().is_none());
        assert!(state.mark_frontend_ready().is_none());
    }

    #[test]
    fn hot_links_are_not_stored_after_frontend_is_ready() {
        let state = DeepLinkState::default();
        assert!(state.mark_frontend_ready().is_none());

        state.store_if_frontend_not_ready(sample_request("Hot"));

        assert!(
            state.mark_frontend_ready().is_none(),
            "hot links are delivered by live event only and must not replay"
        );
    }

    #[test]
    fn rebuilt_frontend_receives_links_queued_while_window_was_absent() {
        let state = DeepLinkState::default();
        assert!(state.mark_frontend_ready().is_none());
        assert!(state.mark_frontend_not_ready());
        state.store_if_frontend_not_ready(sample_request("WhileAbsent"));
        state.store_if_frontend_not_ready(sample_request("WhileRebuilding"));

        assert_eq!(state.mark_frontend_ready().unwrap().name, "WhileRebuilding");
        assert!(state.mark_frontend_ready().is_none());
        state.store_if_frontend_not_ready(sample_request("HotAfterRebuild"));
        assert!(state.mark_frontend_ready().is_none());
    }

    #[test]
    fn failed_window_destroy_can_restore_live_delivery() {
        let state = DeepLinkState::default();
        assert!(!state.mark_frontend_not_ready());
        state.mark_frontend_ready();
        let was_ready = state.mark_frontend_not_ready();
        assert!(was_ready);
        if was_ready {
            state.mark_frontend_ready();
        }
        state.store_if_frontend_not_ready(sample_request("Live"));
        assert!(state.mark_frontend_ready().is_none());
    }
}
