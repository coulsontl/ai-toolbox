use std::fs;

use ai_toolbox_lib::coding::kimi::adapter;
use ai_toolbox_lib::coding::kimi::constants::{
    KIMI_CONFIG_FILE, KIMI_CREDENTIALS_DIR, KIMI_HOME_ENV_KEY, KIMI_PLUGINS_DIR, KIMI_PROMPT_FILE,
    KIMI_SESSIONS_DIR, KIMI_SKILLS_DIR,
};
use ai_toolbox_lib::coding::open_code::shell_env;
use ai_toolbox_lib::coding::runtime_location::{
    self, get_kimi_config_path_async, get_kimi_prompt_path_async, get_kimi_runtime_location_async,
};
use ai_toolbox_lib::db::helpers::db_put;
use ai_toolbox_lib::db::schema::DbTable;
use ai_toolbox_lib::db::sqlite_state::SqliteDbState;
use tempfile::TempDir;

use super::kimi_provider_and_config::KIMI_TEST_MUTEX;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime")
        .block_on(future)
}

struct EnvVarGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, val: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, val);
        Self { key, original }
    }

    fn remove(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(orig) = &self.original {
            std::env::set_var(self.key, orig);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn kimi_default_runtime_location_points_to_home_kimi_code() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _env_guard = EnvVarGuard::remove(KIMI_HOME_ENV_KEY);

    // The default-resolution chain also consults shell rc files. A developer
    // machine exporting KIMI_CODE_HOME there makes the "default" branch
    // unobservable from this process — skip rather than flake.
    if shell_env::get_env_from_shell_config(KIMI_HOME_ENV_KEY).is_some() {
        eprintln!(
            "skipping default-location assertions: {} is exported in shell rc files",
            KIMI_HOME_ENV_KEY
        );
        return;
    }

    let state = SqliteDbState::in_memory_for_test().expect("sqlite state");
    block_on(async {
        runtime_location::refresh_runtime_location_cache_for_module_async(&state, "kimi")
            .await
            .expect("refresh cache");
    });

    let loc = block_on(get_kimi_runtime_location_async(&state)).expect("get runtime loc");
    let home = dirs::home_dir().expect("home dir");
    let expected_root = home.join(".kimi-code");

    assert_eq!(loc.host_path, expected_root);
    assert_eq!(loc.source, "default");

    let config_path = block_on(get_kimi_config_path_async(&state)).expect("config path");
    assert_eq!(config_path, expected_root.join(KIMI_CONFIG_FILE));

    let prompt_path = block_on(get_kimi_prompt_path_async(&state)).expect("prompt path");
    assert_eq!(prompt_path, expected_root.join(KIMI_PROMPT_FILE));
}

#[test]
fn kimi_env_var_overrides_default_runtime_location() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_env_dir = TempDir::new().expect("temp env dir");
    let _env_guard = EnvVarGuard::set(
        KIMI_HOME_ENV_KEY,
        temp_env_dir.path().to_str().expect("utf8 path"),
    );

    let state = SqliteDbState::in_memory_for_test().expect("sqlite state");
    block_on(async {
        runtime_location::refresh_runtime_location_cache_for_module_async(&state, "kimi")
            .await
            .expect("refresh cache");
    });

    let loc = block_on(get_kimi_runtime_location_async(&state)).expect("get runtime loc");
    assert_eq!(loc.host_path, temp_env_dir.path());
    assert_eq!(loc.source, "env");

    let config_path = block_on(get_kimi_config_path_async(&state)).expect("config path");
    assert_eq!(config_path, temp_env_dir.path().join(KIMI_CONFIG_FILE));

    let prompt_path = block_on(get_kimi_prompt_path_async(&state)).expect("prompt path");
    assert_eq!(prompt_path, temp_env_dir.path().join(KIMI_PROMPT_FILE));
}

#[test]
fn kimi_custom_db_root_dir_takes_highest_precedence() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_env_dir = TempDir::new().expect("temp env dir");
    let _env_guard = EnvVarGuard::set(
        KIMI_HOME_ENV_KEY,
        temp_env_dir.path().to_str().expect("utf8 path"),
    );

    let temp_db_dir = TempDir::new().expect("temp db dir");
    let state = SqliteDbState::in_memory_for_test().expect("sqlite state");

    // Put custom root_dir in DB
    let custom_root = temp_db_dir.path().join("db_kimi_dir");
    fs::create_dir_all(&custom_root).expect("create custom root");

    let common_val = adapter::common_to_db_value("", Some(custom_root.to_str().unwrap()));
    state
        .with_conn(|conn| db_put(conn, DbTable::KimiCommonConfig, "common", &common_val))
        .expect("db_put common");

    block_on(async {
        runtime_location::refresh_runtime_location_cache_for_module_async(&state, "kimi")
            .await
            .expect("refresh cache");
    });

    let loc = block_on(get_kimi_runtime_location_async(&state)).expect("get runtime loc");
    assert_eq!(
        loc.host_path, custom_root,
        "Custom DB root dir must take precedence over env var"
    );
    assert_eq!(loc.source, "custom");

    // Test derived paths
    let config_path = block_on(get_kimi_config_path_async(&state)).expect("config path");
    assert_eq!(config_path, custom_root.join(KIMI_CONFIG_FILE));

    let prompt_path = block_on(get_kimi_prompt_path_async(&state)).expect("prompt path");
    assert_eq!(prompt_path, custom_root.join(KIMI_PROMPT_FILE));

    let skills_path = custom_root.join(KIMI_SKILLS_DIR);
    let plugins_path = custom_root.join(KIMI_PLUGINS_DIR);
    let sessions_path = custom_root.join(KIMI_SESSIONS_DIR);
    let credentials_path = custom_root.join(KIMI_CREDENTIALS_DIR);

    assert_eq!(skills_path, custom_root.join("skills"));
    assert_eq!(plugins_path, custom_root.join("plugins"));
    assert_eq!(sessions_path, custom_root.join("sessions"));
    assert_eq!(credentials_path, custom_root.join("credentials"));
}
