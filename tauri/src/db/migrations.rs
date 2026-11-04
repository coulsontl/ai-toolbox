use rusqlite::Connection;

use super::schema::{sql_string_literal, JsonFieldPath};

pub const TARGET_SCHEMA_VERSION: i32 = 34;
const FUTURE_SCHEMA_ERROR_PREFIX: &str = "AI_TOOLBOX_SQLITE_SCHEMA_TOO_NEW";

// ---- the consolidation table ------------------------------------------------
//
// Every per-CLI table that v33 folds into one of the three shared tables, as
// (old table, cli_key, kind). `kind` is only meaningful for `cli_config`; the
// `cli_key` is what the new row's scope column is set to.
//
// This list is written out rather than derived so that the mapping is readable
// and reviewable: guessing a cli_key from a table name (`gemini_cli_*` →
// `gemini`, `oh_my_pi_*` → `oh_my_pi`) is exactly the kind of mistake that
// silently files a CLI's presets under the wrong scope.
const CLI_PROVIDER_TABLES: &[(&str, &str)] = &[
    ("claude_provider", "claude"),
    ("claude_desktop_provider", "claude_desktop"),
    ("codex_provider", "codex"),
    ("grok_provider", "grok"),
    ("kimi_provider", "kimi"),
    ("gemini_cli_provider", "gemini"),
    ("antigravity_provider", "antigravity"),
    ("zcode_provider", "zcode"),
    ("workbuddy_provider", "workbuddy"),
    ("codebuddy_provider", "codebuddy"),
];

const CLI_OFFICIAL_ACCOUNT_TABLES: &[(&str, &str)] = &[
    ("codex_official_account", "codex"),
    ("grok_official_account", "grok"),
    ("kimi_official_account", "kimi"),
    ("gemini_cli_official_account", "gemini"),
    ("antigravity_official_account", "antigravity"),
    ("zcode_official_account", "zcode"),
];

const CLI_CONFIG_TABLES: &[(&str, &str, &str)] = &[
    // common
    ("claude_common_config", "claude", "common"),
    ("codex_common_config", "codex", "common"),
    ("grok_common_config", "grok", "common"),
    ("kimi_common_config", "kimi", "common"),
    ("gemini_cli_common_config", "gemini", "common"),
    ("antigravity_common_config", "antigravity", "common"),
    ("zcode_common_config", "zcode", "common"),
    ("opencode_common_config", "opencode", "common"),
    ("openclaw_common_config", "openclaw", "common"),
    // prompt
    ("claude_prompt_config", "claude", "prompt"),
    ("codex_prompt_config", "codex", "prompt"),
    ("grok_prompt_config", "grok", "prompt"),
    ("kimi_prompt_config", "kimi", "prompt"),
    ("gemini_cli_prompt_config", "gemini", "prompt"),
    ("antigravity_prompt_config", "antigravity", "prompt"),
    ("zcode_prompt_config", "zcode", "prompt"),
    ("claude_desktop_prompt_config", "claude_desktop", "prompt"),
    ("codebuddy_prompt_config", "codebuddy", "prompt"),
    ("hermes_prompt_config", "hermes", "prompt"),
    ("dsh_prompt_config", "dsh", "prompt"),
    ("pi_prompt_config", "pi", "prompt"),
    ("oh_my_pi_prompt_config", "oh_my_pi", "prompt"),
    ("omo_native_prompt_config", "omo_native", "prompt"),
    ("opencode_prompt_config", "opencode", "prompt"),
    // settings
    ("workbuddy_settings_config", "workbuddy", "settings"),
    ("codebuddy_settings_config", "codebuddy", "settings"),
    ("hermes_settings_config", "hermes", "settings"),
    ("dsh_settings_config", "dsh", "settings"),
    ("pi_settings_config", "pi", "settings"),
    ("oh_my_pi_settings_config", "oh_my_pi", "settings"),
    ("omo_native_settings_config", "omo_native", "settings"),
    // agents: whole user-authored profiles, several rows per CLI
    ("oh_my_pi_agents_config", "oh_my_pi", "agents"),
    ("omo_native_agents_config", "omo_native", "agents"),
    ("oh_my_openagent_config", "oh_my_openagent", "agents"),
    ("oh_my_opencode_slim_config", "oh_my_opencode_slim", "agents"),
    // global: one singleton row per CLI, id `global`
    ("oh_my_openagent_global_config", "oh_my_openagent", "global"),
    ("oh_my_opencode_slim_global_config", "oh_my_opencode_slim", "global"),
];

/// The nine `api2api_*` tables that were split by entity before the rename.
///
/// They only ever existed on this branch and were never released, so v33 is
/// free to fold them into `relay_entity` in the same step instead of carrying a
/// separate legacy shape. `quota_*` is absent because those two tables were
/// created by v33 itself, never by a released version.
const RELAY_LEGACY_TABLES: &[(&str, &str)] = &[
    ("api2api_channel", "channel"),
    ("api2api_model", "model"),
    ("api2api_plugin", "plugin"),
    ("api2api_plugin_provider", "plugin_provider"),
    ("api2api_plugin_account", "plugin_account"),
    ("api2api_access_key", "access_key"),
    ("api2api_checkin_schedule", "checkin_schedule"),
    ("api2api_checkin_record", "checkin_record"),
    ("relay_entity", "channel"),
];

pub fn run_all(conn: &mut Connection) -> Result<(), String> {
    let current_version = ensure_supported_user_version(conn)?;

    if current_version < 1 {
        run_migration_step(conn, 1, migrate_v1)?;
    }
    if current_version < 2 {
        run_migration_step(conn, 2, migrate_v2)?;
    }
    if current_version < 3 {
        run_migration_step(conn, 3, migrate_v3)?;
    }
    if current_version < 4 {
        run_migration_step(conn, 4, migrate_v4)?;
    }
    if current_version < 5 {
        run_migration_step(conn, 5, migrate_v5)?;
    }
    if current_version < 6 {
        run_migration_step(conn, 6, migrate_v6)?;
    }
    if current_version < 7 {
        run_migration_step(conn, 7, migrate_v7)?;
    }
    if current_version < 8 {
        run_migration_step(conn, 8, migrate_v8)?;
    }
    if current_version < 9 {
        run_migration_step(conn, 9, migrate_v9)?;
    }
    if current_version < 10 {
        run_migration_step(conn, 10, migrate_v10)?;
    }
    if current_version < 11 {
        run_migration_step(conn, 11, migrate_v11)?;
    }
    if current_version < 12 {
        run_migration_step(conn, 12, migrate_v12)?;
    }
    if current_version < 13 {
        run_migration_step(conn, 13, migrate_v13)?;
    }
    if current_version < 14 {
        run_migration_step(conn, 14, migrate_v14)?;
    }
    if current_version < 15 {
        run_migration_step(conn, 15, migrate_v15)?;
    }
    if current_version < 16 {
        run_migration_step(conn, 16, migrate_v16)?;
    }
    if current_version < 17 {
        run_migration_step(conn, 17, migrate_v17)?;
    }
    if current_version < 18 {
        run_migration_step(conn, 18, migrate_v18)?;
    }
    if current_version < 19 {
        run_migration_step(conn, 19, migrate_v19)?;
    }
    if current_version < 20 {
        run_migration_step(conn, 20, migrate_v20)?;
    }
    if current_version < 21 {
        run_migration_step(conn, 21, migrate_v21)?;
    }
    if current_version < 22 {
        run_migration_step(conn, 22, migrate_v22)?;
    }
    if current_version < 23 {
        run_migration_step(conn, 23, migrate_v23)?;
    }
    if current_version < 24 {
        run_migration_step(conn, 24, migrate_v24)?;
    }
    if current_version < 25 {
        run_migration_step(conn, 25, migrate_v25)?;
    }
    if current_version < 26 {
        run_migration_step(conn, 26, migrate_v26)?;
    }
    if current_version < 27 {
        run_migration_step(conn, 27, migrate_v27)?;
    }
    if current_version < 28 {
        run_migration_step(conn, 28, migrate_v28)?;
    }
    if current_version < 29 {
        run_migration_step(conn, 29, migrate_v29)?;
    }
    if current_version < 30 {
        run_migration_step(conn, 30, migrate_v30)?;
    }
    if current_version < 31 {
        run_migration_step(conn, 31, migrate_v31)?;
    }
    if current_version < 32 {
        run_migration_step(conn, 32, migrate_v32)?;
    }
    if current_version < 33 {
        run_migration_step(conn, 33, migrate_v33)?;
    }
    if current_version < 34 {
        run_migration_step(conn, 34, migrate_v34)?;
    }

    Ok(())
}

/// The per-CLI tables a v32 database still has, for tests that need to seed a
/// pre-consolidation fixture.
///
/// Exposed rather than duplicated in the test because the list is the exact
/// complement of [`CLI_PROVIDER_TABLES`] / [`CLI_OFFICIAL_ACCOUNT_TABLES`] /
/// [`CLI_CONFIG_TABLES`]; a test that hard-coded its own copy would keep
/// passing after a table was added to the fold and never migrated.
pub fn legacy_tables_for_test() -> Vec<&'static str> {
    CLI_PROVIDER_TABLES
        .iter()
        .map(|(table, _)| *table)
        .chain(CLI_OFFICIAL_ACCOUNT_TABLES.iter().map(|(table, _)| *table))
        .chain(CLI_CONFIG_TABLES.iter().map(|(table, _, _)| *table))
        .collect()
}

pub fn future_schema_error(current_version: i32, supported_version: i32) -> String {
    format!(
        "{FUTURE_SCHEMA_ERROR_PREFIX}: SQLite schema version {current_version} is newer than supported version {supported_version}"
    )
}

pub fn future_backup_schema_error(schema_version: i64, supported_version: i64) -> String {
    format!(
        "{FUTURE_SCHEMA_ERROR_PREFIX}: Backup SQLite schema version {schema_version} is newer than supported version {supported_version}"
    )
}

pub fn is_future_schema_error(error: &str) -> bool {
    error.contains(FUTURE_SCHEMA_ERROR_PREFIX)
}

pub fn future_schema_user_message(error: &str) -> String {
    format!(
        "当前数据已由更高版本的 AI Toolbox 打开或迁移，不能回退到旧版本继续启动。\n\n请升级到最新版本后再打开应用，或恢复使用旧版本创建的兼容备份。\n\n技术信息：{error}"
    )
}

pub fn future_backup_schema_user_message(error: &str) -> String {
    format!(
        "这个备份由更高版本的 AI Toolbox 创建，当前版本无法恢复。\n\n请升级 AI Toolbox 后再恢复，或选择旧版本创建的兼容备份。\n\n技术信息：{error}"
    )
}

pub fn ensure_supported_user_version(conn: &Connection) -> Result<i32, String> {
    let current_version = get_user_version(conn)?;
    if current_version > TARGET_SCHEMA_VERSION {
        return Err(future_schema_error(current_version, TARGET_SCHEMA_VERSION));
    }
    Ok(current_version)
}

pub fn get_user_version(conn: &Connection) -> Result<i32, String> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("Failed to read SQLite user_version: {error}"))
}

pub fn set_user_version(conn: &Connection, version: i32) -> Result<(), String> {
    conn.execute_batch(&format!("PRAGMA user_version = {version}"))
        .map_err(|error| format!("Failed to set SQLite user_version to {version}: {error}"))
}

fn run_migration_step(
    conn: &Connection,
    target_version: i32,
    migration: fn(&Connection) -> Result<(), String>,
) -> Result<(), String> {
    conn.execute_batch("SAVEPOINT ai_toolbox_schema_migration")
        .map_err(|error| format!("Failed to start schema migration savepoint: {error}"))?;

    let result = (|| {
        migration(conn)?;
        set_user_version(conn, target_version)?;
        Ok(())
    })();

    match result {
        Ok(()) => conn
            .execute_batch("RELEASE ai_toolbox_schema_migration")
            .map_err(|error| format!("Failed to release schema migration savepoint: {error}")),
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO ai_toolbox_schema_migration; RELEASE ai_toolbox_schema_migration",
            );
            Err(error)
        }
    }
}

/// The tables v1 created: the twelve per-CLI provider tables of the day plus
/// every table that still exists under the same name.
///
/// **Frozen.** v1 runs only on databases that predate the v33 consolidation, so
/// it must build the *legacy* shape (no `cli_key` / `kind` columns) and leave
/// the folding to v33. Using `ALL_TABLES` here would create `cli_provider`
/// without its scope column on a fresh database, and v33's `CREATE TABLE IF NOT
/// EXISTS` would then skip the real definition — every later read and write
/// would fail on the missing column.
fn migrate_v1(conn: &Connection) -> Result<(), String> {
    for table in [
        "settings",
        "app_migration",
        "claude_provider",
        "claude_common_config",
        "claude_prompt_config",
        "codex_provider",
        "codex_common_config",
        "codex_prompt_config",
        "codex_official_account",
        "codex_plugin_workspace_roots",
        "gemini_cli_provider",
        "gemini_cli_common_config",
        "gemini_cli_prompt_config",
        "gemini_cli_official_account",
        "antigravity_provider",
        "antigravity_common_config",
        "antigravity_prompt_config",
        "antigravity_official_account",
        "skill",
        "skill_group",
        "skill_repo",
        "skill_preferences",
        "skill_settings",
        "custom_tool",
        "mcp_server",
        "mcp_preferences",
        "favorite_mcp",
        "wsl_sync_config",
        "wsl_file_mapping",
        "ssh_sync_config",
        "ssh_connection",
        "ssh_file_mapping",
        "proxy_gateway_settings",
        "gateway_session_usage_state",
        "image_channel",
        "image_job",
        "image_asset",
        "opencode_common_config",
        "opencode_prompt_config",
        "opencode_favorite_plugin",
        "opencode_favorite_provider",
        "openclaw_common_config",
        "oh_my_openagent_config",
        "oh_my_openagent_global_config",
        "oh_my_opencode_slim_config",
        "oh_my_opencode_slim_global_config",
    ] {
        create_jsonb_table(conn, table)?;
    }

    create_initial_indexes(conn)?;
    Ok(())
}

fn migrate_v2(conn: &Connection) -> Result<(), String> {
    create_proxy_gateway_usage_tables(conn)
}

fn migrate_v3(conn: &Connection) -> Result<(), String> {
    create_model_pricing_table(conn)?;
    seed_model_pricing(conn)
}

fn migrate_v4(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "detail_file", "TEXT")?;
    add_column_if_missing(conn, "proxy_request_logs", "detail_offset", "INTEGER")?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_detail_location
            ON proxy_request_logs(detail_file, detail_offset);",
    )
    .map_err(|error| format!("Failed to create proxy gateway detail location index: {error}"))
}

fn migrate_v5(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(
        conn,
        "proxy_request_logs",
        "pricing_model_source",
        "TEXT NOT NULL DEFAULT 'upstream'",
    )
}

fn migrate_v6(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "pi_settings_config")?;
    create_jsonb_table(conn, "pi_prompt_config")?;
    create_json_index(conn, "pi_prompt_config", &JsonFieldPath::new("is_applied")?)?;
    create_json_index(conn, "pi_prompt_config", &JsonFieldPath::new("sort_index")?)
}

fn migrate_v7(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "route_name", "TEXT")?;
    add_column_if_missing(conn, "proxy_request_logs", "method", "TEXT")?;
    add_column_if_missing(conn, "proxy_request_logs", "path", "TEXT")
}

/// Grok tables.
///
/// **Frozen.** These steps run only on databases that predate the v33
/// consolidation, so they must keep naming the per-CLI tables even though no
/// `DbTable` variant points at them any more. Editing this list to the
/// consolidated names would silently create empty `cli_config` rows on an old
/// database and lose the real ones.
fn migrate_v8(conn: &Connection) -> Result<(), String> {
    for table in [
        "grok_provider",
        "grok_official_account",
        "grok_common_config",
        "grok_prompt_config",
    ] {
        create_jsonb_table(conn, table)?;
    }

    for table in ["grok_provider", "grok_official_account", "grok_prompt_config"] {
        create_json_index(conn, table, &JsonFieldPath::new("is_applied")?)?;
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
    }

    create_json_index(
        conn,
        "grok_official_account",
        &JsonFieldPath::new("provider_id")?,
    )
}

fn migrate_v9(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "oh_my_pi_settings_config")?;
    create_jsonb_table(conn, "oh_my_pi_prompt_config")?;
    create_json_index(
        conn,
        "oh_my_pi_prompt_config",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "oh_my_pi_prompt_config",
        &JsonFieldPath::new("sort_index")?,
    )
}

fn migrate_v10(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "claude_desktop_provider")?;
    create_json_index(
        conn,
        "claude_desktop_provider",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "claude_desktop_provider",
        &JsonFieldPath::new("sort_index")?,
    )?;
    create_jsonb_table(conn, "hermes_settings_config")?;
    create_jsonb_table(conn, "hermes_prompt_config")?;
    create_json_index(
        conn,
        "hermes_prompt_config",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "hermes_prompt_config",
        &JsonFieldPath::new("sort_index")?,
    )
}

fn migrate_v11(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "claude_desktop_prompt_config")?;
    create_json_index(
        conn,
        "claude_desktop_prompt_config",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "claude_desktop_prompt_config",
        &JsonFieldPath::new("sort_index")?,
    )
}

fn migrate_v12(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "dsh_settings_config")?;
    create_jsonb_table(conn, "dsh_prompt_config")?;
    create_json_index(conn, "dsh_prompt_config", &JsonFieldPath::new("is_applied")?)?;
    create_json_index(conn, "dsh_prompt_config", &JsonFieldPath::new("sort_index")?)
}

fn migrate_v13(conn: &Connection) -> Result<(), String> {
    // Surface the upstream's original HTTP status alongside the gateway's synthetic
    // code (e.g. a 200 SSE stream carrying an error envelope is rewritten to 502).
    add_column_if_missing(
        conn,
        "proxy_request_logs",
        "upstream_status_code",
        "INTEGER",
    )
}

fn migrate_v14(conn: &Connection) -> Result<(), String> {
    // Record how the streaming response actually ended for the client, independent
    // of the (already-sent) 200 status. Mid-stream failures on a written 200 are
    // no longer recorded as successes. `stream_outcome` is NULL for non-streaming
    // and legacy rows, which fall back to status-code-based success derivation.
    add_column_if_missing(conn, "proxy_request_logs", "stream_outcome", "TEXT")?;
    // Persist the real error category instead of fabricating "upstream_error" on
    // read-back; NULL for legacy rows.
    add_column_if_missing(conn, "proxy_request_logs", "error_category", "TEXT")?;
    // Persist attempt counts instead of hardcoding 1 on read-back.
    add_column_if_missing(
        conn,
        "proxy_request_logs",
        "attempt_count",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        conn,
        "proxy_request_logs",
        "total_attempt_count",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_stream_outcome
            ON proxy_request_logs(stream_outcome);",
    )
    .map_err(|error| format!("Failed to create proxy gateway stream outcome index: {error}"))
}

fn migrate_v15(conn: &Connection) -> Result<(), String> {
    // Managed MCP groups for the group management modal (mirrors skill_group);
    // membership stays on each server's user_group text.
    create_jsonb_table(conn, "mcp_group")
}

fn migrate_v16(conn: &Connection) -> Result<(), String> {
    for table in [
        "kimi_provider",
        "kimi_official_account",
        "kimi_common_config",
        "kimi_prompt_config",
    ] {
        create_jsonb_table(conn, table)?;
    }

    for table in [
        "kimi_provider",
        "kimi_official_account",
        "kimi_prompt_config",
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("is_applied")?)?;
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
    }

    create_json_index(
        conn,
        "kimi_official_account",
        &JsonFieldPath::new("provider_id")?,
    )
}

fn migrate_v17(conn: &Connection) -> Result<(), String> {
    // Final upstream effort is compact metadata; legacy and session rows stay NULL.
    add_column_if_missing(conn, "proxy_request_logs", "reasoning_effort", "TEXT")
}

fn migrate_v18(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "gateway_session_usage_state")
}

fn migrate_v19(conn: &Connection) -> Result<(), String> {
    // Early v18 databases already have the session ledger but not this column.
    // A new version is required: extending an applied migration cannot repair them.
    add_column_if_missing(
        conn,
        "usage_daily_rollups",
        "latency_sample_count",
        "INTEGER",
    )?;
    // Legacy imports always used the reserved provider id `session` and did
    // not measure HTTP latency. Preserve existing proxy averages on upgrade.
    conn.execute(
        "UPDATE usage_daily_rollups SET latency_sample_count =
             CASE WHEN provider_id = 'session' THEN 0 ELSE request_count END
         WHERE latency_sample_count IS NULL",
        [],
    )
    .map_err(|error| format!("Failed to migrate usage latency samples: {error}"))?;
    Ok(())
}

fn migrate_v20(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "usage_metadata", "BLOB")?;
    add_column_if_missing(
        conn,
        "proxy_request_logs",
        "usage_request_count",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    // Some native providers report orchestration usage outside the four
    // billable categories. Preserve it in totals without inventing a price.
    for table in ["proxy_request_logs", "usage_daily_rollups"] {
        add_column_if_missing(conn, table, "extra_tokens", "INTEGER NOT NULL DEFAULT 0")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_proxy_native_match
         ON proxy_request_logs(app_type, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, created_at);",
    ).map_err(|error| format!("Failed to index native usage reconciliation: {error}"))
}

fn migrate_v21(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(
        conn,
        "proxy_request_logs",
        "transport",
        "TEXT NOT NULL DEFAULT 'http'",
    )?;
    add_column_if_missing(
        conn,
        "proxy_request_logs",
        "request_kind",
        "TEXT NOT NULL DEFAULT 'request'",
    )
}

fn migrate_v22(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "oh_my_pi_agents_config")?;
    create_json_index(
        conn,
        "oh_my_pi_agents_config",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "oh_my_pi_agents_config",
        &JsonFieldPath::new("sort_index")?,
    )
}

fn migrate_v23(conn: &Connection) -> Result<(), String> {
    for table in [
        "antigravity_provider",
        "antigravity_common_config",
        "antigravity_prompt_config",
        "antigravity_official_account",
    ] {
        create_jsonb_table(conn, table)?;
    }
    for table in [
        "antigravity_provider",
        "antigravity_prompt_config",
        "antigravity_official_account",
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("is_applied")?)?;
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
    }
    create_json_index(
        conn,
        "antigravity_official_account",
        &JsonFieldPath::new("provider_id")?,
    )
}

fn migrate_v24(conn: &Connection) -> Result<(), String> {
    for table in ["omo_native_settings_config", "omo_native_agents_config"] {
        create_jsonb_table(conn, table)?;
    }
    create_json_index(
        conn,
        "omo_native_agents_config",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "omo_native_agents_config",
        &JsonFieldPath::new("sort_index")?,
    )
}

fn migrate_v25(conn: &Connection) -> Result<(), String> {
    for table in [
        "zcode_provider",
        "zcode_common_config",
        "zcode_prompt_config",
        "zcode_official_account",
    ] {
        create_jsonb_table(conn, table)?;
    }
    for table in [
        "zcode_provider",
        "zcode_prompt_config",
        "zcode_official_account",
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("is_applied")?)?;
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
    }
    create_json_index(
        conn,
        "zcode_official_account",
        &JsonFieldPath::new("provider_id")?,
    )
}

/// OmO Native 的全局提示词（`<agentDir>/AGENTS.md`）预设表。
///
/// 与其余 CLI 的 prompt 表同构：v6（Pi）、v23（ZCode）等各建各的表，这里补上
/// Native 这一份——Native 的页面在 2026-10-07 才加上全局提示词区块。
fn migrate_v26(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, "omo_native_prompt_config")?;
    create_json_index(
        conn,
        "omo_native_prompt_config",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "omo_native_prompt_config",
        &JsonFieldPath::new("sort_index")?,
    )
}

/// Billed Fast/Ultrafast service tier (Codex `service_tier`) and the provider's
/// effective extra multiplier for it. Compact metadata like v17's
/// `reasoning_effort`: legacy and session rows stay NULL, and NULL on the tier
/// means "no premium tier billed".
fn migrate_v27(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "service_tier", "TEXT")?;
    add_column_if_missing(conn, "proxy_request_logs", "service_tier_multiplier", "TEXT")
}

fn migrate_v28(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "compaction_mode", "TEXT")
}

/// Project dimension. Sessions imported from local CLIs record their working
/// directory; proxy traffic has no project at all, so the column stays NULL
/// there and every consumer coalesces it to `''` ("unknown project").
///
/// The rollup table is **rebuilt** rather than extended: its primary key has to
/// grow the new column, and SQLite cannot alter a primary key in place. Without
/// the rebuild, two projects used on the same day/model would collapse into one
/// archived row and the project split would silently vanish once the detail
/// rows are pruned.
fn migrate_v29(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "project_path", "TEXT")?;
    // Provider-billed credits, kept beside the token counters. Only Qoder
    // reports them today: its built-in routing writes zero tokens to the
    // transcript and records Credits instead, so tokens alone cannot express
    // how much that tool was used. Additive with a NULL/0 default, so every
    // other tool and every pre-upgrade row reads as "no credits reported".
    // Nothing renders it yet; it is collected now so history is not lost.
    add_column_if_missing(conn, "proxy_request_logs", "credits", "REAL")?;
    // Added before the rebuild so the INSERT ... SELECT below can copy it from
    // the old table; the rebuilt table declares it itself.
    add_column_if_missing(conn, "usage_daily_rollups", "credits", "REAL NOT NULL DEFAULT 0")?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_project
            ON proxy_request_logs(project_path);",
    )
    .map_err(|error| format!("Failed to create proxy gateway project index: {error}"))?;

    // A rollup table that already has the column was rebuilt by an earlier
    // attempt (or by a database restored at v29). Rebuilding again would fold
    // the per-project rows back into one unknown-project row, so skip it — but
    // the credits column is added either way, since a database that ran the
    // old two-step migration path may sit on the rebuilt table without it.
    if table_has_column(conn, "usage_daily_rollups", "project_path")? {
        return add_column_if_missing(
            conn,
            "usage_daily_rollups",
            "credits",
            "REAL NOT NULL DEFAULT 0",
        );
    }

    conn.execute_batch(
        "CREATE TABLE usage_daily_rollups_v29 (
            date TEXT NOT NULL,
            app_type TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            project_path TEXT NOT NULL DEFAULT '',
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            avg_latency_ms INTEGER NOT NULL DEFAULT 0,
            latency_sample_count INTEGER,
            extra_tokens INTEGER NOT NULL DEFAULT 0,
            credits REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (date, app_type, provider_id, model, project_path)
        );

        INSERT INTO usage_daily_rollups_v29 (
            date, app_type, provider_id, model, project_path, request_count, success_count,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            total_cost_usd, avg_latency_ms, latency_sample_count, extra_tokens, credits
        )
        SELECT date, app_type, provider_id, model, '',
               request_count, success_count,
               input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
               total_cost_usd, avg_latency_ms, latency_sample_count, extra_tokens,
               COALESCE(credits, 0)
        FROM usage_daily_rollups;

        DROP TABLE usage_daily_rollups;
        ALTER TABLE usage_daily_rollups_v29 RENAME TO usage_daily_rollups;

        CREATE INDEX IF NOT EXISTS idx_usage_rollups_app_date
            ON usage_daily_rollups(app_type, date);
        CREATE INDEX IF NOT EXISTS idx_usage_rollups_provider
            ON usage_daily_rollups(provider_id, app_type);
        CREATE INDEX IF NOT EXISTS idx_usage_rollups_project
            ON usage_daily_rollups(project_path);",
    )
    .map_err(|error| format!("Failed to rebuild usage rollups for project dimension: {error}"))
}

/// 原生用量按 envelope 对照代理请求。主键前缀范围在每次未命中时仍会扫描整段历史，
/// 因此直接索引 envelope，并从候选集合排除纯原生用量。
fn migrate_v30(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_proxy_envelope
         ON proxy_request_logs (
             app_type,
             substr(request_id, length('SESSION:' || app_type || ':' || provider_id || ':') + 1)
         )
         WHERE COALESCE(data_source, 'proxy') = 'proxy' AND request_kind = 'request';",
    )
    .map_err(|error| format!("Failed to create proxy envelope lookup index: {error}"))
}

/// Native/hosted tools the upstream actually invoked, as a JSON array of
/// canonical kinds (`["web_search"]`).
///
/// Detail-only markers (`declared` / `forwarded` triples and the rewrite list)
/// stay in the JSONL record. This column exists because the request list reads
/// the compact table and cannot see them; it carries only what was **observed
/// to run**. NULL means "nothing observed", never "not used": legacy rows,
/// imported session usage and requests the gateway rejected locally were never
/// inspected, and the UI must not read that as a negative.
///
/// No index and no filter: the list renders it per row and nothing queries by
/// it.
fn migrate_v31(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "native_tools_used", "TEXT")
}

/// WorkBuddy desktop + CodeBuddy Code tables.
///
/// The **page** still derives its channel list from `models.json`: that file is
/// the products' own source of truth and a derived view cannot carry an id or
/// survive a round trip. `*_provider` is a separate, gateway-only record set:
/// takeover needs a stable provider id plus persistable enable/sort/failover
/// state, none of which `models.json` expresses. WorkBuddy's four identity
/// documents are edited in place rather than as switchable presets, so only the
/// config-root overrides and CodeBuddy's `CODEBUDDY.md` presets need anything
/// else here.
fn migrate_v32(conn: &Connection) -> Result<(), String> {
    for table in [
        "workbuddy_settings_config",
        "workbuddy_provider",
        "codebuddy_settings_config",
        "codebuddy_prompt_config",
        "codebuddy_provider",
    ] {
        create_jsonb_table(conn, table)?;
    }
    create_json_index(
        conn,
        "codebuddy_prompt_config",
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        "codebuddy_prompt_config",
        &JsonFieldPath::new("sort_index")?,
    )?;
    // The gateway orders providers by `sort_index` then `updated_at`; without
    // this index every candidate load falls back to a full-table scan of a
    // JSONB blob.
    for table in ["workbuddy_provider", "codebuddy_provider"] {
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
        create_json_index(conn, table, &JsonFieldPath::new("is_disabled")?)?;
    }
    Ok(())
}

/// Consolidate the per-CLI tables and create the relay tables.
///
/// Fifty-five tables — one provider table per CLI, one official-account table,
/// twenty-nine config tables — become three, told apart by real `cli_key` /
/// `kind` columns. This step moves every row across and then drops the old
/// tables in the same savepoint, so a failure leaves the database exactly as it
/// was.
///
/// The primary keys differ per table on purpose:
///
/// - `cli_provider` / `cli_official_account` are keyed `(cli_key, id)`. Ids are
///   UUIDs today and never repeat across CLIs, but the scope belongs in the key:
///   a future deeplink import that reuses an id must not overwrite another
///   CLI's provider.
/// - `cli_config` is keyed `(cli_key, kind, id)`. Six CLIs' common configs all
///   use the literal id `common` and two CLIs' globals use `global`, so `id`
///   alone collides. Keeping the id verbatim (rather than synthesising
///   `kimi:common`) keeps every store call site unchanged.
fn migrate_v33(conn: &Connection) -> Result<(), String> {
    create_cli_provider_table(conn)?;
    create_cli_official_account_table(conn)?;
    create_cli_config_table(conn)?;

    for (old_table, cli_key) in CLI_PROVIDER_TABLES {
        copy_scoped_rows(conn, old_table, "cli_provider", &[("cli_key", cli_key)])?;
    }
    for (old_table, cli_key) in CLI_OFFICIAL_ACCOUNT_TABLES {
        copy_scoped_rows(
            conn,
            old_table,
            "cli_official_account",
            &[("cli_key", cli_key)],
        )?;
    }
    for (old_table, cli_key, kind) in CLI_CONFIG_TABLES {
        copy_scoped_rows(
            conn,
            old_table,
            "cli_config",
            &[("cli_key", cli_key), ("kind", kind)],
        )?;
    }

    create_relay_tables(conn)?;
    for (old_table, kind) in RELAY_LEGACY_TABLES {
        copy_scoped_rows(conn, old_table, "relay_entity", &[("kind", kind)])?;
    }

    drop_consolidated_tables(conn)?;
    create_consolidated_indexes(conn)?;
    Ok(())
}

fn create_cli_provider_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cli_provider (
            id TEXT NOT NULL,
            cli_key TEXT NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (cli_key, id)
        );",
    )
    .map_err(|error| format!("Failed to create cli_provider: {error}"))
}

fn create_cli_official_account_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cli_official_account (
            id TEXT NOT NULL,
            cli_key TEXT NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (cli_key, id)
        );",
    )
    .map_err(|error| format!("Failed to create cli_official_account: {error}"))
}

fn create_cli_config_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cli_config (
            id TEXT NOT NULL,
            cli_key TEXT NOT NULL,
            kind TEXT NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (cli_key, kind, id)
        );",
    )
    .map_err(|error| format!("Failed to create cli_config: {error}"))
}

fn create_relay_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS relay_entity (
            id TEXT NOT NULL,
            kind TEXT NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (kind, id)
        );

        CREATE TABLE IF NOT EXISTS relay_quota_snapshot (
            id TEXT NOT NULL,
            kind TEXT NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (kind, id)
        );

        CREATE TABLE IF NOT EXISTS relay_quota_point (
            id TEXT NOT NULL,
            kind TEXT NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (kind, id)
        );",
    )
    .map_err(|error| format!("Failed to create relay tables: {error}"))
}

/// Copy every row of `old_table` into `target`, stamping the given scope columns.
///
/// A missing `old_table` is not an error: a database that was upgraded to v32 by
/// a build predating one of these tables, or a fresh install that reached v33 in
/// one go, simply has nothing to move. `INSERT OR IGNORE` guards the one case
/// that could collide — a database this branch already ran an earlier v33 on.
fn copy_scoped_rows(
    conn: &Connection,
    old_table: &str,
    target: &str,
    scope: &[(&str, &str)],
) -> Result<(), String> {
    if !table_exists(conn, old_table)? || old_table == target {
        return Ok(());
    }

    // Target lists the scope *column names*; source lists their *values* as
    // string literals, which is what stamps the cli_key onto every moved row.
    let mut target_columns = String::from("id");
    let mut source_columns = String::from("id");
    for (column, value) in scope {
        target_columns.push_str(&format!(", {column}"));
        source_columns.push_str(&format!(", {}", sql_string_literal(value)));
    }
    target_columns.push_str(", data, created_at, updated_at");
    source_columns.push_str(", data, created_at, updated_at");

    conn.execute_batch(&format!(
        "INSERT OR IGNORE INTO {target} ({target_columns})
         SELECT {source_columns} FROM {old_table};"
    ))
    .map_err(|error| format!("Failed to move {old_table} into {target}: {error}"))
}

fn drop_consolidated_tables(conn: &Connection) -> Result<(), String> {
    for (table, _) in CLI_PROVIDER_TABLES {
        drop_table_if_exists(conn, table)?;
    }
    for (table, _) in CLI_OFFICIAL_ACCOUNT_TABLES {
        drop_table_if_exists(conn, table)?;
    }
    for (table, _, _) in CLI_CONFIG_TABLES {
        drop_table_if_exists(conn, table)?;
    }
    for (table, _) in RELAY_LEGACY_TABLES {
        // `relay_entity` is the destination the loop above just filled; dropping
        // it would delete the rows that were just written.
        if *table == "relay_entity" {
            continue;
        }
        drop_table_if_exists(conn, table)?;
    }
    Ok(())
}

fn drop_table_if_exists(conn: &Connection, table: &str) -> Result<(), String> {
    conn.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))
        .map_err(|error| format!("Failed to drop legacy table {table}: {error}"))
}

fn create_consolidated_indexes(conn: &Connection) -> Result<(), String> {
    // The three CLI tables are always read inside a scope, so every index leads
    // with the scope column.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_cli_provider_cli_key
            ON cli_provider(cli_key);
        CREATE INDEX IF NOT EXISTS idx_cli_provider_applied
            ON cli_provider(cli_key, CAST(json_extract(data, '$.is_applied') AS INTEGER));
        CREATE INDEX IF NOT EXISTS idx_cli_provider_sort
            ON cli_provider(cli_key, CAST(json_extract(data, '$.sort_index') AS INTEGER));
        CREATE INDEX IF NOT EXISTS idx_cli_provider_disabled
            ON cli_provider(cli_key, CAST(json_extract(data, '$.is_disabled') AS INTEGER));

        CREATE INDEX IF NOT EXISTS idx_cli_official_account_cli_key
            ON cli_official_account(cli_key);
        CREATE INDEX IF NOT EXISTS idx_cli_official_account_applied
            ON cli_official_account(cli_key, CAST(json_extract(data, '$.is_applied') AS INTEGER));
        CREATE INDEX IF NOT EXISTS idx_cli_official_account_sort
            ON cli_official_account(cli_key, CAST(json_extract(data, '$.sort_index') AS INTEGER));
        CREATE INDEX IF NOT EXISTS idx_cli_official_account_provider
            ON cli_official_account(cli_key, json_extract(data, '$.provider_id'));

        CREATE INDEX IF NOT EXISTS idx_cli_config_key_kind
            ON cli_config(cli_key, kind);
        CREATE INDEX IF NOT EXISTS idx_cli_config_applied
            ON cli_config(cli_key, kind, CAST(json_extract(data, '$.is_applied') AS INTEGER));
        CREATE INDEX IF NOT EXISTS idx_cli_config_sort
            ON cli_config(cli_key, kind, CAST(json_extract(data, '$.sort_index') AS INTEGER));

        CREATE INDEX IF NOT EXISTS idx_relay_entity_kind
            ON relay_entity(kind);
        CREATE INDEX IF NOT EXISTS idx_relay_entity_sort
            ON relay_entity(kind, CAST(json_extract(data, '$.sort_index') AS INTEGER));
        CREATE INDEX IF NOT EXISTS idx_relay_entity_channel
            ON relay_entity(kind, json_extract(data, '$.channel_id'));
        CREATE INDEX IF NOT EXISTS idx_relay_entity_plugin
            ON relay_entity(kind, json_extract(data, '$.plugin_id'));
        CREATE INDEX IF NOT EXISTS idx_relay_entity_plugin_provider
            ON relay_entity(kind, json_extract(data, '$.plugin_provider_id'));
        CREATE INDEX IF NOT EXISTS idx_relay_entity_ts
            ON relay_entity(kind, CAST(json_extract(data, '$.ts') AS INTEGER));

        CREATE INDEX IF NOT EXISTS idx_relay_quota_snapshot_kind
            ON relay_quota_snapshot(kind);
        CREATE INDEX IF NOT EXISTS idx_relay_quota_point_kind_at
            ON relay_quota_point(kind, CAST(json_extract(data, '$.at') AS INTEGER));",
    )
    .map_err(|error| format!("Failed to create consolidated indexes: {error}"))
}

/// Fold the last per-CLI tables into `cli_config` and drop the dead migration
/// ledger.
///
/// `codex_plugin_workspace_roots` and the two OpenCode favourite tables were the
/// only CLI-owned tables left outside v33's consolidation. They hold user data
/// keyed by CLI (favourite plugin/provider ids, one workspace-roots record), so
/// they fit `cli_config`'s `(cli_key, kind, id)` key without any schema change.
/// `app_migration` is the retired one-shot data-migration ledger from the
/// SurrealDB era; nothing reads it any more.
///
/// Copies run before drops inside the same SAVEPOINT, so a failure leaves every
/// legacy table and its rows exactly as it was.
fn migrate_v34(conn: &Connection) -> Result<(), String> {
    copy_scoped_rows(
        conn,
        "codex_plugin_workspace_roots",
        "cli_config",
        &[("cli_key", "codex"), ("kind", "workspace_roots")],
    )?;
    copy_scoped_rows(
        conn,
        "opencode_favorite_plugin",
        "cli_config",
        &[("cli_key", "opencode"), ("kind", "favorite_plugin")],
    )?;
    copy_scoped_rows(
        conn,
        "opencode_favorite_provider",
        "cli_config",
        &[("cli_key", "opencode"), ("kind", "favorite_provider")],
    )?;

    for table in [
        "codex_plugin_workspace_roots",
        "opencode_favorite_plugin",
        "opencode_favorite_provider",
        "app_migration",
    ] {
        drop_table_if_exists(conn, table)?;
    }
    Ok(())
}

fn create_jsonb_table(conn: &Connection, table: &str) -> Result<(), String> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {table} (
            id TEXT PRIMARY KEY NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );"
    ))
    .map_err(|error| format!("Failed to create SQLite table {table}: {error}"))
}

/// Indexes v1 created, against the **legacy** table names.
///
/// Frozen for the same reason as [`migrate_v1`]: this runs before the v33
/// consolidation, when the per-CLI tables still exist. The consolidated
/// indexes are created by [`create_consolidated_indexes`] instead.
fn create_initial_indexes(conn: &Connection) -> Result<(), String> {
    for table in [
        "claude_provider",
        "codex_provider",
        "gemini_cli_provider",
        "antigravity_provider",
        "claude_prompt_config",
        "codex_prompt_config",
        "gemini_cli_prompt_config",
        "antigravity_prompt_config",
        "codex_official_account",
        "gemini_cli_official_account",
        "antigravity_official_account",
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("is_applied")?)?;
    }

    for table in [
        "claude_provider",
        "codex_provider",
        "gemini_cli_provider",
        "antigravity_provider",
        "claude_prompt_config",
        "codex_prompt_config",
        "gemini_cli_prompt_config",
        "antigravity_prompt_config",
        "skill",
        "skill_group",
        "mcp_server",
        "opencode_common_config",
        "codex_official_account",
        "gemini_cli_official_account",
        "antigravity_official_account",
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
    }

    for table in ["image_channel", "ssh_connection"] {
        create_json_index(conn, table, &JsonFieldPath::new("sort_order")?)?;
    }

    for table in [
        "opencode_favorite_plugin",
        "opencode_favorite_provider",
        "favorite_mcp",
        "image_job",
        "image_asset",
    ] {
        create_column_index(conn, table, "created_at")?;
    }

    for (table, field) in [
        ("skill", "name"),
        ("mcp_server", "name"),
        ("favorite_mcp", "name"),
        ("opencode_favorite_plugin", "plugin_name"),
        ("opencode_favorite_provider", "provider_id"),
        ("codex_official_account", "provider_id"),
        ("gemini_cli_official_account", "provider_id"),
        ("antigravity_official_account", "provider_id"),
        ("image_asset", "job_id"),
    ] {
        create_json_index(conn, table, &JsonFieldPath::new(field)?)?;
    }

    Ok(())
}

fn create_proxy_gateway_usage_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS proxy_request_logs (
            request_id TEXT PRIMARY KEY NOT NULL,
            provider_id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            model TEXT NOT NULL,
            request_model TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            input_cost_usd TEXT NOT NULL DEFAULT '0',
            output_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_read_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            latency_ms INTEGER NOT NULL DEFAULT 0,
            first_token_ms INTEGER,
            duration_ms INTEGER,
            status_code INTEGER NOT NULL DEFAULT 0,
            error_message TEXT,
            session_id TEXT,
            provider_type TEXT,
            is_streaming INTEGER NOT NULL DEFAULT 0,
            cost_multiplier TEXT NOT NULL DEFAULT '1.0',
            pricing_model_source TEXT NOT NULL DEFAULT 'upstream',
            created_at INTEGER NOT NULL,
            data_source TEXT NOT NULL DEFAULT 'proxy',
            detail_file TEXT,
            detail_offset INTEGER,
            route_name TEXT,
            method TEXT,
            path TEXT,
            upstream_status_code INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_request_logs_provider
            ON proxy_request_logs(provider_id, app_type);
        CREATE INDEX IF NOT EXISTS idx_request_logs_created_at
            ON proxy_request_logs(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_request_logs_model
            ON proxy_request_logs(model);
        CREATE INDEX IF NOT EXISTS idx_request_logs_session
            ON proxy_request_logs(session_id);
        CREATE INDEX IF NOT EXISTS idx_request_logs_status
            ON proxy_request_logs(status_code);
        CREATE INDEX IF NOT EXISTS idx_request_logs_app_created_at
            ON proxy_request_logs(app_type, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_request_logs_detail_location
            ON proxy_request_logs(detail_file, detail_offset);

        CREATE TABLE IF NOT EXISTS usage_daily_rollups (
            date TEXT NOT NULL,
            app_type TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            avg_latency_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, app_type, provider_id, model)
        );

        CREATE INDEX IF NOT EXISTS idx_usage_rollups_app_date
            ON usage_daily_rollups(app_type, date);
        CREATE INDEX IF NOT EXISTS idx_usage_rollups_provider
            ON usage_daily_rollups(provider_id, app_type);",
    )
    .map_err(|error| format!("Failed to create proxy gateway usage tables: {error}"))
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    if table_has_column(conn, table, column)? {
        return Ok(());
    }
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition};"
    ))
    .map_err(|error| format!("Failed to add SQLite column {table}.{column}: {error}"))
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| format!("Failed to inspect SQLite table {table}: {error}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed to read SQLite columns for {table}: {error}"))?;
    for existing in columns {
        if existing
            .map_err(|error| format!("Failed to read SQLite column for {table}: {error}"))?
            .eq_ignore_ascii_case(column)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
        [table],
        |_| Ok(()),
    )
    .map(|_| true)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(false),
        other => Err(format!("Failed to inspect SQLite table {table}: {other}")),
    })
}

fn create_model_pricing_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS model_pricing (
            model_id TEXT PRIMARY KEY NOT NULL,
            display_name TEXT NOT NULL,
            input_cost_per_million TEXT NOT NULL DEFAULT '0',
            output_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
        );",
    )
    .map_err(|error| format!("Failed to create model pricing table: {error}"))
}

const MODEL_PRICING_SEED: &[(&str, &str, &str, &str, &str, &str)] = &[
        (
            "claude-opus-4-7",
            "Claude Opus 4.7",
            "5",
            "25",
            "0.50",
            "6.25",
        ),
        (
            "claude-opus-4-6-20260206",
            "Claude Opus 4.6",
            "5",
            "25",
            "0.50",
            "6.25",
        ),
        (
            "claude-sonnet-4-6-20260217",
            "Claude Sonnet 4.6",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        (
            "claude-opus-4-5-20251101",
            "Claude Opus 4.5",
            "5",
            "25",
            "0.50",
            "6.25",
        ),
        (
            "claude-sonnet-4-5-20250929",
            "Claude Sonnet 4.5",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        (
            "claude-haiku-4-5-20251001",
            "Claude Haiku 4.5",
            "1",
            "5",
            "0.10",
            "1.25",
        ),
        (
            "claude-opus-4-20250514",
            "Claude Opus 4",
            "15",
            "75",
            "1.50",
            "18.75",
        ),
        (
            "claude-opus-4-1-20250805",
            "Claude Opus 4.1",
            "15",
            "75",
            "1.50",
            "18.75",
        ),
        (
            "claude-sonnet-4-20250514",
            "Claude Sonnet 4",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        (
            "claude-3-5-haiku-20241022",
            "Claude 3.5 Haiku",
            "0.80",
            "4",
            "0.08",
            "1",
        ),
        (
            "claude-3-5-sonnet-20241022",
            "Claude 3.5 Sonnet",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        ("gpt-5.5", "GPT-5.5", "5", "30", "0.50", "0"),
        ("gpt-5.5-low", "GPT-5.5", "5", "30", "0.50", "0"),
        ("gpt-5.5-medium", "GPT-5.5", "5", "30", "0.50", "0"),
        ("gpt-5.5-high", "GPT-5.5", "5", "30", "0.50", "0"),
        ("gpt-5.5-xhigh", "GPT-5.5", "5", "30", "0.50", "0"),
        ("gpt-5.4", "GPT-5.4", "2.50", "15", "0.25", "0"),
        ("gpt-5.4-mini", "GPT-5.4 Mini", "0.75", "4.50", "0.075", "0"),
        ("gpt-5.4-nano", "GPT-5.4 Nano", "0.20", "1.25", "0.02", "0"),
        ("gpt-5.3-codex", "GPT-5.3 Codex", "1.75", "14", "0.175", "0"),
        (
            "gpt-5.3-codex-low",
            "GPT-5.3 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        (
            "gpt-5.3-codex-medium",
            "GPT-5.3 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        (
            "gpt-5.3-codex-high",
            "GPT-5.3 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        (
            "gpt-5.3-codex-xhigh",
            "GPT-5.3 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        ("gpt-5.2", "GPT-5.2", "1.75", "14", "0.175", "0"),
        ("gpt-5.2-low", "GPT-5.2", "1.75", "14", "0.175", "0"),
        ("gpt-5.2-medium", "GPT-5.2", "1.75", "14", "0.175", "0"),
        ("gpt-5.2-high", "GPT-5.2", "1.75", "14", "0.175", "0"),
        ("gpt-5.2-xhigh", "GPT-5.2", "1.75", "14", "0.175", "0"),
        ("gpt-5.2-codex", "GPT-5.2 Codex", "1.75", "14", "0.175", "0"),
        (
            "gpt-5.2-codex-low",
            "GPT-5.2 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        (
            "gpt-5.2-codex-medium",
            "GPT-5.2 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        (
            "gpt-5.2-codex-high",
            "GPT-5.2 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        (
            "gpt-5.2-codex-xhigh",
            "GPT-5.2 Codex",
            "1.75",
            "14",
            "0.175",
            "0",
        ),
        ("gpt-5.1", "GPT-5.1", "1.25", "10", "0.125", "0"),
        ("gpt-5.1-low", "GPT-5.1", "1.25", "10", "0.125", "0"),
        ("gpt-5.1-medium", "GPT-5.1", "1.25", "10", "0.125", "0"),
        ("gpt-5.1-high", "GPT-5.1", "1.25", "10", "0.125", "0"),
        ("gpt-5.1-codex", "GPT-5.1 Codex", "1.25", "10", "0.125", "0"),
        ("gpt-5", "GPT-5", "1.25", "10", "0.125", "0"),
        ("gpt-5-low", "GPT-5", "1.25", "10", "0.125", "0"),
        ("gpt-5-medium", "GPT-5", "1.25", "10", "0.125", "0"),
        ("gpt-5-high", "GPT-5", "1.25", "10", "0.125", "0"),
        ("gpt-5-codex", "GPT-5 Codex", "1.25", "10", "0.125", "0"),
        ("gpt-5-codex-low", "GPT-5 Codex", "1.25", "10", "0.125", "0"),
        (
            "gpt-5-codex-medium",
            "GPT-5 Codex",
            "1.25",
            "10",
            "0.125",
            "0",
        ),
        (
            "gpt-5-codex-high",
            "GPT-5 Codex",
            "1.25",
            "10",
            "0.125",
            "0",
        ),
        ("gpt-4.1", "GPT-4.1", "2", "8", "0.50", "0"),
        ("gpt-4.1-mini", "GPT-4.1 Mini", "0.40", "1.60", "0.10", "0"),
        ("gpt-4.1-nano", "GPT-4.1 Nano", "0.10", "0.40", "0.025", "0"),
        ("o3", "OpenAI o3", "2", "8", "0.50", "0"),
        ("o4-mini", "OpenAI o4-mini", "1.10", "4.40", "0.275", "0"),
        (
            "gemini-3.1-pro-preview",
            "Gemini 3.1 Pro Preview",
            "2",
            "12",
            "0.20",
            "0",
        ),
        (
            "gemini-3.1-flash-lite-preview",
            "Gemini 3.1 Flash Lite Preview",
            "0.25",
            "1.50",
            "0.025",
            "0",
        ),
        (
            "gemini-3-pro-preview",
            "Gemini 3 Pro Preview",
            "2",
            "12",
            "0.2",
            "0",
        ),
        (
            "gemini-3-flash-preview",
            "Gemini 3 Flash Preview",
            "0.5",
            "3",
            "0.05",
            "0",
        ),
        (
            "gemini-2.5-pro",
            "Gemini 2.5 Pro",
            "1.25",
            "10",
            "0.125",
            "0",
        ),
        (
            "gemini-2.5-flash",
            "Gemini 2.5 Flash",
            "0.3",
            "2.5",
            "0.03",
            "0",
        ),
        (
            "gemini-2.5-flash-lite",
            "Gemini 2.5 Flash Lite",
            "0.10",
            "0.40",
            "0.01",
            "0",
        ),
        (
            "gemini-2.0-flash",
            "Gemini 2.0 Flash",
            "0.10",
            "0.40",
            "0.025",
            "0",
        ),
        (
            "deepseek-v3.2",
            "DeepSeek V3.2",
            "0.28",
            "0.42",
            "0.028",
            "0",
        ),
        (
            "deepseek-chat",
            "DeepSeek Chat",
            "0.28",
            "0.42",
            "0.028",
            "0",
        ),
];

fn seed_model_pricing(conn: &Connection) -> Result<(), String> {
    for (model_id, display_name, input, output, cache_read, cache_creation) in MODEL_PRICING_SEED {
        conn.execute(
            "INSERT OR IGNORE INTO model_pricing (
                model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            [
                model_id,
                display_name,
                input,
                output,
                cache_read,
                cache_creation,
            ],
        )
        .map_err(|error| format!("Failed to seed model pricing {model_id}: {error}"))?;
    }
    Ok(())
}

fn create_json_index(conn: &Connection, table_name: &str, field_path: &JsonFieldPath) -> Result<(), String> {
    let field_suffix = field_path.segments().join("_");
    let index_name = format!("idx_{table_name}_{field_suffix}");
    let json_path = sql_string_literal(&field_path.to_sql_path());
    conn.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS {index_name}
         ON {table_name} (json_extract(data, {json_path}));"
    ))
    .map_err(|error| format!("Failed to create SQLite index {index_name}: {error}"))
}

fn create_column_index(conn: &Connection, table_name: &str, column: &str) -> Result<(), String> {
    let index_name = format!("idx_{table_name}_{column}");
    conn.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS {index_name} ON {table_name} ({column});"
    ))
    .map_err(|error| format!("Failed to create SQLite index {index_name}: {error}"))
}
