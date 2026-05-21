use ai_toolbox_lib::coding::codex::{
    adapter, import_codex_default_provider_from_local_files, init_codex_provider_from_settings,
    list_codex_providers_for_db, CodexOfficialAccountContent,
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

fn codex_runtime_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("codex runtime lock")
}

struct TestCodexRoot {
    _temp_dir: TempDir,
    root_dir: std::path::PathBuf,
}

impl TestCodexRoot {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let root_dir = temp_dir.path().join("codex-root");
        std::fs::create_dir_all(&root_dir).expect("create codex root");
        Self {
            _temp_dir: temp_dir,
            root_dir,
        }
    }

    fn write_auth(&self, auth: Value) {
        let content = serde_json::to_string_pretty(&auth).expect("serialize auth");
        std::fs::write(self.root_dir.join("auth.json"), content).expect("write auth");
    }

    fn write_config(&self, config: &str) {
        std::fs::write(self.root_dir.join("config.toml"), config).expect("write config");
    }
}

fn db_with_codex_root(root_dir: &std::path::Path) -> SqliteDbState {
    let db = SqliteDbState::in_memory_for_test().expect("sqlite state");
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CodexCommonConfig,
            "common",
            &adapter::to_db_value_common("", Some(&root_dir.to_string_lossy())),
        )
        .map(|_| ())
    })
    .expect("save codex common config");
    block_on(runtime_location::refresh_runtime_location_cache_for_module_async(&db, "codex"))
        .expect("refresh codex runtime cache");
    db
}

fn official_auth() -> Value {
    json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": "header.eyJlbWFpbCI6InJhbHBoQGV4YW1wbGUuY29tIiwiaHR0cHM6Ly9hcGkub3BlbmFpLmNvbS9hdXRoIjp7ImNoYXRncHRfcGxhbl90eXBlIjoicHJvIiwiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8xMjMifX0.signature",
            "account_id": "acct_123"
        },
        "last_refresh": "2026-05-21T00:00:00Z"
    })
}

fn save_official_account(db: &SqliteDbState, provider_id: &str) {
    let now = "2026-05-21T00:00:00Z".to_string();
    let content = CodexOfficialAccountContent {
        provider_id: provider_id.to_string(),
        name: "Ralph".to_string(),
        kind: "oauth".to_string(),
        email: Some("ralph@example.com".to_string()),
        auth_snapshot: serde_json::to_string(&official_auth()).expect("serialize auth snapshot"),
        auth_mode: Some("chatgpt".to_string()),
        account_id: Some("acct_123".to_string()),
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
            DbTable::CodexOfficialAccount,
            "official-account",
            &adapter::to_db_value_official_account(&content),
        )
        .map(|_| ())
    })
    .expect("save official account");
}

#[test]
fn official_only_import_uses_local_official_runtime_without_persisted_account() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(official_auth());
    root.write_config("model = \"gpt-5.2\"\n");
    let db = db_with_codex_root(&root.root_dir);

    let imported = block_on(import_codex_default_provider_from_local_files(&db, true))
        .expect("import local official runtime")
        .expect("provider imported");

    assert_ne!(imported.id, "__local__");
    assert_eq!(imported.name, "默认配置");
    assert_eq!(imported.category, "official");
    assert!(imported.is_applied);
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 1);
}

#[test]
fn lazy_list_imports_local_official_runtime_without_persisted_account() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(official_auth());
    root.write_config("model = \"gpt-5.2\"\n");
    let db = db_with_codex_root(&root.root_dir);

    let providers = block_on(list_codex_providers_for_db(&db)).expect("list providers");

    assert_eq!(providers.len(), 1);
    assert_ne!(providers[0].id, "__local__");
    assert_eq!(providers[0].category, "official");
    assert!(providers[0].is_applied);
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 1);
}

#[test]
fn official_only_import_requires_valid_local_official_auth() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_config("model = \"gpt-5.2\"\n");
    let db = db_with_codex_root(&root.root_dir);

    let imported = block_on(import_codex_default_provider_from_local_files(&db, true))
        .expect("official config without auth should be a no-op");

    assert!(imported.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 0);
}

#[test]
fn import_uses_fresh_provider_id_even_when_official_account_exists() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(official_auth());
    root.write_config("model = \"gpt-5.2\"\n");
    let db = db_with_codex_root(&root.root_dir);
    save_official_account(&db, "persisted-official-provider");

    let imported = block_on(import_codex_default_provider_from_local_files(&db, true))
        .expect("import official provider")
        .expect("provider imported");

    assert_ne!(imported.id, "persisted-official-provider");
    assert_eq!(imported.name, "默认配置");
    assert_eq!(imported.category, "official");
    assert!(imported.is_applied);

    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 1);

    let stored = db
        .with_conn(|conn| db_get(conn, DbTable::CodexProvider, &imported.id))
        .expect("get provider")
        .expect("stored provider");
    assert_eq!(stored["category"], "official");
    assert_eq!(stored["is_applied"], true);
}

#[test]
fn startup_and_lazy_import_are_idempotent() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(official_auth());
    let db = db_with_codex_root(&root.root_dir);

    block_on(init_codex_provider_from_settings(&db)).expect("startup import");
    let second =
        block_on(import_codex_default_provider_from_local_files(&db, true)).expect("lazy import");

    assert!(second.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 1);
}

#[test]
fn startup_keeps_third_party_local_config_temporary_even_with_official_runtime() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(official_auth());
    root.write_config(
        r#"model_provider = "custom"
model = "gpt-test"
[model_providers.custom]
name = "custom"
base_url = "https://example.invalid/v1"
"#,
    );
    let db = db_with_codex_root(&root.root_dir);

    block_on(init_codex_provider_from_settings(&db)).expect("startup import");
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 0);

    let providers = block_on(list_codex_providers_for_db(&db)).expect("list providers");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "__local__");
    assert_eq!(providers[0].category, "custom");
}

#[test]
fn does_not_create_provider_without_local_config_files() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    let db = db_with_codex_root(&root.root_dir);

    let imported = block_on(import_codex_default_provider_from_local_files(&db, true))
        .expect("empty local config import should be a no-op");

    assert!(imported.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 0);
}

#[test]
fn official_only_import_does_not_import_custom_api_key_config() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(json!({
        "OPENAI_API_KEY": "sk-test",
        "auth_mode": "apikey"
    }));
    root.write_config(
        r#"model_provider = "custom"
model = "gpt-test"
[model_providers.custom]
name = "custom"
base_url = "https://example.invalid/v1"
"#,
    );
    let db = db_with_codex_root(&root.root_dir);

    let imported = block_on(import_codex_default_provider_from_local_files(&db, true))
        .expect("custom official-only import");

    assert!(imported.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 0);
}

#[test]
fn official_only_import_does_not_import_custom_top_level_base_url_config() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_config("model = \"gpt-test\"\nbase_url = \"https://example.invalid/v1\"\n");
    let db = db_with_codex_root(&root.root_dir);

    let imported = block_on(import_codex_default_provider_from_local_files(&db, true))
        .expect("custom base-url official-only import");

    assert!(imported.is_none());
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 0);
}

#[test]
fn lazy_list_imports_official_auth_as_persisted_provider_without_local_fallback_id() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(official_auth());
    let db = db_with_codex_root(&root.root_dir);

    let providers = block_on(list_codex_providers_for_db(&db)).expect("list providers");

    assert_eq!(providers.len(), 1);
    assert_ne!(providers[0].id, "__local__");
    assert_eq!(providers[0].category, "official");
    assert!(providers[0].is_applied);

    let second_list = block_on(list_codex_providers_for_db(&db)).expect("list providers again");
    assert_eq!(second_list.len(), 1);
    assert_eq!(second_list[0].id, providers[0].id);
}

#[test]
fn lazy_list_keeps_custom_local_config_as_temporary_provider() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(json!({
        "OPENAI_API_KEY": "sk-test",
        "auth_mode": "apikey"
    }));
    root.write_config(
        r#"model_provider = "custom"
model = "gpt-test"
[model_providers.custom]
name = "custom"
base_url = "https://example.invalid/v1"
"#,
    );
    let db = db_with_codex_root(&root.root_dir);

    let providers = block_on(list_codex_providers_for_db(&db)).expect("list providers");

    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "__local__");
    assert_eq!(providers[0].category, "custom");
    let count = db
        .with_conn(|conn| db_count(conn, DbTable::CodexProvider))
        .expect("count providers");
    assert_eq!(count, 0);
}

#[test]
fn command_level_e2e_imported_provider_unlocks_official_local_account() {
    let _lock = codex_runtime_lock();
    let root = TestCodexRoot::new();
    root.write_auth(official_auth());
    let db = db_with_codex_root(&root.root_dir);

    let providers = block_on(list_codex_providers_for_db(&db)).expect("list providers");
    let provider = providers.first().expect("provider imported by lazy list");

    let accounts = block_on(
        ai_toolbox_lib::coding::codex::list_codex_official_accounts_for_provider(&db, &provider.id),
    )
    .expect("list official accounts");

    assert_eq!(accounts.len(), 1);
    assert_ne!(provider.id, "__local__");
    assert_eq!(accounts[0].id, "__local__");
    assert_eq!(accounts[0].provider_id, provider.id);
    assert!(accounts[0].is_virtual);
    assert_eq!(accounts[0].email.as_deref(), Some("ralph@example.com"));
}
