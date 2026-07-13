use rusqlite::Connection;

use super::schema::{sql_string_literal, DbTable, JsonFieldPath, ALL_TABLES};

pub const TARGET_SCHEMA_VERSION: i32 = 8;
const FUTURE_SCHEMA_ERROR_PREFIX: &str = "AI_TOOLBOX_SQLITE_SCHEMA_TOO_NEW";

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

    Ok(())
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

fn migrate_v1(conn: &Connection) -> Result<(), String> {
    for table in ALL_TABLES {
        create_jsonb_table(conn, *table)?;
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
    create_jsonb_table(conn, DbTable::PiSettingsConfig)?;
    create_jsonb_table(conn, DbTable::PiPromptConfig)?;
    create_json_index(
        conn,
        DbTable::PiPromptConfig,
        &JsonFieldPath::new("is_applied")?,
    )?;
    create_json_index(
        conn,
        DbTable::PiPromptConfig,
        &JsonFieldPath::new("sort_index")?,
    )
}

fn migrate_v7(conn: &Connection) -> Result<(), String> {
    add_column_if_missing(conn, "proxy_request_logs", "route_name", "TEXT")?;
    add_column_if_missing(conn, "proxy_request_logs", "method", "TEXT")?;
    add_column_if_missing(conn, "proxy_request_logs", "path", "TEXT")
}

fn migrate_v8(conn: &Connection) -> Result<(), String> {
    for table in [
        DbTable::GrokProvider,
        DbTable::GrokOfficialAccount,
        DbTable::GrokCommonConfig,
        DbTable::GrokPromptConfig,
    ] {
        create_jsonb_table(conn, table)?;
    }

    for table in [
        DbTable::GrokProvider,
        DbTable::GrokOfficialAccount,
        DbTable::GrokPromptConfig,
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("is_applied")?)?;
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
    }

    create_json_index(
        conn,
        DbTable::GrokOfficialAccount,
        &JsonFieldPath::new("provider_id")?,
    )
}

fn create_jsonb_table(conn: &Connection, table: DbTable) -> Result<(), String> {
    let table_name = table.name();
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {table_name} (
            id TEXT PRIMARY KEY NOT NULL,
            data BLOB NOT NULL CHECK (json_valid(data, 4)),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );"
    ))
    .map_err(|error| format!("Failed to create SQLite table {table_name}: {error}"))
}

fn create_initial_indexes(conn: &Connection) -> Result<(), String> {
    for table in [
        DbTable::ClaudeProvider,
        DbTable::CodexProvider,
        DbTable::GeminiCliProvider,
        DbTable::ClaudePromptConfig,
        DbTable::CodexPromptConfig,
        DbTable::GeminiCliPromptConfig,
        DbTable::PiPromptConfig,
        DbTable::OpenCodePromptConfig,
        DbTable::OhMyOpenAgentConfig,
        DbTable::OhMyOpenCodeSlimConfig,
        DbTable::CodexOfficialAccount,
        DbTable::GeminiCliOfficialAccount,
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("is_applied")?)?;
    }

    for table in [
        DbTable::ClaudeProvider,
        DbTable::CodexProvider,
        DbTable::GeminiCliProvider,
        DbTable::ClaudePromptConfig,
        DbTable::CodexPromptConfig,
        DbTable::GeminiCliPromptConfig,
        DbTable::PiPromptConfig,
        DbTable::OpenCodePromptConfig,
        DbTable::Skill,
        DbTable::SkillGroup,
        DbTable::McpServer,
        DbTable::OhMyOpenAgentConfig,
        DbTable::OhMyOpenCodeSlimConfig,
        DbTable::CodexOfficialAccount,
        DbTable::GeminiCliOfficialAccount,
    ] {
        create_json_index(conn, table, &JsonFieldPath::new("sort_index")?)?;
    }

    for table in [DbTable::ImageChannel, DbTable::SshConnection] {
        create_json_index(conn, table, &JsonFieldPath::new("sort_order")?)?;
    }

    for table in [
        DbTable::OpenCodeFavoritePlugin,
        DbTable::OpenCodeFavoriteProvider,
        DbTable::FavoriteMcp,
        DbTable::ImageJob,
        DbTable::ImageAsset,
    ] {
        create_column_index(conn, table, "created_at")?;
    }

    for (table, field) in [
        (DbTable::Skill, "name"),
        (DbTable::McpServer, "name"),
        (DbTable::FavoriteMcp, "name"),
        (DbTable::OpenCodeFavoritePlugin, "plugin_name"),
        (DbTable::OpenCodeFavoriteProvider, "provider_id"),
        (DbTable::CodexOfficialAccount, "provider_id"),
        (DbTable::GeminiCliOfficialAccount, "provider_id"),
        (DbTable::ImageAsset, "job_id"),
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
            path TEXT
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
            return Ok(());
        }
    }
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition};"
    ))
    .map_err(|error| format!("Failed to add SQLite column {table}.{column}: {error}"))
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

fn seed_model_pricing(conn: &Connection) -> Result<(), String> {
    let pricing_data = [
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

    for (model_id, display_name, input, output, cache_read, cache_creation) in pricing_data {
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

fn create_json_index(
    conn: &Connection,
    table: DbTable,
    field_path: &JsonFieldPath,
) -> Result<(), String> {
    let table_name = table.name();
    let field_suffix = field_path.segments().join("_");
    let index_name = format!("idx_{table_name}_{field_suffix}");
    let json_path = sql_string_literal(&field_path.to_sql_path());
    conn.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS {index_name}
         ON {table_name} (json_extract(data, {json_path}));"
    ))
    .map_err(|error| format!("Failed to create SQLite index {index_name}: {error}"))
}

fn create_column_index(conn: &Connection, table: DbTable, column: &str) -> Result<(), String> {
    let table_name = table.name();
    let index_name = format!("idx_{table_name}_{column}");
    conn.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS {index_name} ON {table_name} ({column});"
    ))
    .map_err(|error| format!("Failed to create SQLite index {index_name}: {error}"))
}
