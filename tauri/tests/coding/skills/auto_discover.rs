//! End-to-end coverage for the auto-discover pass (issue #446).
//!
//! These run the real pass against a mock Tauri app and a temporary central
//! directory, so they cover the user-visible contract rather than internals:
//! newly discovered Skills get adopted and distributed, existing bindings are
//! never resurrected, and an unmanaged same-named tool directory survives.

use std::path::Path;

use ai_toolbox_lib::coding::skills::auto_discover::{normalize_custom_dir, run_pass};
use ai_toolbox_lib::coding::skills::skill_store;
use ai_toolbox_lib::coding::skills::tool_adapters::CustomTool;
use ai_toolbox_lib::db::SqliteDbState;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::Manager;

fn create_app() -> tauri::App<MockRuntime> {
    mock_builder()
        .manage(SqliteDbState::in_memory_for_test().expect("create test database"))
        .build(mock_context(noop_assets()))
        .expect("build mock app")
}

fn write_skill(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("create skill dir");
    std::fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\n---\n\nbody\n"))
        .expect("write SKILL.md");
}

async fn add_tool(state: &SqliteDbState, key: &str, skills_dir: &Path) {
    skill_store::save_custom_tool(
        state,
        &CustomTool {
            key: key.to_string(),
            display_name: key.to_string(),
            relative_skills_dir: skills_dir.to_string_lossy().to_string(),
            relative_detect_dir: skills_dir.to_string_lossy().to_string(),
            created_at: 0,
            force_copy: true,
            icon_url: None,
        },
    )
    .await
    .expect("save custom tool");
}

async fn point_central_repo_at(state: &SqliteDbState, dir: &Path) {
    ai_toolbox_lib::coding::skills::central_repo::save_central_repo_path(state, dir)
        .await
        .expect("save central repo path");
}

fn custom_source_id(dir: &Path) -> String {
    let normalized = dir.to_string_lossy().replace(char::from(92), "/");
    format!("custom:{normalized}")
}

async fn enable_sources(state: &SqliteDbState, sources: &[String], targets: Vec<String>) {
    skill_store::set_setting(
        state,
        "auto_sync_sources",
        &serde_json::to_string(sources).expect("serialize sources"),
    )
    .await
    .expect("save sources");
    skill_store::set_setting(
        state,
        "auto_sync_target_tools",
        &serde_json::to_string(&targets).expect("serialize targets"),
    )
    .await
    .expect("save targets");
}

#[tokio::test]
async fn pass_adopts_new_skills_and_distributes_to_target_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let source = temp.path().join("source");
    let tool_dir = temp.path().join("tool-skills");
    std::fs::create_dir_all(&central).expect("create central");
    std::fs::create_dir_all(&source).expect("create source");
    write_skill(&source, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    add_tool(state.inner(), "probe", &tool_dir).await;
    enable_sources(
        state.inner(),
        &[custom_source_id(&source)],
        vec!["probe".to_string()],
    )
    .await;
    // Register the custom source through preferences.
    skill_store::set_setting(
        state.inner(),
        "auto_sync_custom_dirs",
        &serde_json::to_string(&[source.to_string_lossy().to_string()]).expect("serialize dirs"),
    )
    .await
    .expect("save custom dirs");

    let result = run_pass(app.handle(), state.inner(), false)
        .await
        .expect("run pass");

    assert_eq!(result.adopted, 1, "one new Skill should be adopted");
    assert_eq!(result.synced_targets, 1, "one target should be synced");
    assert!(tool_dir.join("alpha").exists(), "Skill must be distributed");

    // The copy source keeps the original directory as source_ref.
    let skill = skill_store::get_skill_by_name(state.inner(), "alpha")
        .await
        .expect("query skill")
        .expect("skill adopted");
    assert_eq!(skill.source_type, "local");
    assert_eq!(
        skill.source_ref.as_deref(),
        Some(source.join("alpha").to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn second_pass_is_idempotent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let source = temp.path().join("source");
    let tool_dir = temp.path().join("tool-skills");
    std::fs::create_dir_all(&central).expect("create central");
    std::fs::create_dir_all(&source).expect("create source");
    write_skill(&source, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    add_tool(state.inner(), "probe", &tool_dir).await;
    enable_sources(
        state.inner(),
        &[custom_source_id(&source)],
        vec!["probe".to_string()],
    )
    .await;
    skill_store::set_setting(
        state.inner(),
        "auto_sync_custom_dirs",
        &serde_json::to_string(&[source.to_string_lossy().to_string()]).expect("serialize dirs"),
    )
    .await
    .expect("save custom dirs");

    let first = run_pass(app.handle(), state.inner(), false)
        .await
        .expect("first pass");
    let second = run_pass(app.handle(), state.inner(), false)
        .await
        .expect("second pass");

    assert_eq!(first.adopted, 1);
    assert_eq!(second.adopted, 0, "a settled source adopts nothing new");
    assert_eq!(second.refreshed, 0, "unchanged content must not refresh");
}

/// A Skill copied into the central repo is visible from the central source as
/// well. The central directory must not claim ownership of it, otherwise the
/// copy looks "changed" on the next tick and triggers an endless re-sync.
#[tokio::test]
async fn copied_skills_are_not_owned_by_the_central_source() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let source = temp.path().join("source");
    let tool_dir = temp.path().join("tool-skills");
    std::fs::create_dir_all(&central).expect("create central");
    std::fs::create_dir_all(&source).expect("create source");
    write_skill(&source, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    add_tool(state.inner(), "probe", &tool_dir).await;
    enable_sources(
        state.inner(),
        &[custom_source_id(&source)],
        vec!["probe".to_string()],
    )
    .await;
    skill_store::set_setting(
        state.inner(),
        "auto_sync_custom_dirs",
        &serde_json::to_string(&[source.to_string_lossy().to_string()]).expect("serialize dirs"),
    )
    .await
    .expect("save custom dirs");

    // Sanity: the copy really landed in the central repo, so the central source
    // will find it on the next pass.
    run_pass(app.handle(), state.inner(), false)
        .await
        .expect("first pass");
    assert!(central.join("alpha").is_dir(), "copy landed in central repo");

    for _ in 0..3 {
        let repeat = run_pass(app.handle(), state.inner(), false)
            .await
            .expect("repeat pass");
        assert_eq!(
            repeat.refreshed, 0,
            "a copied Skill must not be refreshed by the central source"
        );
    }
}

/// A never-configured install distributes to the cross-tool directory only.
/// Falling back to "every installed tool" would make a background pass write
/// into CLIs the user never pointed it at.
#[tokio::test]
async fn unconfigured_targets_default_to_the_universal_directory_only() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let source = temp.path().join("source");
    let probe_dir = temp.path().join("probe-skills");
    std::fs::create_dir_all(&central).expect("create central");
    std::fs::create_dir_all(&source).expect("create source");
    write_skill(&source, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    // A second installed tool that must NOT receive the Skill by default.
    add_tool(state.inner(), "probe", &probe_dir).await;
    skill_store::set_setting(
        state.inner(),
        "auto_sync_sources",
        &serde_json::to_string(&[custom_source_id(&source)]).expect("serialize sources"),
    )
    .await
    .expect("save sources");
    skill_store::set_setting(
        state.inner(),
        "auto_sync_custom_dirs",
        &serde_json::to_string(&[source.to_string_lossy().to_string()]).expect("serialize dirs"),
    )
    .await
    .expect("save custom dirs");

    let result = run_pass(app.handle(), state.inner(), false)
        .await
        .expect("run pass");

    assert_eq!(result.adopted, 1, "the Skill is adopted");
    assert!(
        !probe_dir.join("alpha").exists(),
        "an unconfigured target list must not spread into every installed tool"
    );

    let skill = skill_store::get_skill_by_name(state.inner(), "alpha")
        .await
        .expect("query skill")
        .expect("skill adopted");
    assert!(
        skill
            .enabled_tools
            .iter()
            .all(|tool| tool == ai_toolbox_lib::coding::skills::auto_discover::DEFAULT_TARGET_TOOL),
        "only the default target is bound, got {:?}",
        skill.enabled_tools
    );
}

/// The dialog shows an expanded absolute path, while custom dirs are stored in
/// portable `~/...` form. Removal must normalize the input first, otherwise the
/// ids never match and the button silently does nothing.
#[test]
fn custom_dir_id_matches_the_stored_form() {
    let home = dirs::home_dir().expect("home dir");
    let absolute = home.join("ai-toolbox-probe-skills");
    let stored = normalize_custom_dir(&absolute.to_string_lossy()).expect("normalize absolute");
    assert_eq!(stored, "~/ai-toolbox-probe-skills");

    // The same directory arriving in stored form must normalize to itself.
    let again = normalize_custom_dir(&stored).expect("normalize stored");
    assert_eq!(again, stored);
}

#[tokio::test]
async fn existing_tool_bindings_are_not_resurrected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let source = temp.path().join("source");
    let tool_dir = temp.path().join("tool-skills");
    std::fs::create_dir_all(&central).expect("create central");
    std::fs::create_dir_all(&source).expect("create source");
    write_skill(&source, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    add_tool(state.inner(), "probe", &tool_dir).await;
    enable_sources(
        state.inner(),
        &[custom_source_id(&source)],
        vec!["probe".to_string()],
    )
    .await;
    skill_store::set_setting(
        state.inner(),
        "auto_sync_custom_dirs",
        &serde_json::to_string(&[source.to_string_lossy().to_string()]).expect("serialize dirs"),
    )
    .await
    .expect("save custom dirs");

    run_pass(app.handle(), state.inner(), false)
        .await
        .expect("first pass");

    // Simulate the user un-syncing the tool by hand.
    let skill = skill_store::get_skill_by_name(state.inner(), "alpha")
        .await
        .expect("query skill")
        .expect("skill adopted");
    skill_store::delete_skill_target(state.inner(), &skill.id, "probe")
        .await
        .expect("unsync probe");
    std::fs::remove_dir_all(tool_dir.join("alpha")).expect("remove target");

    run_pass(app.handle(), state.inner(), false)
        .await
        .expect("second pass");

    assert!(
        !tool_dir.join("alpha").exists(),
        "an existing Skill must not be re-synced after the user un-synced it"
    );
}

#[tokio::test]
async fn foreign_same_named_tool_directory_is_never_overwritten() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let source = temp.path().join("source");
    let tool_dir = temp.path().join("tool-skills");
    std::fs::create_dir_all(&central).expect("create central");
    std::fs::create_dir_all(&source).expect("create source");
    std::fs::create_dir_all(tool_dir.join("alpha")).expect("create foreign target");
    std::fs::write(tool_dir.join("alpha").join("keep.txt"), "user data").expect("write foreign file");
    write_skill(&source, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    add_tool(state.inner(), "probe", &tool_dir).await;
    enable_sources(
        state.inner(),
        &[custom_source_id(&source)],
        vec!["probe".to_string()],
    )
    .await;
    skill_store::set_setting(
        state.inner(),
        "auto_sync_custom_dirs",
        &serde_json::to_string(&[source.to_string_lossy().to_string()]).expect("serialize dirs"),
    )
    .await
    .expect("save custom dirs");

    let result = run_pass(app.handle(), state.inner(), false)
        .await
        .expect("run pass");

    assert_eq!(result.adopted, 1, "the Skill is still adopted");
    assert_eq!(result.synced_targets, 0, "the foreign target is not replaced");
    assert_eq!(
        std::fs::read_to_string(tool_dir.join("alpha").join("keep.txt")).expect("read kept file"),
        "user data"
    );
    assert!(
        !result.errors.is_empty(),
        "the skipped target must be reported to the caller"
    );
}

#[tokio::test]
async fn skill_in_a_tool_source_is_not_synced_back_to_that_tool() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let tool_dir = temp.path().join("tool-skills");
    std::fs::create_dir_all(&central).expect("create central");
    write_skill(&tool_dir, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    add_tool(state.inner(), "probe", &tool_dir).await;
    enable_sources(state.inner(), &["tool:probe".to_string()], vec!["probe".to_string()]).await;

    let result = run_pass(app.handle(), state.inner(), false)
        .await
        .expect("run pass");

    assert_eq!(result.adopted, 1, "the tool source is adopted");
    assert_eq!(
        result.synced_targets, 0,
        "a Skill imported from a tool folder is never synced back into it"
    );
    assert!(tool_dir.join("alpha").join("SKILL.md").is_file());
}

#[tokio::test]
async fn disabled_skills_are_not_refreshed_or_distributed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let central = temp.path().join("central");
    let source = temp.path().join("source");
    let tool_dir = temp.path().join("tool-skills");
    std::fs::create_dir_all(&central).expect("create central");
    std::fs::create_dir_all(&source).expect("create source");
    write_skill(&source, "alpha");

    let app = create_app();
    let state = app.state::<SqliteDbState>();
    point_central_repo_at(state.inner(), &central).await;
    add_tool(state.inner(), "probe", &tool_dir).await;
    enable_sources(
        state.inner(),
        &[custom_source_id(&source)],
        vec!["probe".to_string()],
    )
    .await;
    skill_store::set_setting(
        state.inner(),
        "auto_sync_custom_dirs",
        &serde_json::to_string(&[source.to_string_lossy().to_string()]).expect("serialize dirs"),
    )
    .await
    .expect("save custom dirs");

    run_pass(app.handle(), state.inner(), false)
        .await
        .expect("first pass");

    let skill = skill_store::get_skill_by_name(state.inner(), "alpha")
        .await
        .expect("query skill")
        .expect("skill adopted");
    let mut disabled = skill.clone();
    disabled.management_enabled = false;
    disabled.enabled_tools.clear();
    disabled.sync_details = Some(serde_json::json!({}));
    skill_store::upsert_skill(state.inner(), &disabled)
        .await
        .expect("disable skill");
    std::fs::remove_dir_all(tool_dir.join("alpha")).expect("remove target");

    // Change the content so a fingerprint-based refresh would fire if the
    // disabled state were ignored.
    write_skill(&source, "alpha");

    let result = run_pass(app.handle(), state.inner(), false)
        .await
        .expect("second pass");

    assert_eq!(result.refreshed, 0, "disabled Skills are never refreshed");
    assert!(
        !tool_dir.join("alpha").exists(),
        "disabled Skills are never distributed"
    );
}

