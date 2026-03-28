mod mcp_github_copilot_key_v1;
mod oh_my_openagent_rename_v1;

use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub enum MigrationOutcome {
    Applied,
    SkippedNoOp,
}

type MigrationFuture<'a> = Pin<Box<dyn Future<Output = Result<MigrationOutcome, String>> + Send + 'a>>;
type MigrationRunner =
    for<'a> fn(&'a surrealdb::Surreal<surrealdb::engine::local::Db>) -> MigrationFuture<'a>;

struct DbMigration {
    id: &'static str,
    description: &'static str,
    runner: MigrationRunner,
}

const REGISTERED_MIGRATIONS: &[DbMigration] = &[
    DbMigration {
        id: oh_my_openagent_rename_v1::MIGRATION_ID,
        description: "Rename Oh My OpenAgent main persistence contracts",
        runner: oh_my_openagent_rename_v1::run_migration,
    },
    DbMigration {
        id: mcp_github_copilot_key_v1::MIGRATION_ID,
        description: "Normalize MCP GitHub Copilot legacy IntelliJ tool keys",
        runner: mcp_github_copilot_key_v1::run_migration,
    },
];

/// Execute all registered database migrations in order.
///
/// Every app startup goes through this entry point so migration behavior stays
/// deterministic for both fresh installs and upgrades. Each migration is
/// responsible for deciding whether it is a true upgrade or a no-op.
pub async fn run_all_db_migrations(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(), String> {
    for migration in REGISTERED_MIGRATIONS {
        if has_migration(db, migration.id).await? {
            continue;
        }

        match (migration.runner)(db).await? {
            MigrationOutcome::Applied => {
                log::info!(
                    "Database migration applied: {} ({})",
                    migration.id,
                    migration.description
                );
            }
            MigrationOutcome::SkippedNoOp => {
                log::info!(
                    "Database migration skipped as no-op: {} ({})",
                    migration.id,
                    migration.description
                );
            }
        }
    }

    Ok(())
}

pub async fn has_migration(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    migration_id: &str,
) -> Result<bool, String> {
    let record_id = migration_record_id(migration_id);
    let result: Result<Vec<Value>, _> = db
        .query(format!("SELECT * FROM {} LIMIT 1", record_id))
        .await
        .map_err(|error| format!("Failed to query migration marker '{}': {}", migration_id, error))?
        .take(0);

    Ok(result.map(|records| !records.is_empty()).unwrap_or(false))
}

pub async fn mark_migration_applied(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    migration_id: &str,
    status: &str,
) -> Result<(), String> {
    let migration_id_owned = migration_id.to_string();
    let status_owned = status.to_string();
    db.query(format!(
        "UPSERT {} CONTENT {{ migration_id: $migration_id, status: $status, applied_at: time::now() }}",
        migration_record_id(migration_id)
    ))
    .bind(("migration_id", migration_id_owned))
    .bind(("status", status_owned))
    .await
    .map_err(|error| format!("Failed to write migration marker '{}': {}", migration_id, error))?;

    Ok(())
}

pub fn migration_record_id(migration_id: &str) -> String {
    format!("app_migration:`{}`", migration_id)
}

pub async fn count_records(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    table_name: &str,
) -> Result<i64, String> {
    let result: Result<Vec<Value>, _> = db
        .query(format!("SELECT count() FROM {} GROUP ALL", table_name))
        .await
        .map_err(|error| format!("Failed to count records in {}: {}", table_name, error))?
        .take(0);

    Ok(result
        .ok()
        .and_then(|records| records.first().cloned())
        .and_then(|record| record.get("count").and_then(|value| value.as_i64()))
        .unwrap_or(0))
}

pub async fn load_table_records(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    table_name: &str,
) -> Result<Vec<Value>, String> {
    db.query(format!(
        "SELECT *, type::string(id) as id FROM {}",
        table_name
    ))
        .await
        .map_err(|error| format!("Failed to read records from {}: {}", table_name, error))?
        .take(0)
        .map_err(|error| format!("Failed to deserialize records from {}: {}", table_name, error))
}
