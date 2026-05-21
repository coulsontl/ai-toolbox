use ai_toolbox_lib::coding::gemini_cli::{
    adapter, import_gemini_cli_default_provider_from_local_files,
    init_gemini_cli_provider_from_settings, list_gemini_cli_providers_for_db,
    GeminiCliOfficialAccountContent,
};
use ai_toolbox_lib::coding::runtime_location;
use ai_toolbox_lib::db::helpers::{db_count, db_get, db_put};
use ai_toolbox_lib::db::schema::DbTable;
use ai_toolbox_lib::db::sqlite_state::SqliteDbState;
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(future)
}

fn gemini_cli_runtime_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("gemini cli runtime lock")
}

struct TestGeminiCliRoot {
    _temp_dir: TempDir,
    root_dir: std::path::PathBuf,
}

impl TestGeminiCliRoot {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let root_dir = temp_dir.path().join("gemini-root");
        std::fs::create_dir_all(&root_dir).expect("create gemini root");
        Self {
            _temp_dir: temp_dir,
            root_dir,
        }
    }

    fn write_env(&self, content: &str) {
        std::fs::write(self.root_dir.join(".env"), content).expect("write env");
    }

    fn write_settings(&self, settings: Value) {
        let content = serde_json::to_string_pretty(&settings).expect("serialize settings");
        std::fs::write(self.root_dir.join("settings.json"), content).expect("write settings");
    }

    fn write_oauth_creds(&self, auth: Value) {
        let content = serde_json::to_string_pretty(&auth).expect("serialize oauth creds");
        std::fs::write(self.root_dir.join("oauth_creds.json"), content).expect("write oauth creds");
    }
}

fn db_with_gemini_cli_root(root_dir: &std::path::Path) -> SqliteDbState {
    let db = SqliteDbState::in_memory_for_test().expect("sqlite state");
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::GeminiCliCommonConfig,
            "common",
            &adapter::to_db_value_common("{}", Some(&root_dir.to_string_lossy())),
        )
        .map(|_| ())
    })
    .expect("save gemini common config");
    block_on(runtime_location::refresh_runtime_location_cache_for_module_async(&db, "geminicli"))
        .expect("refresh gemini runtime cache");
    db
}

fn official_settings() -> Value {
    json!({
        "security": {
            "auth": {
                "selectedType": "oauth-personal"
            }
        }
    })
}

fn official_auth_snapshot() -> Value {
    json!({
        "access_token": "access-token",
        "refresh_token": "refresh-token",
        "expiry_date": 1_800_000_000_000_i64,
        "email": "ralph@example.com",
        "project_id": "project-123"
    })
}

fn save_official_account(db: &SqliteDbState, provider_id: &str) {
    let now = "2026-05-21T00:00:00Z".to_string();
    let content = GeminiCliOfficialAccountContent {
        provider_id: provider_id.to_string(),
        name: "Ralph".to_string(),
        kind: "oauth".to_string(),
        email: Some("ralph@example.com".to_string()),
        auth_snapshot: serde_json::to_string(&official_auth_snapshot())
            .expect("serialize auth snapshot"),
        auth_mode: Some("oauth-personal".to_string()),
        account_id: Some("project-123".to_string()),
        project_id: Some("project-123".to_string()),
        plan_type: Some("pro".to_string()),
        last_refresh: Some(now.clone()),
        limit_short_label: None,
        limit_5h_text: None,
        limit_weekly_text: None,
        limit_5h_reset_at: None,
        limit_weekly_reset_at: None,
        last_limits_fetched_at: None,
        last_error: None,
        sort_index: Some(0),
        is_applied: true,
        created_at: now.clone(),
        updated_at: now,
    };
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::GeminiCliOfficialAccount,
            "official-account",
            &adapter::to_db_value_official_account(&content),
        )
        .map(|_| ())
    })
    .expect("save official account");
}

#[test]
fn official_only_import_requires_local_official_runtime() {
    let _lock = gemini_cli_runtime_lock();
    let root = TestGeminiCliRoot::new();
    root.write_settings(official_settings());
    root.write_env("GEMINI_MODEL=gemini-3.1-pro-preview\n");
    let db = db_with_gemini_cli_root(&root.root_dir);

    let imported = block_on(import_gemini_cli_default_provider_from_local_files(
        &db, true,
    ))
    .expect("settings file alone should be a no-op");

    assert!(imported.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::GeminiCliProvider))
        .expect("count providers");
    assert_eq!(count, 0);
}

#[test]
fn lazy_list_imports_local_official_runtime_without_persisted_account() {
    let _lock = gemini_cli_runtime_lock();
    let root = TestGeminiCliRoot::new();
    root.write_settings(official_settings());
    root.write_env("GEMINI_MODEL=gemini-3.1-pro-preview\n");
    root.write_oauth_creds(official_auth_snapshot());
    let db = db_with_gemini_cli_root(&root.root_dir);

    let providers = block_on(list_gemini_cli_providers_for_db(&db)).expect("list providers");

    assert_eq!(providers.len(), 1);
    assert_ne!(providers[0].id, "__local__");
    assert_eq!(providers[0].category, "official");
    assert!(providers[0].is_applied);
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::GeminiCliProvider))
        .expect("count providers");
    assert_eq!(count, 1);
}

#[test]
fn imports_oauth_creds_without_settings_as_official_default_provider() {
    let _lock = gemini_cli_runtime_lock();
    let root = TestGeminiCliRoot::new();
    root.write_oauth_creds(official_auth_snapshot());
    let db = db_with_gemini_cli_root(&root.root_dir);

    let imported = block_on(import_gemini_cli_default_provider_from_local_files(
        &db, true,
    ))
    .expect("import official provider")
    .expect("provider imported");

    assert_ne!(imported.id, "__local__");
    assert_eq!(imported.category, "official");
    assert!(imported.is_applied);
}

#[test]
fn import_uses_fresh_provider_id_even_when_official_account_exists() {
    let _lock = gemini_cli_runtime_lock();
    let root = TestGeminiCliRoot::new();
    root.write_settings(official_settings());
    root.write_env("GEMINI_MODEL=gemini-3.1-pro-preview\n");
    root.write_oauth_creds(official_auth_snapshot());
    let db = db_with_gemini_cli_root(&root.root_dir);
    save_official_account(&db, "persisted-official-provider");

    let imported = block_on(import_gemini_cli_default_provider_from_local_files(
        &db, true,
    ))
    .expect("import official provider")
    .expect("provider imported");

    assert_ne!(imported.id, "persisted-official-provider");
    assert_eq!(imported.name, "默认配置");
    assert_eq!(imported.category, "official");
    assert!(imported.is_applied);

    let count = db
        .with_conn(|conn| db_count(conn, DbTable::GeminiCliProvider))
        .expect("count providers");
    assert_eq!(count, 1);

    let stored = db
        .with_conn(|conn| db_get(conn, DbTable::GeminiCliProvider, &imported.id))
        .expect("get provider")
        .expect("stored provider");
    assert_eq!(stored["category"], "official");
    assert_eq!(stored["is_applied"], true);
}

#[test]
fn startup_and_lazy_import_are_idempotent() {
    let _lock = gemini_cli_runtime_lock();
    let root = TestGeminiCliRoot::new();
    root.write_settings(official_settings());
    root.write_oauth_creds(official_auth_snapshot());
    let db = db_with_gemini_cli_root(&root.root_dir);

    block_on(init_gemini_cli_provider_from_settings(&db)).expect("startup import");
    let second = block_on(import_gemini_cli_default_provider_from_local_files(
        &db, true,
    ))
    .expect("lazy import");

    assert!(second.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::GeminiCliProvider))
        .expect("count providers");
    assert_eq!(count, 1);
}

#[test]
fn startup_keeps_third_party_local_config_temporary_even_with_official_runtime() {
    let _lock = gemini_cli_runtime_lock();
    let root = TestGeminiCliRoot::new();
    root.write_settings(json!({
        "security": {
            "auth": {
                "selectedType": "gemini-api-key"
            }
        }
    }));
    root.write_env(
        "GEMINI_API_KEY=sk-test\nGOOGLE_GEMINI_BASE_URL=https://example.invalid/v1beta\nGEMINI_MODEL=gemini-test\n",
    );
    root.write_oauth_creds(official_auth_snapshot());
    let db = db_with_gemini_cli_root(&root.root_dir);

    block_on(init_gemini_cli_provider_from_settings(&db)).expect("startup import");
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::GeminiCliProvider))
        .expect("count providers");
    assert_eq!(count, 0);

    let providers = block_on(list_gemini_cli_providers_for_db(&db)).expect("list providers");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "__local__");
    assert_eq!(providers[0].category, "custom");
}

#[test]
fn does_not_create_provider_without_local_config_files() {
    let _lock = gemini_cli_runtime_lock();
    let root = TestGeminiCliRoot::new();
    let db = db_with_gemini_cli_root(&root.root_dir);

    let imported = block_on(import_gemini_cli_default_provider_from_local_files(
        &db, true,
    ))
    .expect("empty local config import should be a no-op");

    assert!(imported.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::GeminiCliProvider))
        .expect("count providers");
    assert_eq!(count, 0);
}
