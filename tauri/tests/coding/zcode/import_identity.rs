//! Integration coverage for the ZCode import identity chain.
//!
//! Guards the two regressions fixed together: imported rows used to get a
//! generated row id while their settings carried the registry identity, which
//! made "set as default" fail to find the registry entry; and an import left
//! `providerId` blank, which degraded the card and made every later write be
//! rejected by the backend projection. The select fallback heals rows created
//! before the fix.

use std::path::Path;

use ai_toolbox_lib::coding::zcode::adapter;
use ai_toolbox_lib::coding::zcode::commands::{
    derive_zcode_row_id_and_settings, select_zcode_provider_internal_without_events,
    zcode_provider_id_slug,
};
use ai_toolbox_lib::coding::zcode::types::ZcodeProviderInput;
use ai_toolbox_lib::db::helpers::db_put;
use ai_toolbox_lib::db::schema::{Cli, CliConfigKind, DbTable};
use ai_toolbox_lib::db::sqlite_state::SqliteDbState;
use serde_json::{json, Value};
use tempfile::TempDir;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime")
        .block_on(future)
}

/// An in-memory DB whose ZCode data root points at `root`.
fn state_with_root(root: &Path) -> SqliteDbState {
    let state = SqliteDbState::in_memory_for_test().expect("sqlite state");
    let common = json!({ "config": "{}", "root_dir": root.to_str().unwrap() });
    state
        .with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Common), "common", &common))
        .expect("db_put zcode common");
    state
}

fn import_input(settings_config: &str) -> ZcodeProviderInput {
    ZcodeProviderInput {
        id: None,
        name: "DeepSeek Chat".to_string(),
        category: "custom".to_string(),
        settings_config: settings_config.to_string(),
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        is_disabled: None,
        meta: None,
    }
}

fn registry_with_provider(provider_id: &str) -> Value {
    json!({
        "schemaVersion": 1,
        "config": {
            "providerOrder": [provider_id],
            "providerConfigRules": { "providerRules": [{ "providerId": provider_id }] },
            "modelConfigRules": {
                "providerModelRules": [],
                "manualProviderModelRules": [],
            },
        },
    })
}

#[test]
fn slug_mirrors_the_frontend_builder() {
    assert_eq!(zcode_provider_id_slug("DeepSeek Chat"), "deepseek-chat");
    assert_eq!(zcode_provider_id_slug("  Foo--Bar  "), "foo-bar");
    assert_eq!(zcode_provider_id_slug("!!!"), "provider");
}

#[test]
fn a_blank_provider_id_is_filled_from_the_name_and_used_as_the_row_id() {
    let root = TempDir::new().expect("tempdir");
    let state = state_with_root(root.path());

    let settings = json!({
        "providerId": "",
        "providerName": "DeepSeek Chat",
        "config": { "api": { "type": "openai-chat-completions" } },
        "models": [],
    })
    .to_string();
    let (row_id, settings_config) = derive_zcode_row_id_and_settings(&state, &import_input(&settings))
        .expect("derive should succeed");

    assert_eq!(row_id, "custom:deepseek-chat");
    let parsed: Value = serde_json::from_str(&settings_config).expect("valid settings");
    assert_eq!(parsed["providerId"], "custom:deepseek-chat");
    // Unknown keys survive the round trip.
    assert_eq!(parsed["providerName"], "DeepSeek Chat");
}

#[test]
fn a_managed_provider_id_becomes_the_row_id_unchanged() {
    let root = TempDir::new().expect("tempdir");
    let state = state_with_root(root.path());

    let settings = json!({
        "providerId": "custom:favorite-row",
        "config": { "api": { "type": "openai-chat-completions" } },
        "models": [],
    })
    .to_string();
    let (row_id, settings_config) =
        derive_zcode_row_id_and_settings(&state, &import_input(&settings)).expect("derive");

    assert_eq!(row_id, "custom:favorite-row");
    assert_eq!(settings_config, settings);
}

#[test]
fn a_taken_provider_id_is_suffixed_in_the_row_id_and_the_settings() {
    let root = TempDir::new().expect("tempdir");
    let state = state_with_root(root.path());

    // An existing row already owns the id the import wants.
    let existing = adapter::to_db_value_provider(&ai_toolbox_lib::coding::zcode::types::ZcodeProviderContent {
        name: "Existing".to_string(),
        category: "custom".to_string(),
        settings_config: json!({ "providerId": "custom:deepseek-chat" }).to_string(),
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: 0,
        meta: None,
        is_disabled: false,
    });
    state
        .with_conn(|conn| db_put(conn, DbTable::CliProvider(Cli::Zcode), "custom:deepseek-chat", &existing))
        .expect("db_put existing");

    let settings = json!({
        "providerId": "custom:deepseek-chat",
        "config": { "api": { "type": "openai-chat-completions" } },
        "models": [],
    })
    .to_string();
    let (row_id, settings_config) = derive_zcode_row_id_and_settings(&state, &import_input(&settings))
        .expect("derive should succeed");

    assert_eq!(row_id, "custom:deepseek-chat-2");
    let parsed: Value = serde_json::from_str(&settings_config).expect("valid settings");
    // The suffix is mirrored into the settings so the projection writes the
    // row's own registry entry instead of overwriting the existing one.
    assert_eq!(parsed["providerId"], "custom:deepseek-chat-2");
}

#[test]
fn a_reserved_provider_id_falls_back_to_a_generated_row_id() {
    let root = TempDir::new().expect("tempdir");
    let state = state_with_root(root.path());

    let settings = json!({
        "providerId": "builtin:anthropic",
        "config": { "api": { "type": "anthropic-messages" } },
        "models": [],
    })
    .to_string();
    let (row_id, settings_config) =
        derive_zcode_row_id_and_settings(&state, &import_input(&settings)).expect("derive");

    // The row keeps a generated id; the settings are untouched so the later
    // projection answers with the reserved-id error instead of rewriting it.
    assert!(!row_id.starts_with("custom:"));
    assert_eq!(settings_config, settings);
}

#[test]
fn select_resolves_a_legacy_generated_row_id_through_the_stored_settings() {
    let root = TempDir::new().expect("tempdir");
    let state = state_with_root(root.path());
    let registry_dir = root.path().join("v2");
    std::fs::create_dir_all(&registry_dir).expect("registry dir");
    std::fs::write(
        registry_dir.join("provider_config.json"),
        serde_json::to_string_pretty(&registry_with_provider("custom:legacy-row")).unwrap(),
    )
    .expect("registry file");

    // A row created by the old import path: generated row id, settings carry
    // the registry identity.
    let legacy = adapter::to_db_value_provider(&ai_toolbox_lib::coding::zcode::types::ZcodeProviderContent {
        name: "Legacy import".to_string(),
        category: "custom".to_string(),
        settings_config: json!({
            "providerId": "custom:legacy-row",
            "config": { "api": { "type": "openai-chat-completions" } },
            "models": [{ "modelId": "m1", "ruleKind": "smart" }],
        })
        .to_string(),
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: 0,
        meta: None,
        is_disabled: false,
    });
    state
        .with_conn(|conn| db_put(conn, DbTable::CliProvider(Cli::Zcode), "generated-row-id", &legacy))
        .expect("db_put legacy row");

    block_on(select_zcode_provider_internal_without_events(
        &state,
        "generated-row-id",
        "m1",
    ))
    .expect("select should resolve the row id");

    let registry_text = std::fs::read_to_string(registry_dir.join("provider_config.json"))
        .expect("registry readable");
    let registry: Value = serde_json::from_str(&registry_text).expect("valid registry");
    assert_eq!(
        registry["config"]["defaultModelSelection"]["providerId"],
        "custom:legacy-row"
    );
    assert_eq!(registry["config"]["defaultModelSelection"]["modelId"], "m1");

    // The applied flag lands on the row whose settings carry the registry id.
    let stored = state
        .with_conn(|conn| ai_toolbox_lib::db::helpers::db_get(conn, DbTable::CliProvider(Cli::Zcode), "generated-row-id"))
        .expect("db_get")
        .expect("row exists");
    assert_eq!(
        stored.get("is_applied").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn select_still_answers_not_present_for_an_unknown_row() {
    let root = TempDir::new().expect("tempdir");
    let state = state_with_root(root.path());
    let registry_dir = root.path().join("v2");
    std::fs::create_dir_all(&registry_dir).expect("registry dir");
    std::fs::write(
        registry_dir.join("provider_config.json"),
        serde_json::to_string_pretty(&registry_with_provider("custom:other")).unwrap(),
    )
    .expect("registry file");

    let error = block_on(select_zcode_provider_internal_without_events(
        &state,
        "no-such-row",
        "m1",
    ))
    .expect_err("unknown row must fail");
    assert!(error.contains("not present in the ZCode registry"));
}
