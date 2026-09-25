use std::fs;
use std::sync::{LazyLock, Mutex};

use ai_toolbox_lib::coding::kimi::adapter;
use ai_toolbox_lib::coding::kimi::commands::{
    apply_kimi_provider_to_file, delete_kimi_provider_internal, disable_kimi_prompt_runtime,
    list_kimi_providers_for_db, write_common_config_without_provider,
    write_kimi_prompt_and_mark_applied,
};
use ai_toolbox_lib::coding::kimi::official_accounts;
use ai_toolbox_lib::coding::kimi::types::{
    KimiOfficialAccount, KimiPromptConfig, KimiPromptConfigContent, KimiProvider,
    KimiProviderContent,
};
use ai_toolbox_lib::coding::runtime_location::{self, get_kimi_prompt_path_async};
use ai_toolbox_lib::db::helpers::{db_get, db_list, db_put};
use ai_toolbox_lib::db::schema::DbTable;
use ai_toolbox_lib::db::sqlite_state::SqliteDbState;
use serde_json::json;
use tempfile::TempDir;

/// Lock to serialize tests touching Kimi runtime location or environment across all test files.
pub(crate) static KIMI_TEST_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime")
        .block_on(future)
}

/// Helper to set up a test environment with a dedicated root directory configured in DB.
fn setup_test_env() -> (TempDir, SqliteDbState) {
    let temp_dir = TempDir::new().expect("tempdir");
    let state = SqliteDbState::in_memory_for_test().expect("sqlite state");

    // Configure the custom root_dir in KimiCommonConfig so all commands point to temp_dir
    let common_val = adapter::common_to_db_value("", Some(temp_dir.path().to_str().unwrap()));
    state
        .with_conn(|conn| db_put(conn, DbTable::KimiCommonConfig, "common", &common_val))
        .expect("db_put common config");

    block_on(async {
        runtime_location::refresh_runtime_location_cache_for_module_async(&state, "kimi")
            .await
            .expect("refresh cache");
    });

    (temp_dir, state)
}

#[test]
fn provider_crud_db_roundtrip_and_apply_to_config_toml() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    // 1. Prepare existing config.toml with unmanaged/runtime fields and comments
    let config_path = temp_dir.path().join("config.toml");
    let initial_toml = r#"# User top-level comments
default_model = "unmanaged-model"

[unmanaged_section]
feature_flag = true
retry_count = 3

[providers.existing_custom]
type = "custom"
base_url = "https://existing.api/v1"
api_key = "sk-existing"
"#;
    fs::write(&config_path, initial_toml).expect("write initial toml");

    // 2. Create provider content and persist into DB
    let provider_id = "provider_alpha";
    let provider_settings = json!({
        "auth": {
            "API_KEY": "sk-alpha-secret"
        },
        "defaultModelKey": "alpha-fast",
        "modelCatalog": {
            "models": [
                {
                    "key": "alpha-fast",
                    "model": "alpha-fast-backend",
                    "provider": "alpha",
                    "displayName": "Alpha Fast Model",
                    "maxContextSize": 128000
                }
            ]
        },
        "providerConfigs": {
            "alpha": {
                "type": "custom",
                "base_url": "https://api.alpha.ai/v1"
            }
        },
        "config": ""
    });

    let provider_content = KimiProviderContent {
        name: "Alpha Provider".to_string(),
        category: "custom".to_string(),
        settings_config: serde_json::to_string(&provider_settings).unwrap(),
        source_provider_id: None,
        website_url: Some("https://api.alpha.ai".to_string()),
        notes: Some("Alpha test provider".to_string()),
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: None,
        is_applied: true,
        is_disabled: false,
        created_at: "2026-03-31T00:00:00Z".to_string(),
        updated_at: "2026-03-31T00:00:00Z".to_string(),
    };

    let db_val = adapter::provider_to_db_value(&provider_content);
    state
        .with_conn(|conn| db_put(conn, DbTable::KimiProvider, provider_id, &db_val))
        .expect("db_put provider");

    // 3. Read back from DB and verify fields via adapter and db_list
    let raw_db_row: Option<serde_json::Value> = state
        .with_conn(|conn| db_get(conn, DbTable::KimiProvider, provider_id))
        .expect("db_get");
    assert!(raw_db_row.is_some(), "Provider should exist in DB");
    let read_provider: KimiProvider = adapter::provider_from_db_value(raw_db_row.unwrap());
    assert_eq!(read_provider.id, provider_id);
    assert_eq!(read_provider.name, "Alpha Provider");
    assert_eq!(read_provider.category, "custom");
    assert_eq!(
        read_provider.website_url.as_deref(),
        Some("https://api.alpha.ai")
    );
    assert_eq!(read_provider.notes.as_deref(), Some("Alpha test provider"));
    assert!(read_provider.is_applied);

    let provider_list = list_kimi_providers_for_db(&state).expect("list providers");
    assert_eq!(provider_list.len(), 1);
    assert_eq!(provider_list[0].id, provider_id);

    // 4. Apply provider to config.toml file
    block_on(async {
        apply_kimi_provider_to_file(&state, provider_id)
            .await
            .expect("apply provider");
    });

    // 5. Verify config.toml disk content:
    // - Alpha provider managed fields are present
    // - Unmanaged existing sections/fields are preserved
    assert!(config_path.exists(), "config.toml should exist");
    let written_toml = fs::read_to_string(&config_path).expect("read config.toml");

    let parsed: toml_edit::DocumentMut = written_toml.parse().expect("valid toml");
    assert_eq!(
        parsed.get("default_model").and_then(|v| v.as_str()),
        Some("alpha-fast"),
        "default_model should be updated to alpha-fast"
    );
    assert!(
        parsed.get("unmanaged_section").is_some(),
        "unmanaged_section should be preserved"
    );
    assert_eq!(
        parsed["unmanaged_section"]
            .get("retry_count")
            .and_then(|v| v.as_integer()),
        Some(3)
    );
    assert!(
        parsed
            .get("providers")
            .and_then(|p| p.get("alpha"))
            .is_some(),
        "[providers.alpha] should be written"
    );
    assert_eq!(
        parsed["providers"]["alpha"]["base_url"].as_str(),
        Some("https://api.alpha.ai/v1")
    );
    assert_eq!(
        parsed["providers"]["alpha"]["api_key"].as_str(),
        Some("sk-alpha-secret")
    );
    assert!(
        parsed
            .get("models")
            .and_then(|m| m.get("alpha-fast"))
            .is_some(),
        "[models.alpha-fast] should be written"
    );
}

#[test]
fn provider_switching_cleans_managed_tables_and_replaces_projection() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();
    let config_path = temp_dir.path().join("config.toml");

    // 1. Initial config with unmanaged runtime settings
    let initial_toml = r#"
[runtime_extra]
telemetry = false

[providers.unmanaged_third_party]
type = "anthropic"
api_key = "sk-ant"
"#;
    fs::write(&config_path, initial_toml).expect("write initial toml");

    // 2. Define Provider A (Custom with providers.provider_a and models.model_a)
    let provider_a_id = "provider_a";
    let provider_a_settings = json!({
        "auth": {
            "API_KEY": "sk-key-a"
        },
        "defaultModelKey": "model_a",
        "modelCatalog": {
            "models": [
                {
                    "key": "model_a",
                    "model": "model-a-core",
                    "provider": "provider_a"
                }
            ]
        },
        "providerConfigs": {
            "provider_a": {
                "type": "custom",
                "base_url": "https://a.api/v1"
            }
        },
        "config": ""
    });
    let provider_a_content = KimiProviderContent {
        name: "Provider A".to_string(),
        category: "custom".to_string(),
        settings_config: serde_json::to_string(&provider_a_settings).unwrap(),
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: None,
        is_applied: true,
        is_disabled: false,
        created_at: "2026-03-31T00:00:00Z".to_string(),
        updated_at: "2026-03-31T00:00:00Z".to_string(),
    };
    state
        .with_conn(|conn| {
            db_put(
                conn,
                DbTable::KimiProvider,
                provider_a_id,
                &adapter::provider_to_db_value(&provider_a_content),
            )
        })
        .expect("db_put provider_a");

    // Apply Provider A
    block_on(async {
        apply_kimi_provider_to_file(&state, provider_a_id)
            .await
            .expect("apply provider_a");
    });

    let toml_a = fs::read_to_string(&config_path).expect("read toml A");
    let doc_a: toml_edit::DocumentMut = toml_a.parse().expect("valid toml");
    assert!(
        doc_a
            .get("providers")
            .and_then(|p| p.get("provider_a"))
            .is_some(),
        "provider_a must exist after apply A"
    );
    assert!(
        doc_a.get("models").and_then(|m| m.get("model_a")).is_some(),
        "model_a must exist after apply A"
    );
    assert_eq!(
        doc_a.get("default_model").and_then(|v| v.as_str()),
        Some("model_a")
    );

    // 3. Define Provider B (Custom with providers.provider_b and models.model_b)
    let provider_b_id = "provider_b";
    let provider_b_settings = json!({
        "auth": {
            "API_KEY": "sk-key-b"
        },
        "defaultModelKey": "model_b",
        "modelCatalog": {
            "models": [
                {
                    "key": "model_b",
                    "model": "model-b-core",
                    "provider": "provider_b"
                }
            ]
        },
        "providerConfigs": {
            "provider_b": {
                "type": "custom",
                "base_url": "https://b.api/v1"
            }
        },
        "config": ""
    });
    let provider_b_content = KimiProviderContent {
        name: "Provider B".to_string(),
        category: "custom".to_string(),
        settings_config: serde_json::to_string(&provider_b_settings).unwrap(),
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(1),
        meta: None,
        is_applied: true,
        is_disabled: false,
        created_at: "2026-03-31T00:00:00Z".to_string(),
        updated_at: "2026-03-31T00:00:00Z".to_string(),
    };
    state
        .with_conn(|conn| {
            db_put(
                conn,
                DbTable::KimiProvider,
                provider_b_id,
                &adapter::provider_to_db_value(&provider_b_content),
            )
        })
        .expect("db_put provider_b");

    // Switch/Apply Provider B
    block_on(async {
        apply_kimi_provider_to_file(&state, provider_b_id)
            .await
            .expect("apply provider_b");
    });

    let toml_b = fs::read_to_string(&config_path).expect("read toml B");
    let doc_b: toml_edit::DocumentMut = toml_b.parse().expect("valid toml");

    // Provider A's managed tables must be cleaned out
    assert!(
        doc_b
            .get("providers")
            .and_then(|p| p.get("provider_a"))
            .is_none(),
        "provider_a should be removed when switching to provider_b"
    );
    assert!(
        doc_b.get("models").and_then(|m| m.get("model_a")).is_none(),
        "model_a should be removed when switching to provider_b"
    );

    // Provider B's managed tables must be present
    assert!(
        doc_b
            .get("providers")
            .and_then(|p| p.get("provider_b"))
            .is_some(),
        "provider_b must be present"
    );
    assert!(
        doc_b.get("models").and_then(|m| m.get("model_b")).is_some(),
        "model_b must be present"
    );
    assert_eq!(
        doc_b.get("default_model").and_then(|v| v.as_str()),
        Some("model_b")
    );

    // Unmanaged third party and runtime settings are preserved
    assert!(
        doc_b.get("runtime_extra").is_some(),
        "runtime_extra should be preserved"
    );
    assert!(
        doc_b
            .get("providers")
            .and_then(|p| p.get("unmanaged_third_party"))
            .is_some(),
        "unmanaged_third_party in providers should be preserved"
    );
}

#[test]
fn common_config_save_and_reapply_preserves_provider_projection() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();
    let config_path = temp_dir.path().join("config.toml");

    // 1. Apply a provider first
    let provider_id = "provider_base";
    let provider_settings = json!({
        "auth": {
            "API_KEY": "sk-base"
        },
        "defaultModelKey": "base-model",
        "modelCatalog": {
            "models": [
                {
                    "key": "base-model",
                    "model": "base-model-id",
                    "provider": "base"
                }
            ]
        },
        "providerConfigs": {
            "base": {
                "type": "custom",
                "base_url": "https://base.api/v1"
            }
        },
        "config": ""
    });
    let provider_content = KimiProviderContent {
        name: "Base Provider".to_string(),
        category: "custom".to_string(),
        settings_config: serde_json::to_string(&provider_settings).unwrap(),
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: None,
        is_applied: true,
        is_disabled: false,
        created_at: "2026-03-31T00:00:00Z".to_string(),
        updated_at: "2026-03-31T00:00:00Z".to_string(),
    };
    state
        .with_conn(|conn| {
            db_put(
                conn,
                DbTable::KimiProvider,
                provider_id,
                &adapter::provider_to_db_value(&provider_content),
            )
        })
        .expect("db_put provider");

    block_on(async {
        apply_kimi_provider_to_file(&state, provider_id)
            .await
            .expect("apply provider");
    });

    // 2. Save common config directly in DB
    let common_toml = r#"# Common config additions
max_steps = 50
temperature = 0.7
"#;
    let common_val =
        adapter::common_to_db_value(common_toml, Some(temp_dir.path().to_str().unwrap()));
    state
        .with_conn(|conn| db_put(conn, DbTable::KimiCommonConfig, "common", &common_val))
        .expect("db_put common");

    // Reapply provider (which reads common config and combines)
    block_on(async {
        apply_kimi_provider_to_file(&state, provider_id)
            .await
            .expect("reapply provider");
    });

    // 3. Verify config.toml contains both common settings and active provider projection
    let written = fs::read_to_string(&config_path).expect("read config.toml");
    let doc: toml_edit::DocumentMut = written.parse().expect("valid toml");

    assert_eq!(
        doc.get("max_steps").and_then(|v| v.as_integer()),
        Some(50),
        "Common config max_steps should be merged"
    );
    assert_eq!(
        doc.get("default_model").and_then(|v| v.as_str()),
        Some("base-model"),
        "Provider default_model must be preserved"
    );
    assert!(
        doc.get("providers").and_then(|p| p.get("base")).is_some(),
        "Provider base section must be preserved"
    );
}

#[test]
fn prompt_config_crud_and_atomic_write_to_agents_md() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    let agents_md_path = temp_dir.path().join("AGENTS.md");

    // 1. Create a prompt in DB (not applied yet — the write path flips it)
    let prompt_id = "prompt_dev";
    let prompt_content = KimiPromptConfigContent {
        name: "Dev Instructions".to_string(),
        content: "# Instructions\nYou are an expert developer.\nFollow best practices.".to_string(),
        is_applied: false,
        sort_index: Some(0),
        created_at: "2026-03-31T00:00:00Z".to_string(),
        updated_at: "2026-03-31T00:00:00Z".to_string(),
    };

    let prompt_db_val = adapter::prompt_to_db_value(&prompt_content);
    state
        .with_conn(|conn| db_put(conn, DbTable::KimiPromptConfig, prompt_id, &prompt_db_val))
        .expect("db_put prompt");

    // 2. Read prompt back from DB
    let raw_prompts = state
        .with_conn(|conn| db_list(conn, DbTable::KimiPromptConfig, None))
        .expect("db_list prompt");
    assert_eq!(raw_prompts.len(), 1);
    let read_prompt: KimiPromptConfig = adapter::prompt_from_db_value(raw_prompts[0].clone());
    assert_eq!(read_prompt.id, prompt_id);
    assert_eq!(read_prompt.name, "Dev Instructions");
    assert!(read_prompt.content.contains("You are an expert developer"));
    assert!(!read_prompt.is_applied);

    // 3. Production path: atomic write to AGENTS.md at runtime location
    //    plus the DB is_applied flip.
    let prompt_path = block_on(get_kimi_prompt_path_async(&state)).expect("prompt path");
    assert_eq!(prompt_path, agents_md_path);
    block_on(write_kimi_prompt_and_mark_applied(&state, prompt_id)).expect("apply prompt");

    assert!(agents_md_path.exists());
    let written_content = fs::read_to_string(&agents_md_path).expect("read AGENTS.md");
    assert_eq!(written_content, prompt_content.content);
    let applied_raw = state
        .with_conn(|conn| db_get(conn, DbTable::KimiPromptConfig, prompt_id))
        .expect("db_get prompt")
        .expect("prompt row exists");
    assert!(
        adapter::prompt_from_db_value(applied_raw).is_applied,
        "apply must mark the prompt applied"
    );
}

#[test]
fn disable_prompt_clears_agents_md_and_keeps_record() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    let agents_md_path = temp_dir.path().join("AGENTS.md");
    let prompt_id = "prompt_disable";
    let prompt_content = KimiPromptConfigContent {
        name: "To Disable".to_string(),
        content: "# Temp\nShould be cleared after disable.".to_string(),
        is_applied: false,
        sort_index: Some(0),
        created_at: "2026-03-31T00:00:00Z".to_string(),
        updated_at: "2026-03-31T00:00:00Z".to_string(),
    };
    state
        .with_conn(|conn| {
            db_put(
                conn,
                DbTable::KimiPromptConfig,
                prompt_id,
                &adapter::prompt_to_db_value(&prompt_content),
            )
        })
        .expect("db_put prompt");

    // Apply first: the runtime file gets content and the record is marked applied.
    block_on(write_kimi_prompt_and_mark_applied(&state, prompt_id)).expect("apply prompt");
    assert!(agents_md_path.exists());

    // Disable: the runtime file must be emptied while the DB record survives
    // (unapplied) so the prompt can be re-applied later.
    block_on(disable_kimi_prompt_runtime(&state)).expect("disable prompt");

    let cleared = fs::read_to_string(&agents_md_path).expect("read AGENTS.md");
    assert_eq!(cleared, "", "disable must empty the runtime prompt file");

    let record = state
        .with_conn(|conn| db_get(conn, DbTable::KimiPromptConfig, prompt_id))
        .expect("db_get prompt")
        .expect("disable must keep the DB record");
    let read_back = adapter::prompt_from_db_value(record);
    assert!(!read_back.is_applied, "disable must unapply the prompt");
    assert_eq!(
        read_back.content, prompt_content.content,
        "disable must keep the prompt content in DB"
    );
}

#[test]
fn official_account_credentials_file_written_and_read() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    // Production write path: write_credential_file persists the account's
    // auth_snapshot under credentials/<name>.json with 0600 permissions.
    let snapshot = json!({
        "access_token": "tok_official_secret_123",
        "refresh_token": "ref_official_456",
        "token_endpoint": "https://auth.kimi.com/oauth/token",
    });
    let account = KimiOfficialAccount {
        id: "account_kimi_official".to_string(),
        provider_id: "kimi_official".to_string(),
        name: "Kimi Official Pro".to_string(),
        kind: "official".to_string(),
        email: None,
        subject: None,
        auth_snapshot: Some(snapshot.to_string()),
        token_endpoint: Some("https://auth.kimi.com/oauth/token".to_string()),
        expires_at: None,
        last_refresh: None,
        last_error: None,
        plan_type: None,
        limit_weekly_text: None,
        limit_monthly_text: None,
        limit_weekly_reset_at: None,
        limit_monthly_reset_at: None,
        last_limits_fetched_at: None,
        is_applied: true,
        sort_index: Some(0),
        created_at: "2026-03-31T00:00:00Z".to_string(),
        updated_at: "2026-03-31T00:00:00Z".to_string(),
    };

    block_on(official_accounts::write_credential_file(&state, &account))
        .expect("write credential file");

    let cred_file = temp_dir
        .path()
        .join("credentials")
        .join(format!("{}.json", account.name));
    assert!(cred_file.exists());
    let read_back: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cred_file).unwrap()).unwrap();
    assert_eq!(read_back["access_token"], "tok_official_secret_123");
    assert_eq!(read_back["refresh_token"], "ref_official_456");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&cred_file)
            .expect("credential file metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "credential file must be 0600");
    }
}

#[test]
fn legacy_credential_names_are_migrated_to_fixed_file_name() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    // Rows created before the credential name was fixed store
    // `kimi-<provider_id>`; the real Kimi CLI only reads
    // credentials/kimi-code.json, so the migration must adopt both the row
    // and the on-disk file.
    let snapshot = json!({
        "access_token": "tok_legacy",
        "refresh_token": "ref_legacy",
    });
    let legacy_row = json!({
        "provider_id": "kimi_official",
        "name": "kimi-managed-kimi-code",
        "kind": "official",
        "auth_snapshot": snapshot.to_string(),
        "expires_at": 4102444800i64,
        "is_applied": true,
        "sort_index": 0,
        "created_at": "2026-03-31T00:00:00Z",
        "updated_at": "2026-03-31T00:00:00Z",
    });
    state
        .with_conn(|conn| {
            db_put(
                conn,
                DbTable::KimiOfficialAccount,
                "account_legacy",
                &legacy_row,
            )
        })
        .expect("db_put legacy account");
    let credentials_dir = temp_dir.path().join("credentials");
    fs::create_dir_all(&credentials_dir).expect("create credentials dir");
    let legacy_file = credentials_dir.join("kimi-managed-kimi-code.json");
    fs::write(&legacy_file, snapshot.to_string()).expect("write legacy credential file");

    block_on(official_accounts::migrate_legacy_credential_names(&state))
        .expect("migrate legacy credential names");

    let accounts =
        official_accounts::list_kimi_official_accounts_with_state(&state).expect("list accounts");
    assert_eq!(accounts.len(), 1);
    assert_eq!(
        accounts[0].name, "kimi-code",
        "row must adopt the fixed name"
    );
    assert!(
        !legacy_file.exists(),
        "legacy credential file must be renamed"
    );
    let target_file = credentials_dir.join("kimi-code.json");
    assert!(target_file.exists());
    let read_back: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&target_file).unwrap()).unwrap();
    assert_eq!(read_back["refresh_token"], "ref_legacy");
}

#[test]
fn common_config_without_provider_merges_into_live_file_without_clobbering() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();
    let config_path = temp_dir.path().join("config.toml");

    // Live config.toml keeps unmanaged user sections, including a manual
    // [providers] table that is not tracked in the DB at all.
    let initial_toml = r#"
[unmanaged_section]
feature_flag = true

[stale_section]
old_common_key = 1

[providers.user_manual]
type = "openai"
base_url = "https://manual.example.com/v1"
api_key = "sk-manual"
"#;
    fs::write(&config_path, initial_toml).expect("write initial toml");

    // Previously managed common config (what AI Toolbox wrote last time).
    let previous_common = "[stale_section]\nold_common_key = 1\n";

    // New common config replaces the managed fields only.
    block_on(async {
        write_common_config_without_provider(
            &state,
            Some(previous_common),
            "[fresh_section]\nnew_common_key = 2\n",
        )
        .await
        .expect("write common config without provider");
    });

    let written = fs::read_to_string(&config_path).expect("read config.toml");
    let doc: toml_edit::DocumentMut = written.parse().expect("valid toml");
    // Previously managed section removed.
    assert!(
        doc.get("stale_section").is_none(),
        "stale managed section must be removed"
    );
    // New managed section written.
    assert_eq!(
        doc.get("fresh_section")
            .and_then(|s| s.get("new_common_key"))
            .and_then(|v| v.as_integer()),
        Some(2)
    );
    // Unmanaged user content must survive the save.
    assert!(
        doc.get("unmanaged_section").is_some(),
        "unmanaged section must be preserved"
    );
    assert!(
        doc.get("providers")
            .and_then(|p| p.get("user_manual"))
            .is_some(),
        "user-managed [providers] must not be clobbered when no provider is applied"
    );
    assert_eq!(
        doc["providers"]["user_manual"]["api_key"].as_str(),
        Some("sk-manual")
    );
}

#[test]
fn delete_kimi_provider_internal_rejects_applied_provider() {
    let _guard = KIMI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (_temp_dir, state) = setup_test_env();

    let insert = |state: &SqliteDbState, id: &str, is_applied: bool| {
        let content = KimiProviderContent {
            name: format!("Provider {id}"),
            category: "custom".to_string(),
            settings_config: r#"{"auth":{},"config":""}"#.to_string(),
            source_provider_id: None,
            website_url: None,
            notes: None,
            icon: None,
            icon_color: None,
            sort_index: Some(0),
            meta: None,
            is_applied,
            is_disabled: false,
            created_at: "2026-03-31T00:00:00Z".to_string(),
            updated_at: "2026-03-31T00:00:00Z".to_string(),
        };
        state
            .with_conn(|conn| {
                db_put(
                    conn,
                    DbTable::KimiProvider,
                    id,
                    &adapter::provider_to_db_value(&content),
                )
            })
            .expect("db_put provider");
    };

    insert(&state, "provider_applied", true);
    insert(&state, "provider_idle", false);

    // Applied provider must not be deletable: deleting it would leave its
    // projected tables in config.toml without an applied snapshot.
    let error = block_on(async { delete_kimi_provider_internal(&state, "provider_applied").await })
        .expect_err("applied provider must not be deletable");
    assert!(error.contains("applied"), "unexpected error: {error}");
    let still_there: Option<serde_json::Value> = state
        .with_conn(|conn| db_get(conn, DbTable::KimiProvider, "provider_applied"))
        .expect("db_get");
    assert!(still_there.is_some(), "applied provider must remain in DB");

    // Non-applied provider deletes normally.
    block_on(async { delete_kimi_provider_internal(&state, "provider_idle").await })
        .expect("non-applied provider must be deletable");
    let gone: Option<serde_json::Value> = state
        .with_conn(|conn| db_get(conn, DbTable::KimiProvider, "provider_idle"))
        .expect("db_get");
    assert!(gone.is_none());
}
