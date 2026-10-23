pub mod all_api_hub;
pub mod antigravity;
pub mod auth_refresh;
pub mod cc_switch;
pub mod claude_code;
pub mod claude_desktop;
pub mod cli_extensions;
pub mod cli_resolver;
pub mod codex;
pub mod config_cleanup;
// Shared host (local vs WSL Direct) for provider config value resolution.
pub mod config_value_host;
pub mod deeplink;
pub mod dsh;
pub mod gemini_cli;
pub mod grok;
pub mod hermes;
pub mod image;
pub mod kimi;
// Shared identity of the "local file bridge" record every CLI shows when no
// preset is applied (`__local__` / `default`).
pub mod local_bridge;
pub mod magic_context;
pub mod mcp;
pub mod oh_my_openagent;
pub mod oh_my_opencode_slim;
pub mod oh_my_pi;
// Comment-preserving JSONC patch helpers for the shared `~/.omo/omo.jsonc`
// (used by both `oh_my_openagent` for `[opencode]` and `omo_native` for `[native]`).
// Public so the isolation integration test can assert the two blocks stay independent.
pub mod omo_jsonc_patch;
pub mod omo_native;
// Resolver for OMP-style config values (exact env name or literal / !command).
pub mod omp_config_value;
pub mod open_claw;
pub mod open_code;
pub mod pi;
// Resolver for Pi-style config values (env interpolation / !command) used by the
// common model-discovery and connectivity commands.
pub mod pi_config_value;
pub mod preset_models;
// Rebuild a provider map in a new key order; shared by the drag-sortable CLIs
// (pi / oh_my_pi / dsh) so the rule cannot drift between their reorder commands.
pub(crate) mod provider_order;
pub mod proxy_gateway;
pub mod reapply_applied_runtime;
pub mod runtime_location;
pub mod session_manager;
pub mod skills;
pub mod ssh;
pub mod tools;
pub(crate) mod url_utils;
pub mod wsl;
pub mod zcode;

mod db_id;
pub(crate) mod file_io;
#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{LazyLock, Mutex, MutexGuard};

    static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        // Recover from a poisoned lock instead of propagating the panic: other
        // tests still need to take this lock even after one holder panicked
        // (e.g. an optional CLI helper that only exists on some dev machines).
        TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

mod prompt_file;
pub use db_id::{
    db_build_id, db_clean_id, db_extract_id, db_extract_id_opt, db_new_id, db_record_id,
};

mod path_expand;
pub use path_expand::expand_local_path;
