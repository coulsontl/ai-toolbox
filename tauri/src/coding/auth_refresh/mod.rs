//! Shared official-account OAuth token refresh scheduler.
//!
//! - **Startup pass**: after `initial_delay`, every tool with `run_on_startup` runs once.
//! - **Interval pass**: tools with `interval` run on their own cadence.
//! - Per-tool logic (lead window, HTTP refresh, auth file shape) stays in each CLI module.
//! - On-demand paths (apply / limits) still call tool-local `ensure_fresh` directly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::AppHandle;
use tauri::Manager;

use crate::db::SqliteDbState;

mod providers;

const GLOBAL_INITIAL_DELAY: Duration = Duration::from_secs(90);
const TICK_GRANULARITY: Duration = Duration::from_secs(60);

static STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficialAuthTool {
    Grok,
    Codex,
    GeminiCli,
    Antigravity,
    Kimi,
    /// Plugin accounts: background OAuth renewal plus the optional daily
    /// check-in each account opted into.
    Plugins,
}

#[derive(Debug, Clone)]
pub struct AuthRefreshConfig {
    pub tool_id: &'static str,
    /// Participate in the post-startup first pass.
    pub run_on_startup: bool,
    /// Periodic pass interval; `None` means no interval pass.
    pub interval: Option<Duration>,
}

impl OfficialAuthTool {
    pub const ALL: [OfficialAuthTool; 6] = [
        OfficialAuthTool::Grok,
        OfficialAuthTool::Codex,
        OfficialAuthTool::GeminiCli,
        OfficialAuthTool::Antigravity,
        OfficialAuthTool::Kimi,
        OfficialAuthTool::Plugins,
    ];

    pub fn config(self) -> AuthRefreshConfig {
        match self {
            // Short-lived access tokens (~hours): startup + 15m.
            OfficialAuthTool::Grok => AuthRefreshConfig {
                tool_id: "grok",
                run_on_startup: true,
                interval: Some(Duration::from_secs(15 * 60)),
            },
            // Long-lived ChatGPT tokens; apply path already ensure_fresh.
            // Startup scan only; no aggressive interval.
            OfficialAuthTool::Codex => AuthRefreshConfig {
                tool_id: "codex",
                run_on_startup: true,
                interval: Some(Duration::from_secs(12 * 60 * 60)),
            },
            // ~hour tokens with 5m lead on ensure_fresh.
            OfficialAuthTool::GeminiCli => AuthRefreshConfig {
                tool_id: "gemini_cli",
                run_on_startup: true,
                interval: Some(Duration::from_secs(15 * 60)),
            },
            OfficialAuthTool::Antigravity => AuthRefreshConfig {
                tool_id: "antigravity",
                run_on_startup: true,
                interval: Some(Duration::from_secs(15 * 60)),
            },
            // ~15m access tokens with a 5m refresh lead (official_accounts.rs):
            // a 10m interval lets passes fire before expiry instead of chasing
            // tokens that just expired.
            OfficialAuthTool::Kimi => AuthRefreshConfig {
                tool_id: "kimi",
                run_on_startup: true,
                interval: Some(Duration::from_secs(10 * 60)),
            },
            // Plugin OAuth tokens and optional daily check-ins: the pass is
            // cheap when nothing is due (it only reads plugin-auth.json and
            // SQLite) and never starts Bun without a candidate.
            OfficialAuthTool::Plugins => AuthRefreshConfig {
                tool_id: "plugins",
                run_on_startup: true,
                interval: Some(Duration::from_secs(15 * 60)),
            },
        }
    }

    async fn run_pass(self, db: &SqliteDbState, app: &AppHandle) -> Result<(), String> {
        match self {
            OfficialAuthTool::Grok => providers::grok_refresh_applied_pass(db, app).await,
            OfficialAuthTool::Codex => providers::codex_refresh_applied_pass(db, app).await,
            OfficialAuthTool::GeminiCli => {
                providers::gemini_cli_refresh_applied_pass(db, app).await
            }
            OfficialAuthTool::Antigravity => {
                providers::antigravity_refresh_applied_pass(db, app).await
            }
            OfficialAuthTool::Kimi => providers::kimi_refresh_applied_pass(db, app).await,
            OfficialAuthTool::Plugins => providers::plugins_background_pass(db, app).await,
        }
    }
}

struct ToolScheduleState {
    tool: OfficialAuthTool,
    last_interval_run: Option<Instant>,
    in_flight: bool,
}

/// Start the shared auth-refresh loop once per process.
pub fn start(app: AppHandle) {
    if STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let mut states: Vec<ToolScheduleState> = OfficialAuthTool::ALL
            .into_iter()
            .map(|tool| ToolScheduleState {
                tool,
                last_interval_run: None,
                in_flight: false,
            })
            .collect();

        tokio::time::sleep(GLOBAL_INITIAL_DELAY).await;

        // Startup pass: every tool with run_on_startup.
        for state in &mut states {
            if !state.tool.config().run_on_startup {
                continue;
            }
            run_tool_pass(state, &app, "startup").await;
        }

        loop {
            tokio::time::sleep(TICK_GRANULARITY).await;
            let now = Instant::now();
            for state in &mut states {
                let Some(interval) = state.tool.config().interval else {
                    continue;
                };
                let due = match state.last_interval_run {
                    None => true,
                    Some(last) => now.duration_since(last) >= interval,
                };
                if !due || state.in_flight {
                    continue;
                }
                run_tool_pass(state, &app, "interval").await;
            }
        }
    });
}

async fn run_tool_pass(state: &mut ToolScheduleState, app: &AppHandle, kind: &str) {
    if state.in_flight {
        return;
    }
    state.in_flight = true;
    let tool = state.tool;
    let tool_id = tool.config().tool_id;
    let result = if let Some(db_state) = app.try_state::<SqliteDbState>() {
        tool.run_pass(db_state.inner(), app).await
    } else {
        Err("SqliteDbState not ready".to_string())
    };
    if let Err(error) = result {
        log::debug!("auth_refresh {kind} pass failed for {tool_id}: {error}");
    }
    // Startup and interval share last_interval_run: the next interval pass is
    // "full interval after the last completed pass", not wall-clock from process start.
    state.last_interval_run = Some(Instant::now());
    state.in_flight = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_have_stable_ids() {
        let ids: Vec<_> = OfficialAuthTool::ALL
            .into_iter()
            .map(|tool| tool.config().tool_id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "grok",
                "codex",
                "gemini_cli",
                "antigravity",
                "kimi",
                "plugins"
            ]
        );
    }

    #[test]
    fn plugins_run_on_startup_with_a_fifteen_minute_interval() {
        let config = OfficialAuthTool::Plugins.config();
        assert!(config.run_on_startup);
        assert_eq!(config.interval, Some(Duration::from_secs(15 * 60)));
    }

    #[test]
    fn grok_runs_on_startup_with_short_interval() {
        let config = OfficialAuthTool::Grok.config();
        assert!(config.run_on_startup);
        assert_eq!(config.interval, Some(Duration::from_secs(15 * 60)));
    }

    #[test]
    fn codex_runs_on_startup_with_long_interval() {
        let config = OfficialAuthTool::Codex.config();
        assert!(config.run_on_startup);
        assert_eq!(config.interval, Some(Duration::from_secs(12 * 60 * 60)));
    }
}
