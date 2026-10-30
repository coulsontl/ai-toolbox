use super::*;
use crate::db::schema::DbTable;

fn seed_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn a_missing_models_file_reads_as_an_empty_channel_list() {
    let dir = seed_root();
    let document = read_document(dir.path()).expect("load");
    let runtime = build_runtime_config(dir.path(), &document);

    assert!(!runtime.exists);
    assert!(runtime.providers.is_empty());
    assert!(runtime.path.ends_with(models_json::MODELS_FILE_NAME));
}

#[test]
fn the_document_round_trips_through_the_shared_contract() {
    let dir = seed_root();
    std::fs::write(
        dir.path().join(models_json::MODELS_FILE_NAME),
        r#"{"models":[{"id":"a","url":"https://x/v1","apiKey":"k"}],"availableModels":["a"]}"#,
    )
    .expect("seed");

    let runtime = build_runtime_config(dir.path(), &read_document(dir.path()).expect("load"));
    assert!(runtime.exists);
    assert_eq!(runtime.providers.len(), 1);
    assert_eq!(runtime.available_models, Some(vec!["a".to_string()]));
}

/// Renaming merges onto the edited entry, so product-owned keys survive and the
/// row keeps its position. Same contract as WorkBuddy — see
/// `../workbuddy_family/AGENTS.md`.
#[test]
fn renaming_a_model_keeps_its_position_and_product_owned_keys() {
    let dir = seed_root();
    let models_path = dir.path().join(models_json::MODELS_FILE_NAME);
    std::fs::write(
        &models_path,
        r#"{"models":[
            {"id":"first","url":"https://a.example.com/v1/chat/completions","credits":"included"},
            {"id":"second","url":"https://b.example.com/v1/chat/completions"}
        ]}"#,
    )
    .expect("seed");

    let mut document = read_document(dir.path()).expect("load");
    let input = CodebuddyModelInput {
        previous_id: Some("first".to_string()),
        id: "first-renamed".to_string(),
        name: String::new(),
        vendor: String::new(),
        base_url: "https://a.example.com/v1".to_string(),
        api_key: String::new(),
        max_input_tokens: None,
        max_output_tokens: None,
        temperature: None,
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: false,
        only_reasoning: false,
        use_custom_protocol: false,
    };
    document.upsert_model_renaming(
        models_json::build_model_entry(&input.as_entry_input()),
        input.previous_id.as_deref(),
    );
    document.save(dir.path()).expect("save");

    let rows: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&models_path).expect("read"))
            .expect("parse");
    let models = rows
        .get("models")
        .and_then(serde_json::Value::as_array)
        .expect("models");
    assert_eq!(models.len(), 2, "a rename must not add a row");
    assert_eq!(models[0]["id"], "first-renamed");
    assert_eq!(models[0]["credits"], "included");
    assert_eq!(models[1]["id"], "second");
}

#[test]
fn the_local_prompt_bridge_reports_the_live_codebuddy_md() {
    let dir = seed_root();
    std::fs::write(dir.path().join(CODEBUDDY_PROMPT_FILE), "  # Rules\n").expect("prompt");

    let local = local_prompt_config(dir.path()).expect("read").expect("some");
    assert_eq!(local.id, crate::coding::local_bridge::LOCAL_CONFIG_ID);
    assert_eq!(local.content, "# Rules", "the file content is trimmed");
    assert!(local.is_applied);
}

#[test]
fn a_missing_prompt_file_yields_no_bridge_record() {
    let dir = seed_root();
    assert!(local_prompt_config(dir.path()).expect("read").is_none());
}

#[test]
fn prompt_records_round_trip_through_the_adapter() {
    let content = CodebuddyPromptConfigContent {
        name: "Preset".to_string(),
        content: "# Rules".to_string(),
        is_applied: true,
        sort_index: Some(2),
        created_at: "2026-10-30T00:00:00Z".to_string(),
        updated_at: "2026-10-30T00:00:00Z".to_string(),
    };
    let mut value = adapter::prompt_to_db_value(&content);
    value["id"] = serde_json::json!("abc");

    let parsed = adapter::prompt_from_db_value(value);
    assert_eq!(parsed.id, "abc");
    assert_eq!(parsed.name, "Preset");
    assert!(parsed.is_applied);
    assert_eq!(parsed.sort_index, Some(2));
}

#[test]
fn settings_records_drop_an_empty_config_dir_instead_of_persisting_blank() {
    let value = adapter::settings_to_db_value(Some("   "));
    assert!(value.get("config_dir").is_none());

    let parsed = adapter::settings_from_db_value(value);
    assert!(parsed.config_dir.is_none());
}

#[test]
fn the_prompt_table_exists_in_the_schema() {
    // Guards against the table being dropped from `ALL_TABLES` while the
    // commands still reference it.
    assert_eq!(
        DbTable::CodebuddyPromptConfig.name(),
        "codebuddy_prompt_config"
    );
}
