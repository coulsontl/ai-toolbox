//! Process-wide serialization for WSL sync work.
//!
//! Every sync entry point (manual sync, the 14 `wsl-sync-request-*` listeners,
//! `mcp-changed`, `skills-changed`, startup sync, post-restore sync) spawns its
//! own task. Without a shared lock two runs can read the same WSL config file
//! and write it back on top of each other, so the later writer silently drops
//! the earlier one's update. Serializing at this boundary keeps each run's
//! read-modify-write whole.
//!
//! Lock order when a run needs several of them: this lock first, then
//! `skills_sync::SKILLS_WSL_SYNC_LOCK`. Never acquire them in the other order.

use std::sync::OnceLock;

use tokio::sync::Mutex;

static WSL_SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Hold the returned guard for the duration of one sync run.
pub async fn acquire() -> tokio::sync::MutexGuard<'static, ()> {
    WSL_SYNC_LOCK.get_or_init(|| Mutex::new(())).lock().await
}
