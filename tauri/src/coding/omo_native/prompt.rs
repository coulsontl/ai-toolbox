//! 全局提示词命令层。
//!
//! 运行时产物是引擎状态目录里的 `AGENTS.md`（默认 `~/.omo/agent/AGENTS.md`），
//! 由 senpi 引擎作为项目规则文件读取。命令集合与 Pi / OMP / ZCode 完全同构，
//! 前端因此可以复用共享组件 `GlobalPromptSettings`。
//!
//! 与 `commands.rs` 的分工：那里管 `[native]` 块与 provider，这里只管提示词预设。

use chrono::Local;
use serde_json::json;
use std::path::PathBuf;
use tauri::Emitter;

use super::constants;
use super::types::{OmoNativePromptConfig, OmoNativePromptConfigContent, OmoNativePromptConfigInput};
use crate::coding::db_id::db_new_id;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::runtime_location;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{Cli, CliConfigKind, DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;

const PRODUCT_NAME: &str = "OmO";

/// 全局提示词文件路径（`<agentDir>/AGENTS.md`）。
pub async fn get_omo_native_prompt_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    get_prompt_path_async(db).await
}

async fn get_prompt_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    let location = runtime_location::get_omo_native_runtime_location_async(db).await?;
    Ok(location.host_path.join(constants::OMO_NATIVE_PROMPT_FILE))
}

fn emit_config_changed<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());
}

fn prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?]))
}

fn put_prompt_to_sqlite(
    db: &SqliteDbState,
    id: &str,
    content: &OmoNativePromptConfigContent,
) -> Result<(), String> {
    let value = super::adapter::prompt_to_db_value(content);
    db.with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt), id, &value))
}

fn get_prompt_from_sqlite(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<OmoNativePromptConfig>, String> {
    Ok(db
        .with_conn(|conn| db_get(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt), id))?
        .map(super::adapter::prompt_from_db_value))
}

/// 本地 `AGENTS.md` 的只读桥接态：库里没有已应用预设时，把磁盘上那份
/// 现成内容当成一条虚拟记录展示，用户可以直接「收编」成预设。
async fn get_local_prompt_config(db: &SqliteDbState) -> Result<Option<OmoNativePromptConfig>, String> {
    let prompt_path = get_prompt_path_async(db).await?;
    if !prompt_path.exists() {
        return Ok(None);
    }
    let Some(content) = read_prompt_content_file(&prompt_path, PRODUCT_NAME)? else {
        return Ok(None);
    };
    Ok(Some(OmoNativePromptConfig {
        id: crate::coding::local_bridge::LOCAL_CONFIG_ID.to_string(),
        // 与多数 CLI 一致（antigravity / claude_code / claude_desktop /
        // gemini_cli / oh_my_openagent / open_code 都叫 `default`）。卡片的副标题
        // 已经写了「来自本地 AGENTS.md」，名字里再带一次文件名是重复。
        name: "default".to_string(),
        content,
        is_applied: false,
        sort_index: Some(-1),
        created_at: None,
        updated_at: None,
    }))
}

async fn write_prompt_content_to_file(
    db: &SqliteDbState,
    content: Option<&str>,
) -> Result<(), String> {
    let path = get_prompt_path_async(db).await?;
    write_prompt_content_file(&path, content, PRODUCT_NAME)
}

#[tauri::command]
pub async fn list_omo_native_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OmoNativePromptConfig>, String> {
    let db = state.db();
    let mut prompts = db.with_conn(|conn| {
        Ok(
            db_list(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt), Some(&prompt_order()?))?
                .into_iter()
                .map(super::adapter::prompt_from_db_value)
                .collect::<Vec<_>>(),
        )
    })?;
    if !prompts.iter().any(|prompt| prompt.is_applied) {
        if let Some(local_prompt) = get_local_prompt_config(&db).await? {
            prompts.insert(0, local_prompt);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_omo_native_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativePromptConfigInput,
) -> Result<OmoNativePromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let next_sort_index = db.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt),
            &JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;
    let content = OmoNativePromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };
    let prompt_id = db_new_id();
    put_prompt_to_sqlite(&db, &prompt_id, &content)?;
    emit_config_changed(&app);
    Ok(super::adapter::prompt_from_db_value(json!({
        "id": prompt_id,
        "name": content.name,
        "content": content.content,
        "is_applied": content.is_applied,
        "sort_index": content.sort_index,
        "created_at": content.created_at,
        "updated_at": content.updated_at
    })))
}

#[tauri::command]
pub async fn update_omo_native_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativePromptConfigInput,
) -> Result<OmoNativePromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let existing = get_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;
    let content = OmoNativePromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied: existing.is_applied,
        sort_index: existing.sort_index,
        created_at: existing.created_at.unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    put_prompt_to_sqlite(&db, &config_id, &content)?;
    // 已应用的预设改了内容要同步落盘，否则界面与磁盘上的 `AGENTS.md` 会脱节。
    if existing.is_applied {
        write_prompt_content_to_file(&db, Some(input.content.as_str())).await?;
    }
    emit_config_changed(&app);
    get_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found after update", config_id))
}

#[tauri::command]
pub async fn delete_omo_native_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    // 只删数据库记录，保留磁盘上的 `AGENTS.md`——与其余 CLI 一致：
    // 删一条预设不应该顺手清掉用户正在用的运行时文件。
    let db = state.db();
    db.with_conn(|conn| db_delete(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt), &id).map(|_| ()))?;
    emit_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn apply_omo_native_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_omo_native_prompt_config_internal(&state.db(), &config_id).await?;
    emit_config_changed(&app);
    Ok(())
}

/// 托盘用的版本：接受泛型 `AppHandle<R>`（`#[tauri::command]` 只收具体的
/// `AppHandle`，托盘那边拿到的是 `AppHandle<R>`）。写盘逻辑与命令版共用。
pub async fn apply_omo_native_prompt_config_from_tray<R: tauri::Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_omo_native_prompt_config_internal(db, config_id).await?;
    let _ = tauri::Emitter::emit(app, "config-changed", "omo_native");
    Ok(())
}

/// 恢复备份时用的无事件版本：写盘与标记照做，但**不发** `config-changed` /
/// WSL 同步事件——恢复流程自己会广播一次总事件，逐条再发会让前端在恢复中途
/// 读到半成品状态。
pub async fn apply_omo_native_prompt_config_internal_without_events(
    db: &SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    apply_omo_native_prompt_config_internal(db, config_id).await
}

async fn apply_omo_native_prompt_config_internal(
    db: &SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    if config_id == crate::coding::local_bridge::LOCAL_CONFIG_ID {
        let local_prompt = get_local_prompt_config(db)
            .await?
            .ok_or_else(|| "Local OmO prompt not found".to_string())?;
        return write_prompt_content_to_file(db, Some(local_prompt.content.as_str())).await;
    }

    let prompt = get_prompt_from_sqlite(db, config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt), Some(config_id), &now)
    })?;
    write_prompt_content_to_file(db, Some(prompt.content.as_str())).await
}

/// 停用已应用的预设：清掉全部 applied 标记并清空运行时文件，记录本身保留。
#[tauri::command]
pub async fn disable_omo_native_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    get_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt), None, &now)
    })?;
    write_prompt_content_to_file(&db, Some("")).await?;
    emit_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn reorder_omo_native_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Prompt),
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    emit_config_changed(&app);
    Ok(())
}

/// 把本地 `AGENTS.md` 收编成一条预设并立即应用。
#[tauri::command]
pub async fn save_omo_native_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativePromptConfigInput,
) -> Result<OmoNativePromptConfig, String> {
    let db = state.db();
    // 前端提交空内容（用户只想把本地文件存成预设）时，回读磁盘上的原文，
    // 而不是写出一条空预设。
    let content = if input.content.trim().is_empty() {
        get_local_prompt_config(&db)
            .await?
            .map(|prompt| prompt.content)
            .unwrap_or_default()
    } else {
        input.content
    };
    let created = create_omo_native_prompt_config(
        state.clone(),
        app.clone(),
        OmoNativePromptConfigInput {
            id: None,
            name: input.name,
            content,
        },
    )
    .await?;
    apply_omo_native_prompt_config(state.clone(), app.clone(), created.id.clone()).await?;
    Ok(get_prompt_from_sqlite(state.db(), &created.id)?.unwrap_or(created))
}
