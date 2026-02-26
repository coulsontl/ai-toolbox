use std::path::Path;
use std::sync::Arc;
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;
use tokio::sync::Mutex;

pub struct DbState(pub Arc<Mutex<Surreal<surrealdb::engine::local::Db>>>);

/// clog 压缩阈值（字节）
const COMPACT_THRESHOLD: u64 = 1 * 1024 * 1024;

/// 检查 clog 是否需要压缩
pub fn needs_compact(db_path: &Path) -> bool {
    get_clog_dir_size(db_path) >= COMPACT_THRESHOLD
}

/// 安全压缩：通过 SurrealDB export/import 重建数据库。
/// 不使用 surrealkv 原生 compact（0.9.3 会损坏数据）。
///
/// 流程: open → export SurrealQL → close → 删除 DB → reopen → import
/// 返回新的 DB 连接。
pub async fn safe_compact(
    db: Surreal<surrealdb::engine::local::Db>,
    db_path: &Path,
) -> Result<Surreal<surrealdb::engine::local::Db>, String> {
    let clog_before = get_clog_dir_size(db_path);
    let before_mb = clog_before as f64 / 1024.0 / 1024.0;
    log::info!("开始安全压缩数据库 (clog: {:.1}MB)...", before_mb);

    // Export to temp file (placed in parent dir so it survives db dir deletion)
    let export_path = db_path.with_extension("surql.tmp");
    db.export(&export_path)
        .await
        .map_err(|e| format!("Export failed: {}", e))?;
    log::info!("数据库已导出到临时文件");

    // Close DB connection
    drop(db);

    // Delete database directory
    std::fs::remove_dir_all(db_path)
        .map_err(|e| format!("Failed to remove database directory: {}", e))?;
    std::fs::create_dir_all(db_path)
        .map_err(|e| format!("Failed to recreate database directory: {}", e))?;

    // Reopen fresh DB
    let db = Surreal::new::<SurrealKv>(db_path.to_path_buf())
        .await
        .map_err(|e| format!("Failed to reopen database: {}", e))?;
    db.use_ns("ai_toolbox")
        .use_db("main")
        .await
        .map_err(|e| format!("Failed to select ns/db: {}", e))?;

    // Import data
    db.import(&export_path)
        .await
        .map_err(|e| format!("Import failed: {}", e))?;

    // Clean up temp file
    let _ = std::fs::remove_file(&export_path);

    let clog_after = get_clog_dir_size(db_path);
    let after_mb = clog_after as f64 / 1024.0 / 1024.0;
    log::info!(
        "安全压缩完成: {:.1}MB → {:.1}MB (节省 {:.1}MB)",
        before_mb,
        after_mb,
        before_mb - after_mb
    );

    Ok(db)
}

/// 获取数据库 clog 目录的总大小（字节）
pub fn get_clog_dir_size(db_path: &Path) -> u64 {
    let clog_dir = db_path.join("clog");
    if !clog_dir.exists() {
        return 0;
    }
    get_dir_size(&clog_dir)
}

/// 递归计算目录大小
fn get_dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                total += get_dir_size(&entry_path);
            } else if let Ok(metadata) = entry.metadata() {
                total += metadata.len();
            }
        }
    }
    total
}
