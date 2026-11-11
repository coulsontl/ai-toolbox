use super::*;
use serde_json::json;

fn entry(id: &str, url: &str, api_key: &str) -> Value {
    json!({ "id": id, "url": url, "apiKey": api_key })
}

#[test]
fn a_bare_array_stays_a_bare_array() {
    let raw = r#"[{"id":"a","url":"https://x/v1/chat/completions","apiKey":"k"}]"#;
    let mut document = ModelsDocument::parse(raw, true).expect("parse");
    assert_eq!(document.shape, ModelsDocumentShape::Array);
    assert_eq!(document.models.len(), 1);

    document.upsert_model(entry("b", "https://x/v1/chat/completions", "k"));
    let written = document.to_value();
    assert!(written.is_array(), "array shape must survive a write");
    assert_eq!(written.as_array().unwrap().len(), 2);
}

#[test]
fn an_object_document_keeps_keys_it_does_not_own() {
    let raw = r#"{
        "models": [{"id":"a","url":"https://x/v1/chat/completions","apiKey":"k"}],
        "availableModels": ["a","fast-model"],
        "mergeStrategy": "smartMerge"
    }"#;
    let mut document = ModelsDocument::parse(raw, true).expect("parse");
    document.upsert_model(entry("b", "https://x/v1/chat/completions", "k"));

    let written = document.to_value();
    assert_eq!(
        written["availableModels"],
        json!(["a", "fast-model"]),
        "availableModels must never be narrowed: an absent/empty list shows every model"
    );
    assert_eq!(written["mergeStrategy"], json!("smartMerge"));
    assert_eq!(written["models"].as_array().unwrap().len(), 2);
}

#[test]
fn upsert_merges_onto_an_existing_entry_instead_of_replacing_it() {
    let raw = r#"{"models":[{
        "id":"a",
        "url":"https://x/v1/chat/completions",
        "apiKey":"old",
        "credits":"x0.21",
        "tags":["custom"],
        "reasoning":{"effort":"medium"}
    }]}"#;
    let mut document = ModelsDocument::parse(raw, true).expect("parse");
    document.upsert_model(json!({
        "id": "a",
        "url": "https://x/v1/chat/completions",
        "apiKey": "new",
        "supportsToolCall": true
    }));

    let saved = &document.to_value()["models"][0];
    assert_eq!(saved["apiKey"], json!("new"));
    assert_eq!(saved["supportsToolCall"], json!(true));
    assert_eq!(saved["credits"], json!("x0.21"), "product-owned keys survive");
    assert_eq!(saved["tags"], json!(["custom"]));
    assert_eq!(saved["reasoning"], json!({"effort":"medium"}));
}

/// Renaming merges onto the edited entry: position and product-owned keys stay,
/// and the row count does not grow.
#[test]
fn renaming_an_entry_keeps_its_slot_and_extras() {
    let raw = r#"{"models":[
        {"id":"a","url":"https://x/v1/chat/completions","credits":"x0.21"},
        {"id":"b","url":"https://y/v1/chat/completions"}
    ]}"#;
    let mut document = ModelsDocument::parse(raw, true).expect("parse");
    document.upsert_model_renaming(
        json!({"id":"a2","url":"https://x/v1/chat/completions","supportsToolCall":true}),
        Some("a"),
    );

    assert_eq!(document.models.len(), 2, "a rename must not add a row");
    assert_eq!(document.models[0]["id"], json!("a2"), "the edited row keeps slot 0");
    assert_eq!(document.models[0]["credits"], json!("x0.21"));
    assert_eq!(document.models[0]["supportsToolCall"], json!(true));
    assert_eq!(document.models[1]["id"], json!("b"));
}

/// A rename onto an id that already exists elsewhere leaves one row, not two.
#[test]
fn renaming_onto_an_existing_id_drops_the_stray_row() {
    let raw = r#"{"models":[
        {"id":"a","url":"https://x/v1/chat/completions","credits":"kept"},
        {"id":"b","url":"https://y/v1/chat/completions"}
    ]}"#;
    let mut document = ModelsDocument::parse(raw, true).expect("parse");
    document.upsert_model_renaming(
        json!({"id":"b","url":"https://x/v1/chat/completions"}),
        Some("a"),
    );

    assert_eq!(document.models.len(), 1, "one row survives a colliding rename");
    assert_eq!(document.models[0]["id"], json!("b"));
    assert_eq!(
        document.models[0]["credits"],
        json!("kept"),
        "the edited entry wins, not the one that already had the target id"
    );
}

#[test]
fn upsert_appends_an_unknown_id_and_remove_drops_it() {
    let mut document = ModelsDocument::default();
    document.upsert_model(entry("a", "https://x/v1", "k"));
    document.upsert_model(json!({"id":"", "name":"no id"}));
    assert_eq!(document.models.len(), 1, "an entry without an id is ignored");

    assert!(document.remove_model("a"));
    assert!(!document.remove_model("a"));
    assert!(document.models.is_empty());
}

#[test]
fn channel_grouping_ignores_a_trailing_chat_completions_suffix() {
    let raw = r#"{"models":[
        {"id":"a","url":"https://x/v1/chat/completions","apiKey":"k"},
        {"id":"b","url":"https://x/v1","apiKey":"k"},
        {"id":"c","url":"https://x/v1/chat/completions","apiKey":"other"}
    ]}"#;
    let document = ModelsDocument::parse(raw, true).expect("parse");
    let members = document.models_for_channel("https://x/v1", "k");
    assert_eq!(members.len(), 2);
    assert_eq!(members[0]["id"], json!("a"));
    assert_eq!(members[1]["id"], json!("b"));
}

#[test]
fn to_chat_completions_url_normalizes_like_the_product() {
    assert_eq!(
        to_chat_completions_url("https://x/v1"),
        "https://x/v1/chat/completions"
    );
    assert_eq!(
        to_chat_completions_url("https://x/v1/"),
        "https://x/v1/chat/completions"
    );
    assert_eq!(
        to_chat_completions_url("https://x/v1/chat/completions"),
        "https://x/v1/chat/completions"
    );
    assert_eq!(
        to_chat_completions_url("https://x/v1/chat/completions/chat/completions"),
        "https://x/v1/chat/completions",
        "a doubled suffix collapses, matching normalizeChatCompletionsUrl"
    );
    assert_eq!(
        to_chat_completions_url("https://x/v1?trace=1"),
        "https://x/v1/chat/completions?trace=1",
        "the query string is preserved"
    );
    assert_eq!(to_chat_completions_url("   "), "");
}

#[test]
fn build_model_entry_writes_the_fields_the_products_validate() {
    let supported_efforts = vec!["low".to_string(), "high".to_string()];
    let built = build_model_entry(&ModelEntryInput {
        id: "a",
        name: Some("  Alpha  "),
        vendor: Some("Acme"),
        base_url: "https://x/v1",
        api_key: Some("sk-1"),
        max_input_tokens: Some(200_000),
        max_output_tokens: Some(8192),
        temperature: Some(1.0),
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: true,
        only_reasoning: false,
        use_custom_protocol: false,
        can_disable_thinking: false,
        default_effort: Some("high"),
        supported_efforts: Some(&supported_efforts),
    });
    assert_eq!(built["id"], json!("a"));
    assert_eq!(built["name"], json!("Alpha"));
    assert_eq!(built["url"], json!("https://x/v1/chat/completions"));
    assert_eq!(built["maxInputTokens"], json!(200_000));
    assert_eq!(built["supportsImages"], json!(false));
    // Written even when false. Omitting them (the first version) made turning
    // either switch **off** a silent no-op: `upsert_model_renaming` deep-merges
    // onto the existing entry, so a key missing from the patch keeps its old
    // value and a `true` on disk could never be cleared from the UI.
    assert_eq!(built["onlyReasoning"], json!(false));
    assert_eq!(built["useCustomProtocol"], json!(false));
    assert_eq!(
        built["reasoning"],
        json!({
            "canDisableThinking": false,
            "defaultEffort": "high",
            "supportedEfforts": ["low", "high"],
        })
    );
}

/// Turning `supportsReasoning` off must clear the three fields this form owns,
/// but not product-owned keys on the same object.
#[test]
fn turning_reasoning_off_clears_only_the_fields_this_patch_owns() {
    let mut document = ModelsDocument::parse(
        r#"[{"id":"a","url":"https://x/v1/chat/completions","supportsReasoning":true,
            "reasoning":{"supportedEfforts":["low","high"],"defaultEffort":"high","summary":"auto"}}]"#,
        false,
    )
    .expect("parse");

    document.upsert_model(build_model_entry(&ModelEntryInput {
        id: "a",
        name: None,
        vendor: None,
        base_url: "https://x/v1",
        api_key: None,
        max_input_tokens: None,
        max_output_tokens: None,
        temperature: None,
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: false,
        only_reasoning: false,
        use_custom_protocol: false,
        can_disable_thinking: true,
        default_effort: None,
        supported_efforts: None,
    }));

    assert_eq!(document.models[0]["supportsReasoning"], json!(false));
    assert_eq!(
        document.models[0]["reasoning"],
        json!({"summary": "auto"}),
        "only the fields this patch owns are removed; product keys stay"
    );
}

/// A brand-new entry must not receive the `null` merge directives: those mean
/// "delete this key" and only make sense against an existing entry. Without
/// pruning, a new model would carry `reasoning.supportedEfforts: null` on disk.
#[test]
fn a_new_entry_does_not_store_null_merge_directives() {
    let minimal_input = |supports_reasoning: bool| ModelEntryInput {
        id: "fresh",
        name: None,
        vendor: None,
        base_url: "https://x/v1",
        api_key: None,
        max_input_tokens: None,
        max_output_tokens: None,
        temperature: None,
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning,
        only_reasoning: false,
        use_custom_protocol: false,
        can_disable_thinking: true,
        default_effort: None,
        supported_efforts: None,
    };

    let mut with_reasoning = ModelsDocument::parse("[]", false).expect("parse");
    with_reasoning.upsert_model(build_model_entry(&minimal_input(true)));
    assert!(
        with_reasoning.models[0].get("reasoning").is_none(),
        "an all-default reasoning patch must not write null fields or an empty object"
    );

    let mut without_reasoning = ModelsDocument::parse("[]", false).expect("parse");
    without_reasoning.upsert_model(build_model_entry(&minimal_input(false)));
    assert!(
        without_reasoning.models[0].get("reasoning").is_none(),
        "a disabled reasoning switch must not write `reasoning` into a new entry"
    );
}

/// Clearing the effort list removes those keys but keeps product-owned keys
/// inside the same object (`summary` here).
/// A default effort outside the supported list is contradictory; drop it
/// instead of writing an entry WorkBuddy cannot honor.
#[test]
fn a_default_effort_outside_the_supported_list_is_not_written() {
    let supported_efforts = vec!["low".to_string()];
    let mut document = ModelsDocument::parse("[]", false).expect("parse");
    document.upsert_model(build_model_entry(&ModelEntryInput {
        id: "a",
        name: None,
        vendor: None,
        base_url: "https://x/v1",
        api_key: None,
        max_input_tokens: None,
        max_output_tokens: None,
        temperature: None,
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: true,
        only_reasoning: false,
        use_custom_protocol: false,
        can_disable_thinking: true,
        default_effort: Some("max"),
        supported_efforts: Some(&supported_efforts),
    }));

    assert_eq!(
        document.models[0]["reasoning"],
        json!({"supportedEfforts": ["low"]}),
        "the unsupported default must be omitted, leaving the effort menu intact"
    );
}

#[test]
fn clearing_efforts_removes_only_the_keys_this_patch_owns() {
    let mut document = ModelsDocument::parse(
        r#"[{"id":"a","url":"https://x/v1/chat/completions","supportsReasoning":true,
            "reasoning":{"supportedEfforts":["low","high"],"defaultEffort":"high",
            "canDisableThinking":false,"summary":"auto"}}]"#,
        false,
    )
    .expect("parse");

    document.upsert_model(build_model_entry(&ModelEntryInput {
        id: "a",
        name: None,
        vendor: None,
        base_url: "https://x/v1",
        api_key: None,
        max_input_tokens: None,
        max_output_tokens: None,
        temperature: None,
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: true,
        only_reasoning: false,
        use_custom_protocol: false,
        can_disable_thinking: true,
        default_effort: None,
        supported_efforts: None,
    }));

    assert_eq!(
        document.models[0]["reasoning"],
        json!({"summary": "auto"}),
        "cleared efforts/default/disable flags must not drop a product-owned key"
    );
}

/// The other half of the rule above: an edit must be able to flip `true` back
/// to `false` on an entry that already carries the key.
#[test]
fn a_second_edit_can_clear_a_previously_enabled_flag() {
    let mut document = ModelsDocument::parse("[]", false).expect("empty array parses");
    let input = |only_reasoning: bool, use_custom_protocol: bool| {
        build_model_entry(&ModelEntryInput {
            id: "a",
            name: Some("Alpha"),
            vendor: None,
            base_url: "https://x/v1",
            api_key: Some("sk-1"),
            max_input_tokens: None,
            max_output_tokens: None,
            temperature: None,
            supports_tool_call: true,
            supports_images: false,
            supports_reasoning: false,
            only_reasoning,
            use_custom_protocol,
            can_disable_thinking: true,
            default_effort: None,
            supported_efforts: None,
        })
    };

    document.upsert_model_renaming(input(true, true), None);
    assert_eq!(document.models[0]["onlyReasoning"], json!(true));
    assert_eq!(document.models[0]["useCustomProtocol"], json!(true));

    document.upsert_model_renaming(input(false, false), None);
    assert_eq!(
        document.models[0]["onlyReasoning"],
        json!(false),
        "turning the switch off must reach the file"
    );
    assert_eq!(document.models[0]["useCustomProtocol"], json!(false));
}

#[test]
fn tolerant_parsing_accepts_the_shapes_the_desktop_loader_accepts() {
    let with_comments = r#"{
        // the app strips these before parsing
        "models": [
            {"id":"a","url":"https://x/v1/chat/completions","apiKey":"k"}, /* trailing */
        ],
    }"#;
    let document = ModelsDocument::parse(with_comments, true).expect("commented JSON parses");
    assert_eq!(document.models.len(), 1);

    let with_bom = "\u{feff}{\"models\":[]}";
    assert!(ModelsDocument::parse(with_bom, true).is_ok());
}

#[test]
fn a_malformed_file_reports_an_error_instead_of_silently_losing_models() {
    let error = ModelsDocument::parse("{ not json", true).unwrap_err();
    assert!(error.contains("not valid JSON"), "got: {error}");
}

#[test]
fn round_trip_through_disk_preserves_shape_and_extras() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(
        root.join(MODELS_FILE_NAME),
        r#"{"models":[{"id":"a","url":"https://x/v1/chat/completions","apiKey":"k","credits":"x1"}],"availableModels":["a"],"extra":{"keep":true}}"#,
    )
    .expect("seed");

    let mut document = ModelsDocument::load(root).expect("load");
    document.upsert_model(json!({"id":"b","url":"https://x/v1/chat/completions","apiKey":"k"}));
    document.save(root).expect("save");

    let reread = ModelsDocument::load(root).expect("reload");
    assert_eq!(reread.models.len(), 2);
    assert_eq!(reread.to_value()["availableModels"], json!(["a"]));
    assert_eq!(reread.to_value()["extra"], json!({"keep":true}));
    assert_eq!(reread.models[0]["credits"], json!("x1"));
}

#[test]
fn a_missing_file_is_an_empty_document_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let document = ModelsDocument::load(dir.path()).expect("missing file is fine");
    assert!(!document.existed);
    assert!(document.models.is_empty());
    assert_eq!(
        document.to_value(),
        json!({"models": []}),
        "the default shape is an object so the first write matches the product's own file"
    );
}
