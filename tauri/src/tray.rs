//! System Tray Module
//!
//! Provides system tray icon and menu with flat structure:
//! - Open Main Window
//! - ─── OpenCode ────
//! - 主模型 / 小模型 (with submenus for model selection)
//! - ─── OpenCode 插件 ────
//! - Plugin options (with checkmarks for enabled plugins)
//! - ─── Oh My OpenCode ───
//! - Config options (with checkmarks for applied config)
//! - ─── Claude Code ───
//! - Provider options (with checkmarks for applied provider)
//! - ─── MCP Servers ───
//! - MCP server options (with submenus for tool selection)
//! - Quit

use crate::coding::claude_code::tray_support as claude_tray;
use crate::coding::codex::tray_support as codex_tray;
use crate::coding::mcp::tray_support as mcp_tray;
use crate::coding::oh_my_opencode::tray_support as omo_tray;
use crate::coding::oh_my_opencode_slim::tray_support as omo_slim_tray;
use crate::coding::open_claw::tray_support as openclaw_tray;
use crate::coding::open_code::tray_support as opencode_tray;
use crate::coding::skills::tray_support as skills_tray;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};

#[derive(Clone, Copy)]
struct TrayTexts {
    show_window: &'static str,
    quit: &'static str,
    main_model: &'static str,
    small_model: &'static str,
    global_prompt: &'static str,
    opencode_header: &'static str,
    opencode_plugins_header: &'static str,
    omo_header: &'static str,
    omo_slim_header: &'static str,
    claude_header: &'static str,
    codex_header: &'static str,
    openclaw_header: &'static str,
    skills_header: &'static str,
    mcp_header: &'static str,
    no_config: &'static str,
    no_model: &'static str,
    no_tools: &'static str,
}

fn is_english_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("en-US") || language.to_ascii_lowercase().starts_with("en")
}

fn tray_texts(language: &str) -> TrayTexts {
    if is_english_language(language) {
        TrayTexts {
            show_window: "Open Main Window",
            quit: "Quit",
            main_model: "Main Model",
            small_model: "Small Model",
            global_prompt: "Global Prompt",
            opencode_header: "OpenCode",
            opencode_plugins_header: "OpenCode Plugins",
            omo_header: "Oh My OpenCode",
            omo_slim_header: "Oh My OpenCode Slim",
            claude_header: "Claude Code",
            codex_header: "Codex",
            openclaw_header: "OpenClaw",
            skills_header: "Skills",
            mcp_header: "MCP Servers",
            no_config: "  No configs",
            no_model: "  No models",
            no_tools: "  No tools",
        }
    } else {
        TrayTexts {
            show_window: "打开主界面",
            quit: "退出",
            main_model: "主模型",
            small_model: "小模型",
            global_prompt: "全局提示词",
            opencode_header: "OpenCode",
            opencode_plugins_header: "OpenCode 插件",
            omo_header: "Oh My OpenCode",
            omo_slim_header: "Oh My OpenCode Slim",
            claude_header: "Claude Code",
            codex_header: "Codex",
            openclaw_header: "OpenClaw",
            skills_header: "Skills",
            mcp_header: "MCP Servers",
            no_config: "  暂无配置",
            no_model: "  暂无模型",
            no_tools: "  暂无工具",
        }
    }
}

/// Prevents concurrent refresh_tray_menus execution
static TRAY_REFRESHING: AtomicBool = AtomicBool::new(false);
/// Signals that another refresh was requested during the current one
static TRAY_REFRESH_PENDING: AtomicBool = AtomicBool::new(false);
const TRAY_SHOW_MENU_ID: &str = "show";
const TRAY_QUIT_MENU_ID: &str = "app_quit";

fn request_app_exit<R: Runtime>(app: &AppHandle<R>) {
    crate::APP_EXIT_REQUESTED.store(true, Ordering::SeqCst);
    app.exit(0);
}

#[cfg(target_os = "macos")]
use tauri::image::Image;

#[cfg(target_os = "macos")]
fn macos_tray_icon() -> Option<Image<'static>> {
    const ICON_BYTES: &[u8] = include_bytes!("../icons/tray/macos/statusbar_template@3x.png");

    match Image::from_bytes(ICON_BYTES) {
        Ok(icon) => Some(icon),
        Err(err) => {
            log::warn!("Failed to load macOS tray icon: {err}");
            None
        }
    }
}

/// 命令：刷新托盘菜单
#[tauri::command]
pub async fn refresh_tray_menu<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    refresh_tray_menus(&app).await
}

/// Create system tray icon and menu
pub fn create_tray<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let texts = tauri::async_runtime::block_on(async {
        crate::settings::commands::get_settings(app.state())
            .await
            .map(|settings| tray_texts(&settings.language))
            .unwrap_or_else(|_| tray_texts("zh-CN"))
    });

    let quit_item = MenuItem::with_id(app, TRAY_QUIT_MENU_ID, texts.quit, true, None::<&str>)?;
    let show_item = MenuItem::with_id(
        app,
        TRAY_SHOW_MENU_ID,
        texts.show_window,
        true,
        None::<&str>,
    )?;

    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut tray_builder = TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(move |app, event| {
            let event_id = event.id().as_ref().to_string();

            if event_id == TRAY_SHOW_MENU_ID {
                // macOS: Switch back to Regular mode to show in Dock
                #[cfg(target_os = "macos")]
                {
                    use tauri::ActivationPolicy;
                    let _ = app.set_activation_policy(ActivationPolicy::Regular);
                }

                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            } else if event_id == TRAY_QUIT_MENU_ID {
                request_app_exit(app);
            } else if event_id.starts_with("omo_config_") {
                let config_id = event_id.strip_prefix("omo_config_").unwrap().to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        omo_tray::apply_oh_my_opencode_config(&app_handle, &config_id).await
                    {
                        eprintln!("Failed to apply Oh My OpenCode config: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("omo_slim_config_") {
                let config_id = event_id
                    .strip_prefix("omo_slim_config_")
                    .unwrap()
                    .to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        omo_slim_tray::apply_oh_my_opencode_slim_config(&app_handle, &config_id)
                            .await
                    {
                        eprintln!("Failed to apply Oh My OpenCode Slim config: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("claude_provider_") {
                let provider_id = event_id
                    .strip_prefix("claude_provider_")
                    .unwrap()
                    .to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        claude_tray::apply_claude_code_provider(&app_handle, &provider_id).await
                    {
                        eprintln!("Failed to apply Claude provider: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("claude_prompt_") {
                let config_id = event_id.strip_prefix("claude_prompt_").unwrap().to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        claude_tray::apply_claude_prompt_config(&app_handle, &config_id).await
                    {
                        eprintln!("Failed to apply Claude prompt config: {}", e);
                    }
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("opencode_model_") {
                // Parse: opencode_model_main|small_provider/model_id
                let remaining = event_id.strip_prefix("opencode_model_").unwrap();
                let (model_type, item_id) = remaining.split_once('_').unwrap();
                let model_type = model_type.to_string();
                let item_id = item_id.to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        opencode_tray::apply_opencode_model(&app_handle, &model_type, &item_id)
                            .await
                    {
                        eprintln!("Failed to apply OpenCode model: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("opencode_plugin_") {
                let plugin_name = event_id
                    .strip_prefix("opencode_plugin_")
                    .unwrap()
                    .to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        opencode_tray::apply_opencode_plugin(&app_handle, &plugin_name).await
                    {
                        eprintln!("Failed to apply OpenCode plugin: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("opencode_prompt_") {
                let config_id = event_id
                    .strip_prefix("opencode_prompt_")
                    .unwrap()
                    .to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        opencode_tray::apply_opencode_prompt_config(&app_handle, &config_id).await
                    {
                        eprintln!("Failed to apply OpenCode prompt config: {}", e);
                    }
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("codex_provider_") {
                let provider_id = event_id
                    .strip_prefix("codex_provider_")
                    .unwrap()
                    .to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        codex_tray::apply_codex_provider(&app_handle, &provider_id).await
                    {
                        eprintln!("Failed to apply Codex provider: {}", e);
                    }
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("codex_prompt_") {
                let config_id = event_id.strip_prefix("codex_prompt_").unwrap().to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        codex_tray::apply_codex_prompt_config(&app_handle, &config_id).await
                    {
                        eprintln!("Failed to apply Codex prompt config: {}", e);
                    }
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("openclaw_model_") {
                let item_id = event_id
                    .strip_prefix("openclaw_model_")
                    .unwrap()
                    .to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = openclaw_tray::apply_openclaw_model(&app_handle, &item_id).await
                    {
                        eprintln!("Failed to apply OpenClaw model: {}", e);
                    }
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("skill_tool_") {
                // Parse: skill_tool_{skill_id}\x01{tool_key}
                let remaining = event_id.strip_prefix("skill_tool_").unwrap();
                if let Some(sep_pos) = remaining.find('\x01') {
                    let skill_id = remaining[..sep_pos].to_string();
                    let tool_key = remaining[sep_pos + 1..].to_string();
                    let app_handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) =
                            skills_tray::apply_skills_tool_toggle(&app_handle, &skill_id, &tool_key)
                                .await
                        {
                            eprintln!("Failed to toggle skill tool: {}", e);
                        }
                        let _ = refresh_tray_menus(&app_handle).await;
                    });
                }
            } else if event_id.starts_with("mcp_tool_") {
                // Parse: mcp_tool_{server_id}\x01{tool_key}
                let remaining = event_id.strip_prefix("mcp_tool_").unwrap();
                if let Some(sep_pos) = remaining.find('\x01') {
                    let server_id = remaining[..sep_pos].to_string();
                    let tool_key = remaining[sep_pos + 1..].to_string();
                    let app_handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) =
                            mcp_tray::apply_mcp_tool_toggle(&app_handle, &server_id, &tool_key)
                                .await
                        {
                            eprintln!("Failed to toggle MCP tool: {}", e);
                        }
                        let _ = refresh_tray_menus(&app_handle).await;
                    });
                }
            }
        })
        // macOS: 左键点击也显示菜单（与右键行为一致）
        .show_menu_on_left_click(true);

    #[cfg(target_os = "macos")]
    {
        if let Some(icon) = macos_tray_icon() {
            tray_builder = tray_builder.icon(icon).icon_as_template(true);
        } else if let Some(icon) = app.default_window_icon() {
            log::warn!("Falling back to default window icon for tray");
            tray_builder = tray_builder.icon(icon.clone());
        } else {
            log::warn!("Failed to load macOS tray icon for tray");
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(icon) = app.default_window_icon() {
            tray_builder = tray_builder.icon(icon.clone());
        } else {
            log::warn!("Failed to get default window icon for tray");
        }
    }

    let _tray = tray_builder.build(app)?;

    // Store tray in app state for later updates
    app.manage(_tray);

    // Initial menu refresh
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = refresh_tray_menus(&app_clone).await;
    });

    Ok(())
}

/// Refresh tray menus with deduplication (coalescing pattern)
pub async fn refresh_tray_menus<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // If already refreshing, mark pending and return
    if TRAY_REFRESHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        TRAY_REFRESH_PENDING.store(true, Ordering::SeqCst);
        return Ok(());
    }

    loop {
        TRAY_REFRESH_PENDING.store(false, Ordering::SeqCst);
        let result = refresh_tray_menus_inner(app).await;

        if !TRAY_REFRESH_PENDING.load(Ordering::SeqCst) {
            TRAY_REFRESHING.store(false, Ordering::SeqCst);
            return result;
        }
        // A new request came in during refresh, loop once more
    }
}

/// Refresh tray menus with flat structure
async fn refresh_tray_menus_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let (visible_tabs, texts) = match crate::settings::commands::get_settings(app.state()).await {
        Ok(settings) => (settings.visible_tabs, tray_texts(&settings.language)),
        Err(err) => {
            log::warn!("Failed to read settings for tray visibility: {err}");
            (
                vec![
                    "opencode".to_string(),
                    "claudecode".to_string(),
                    "codex".to_string(),
                    "openclaw".to_string(),
                ],
                tray_texts("zh-CN"),
            )
        }
    };

    let is_tab_visible = |tab: &str| visible_tabs.iter().any(|item| item == tab);

    // Check if modules are enabled
    let opencode_enabled =
        is_tab_visible("opencode") && opencode_tray::is_enabled_for_tray(app).await;
    let omo_enabled = is_tab_visible("opencode") && omo_tray::is_enabled_for_tray(app).await;
    let omo_slim_enabled =
        is_tab_visible("opencode") && omo_slim_tray::is_enabled_for_tray(app).await;
    let claude_enabled =
        is_tab_visible("claudecode") && claude_tray::is_enabled_for_tray(app).await;
    let codex_enabled = is_tab_visible("codex") && codex_tray::is_enabled_for_tray(app).await;
    let openclaw_enabled =
        is_tab_visible("openclaw") && openclaw_tray::is_enabled_for_tray(app).await;
    let opencode_plugins_enabled =
        is_tab_visible("opencode") && opencode_tray::is_plugins_enabled_for_tray(app).await;
    let skills_enabled = skills_tray::is_skills_enabled_for_tray(app).await;

    // Get data from modules (only if enabled)
    let (mut main_model_data, mut small_model_data) = if opencode_enabled {
        opencode_tray::get_opencode_tray_model_data(app).await?
    } else {
        (
            opencode_tray::TrayModelData {
                title: texts.main_model.to_string(),
                current_display: String::new(),
                items: vec![],
            },
            opencode_tray::TrayModelData {
                title: texts.small_model.to_string(),
                current_display: String::new(),
                items: vec![],
            },
        )
    };
    main_model_data.title = texts.main_model.to_string();
    small_model_data.title = texts.small_model.to_string();

    let mut opencode_plugin_data = if opencode_plugins_enabled {
        opencode_tray::get_opencode_tray_plugin_data(app).await?
    } else {
        opencode_tray::TrayPluginData {
            title: texts.opencode_plugins_header.to_string(),
            items: vec![],
        }
    };
    opencode_plugin_data.title = texts.opencode_plugins_header.to_string();

    let mut opencode_prompt_data = if opencode_enabled {
        opencode_tray::get_opencode_prompt_tray_data(app).await?
    } else {
        opencode_tray::TrayPromptData {
            title: texts.global_prompt.to_string(),
            current_display: String::new(),
            items: vec![],
        }
    };
    opencode_prompt_data.title = texts.global_prompt.to_string();

    let mut omo_data = if omo_enabled {
        omo_tray::get_oh_my_opencode_tray_data(app).await?
    } else {
        omo_tray::TrayConfigData {
            title: texts.omo_header.to_string(),
            items: vec![],
        }
    };
    omo_data.title = texts.omo_header.to_string();

    let mut omo_slim_data = if omo_slim_enabled {
        omo_slim_tray::get_oh_my_opencode_slim_tray_data(app).await?
    } else {
        omo_slim_tray::TrayConfigData {
            title: texts.omo_slim_header.to_string(),
            items: vec![],
        }
    };
    omo_slim_data.title = texts.omo_slim_header.to_string();

    let mut claude_data = if claude_enabled {
        claude_tray::get_claude_code_tray_data(app).await?
    } else {
        claude_tray::TrayProviderData {
            title: texts.claude_header.to_string(),
            items: vec![],
        }
    };
    claude_data.title = texts.claude_header.to_string();

    let mut claude_prompt_data = if claude_enabled {
        claude_tray::get_claude_prompt_tray_data(app).await?
    } else {
        claude_tray::TrayPromptData {
            title: texts.global_prompt.to_string(),
            current_display: String::new(),
            items: vec![],
        }
    };
    claude_prompt_data.title = texts.global_prompt.to_string();

    let mut codex_data = if codex_enabled {
        codex_tray::get_codex_tray_data(app).await?
    } else {
        codex_tray::TrayProviderData {
            title: texts.codex_header.to_string(),
            items: vec![],
        }
    };
    codex_data.title = texts.codex_header.to_string();

    let mut codex_prompt_data = if codex_enabled {
        codex_tray::get_codex_prompt_tray_data(app).await?
    } else {
        codex_tray::TrayPromptData {
            title: texts.global_prompt.to_string(),
            current_display: String::new(),
            items: vec![],
        }
    };
    codex_prompt_data.title = texts.global_prompt.to_string();

    let mut openclaw_model_data = if openclaw_enabled {
        openclaw_tray::get_openclaw_tray_model_data(app).await?
    } else {
        openclaw_tray::TrayModelData {
            title: texts.main_model.to_string(),
            current_display: String::new(),
            items: vec![],
        }
    };
    openclaw_model_data.title = texts.main_model.to_string();

    let mut skills_data = if skills_enabled {
        skills_tray::get_skills_tray_data(app).await?
    } else {
        skills_tray::TraySkillData {
            title: texts.skills_header.to_string(),
            items: vec![],
        }
    };
    skills_data.title = texts.skills_header.to_string();
    let mcp_enabled = mcp_tray::is_mcp_enabled_for_tray(app).await;
    let mut mcp_data = if mcp_enabled {
        mcp_tray::get_mcp_tray_data(app).await?
    } else {
        mcp_tray::TrayMcpData {
            title: texts.mcp_header.to_string(),
            items: vec![],
        }
    };
    mcp_data.title = texts.mcp_header.to_string();

    // Build flat menu - all menu items created in same scope to ensure valid lifetime
    let quit_item = MenuItem::with_id(app, TRAY_QUIT_MENU_ID, texts.quit, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let show_item = MenuItem::with_id(
        app,
        TRAY_SHOW_MENU_ID,
        texts.show_window,
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let separator1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;

    // OpenCode Model section (only if enabled)
    let opencode_model_header = if opencode_enabled {
        Some(
            MenuItem::with_id(
                app,
                "opencode_model_header",
                texts.opencode_header,
                false,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    let main_model_submenu = if opencode_enabled {
        Some(build_model_submenu(app, &main_model_data, "main", texts).await?)
    } else {
        None
    };

    let small_model_submenu = if opencode_enabled {
        Some(build_model_submenu(app, &small_model_data, "small", texts).await?)
    } else {
        None
    };

    // OpenClaw model submenu (built early, before non-Send types)
    let openclaw_has_items = openclaw_enabled && !openclaw_model_data.items.is_empty();
    let openclaw_submenu = if openclaw_has_items {
        Some(build_openclaw_model_submenu(
            app,
            &openclaw_model_data,
            texts,
        )?)
    } else {
        None
    };

    // OpenCode Plugin section (only if enabled)
    let opencode_plugin_header =
        if opencode_plugins_enabled && !opencode_plugin_data.items.is_empty() {
            Some(
                MenuItem::with_id(
                    app,
                    "opencode_plugin_header",
                    &opencode_plugin_data.title,
                    false,
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };

    // Build OpenCode Plugin items
    let mut opencode_plugin_items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if opencode_plugins_enabled && !opencode_plugin_data.items.is_empty() {
        for item in opencode_plugin_data.items {
            let item_id = format!("opencode_plugin_{}", item.id);
            let menu_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
                CheckMenuItem::with_id(
                    app,
                    &item_id,
                    &item.display_name,
                    !item.is_disabled, // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,  // checked: 是否已启用
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?,
            );
            opencode_plugin_items.push(menu_item);
        }
    }

    let opencode_prompt_submenu = if opencode_enabled && !opencode_prompt_data.items.is_empty() {
        Some(build_prompt_submenu(app, &opencode_prompt_data, texts)?)
    } else {
        None
    };

    // Skills section (only if enabled)
    let skills_has_items = skills_enabled && !skills_data.items.is_empty();
    let skills_header = if skills_has_items {
        Some(
            MenuItem::with_id(
                app,
                "skills_header",
                &skills_data.title,
                false,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    // Build Skills submenus - each skill gets a submenu with tools as CheckMenuItems
    let mut skills_submenus: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if skills_has_items {
        for skill in skills_data.items {
            let skill_submenu = build_skill_submenu(app, &skill, texts)?;
            let boxed: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(skill_submenu);
            skills_submenus.push(boxed);
        }
    }

    // MCP section (only if enabled)
    let mcp_has_items = mcp_enabled && !mcp_data.items.is_empty();
    let mcp_header = if mcp_has_items {
        Some(
            MenuItem::with_id(app, "mcp_header", &mcp_data.title, false, None::<&str>)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    // Build MCP submenus - each server gets a submenu with tools as CheckMenuItems
    let mut mcp_submenus: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if mcp_has_items {
        for server in mcp_data.items {
            let mcp_submenu = build_mcp_submenu(app, &server, texts)?;
            let boxed: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(mcp_submenu);
            mcp_submenus.push(boxed);
        }
    }

    // Oh My OpenCode section (only if enabled)
    let omo_header = if omo_enabled {
        Some(
            MenuItem::with_id(app, "omo_header", &omo_data.title, false, None::<&str>)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    // Build Oh My OpenCode items
    let mut omo_items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if omo_enabled && omo_data.items.is_empty() {
        let empty_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
            MenuItem::with_id(app, "omo_empty", texts.no_config, false, None::<&str>)
                .map_err(|e| e.to_string())?,
        );
        omo_items.push(empty_item);
    } else if omo_enabled {
        for item in omo_data.items {
            let item_id = format!("omo_config_{}", item.id);
            let menu_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
                CheckMenuItem::with_id(
                    app,
                    &item_id,
                    &item.display_name,
                    !item.is_disabled, // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,  // checked: 是否已应用
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?,
            );
            omo_items.push(menu_item);
        }
    }

    // Oh My OpenCode Slim section (only if enabled)
    let omo_slim_header = if omo_slim_enabled {
        Some(
            MenuItem::with_id(
                app,
                "omo_slim_header",
                &omo_slim_data.title,
                false,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    // Build Oh My OpenCode Slim items
    let mut omo_slim_items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if omo_slim_enabled && omo_slim_data.items.is_empty() {
        let empty_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
            MenuItem::with_id(app, "omo_slim_empty", texts.no_config, false, None::<&str>)
                .map_err(|e| e.to_string())?,
        );
        omo_slim_items.push(empty_item);
    } else if omo_slim_enabled {
        for item in omo_slim_data.items {
            let item_id = format!("omo_slim_config_{}", item.id);
            let menu_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
                CheckMenuItem::with_id(
                    app,
                    &item_id,
                    &item.display_name,
                    !item.is_disabled, // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,  // checked: 是否已应用
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?,
            );
            omo_slim_items.push(menu_item);
        }
    }

    // Check if modules have items (must be done before consuming items in for loops)
    let claude_has_items = claude_enabled && !claude_data.items.is_empty();
    let codex_has_items = codex_enabled && !codex_data.items.is_empty();
    let claude_has_prompt_items = claude_enabled && !claude_prompt_data.items.is_empty();
    let codex_has_prompt_items = codex_enabled && !codex_prompt_data.items.is_empty();
    let claude_has_section = claude_enabled && (claude_has_items || claude_has_prompt_items);
    let codex_has_section = codex_enabled && (codex_has_items || codex_has_prompt_items);
    let claude_prompt_submenu = if claude_has_prompt_items {
        Some(build_named_prompt_submenu(
            app,
            "claude",
            &claude_prompt_data,
            texts,
        )?)
    } else {
        None
    };
    let codex_prompt_submenu = if codex_has_prompt_items {
        Some(build_named_prompt_submenu(
            app,
            "codex",
            &codex_prompt_data,
            texts,
        )?)
    } else {
        None
    };

    // Claude Code section (only if enabled and has items)
    let claude_header = if claude_has_section {
        Some(
            MenuItem::with_id(
                app,
                "claude_header",
                &claude_data.title,
                false,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    // Build Claude Code items (only if has items)
    let mut claude_items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if claude_has_items {
        for item in claude_data.items {
            let item_id = format!("claude_provider_{}", item.id);
            let menu_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
                CheckMenuItem::with_id(
                    app,
                    &item_id,
                    &item.display_name,
                    !item.is_disabled, // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,  // checked: 是否已应用
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?,
            );
            claude_items.push(menu_item);
        }
    }

    let codex_header = if codex_has_section {
        Some(
            MenuItem::with_id(app, "codex_header", &codex_data.title, false, None::<&str>)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    // Build Codex items (only if has items)
    let mut codex_items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if codex_has_items {
        for item in codex_data.items {
            let item_id = format!("codex_provider_{}", item.id);
            let menu_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
                CheckMenuItem::with_id(
                    app,
                    &item_id,
                    &item.display_name,
                    !item.is_disabled,
                    item.is_selected,
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?,
            );
            codex_items.push(menu_item);
        }
    }

    // OpenClaw section (only if enabled and has items)
    let openclaw_header = if openclaw_has_items {
        Some(
            MenuItem::with_id(
                app,
                "openclaw_header",
                texts.openclaw_header,
                false,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    // Combine all items into a flat menu
    let mut all_items: Vec<&dyn tauri::menu::IsMenuItem<R>> = Vec::new();
    all_items.push(&show_item);
    all_items.push(&separator1);

    // Add OpenCode section if enabled
    if let Some(ref header) = opencode_model_header {
        all_items.push(header);
    }
    if let Some(ref submenu) = main_model_submenu {
        all_items.push(submenu);
    }
    if let Some(ref submenu) = small_model_submenu {
        all_items.push(submenu);
    }
    if let Some(ref submenu) = opencode_prompt_submenu {
        all_items.push(submenu);
    }
    if let Some(ref header) = opencode_plugin_header {
        all_items.push(header);
    }
    for item in &opencode_plugin_items {
        all_items.push(item.as_ref());
    }
    // Add Skills section if enabled
    if let Some(ref header) = skills_header {
        all_items.push(header);
    }
    for item in &skills_submenus {
        all_items.push(item.as_ref());
    }
    // Add MCP section if enabled
    if let Some(ref header) = mcp_header {
        all_items.push(header);
    }
    for item in &mcp_submenus {
        all_items.push(item.as_ref());
    }
    // Add Oh My OpenCode section if enabled
    if let Some(ref header) = omo_header {
        all_items.push(header);
    }
    for item in &omo_items {
        all_items.push(item.as_ref());
    }
    // Add Oh My OpenCode Slim section if enabled
    if let Some(ref header) = omo_slim_header {
        all_items.push(header);
    }
    for item in &omo_slim_items {
        all_items.push(item.as_ref());
    }
    // Add Claude Code section if enabled
    if let Some(ref header) = claude_header {
        all_items.push(header);
    }
    if let Some(ref submenu) = claude_prompt_submenu {
        all_items.push(submenu);
    }
    for item in &claude_items {
        all_items.push(item.as_ref());
    }
    // Add Codex section if enabled
    if let Some(ref header) = codex_header {
        all_items.push(header);
    }
    if let Some(ref submenu) = codex_prompt_submenu {
        all_items.push(submenu);
    }
    for item in &codex_items {
        all_items.push(item.as_ref());
    }
    // Add OpenClaw section if enabled
    if let Some(ref header) = openclaw_header {
        all_items.push(header);
    }
    if let Some(ref submenu) = openclaw_submenu {
        all_items.push(submenu);
    }

    all_items.push(&separator1);
    all_items.push(&quit_item);

    let menu = Menu::with_items(app, &all_items).map_err(|e| e.to_string())?;

    // Update tray menu
    let tray = app.state::<tauri::tray::TrayIcon>();
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;

    Ok(())
}

/// Build a model selection submenu from tray data
async fn build_model_submenu<R: Runtime>(
    app: &AppHandle<R>,
    data: &opencode_tray::TrayModelData,
    model_type: &str, // "main" or "small"
    texts: TrayTexts,
) -> Result<Submenu<R>, String> {
    // Build title with current selection in parentheses
    let title = if data.current_display.is_empty() {
        data.title.clone()
    } else {
        format!("{} ({})", data.title, data.current_display)
    };
    let submenu_id = format!("{}_submenu", data.title);
    let submenu = Submenu::with_id(app, &submenu_id, &title, true).map_err(|e| e.to_string())?;

    if data.items.is_empty() {
        let empty_item = MenuItem::with_id(
            app,
            &format!("{}_empty", data.title),
            texts.no_model,
            false,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        submenu.append(&empty_item).map_err(|e| e.to_string())?;
    } else {
        // Group by provider so the tray menu is easier to scan.
        // - Parent submenu: 主模型/小模型
        // - 2nd level: provider name
        // - Leaf items: only model name (no "Provider / " prefix)
        let mut provider_map: std::collections::HashMap<
            String,                                       // provider_id
            (String, Vec<&opencode_tray::TrayModelItem>), // (provider_label, items)
        > = std::collections::HashMap::new();

        for item in &data.items {
            let provider_id = item.id.split('/').next().unwrap_or(&item.id).to_string();
            let provider_label = item
                .display_name
                .split(" / ")
                .next()
                .unwrap_or(&provider_id)
                .to_string();

            let entry = provider_map
                .entry(provider_id)
                .or_insert_with(|| (provider_label, Vec::new()));
            entry.1.push(item);
        }

        let mut providers: Vec<(String, String, Vec<&opencode_tray::TrayModelItem>)> = provider_map
            .into_iter()
            .map(|(provider_id, (provider_label, items))| (provider_id, provider_label, items))
            .collect();

        // Sort providers by display label for a stable, user-friendly order.
        providers.sort_by(|a, b| a.1.cmp(&b.1));

        for (provider_id, provider_label, mut items) in providers {
            // Sort models by their model label.
            items.sort_by(|a, b| {
                let a_model = a
                    .display_name
                    .split(" / ")
                    .nth(1)
                    .unwrap_or(&a.display_name);
                let b_model = b
                    .display_name
                    .split(" / ")
                    .nth(1)
                    .unwrap_or(&b.display_name);
                a_model.cmp(b_model)
            });

            let safe_provider_id: String = provider_id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();

            let provider_submenu_id = format!(
                "opencode_{}_provider_{}_submenu",
                model_type, safe_provider_id
            );

            let provider_submenu =
                Submenu::with_id(app, &provider_submenu_id, &provider_label, true)
                    .map_err(|e| e.to_string())?;

            for item in &items {
                let item_id = format!("opencode_model_{}_{}", model_type, item.id);
                let model_label = item
                    .display_name
                    .split(" / ")
                    .nth(1)
                    .unwrap_or(&item.display_name);

                let menu_item = CheckMenuItem::with_id(
                    app,
                    &item_id,
                    model_label,
                    true,
                    item.is_selected,
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?;

                provider_submenu
                    .append(&menu_item)
                    .map_err(|e| e.to_string())?;
            }

            submenu
                .append(&provider_submenu)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(submenu)
}

fn build_prompt_submenu<R: Runtime>(
    app: &AppHandle<R>,
    data: &opencode_tray::TrayPromptData,
    texts: TrayTexts,
) -> Result<Submenu<R>, String> {
    let title = if data.current_display.is_empty() {
        data.title.clone()
    } else {
        format!("{} ({})", data.title, data.current_display)
    };
    let submenu = Submenu::with_id(app, "opencode_prompt_submenu", &title, true)
        .map_err(|e| e.to_string())?;

    if data.items.is_empty() {
        let empty_item = MenuItem::with_id(
            app,
            "opencode_prompt_empty",
            texts.no_config,
            false,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        submenu.append(&empty_item).map_err(|e| e.to_string())?;
    } else {
        for item in &data.items {
            let item_id = format!("opencode_prompt_{}", item.id);
            let menu_item = CheckMenuItem::with_id(
                app,
                &item_id,
                &item.display_name,
                true,
                item.is_selected,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?;
            submenu.append(&menu_item).map_err(|e| e.to_string())?;
        }
    }

    Ok(submenu)
}

fn build_named_prompt_submenu<R: Runtime>(
    app: &AppHandle<R>,
    prefix: &str,
    data: &impl NamedPromptTrayData,
    texts: TrayTexts,
) -> Result<Submenu<R>, String> {
    let title = if data.current_display().is_empty() {
        data.title().to_string()
    } else {
        format!("{} ({})", data.title(), data.current_display())
    };
    let submenu = Submenu::with_id(app, format!("{}_prompt_submenu", prefix), &title, true)
        .map_err(|e| e.to_string())?;

    if data.items().is_empty() {
        let empty_item = MenuItem::with_id(
            app,
            format!("{}_prompt_empty", prefix),
            texts.no_config,
            false,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        submenu.append(&empty_item).map_err(|e| e.to_string())?;
    } else {
        for item in data.items() {
            let item_id = format!("{}_prompt_{}", prefix, item.id());
            let menu_item = CheckMenuItem::with_id(
                app,
                &item_id,
                item.display_name(),
                true,
                item.is_selected(),
                None::<&str>,
            )
            .map_err(|e| e.to_string())?;
            submenu.append(&menu_item).map_err(|e| e.to_string())?;
        }
    }

    Ok(submenu)
}

trait NamedPromptTrayItem {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn is_selected(&self) -> bool;
}

trait NamedPromptTrayData {
    type Item: NamedPromptTrayItem;

    fn title(&self) -> &str;
    fn current_display(&self) -> &str;
    fn items(&self) -> &[Self::Item];
}

impl NamedPromptTrayItem for claude_tray::TrayPromptItem {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn is_selected(&self) -> bool {
        self.is_selected
    }
}

impl NamedPromptTrayData for claude_tray::TrayPromptData {
    type Item = claude_tray::TrayPromptItem;

    fn title(&self) -> &str {
        &self.title
    }

    fn current_display(&self) -> &str {
        &self.current_display
    }

    fn items(&self) -> &[Self::Item] {
        &self.items
    }
}

impl NamedPromptTrayItem for codex_tray::TrayPromptItem {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn is_selected(&self) -> bool {
        self.is_selected
    }
}

impl NamedPromptTrayData for codex_tray::TrayPromptData {
    type Item = codex_tray::TrayPromptItem;

    fn title(&self) -> &str {
        &self.title
    }

    fn current_display(&self) -> &str {
        &self.current_display
    }

    fn items(&self) -> &[Self::Item] {
        &self.items
    }
}

/// Build a skill submenu with tool checkmarks
fn build_skill_submenu<R: Runtime>(
    app: &AppHandle<R>,
    skill: &skills_tray::TraySkillItem,
    texts: TrayTexts,
) -> Result<Submenu<R>, String> {
    let submenu_id = format!("skill_{}", skill.id);
    let submenu =
        Submenu::with_id(app, &submenu_id, &skill.display_name, true).map_err(|e| e.to_string())?;

    if skill.tools.is_empty() {
        let empty_item = MenuItem::with_id(
            app,
            &format!("skill_{}_empty", skill.id),
            texts.no_tools,
            false,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        submenu.append(&empty_item).map_err(|e| e.to_string())?;
    } else {
        for tool in &skill.tools {
            let item_id = format!("skill_tool_{}\x01{}", skill.id, tool.tool_key);
            let menu_item = CheckMenuItem::with_id(
                app,
                &item_id,
                &tool.display_name,
                tool.is_installed, // enabled only if tool is installed
                tool.is_synced,    // checked if synced
                None::<&str>,
            )
            .map_err(|e| e.to_string())?;
            submenu.append(&menu_item).map_err(|e| e.to_string())?;
        }
    }

    Ok(submenu)
}

/// Build an MCP server submenu with tool checkmarks
fn build_mcp_submenu<R: Runtime>(
    app: &AppHandle<R>,
    server: &mcp_tray::TrayMcpServerItem,
    texts: TrayTexts,
) -> Result<Submenu<R>, String> {
    let submenu_id = format!("mcp_{}", server.id);
    let submenu = Submenu::with_id(app, &submenu_id, &server.display_name, true)
        .map_err(|e| e.to_string())?;

    if server.tools.is_empty() {
        let empty_item = MenuItem::with_id(
            app,
            &format!("mcp_{}_empty", server.id),
            texts.no_tools,
            false,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        submenu.append(&empty_item).map_err(|e| e.to_string())?;
    } else {
        for tool in &server.tools {
            let item_id = format!("mcp_tool_{}\x01{}", server.id, tool.tool_key);
            let menu_item = CheckMenuItem::with_id(
                app,
                &item_id,
                &tool.display_name,
                tool.is_installed, // enabled only if tool is installed
                tool.is_enabled,   // checked if enabled
                None::<&str>,
            )
            .map_err(|e| e.to_string())?;
            submenu.append(&menu_item).map_err(|e| e.to_string())?;
        }
    }

    Ok(submenu)
}

/// Build an OpenClaw model selection submenu
fn build_openclaw_model_submenu<R: Runtime>(
    app: &AppHandle<R>,
    data: &openclaw_tray::TrayModelData,
    texts: TrayTexts,
) -> Result<Submenu<R>, String> {
    let title = if data.current_display.is_empty() {
        data.title.clone()
    } else {
        format!("{} ({})", data.title, data.current_display)
    };
    let submenu =
        Submenu::with_id(app, "openclaw_model_submenu", &title, true).map_err(|e| e.to_string())?;

    if data.items.is_empty() {
        let empty_item = MenuItem::with_id(
            app,
            "openclaw_model_empty",
            texts.no_model,
            false,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        submenu.append(&empty_item).map_err(|e| e.to_string())?;
    } else {
        // Group by provider for better readability.
        let mut provider_map: std::collections::HashMap<
            String,                                       // provider_id
            (String, Vec<&openclaw_tray::TrayModelItem>), // (provider_label, items)
        > = std::collections::HashMap::new();

        for item in &data.items {
            let provider_id = item.id.split('/').next().unwrap_or(&item.id).to_string();
            let provider_label = item
                .display_name
                .split(" / ")
                .next()
                .unwrap_or(&provider_id)
                .to_string();

            let entry = provider_map
                .entry(provider_id)
                .or_insert_with(|| (provider_label, Vec::new()));
            entry.1.push(item);
        }

        let mut providers: Vec<(String, String, Vec<&openclaw_tray::TrayModelItem>)> = provider_map
            .into_iter()
            .map(|(provider_id, (provider_label, items))| (provider_id, provider_label, items))
            .collect();

        providers.sort_by(|a, b| a.1.cmp(&b.1));

        for (provider_id, provider_label, mut items) in providers {
            items.sort_by(|a, b| {
                let a_model = a
                    .display_name
                    .split(" / ")
                    .nth(1)
                    .unwrap_or(&a.display_name);
                let b_model = b
                    .display_name
                    .split(" / ")
                    .nth(1)
                    .unwrap_or(&b.display_name);
                a_model.cmp(b_model)
            });

            let safe_provider_id: String = provider_id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();

            let provider_submenu_id = format!("openclaw_provider_{}_submenu", safe_provider_id);

            let provider_submenu =
                Submenu::with_id(app, &provider_submenu_id, &provider_label, true)
                    .map_err(|e| e.to_string())?;

            for item in &items {
                let item_id = format!("openclaw_model_{}", item.id);
                let model_label = item
                    .display_name
                    .split(" / ")
                    .nth(1)
                    .unwrap_or(&item.display_name);

                let menu_item = CheckMenuItem::with_id(
                    app,
                    &item_id,
                    model_label,
                    true,
                    item.is_selected,
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?;

                provider_submenu
                    .append(&menu_item)
                    .map_err(|e| e.to_string())?;
            }

            submenu
                .append(&provider_submenu)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(submenu)
}
