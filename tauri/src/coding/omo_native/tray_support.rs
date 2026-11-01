//! 托盘菜单数据源。
//!
//! 两个区块，与 Pi 同形：
//! - **默认模型**：`settings.json` 的 `defaultProvider` / `defaultModel`；
//! - **全局提示词**：`omo_native_prompt_config` 表的方案切换。
//!
//! ⚠️ 早先这里只有「方案切换」，理由写的是「Native 的默认模型由 model_profile 与
//! agents 共同决定，没有单一的全局默认模型字段」。**那是错的**：引擎
//! `docs/settings.md` 的 "Model & Thinking" 一节里 `defaultProvider` /
//! `defaultModel` 就是启动时选中的那对，`/model` 里 Ctrl+S 存的就是它们
//! （2026-10-07 用户指出托盘缺这两项，与 Pi 对齐后补上）。
//!
//! ⚠️ **Agent·Category 方案切换已从托盘撤下**（2026-10-07 用户要求「和 pi 一样，
//! 只有默认模型和全局提示词 2 个选项」）。本文件下半部分的 `TrayAgentsConfigItem` /
//! `TrayAgentsConfigData` / `get_omo_native_tray_data` / `apply_omo_native_agents_config`
//! **当前没有 `tray.rs` 调用点**，但**不要删**：`omo_native_agents_config` 表、
//! 全套 `*_agents_config` 命令、备份恢复与 `reapply_applied_runtime::reapply_omo_native`
//! 仍在读写同一份数据（先例见 `oh_my_pi/AGENTS.md` 对 `listOmpAgents` 的同类说明）。

use tauri::{AppHandle, Emitter, Manager, Runtime};
use crate::db::schema::{Cli, CliConfigKind};

/// 无托盘调用点，保留给「方案编辑界面回归」或其它消费者。见模块文档。
#[derive(Debug, Clone)]
pub struct TrayAgentsConfigItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
    /// Disabled configs stay listed (greyed out) rather than disappearing, so a
    /// config the user switched off in the window is still reachable from the
    /// tray — same treatment as the Oh My OpenAgent section.
    pub is_disabled: bool,
}

#[derive(Debug, Clone)]
pub struct TrayAgentsConfigData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayAgentsConfigItem>,
}

impl TrayAgentsConfigData {
    pub fn empty(title: &str) -> Self {
        Self {
            title: title.to_string(),
            current_display: String::new(),
            items: vec![],
        }
    }
}

// ============================================================================
// 默认模型
// ============================================================================

#[derive(Debug, Clone)]
pub struct TrayModelItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
}

#[derive(Debug, Clone)]
pub struct TrayModelData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayModelItem>,
}

impl TrayModelData {
    pub fn empty(title: &str) -> Self {
        Self {
            title: title.to_string(),
            current_display: String::new(),
            items: vec![],
        }
    }
}

/// 默认模型区块：`settings.json` 的 `defaultProvider` / `defaultModel`。
///
/// 候选项来自 `models.json` 的**全部** provider（自定义 + 覆盖内建）——与 Pi 的
/// 托盘一致，那里也是把 `models.json` 里所有 provider 的模型铺平。
///
/// ⚠️ **不做 `omo auth check` 过滤**：那要逐个起进程（48 个约 8 秒），托盘每次
/// 刷新都等不起。Pi 的托盘同样不过滤（`get_pi_tray_data` 只按 `warnings` 灰显）。
/// 没配凭据的渠道点了会失败，但那是引擎会报的错，比一个要等 8 秒的菜单好。
pub async fn get_omo_native_tray_model_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayModelData, String> {
    let state = app.state::<crate::db::SqliteDbState>();
    let config = super::commands::read_omo_native_runtime_config(
        app.state::<crate::db::SqliteDbState>(),
    )
    .await?;
    let current_provider = config
        .model_settings
        .provider_key
        .clone()
        .unwrap_or_default();
    let current_model = config.model_settings.model_id.clone().unwrap_or_default();

    let providers = super::providers::list_omo_native_providers(state).await?;
    let mut items = Vec::new();
    for provider in providers {
        // 只有 `models.json` 里的条目带模型目录；纯内建 provider 的目录要跑
        // `omo --list-models` 才有，托盘不做那次探测（见函数文档）。
        let model_ids = super::providers::model_ids_from_config(&provider.config);
        let display_name = super::providers::display_name_for(&provider);
        for model_id in model_ids {
            items.push(TrayModelItem {
                id: format!("{}/{}", provider.key, model_id),
                display_name: format!("{display_name} / {model_id}"),
                is_selected: provider.key == current_provider && model_id == current_model,
            });
        }
        // 当前选中的模型可能不在目录里（手写的、或渠道被删了）：补一条，
        // 否则菜单上看不出当前用的是什么。
        if provider.key == current_provider
            && !current_model.trim().is_empty()
            && !items.iter().any(|item| item.is_selected)
        {
            items.push(TrayModelItem {
                id: format!("{}/{}", provider.key, current_model),
                display_name: format!("{display_name} / {current_model}"),
                is_selected: true,
            });
        }
    }

    items.sort_by(|left, right| {
        right
            .is_selected
            .cmp(&left.is_selected)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    let current_display = if current_model.trim().is_empty() {
        String::new()
    } else {
        items
            .iter()
            .find(|item| item.is_selected)
            .map(|item| item.display_name.clone())
            .unwrap_or_else(|| current_model.clone())
    };

    Ok(TrayModelData {
        title: "默认模型".to_string(),
        current_display,
        items,
    })
}

/// 从托盘应用一个默认模型（`<provider>/<model>`）。
pub async fn apply_omo_native_model<R: Runtime>(
    app: &AppHandle<R>,
    provider_key: &str,
    model_id: &str,
) -> Result<(), String> {
    let state = app.state::<crate::db::SqliteDbState>();
    super::commands::save_omo_native_model_settings_from_tray(
        &state.db(),
        app,
        &super::types::OmoNativeModelSettingsInput {
            default_provider: Some(provider_key.to_string()),
            default_model: Some(model_id.to_string()),
            // 换模型时思考级别属于旧模型，清掉让引擎按新模型的默认来。
            default_thinking_level: Some(String::new()),
        },
    )
    .await
}

// ============================================================================
// 全局提示词
// ============================================================================

#[derive(Debug, Clone)]
pub struct TrayPromptItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
}

#[derive(Debug, Clone)]
pub struct TrayPromptData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayPromptItem>,
}

impl TrayPromptData {
    pub fn empty(title: &str) -> Self {
        Self {
            title: title.to_string(),
            current_display: String::new(),
            items: vec![],
        }
    }
}

pub async fn get_omo_native_prompt_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayPromptData, String> {
    let configs = super::prompt::list_omo_native_prompt_configs(app.state()).await?;
    let items = configs
        .into_iter()
        .filter(|config| config.id != crate::coding::local_bridge::LOCAL_CONFIG_ID)
        .map(|config| TrayPromptItem {
            id: config.id,
            display_name: config.name,
            is_selected: config.is_applied,
        })
        .collect::<Vec<_>>();

    let current_display = items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default();

    Ok(TrayPromptData {
        title: "全局提示词".to_string(),
        current_display,
        items,
    })
}

pub async fn apply_omo_native_prompt_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    let state = app.state::<crate::db::SqliteDbState>();
    super::prompt::apply_omo_native_prompt_config_from_tray(&state.db(), app, config_id).await
}

pub async fn get_omo_native_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayAgentsConfigData, String> {
    let configs = super::commands::list_omo_native_agents_configs(app.state()).await?;
    let items = configs
        .into_iter()
        // `__local__` is the read-only bridge state for a `[native]` block that
        // was never saved from this app; it cannot be applied, so it stays out
        // of the tray. Disabled configs are kept (greyed out in the menu).
        .filter(|config| config.id != crate::coding::local_bridge::LOCAL_CONFIG_ID)
        .map(|config| TrayAgentsConfigItem {
            id: config.id,
            display_name: config.name,
            is_selected: config.is_applied,
            is_disabled: config.is_disabled,
        })
        .collect::<Vec<_>>();

    // The section title is re-stamped by the tray with its own wording (see
    // `texts.omo_native_header`); this is only the fallback for direct callers.
    Ok(TrayAgentsConfigData {
        title: "OmO".to_string(),
        current_display: String::new(),
        items,
    })
}

pub async fn apply_omo_native_agents_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    let state = app.state::<crate::db::SqliteDbState>();
    let db = state.db();
    super::commands::write_agents_config_to_file(&db, config_id).await?;
    let now = chrono::Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        crate::db::helpers::db_update_applied_status(
            conn,
            crate::db::schema::DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents),
            Some(config_id),
            &now,
        )
    })?;
    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());
    Ok(())
}

pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
}
