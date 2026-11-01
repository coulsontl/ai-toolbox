//! Custom-root-dir resolution for the MCP and Skills pages.
//!
//! `detection.rs` used to gate these behind a per-CLI whitelist, so a module it
//! did not name silently kept reading its *default* path after the user moved
//! the root elsewhere — no error, no log, just the wrong file. The whitelist is
//! gone; these tests pin the behaviour that replaced it, because nothing caught
//! the original bug for several rounds.

use ai_toolbox_lib::coding::runtime_location;
use ai_toolbox_lib::coding::tools::detection::{
    resolve_mcp_config_path_with_db, resolve_skills_path_with_db,
};
use ai_toolbox_lib::coding::tools::types::RuntimeTool;
use ai_toolbox_lib::db::helpers::db_put;
use ai_toolbox_lib::db::schema::{Cli, CliConfigKind, DbTable};
use ai_toolbox_lib::db::sqlite_state::SqliteDbState;
use serde_json::json;
use tempfile::TempDir;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime")
        .block_on(future)
}

/// The minimum `RuntimeTool` shape the path resolvers look at: a key that
/// `runtime_location` knows, no per-tool path override.
fn zcode_runtime_tool() -> RuntimeTool {
    RuntimeTool {
        key: "zcode".to_string(),
        display_name: "ZCode".to_string(),
        is_custom: false,
        relative_skills_dir: None,
        relative_detect_dir: None,
        force_copy: false,
        icon_url: None,
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    }
}

fn store_custom_root_dir(db: &SqliteDbState, root: &str) {
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CliConfig(Cli::Zcode, CliConfigKind::Common),
            "common",
            &json!({
                "config": "",
                "root_dir": root,
                "updated_at": "2026-10-06T00:00:00Z",
            }),
        )
    })
    .expect("store the custom root dir");
}

/// Both entry points must agree, and both must follow the custom root.
///
/// One test rather than two: the sync resolver reads a **process-global**
/// runtime-location cache, so two tests running in parallel would each refresh
/// it to their own temp dir and read back the other's value. Serializing here
/// keeps the assertion honest.
#[test]
fn a_custom_root_dir_reaches_both_the_mcp_and_the_skills_path() {
    let db = SqliteDbState::in_memory_for_test().expect("sqlite state");
    let custom_root = TempDir::new().expect("temp root");
    store_custom_root_dir(&db, &custom_root.path().to_string_lossy());
    block_on(async {
        runtime_location::refresh_runtime_location_cache_for_module_async(&db, "zcode")
            .await
            .expect("refresh the zcode location cache");
    });

    let tool = zcode_runtime_tool();
    let expected_mcp = custom_root.path().join("cli").join("config.json");
    let expected_skills = custom_root.path().join("skills");

    // ZCode reads MCP servers from `cli/config.json` and its skills from
    // `<root>/skills`, so both must land under the custom root.
    assert_eq!(
        resolve_mcp_config_path_with_db(&db, &tool),
        Some(expected_mcp.clone()),
    );
    assert_eq!(
        resolve_skills_path_with_db(&db, &tool),
        Some(expected_skills.clone()),
    );

    // The Tauri commands use the async chain; it must reach the same places.
    let (mcp_path, skills_path) = block_on(async {
        (
            runtime_location::get_tool_mcp_config_path_async(&db, &tool.key).await,
            runtime_location::get_tool_skills_path_async(&db, &tool.key).await,
        )
    });
    assert_eq!(mcp_path, Some(expected_mcp));
    assert_eq!(skills_path, Some(expected_skills));
}