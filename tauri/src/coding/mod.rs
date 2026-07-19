pub mod all_api_hub;
pub mod cc_switch;
pub mod claude_code;
pub mod cli_resolver;
pub mod codex;
pub mod config_cleanup;
pub mod gemini_cli;
pub mod grok;
pub mod image;
pub mod magic_context;
pub mod mcp;
pub mod oh_my_openagent;
pub mod oh_my_opencode_slim;
pub mod open_claw;
pub mod open_code;
pub mod pi;
pub mod preset_models;
pub mod proxy_gateway;
pub mod reapply_applied_runtime;
pub mod runtime_location;
pub mod session_manager;
pub mod skills;
pub mod ssh;
pub mod tools;
pub(crate) mod url_utils;
pub mod wsl;

mod db_id;
#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{LazyLock, Mutex, MutexGuard};

    static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        TEST_ENV_LOCK.lock().expect("test env lock poisoned")
    }
}

mod prompt_file;
pub use db_id::{
    db_build_id, db_clean_id, db_extract_id, db_extract_id_opt, db_new_id, db_record_id,
};

mod path_expand;
pub use path_expand::expand_local_path;
