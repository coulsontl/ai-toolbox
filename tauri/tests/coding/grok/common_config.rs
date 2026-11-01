use std::fs;
use std::sync::{LazyLock, Mutex};

use ai_toolbox_lib::coding::grok::adapter;
use ai_toolbox_lib::coding::grok::commands::write_grok_common_config_without_provider;
use ai_toolbox_lib::coding::runtime_location;
use ai_toolbox_lib::db::helpers::db_put;
use ai_toolbox_lib::db::schema::{Cli, CliConfigKind, DbTable};
use ai_toolbox_lib::db::sqlite_state::SqliteDbState;
use tempfile::TempDir;

/// Serialize tests touching the Grok runtime location or environment.
pub(crate) static GROK_TEST_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime")
        .block_on(future)
}

/// Test environment whose Grok `config.toml` lives in a temp root dir.
fn setup_test_env() -> (TempDir, SqliteDbState) {
    let temp_dir = TempDir::new().expect("tempdir");
    let state = SqliteDbState::in_memory_for_test().expect("sqlite state");

    let common_val = adapter::common_to_db_value("", Some(temp_dir.path().to_str().unwrap()));
    state
        .with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::Grok, CliConfigKind::Common), "common", &common_val))
        .expect("db_put common config");

    block_on(async {
        runtime_location::refresh_runtime_location_cache_for_module_async(&state, "grok")
            .await
            .expect("refresh cache");
    });

    (temp_dir, state)
}

/// The common-config modal only ever stores and sends the slice that excludes
/// the protected sections, and Grok keeps its MCP servers (`[mcp_servers.*]`,
/// written by the MCP page), plugins and marketplace in this same file. Saving
/// without an applied provider used to write that slice as the whole file,
/// which silently deleted all of them.
#[test]
fn common_config_without_provider_keeps_sections_owned_by_other_surfaces() {
    let _guard = GROK_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"# User comment stays put
[mcp_servers.demo]
command = "npx"

[plugins]
enabled = true

[marketplace]
url = "https://marketplace.example.test"

[env]
KEEP = "old"
"#,
    )
    .expect("write initial config.toml");

    block_on(write_grok_common_config_without_provider(
        &state,
        Some("[env]\nKEEP = \"old\"\n"),
        "[env]\nKEEP = \"new\"\nADDED = true\n",
    ))
    .expect("write common config without provider");

    let written = fs::read_to_string(&config_path).expect("read config.toml");
    assert!(
        written.contains("[mcp_servers.demo]"),
        "MCP servers written by the MCP page must survive: {written}"
    );
    assert!(
        written.contains("command = \"npx\""),
        "MCP server body must survive: {written}"
    );
    assert!(
        written.contains("[plugins]"),
        "plugin sections must survive: {written}"
    );
    assert!(
        written.contains("[marketplace]"),
        "marketplace must survive: {written}"
    );
    assert!(
        written.contains("KEEP = \"new\""),
        "edited common keys must be applied: {written}"
    );
    assert!(
        written.contains("ADDED = true"),
        "new common keys must be merged: {written}"
    );
    assert!(
        !written.contains("KEEP = \"old\""),
        "previous common keys must be replaced: {written}"
    );
    assert!(
        written.contains("# User comment stays put"),
        "untouched file content must keep its formatting: {written}"
    );
}

/// First save on a machine without a stored common config, i.e. no previous
/// blob to remove keys for.
#[test]
fn first_common_config_save_without_provider_keeps_mcp_servers() {
    let _guard = GROK_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"[mcp_servers.demo]
url = "https://mcp.example.test/mcp"
"#,
    )
    .expect("write initial config.toml");

    block_on(write_grok_common_config_without_provider(
        &state,
        None,
        "telemetry = { trace_upload = false }\n",
    ))
    .expect("write first common config without provider");

    let written = fs::read_to_string(&config_path).expect("read config.toml");
    assert!(
        written.contains("[mcp_servers.demo]"),
        "a first common-config save must not drop MCP servers: {written}"
    );
    assert!(
        written.contains("trace_upload = false"),
        "the new common config must be merged: {written}"
    );
}

/// Clearing the common config must remove what it previously owned while still
/// leaving the MCP section alone.
#[test]
fn clearing_common_config_without_provider_keeps_mcp_servers() {
    let _guard = GROK_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (temp_dir, state) = setup_test_env();

    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"[mcp_servers.demo]
command = "npx"

[telemetry]
trace_upload = false
"#,
    )
    .expect("write initial config.toml");

    block_on(write_grok_common_config_without_provider(
        &state,
        Some("telemetry = { trace_upload = false }\n"),
        "",
    ))
    .expect("clear common config without provider");

    let written = fs::read_to_string(&config_path).expect("read config.toml");
    assert!(
        written.contains("[mcp_servers.demo]"),
        "clearing the common config must not drop MCP servers: {written}"
    );
    assert!(
        !written.contains("trace_upload"),
        "cleared common keys must be removed: {written}"
    );
}
