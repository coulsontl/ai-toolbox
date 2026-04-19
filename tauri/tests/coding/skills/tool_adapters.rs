use std::collections::HashSet;

use ai_toolbox_lib::coding::skills::tool_adapters::{
    adapter_by_key, default_tool_adapters, runtime_adapter_by_key, CustomTool,
};
use ai_toolbox_lib::coding::tools::BUILTIN_TOOLS;

#[test]
fn default_tool_adapters_cover_all_builtin_skill_tools() {
    let actual_keys: HashSet<&'static str> = default_tool_adapters()
        .into_iter()
        .map(|adapter| adapter.key)
        .collect();
    let expected_keys: HashSet<&'static str> = BUILTIN_TOOLS
        .iter()
        .filter(|tool| tool.relative_skills_dir.is_some())
        .map(|tool| tool.key)
        .collect();

    assert_eq!(actual_keys, expected_keys);
}

#[test]
fn adapter_by_key_returns_qoder_variants() {
    let qoder = adapter_by_key("qoder").expect("qoder should be available in skills adapters");
    assert_eq!(qoder.display_name, "Qoder");
    assert_eq!(qoder.relative_skills_dir, "~/.qoder/skills");

    let qoder_work =
        adapter_by_key("qoder_work").expect("qoder_work should be available in skills adapters");
    assert_eq!(qoder_work.display_name, "QoderWork");
    assert_eq!(qoder_work.relative_skills_dir, "~/.qoderwork/skills");
}

#[test]
fn runtime_adapter_by_key_prefers_builtin_tool_without_custom_entry() {
    let custom_tools: Vec<CustomTool> = Vec::new();
    let runtime_adapter = runtime_adapter_by_key("qoder_work", &custom_tools)
        .expect("qoder_work runtime adapter should resolve");

    assert_eq!(runtime_adapter.key, "qoder_work");
    assert_eq!(runtime_adapter.display_name, "QoderWork");
    assert!(!runtime_adapter.is_custom);
}
