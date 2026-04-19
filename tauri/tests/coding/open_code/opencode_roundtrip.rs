use ai_toolbox_lib::coding::open_code::types::OpenCodeConfig;

fn assert_roundtrip_fixture(fixture_name: &str, fixture_content: &str) {
    let parsed_config = json5::from_str::<OpenCodeConfig>(fixture_content)
        .unwrap_or_else(|error| panic!("failed to parse fixture {fixture_name}: {error}"));
    let serialized_config = serde_json::to_string_pretty(&parsed_config)
        .unwrap_or_else(|error| panic!("failed to serialize fixture {fixture_name}: {error}"));

    let original_value = json5::from_str::<serde_json::Value>(fixture_content)
        .unwrap_or_else(|error| panic!("failed to parse fixture value {fixture_name}: {error}"));
    let roundtripped_value = serde_json::from_str::<serde_json::Value>(&serialized_config)
        .unwrap_or_else(|error| {
            panic!("failed to parse serialized fixture {fixture_name}: {error}")
        });

    assert_eq!(
        roundtripped_value, original_value,
        "fixture {fixture_name} changed shape during roundtrip"
    );
}

#[test]
fn opencode_roundtrip_preserves_plugin_tuple_shape() {
    assert_roundtrip_fixture(
        "plugin_tuple.json5",
        include_str!("fixtures/plugin_tuple.json5"),
    );
}

#[test]
fn opencode_roundtrip_preserves_provider_and_model_extra_fields() {
    assert_roundtrip_fixture(
        "provider_model_extras.json5",
        include_str!("fixtures/provider_model_extras.json5"),
    );
}

#[test]
fn opencode_roundtrip_preserves_mcp_and_other_polymorphic_fields() {
    assert_roundtrip_fixture(
        "mcp_polymorphic.json5",
        include_str!("fixtures/mcp_polymorphic.json5"),
    );
}
