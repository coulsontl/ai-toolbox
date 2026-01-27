//! System Tray Module
//!
//! Provides system tray icon and menu with flat structure:
//! - Open Main Window
//! - ─── OpenCode 模型 ────
//! - 主模型 / 小模型 (with submenus for model selection)
//! - ─── OpenCode 插件 ────
//! - Plugin options (with checkmarks for enabled plugins)
//! - ─── Oh My OpenCode ───
//! - Config options (with checkmarks for applied config)
//! - ─── Claude Code ───
//! - Provider options (with checkmarks for applied provider)
//! - Quit

use crate::coding::open_code::tray_support as opencode_tray;
use crate::coding::oh_my_opencode::tray_support as omo_tray;
use crate::coding::oh_my_opencode_slim::tray_support as omo_slim_tray;
use crate::coding::claude_code::tray_support as claude_tray;
use crate::coding::codex::tray_support as codex_tray;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

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
    let quit_item = PredefinedMenuItem::quit(app, Some("退出"))?;
    let show_item = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut tray_builder = TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(move |app, event| {
            let event_id = event.id().as_ref().to_string();

            if event_id == "show" {
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
            } else if event_id.starts_with("omo_config_") {
                let config_id = event_id.strip_prefix("omo_config_").unwrap().to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = omo_tray::apply_oh_my_opencode_config(&app_handle, &config_id).await {
                        eprintln!("Failed to apply Oh My OpenCode config: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("omo_slim_config_") {
                let config_id = event_id.strip_prefix("omo_slim_config_").unwrap().to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = omo_slim_tray::apply_oh_my_opencode_slim_config(&app_handle, &config_id).await {
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
                    if let Err(e) = claude_tray::apply_claude_code_provider(&app_handle, &provider_id).await {
                        eprintln!("Failed to apply Claude provider: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
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
                    if let Err(e) = opencode_tray::apply_opencode_model(&app_handle, &model_type, &item_id).await {
                        eprintln!("Failed to apply OpenCode model: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("opencode_plugin_") {
                let plugin_name = event_id.strip_prefix("opencode_plugin_").unwrap().to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = opencode_tray::apply_opencode_plugin(&app_handle, &plugin_name).await {
                        eprintln!("Failed to apply OpenCode plugin: {}", e);
                    }
                    // Refresh tray menu to update checkmarks
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            } else if event_id.starts_with("codex_provider_") {
                let provider_id = event_id
                    .strip_prefix("codex_provider_")
                    .unwrap()
                    .to_string();
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = codex_tray::apply_codex_provider(&app_handle, &provider_id).await {
                        eprintln!("Failed to apply Codex provider: {}", e);
                    }
                    let _ = refresh_tray_menus(&app_handle).await;
                });
            }
        })
        // macOS: 左键点击也显示菜单（与右键行为一致）
        .show_menu_on_left_click(true)
        .on_tray_icon_event(move |tray, event| {
            let app = tray.app_handle().clone();

            // 在菜单显示前刷新菜单以确保显示最新的选中状态
            if matches!(event, TrayIconEvent::Click { .. }) {
                tauri::async_runtime::block_on(async {
                    let _ = refresh_tray_menus(&app);
                });
            }
        });

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

/// Refresh tray menus with flat structure
pub async fn refresh_tray_menus<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // Check if modules are enabled
    let opencode_enabled = opencode_tray::is_enabled_for_tray(app).await;
    let omo_enabled = omo_tray::is_enabled_for_tray(app).await;
    let omo_slim_enabled = omo_slim_tray::is_enabled_for_tray(app).await;
    let claude_enabled = claude_tray::is_enabled_for_tray(app).await;
    let codex_enabled = codex_tray::is_enabled_for_tray(app).await;
    let opencode_plugins_enabled = opencode_tray::is_plugins_enabled_for_tray(app).await;

    // Get data from modules (only if enabled)
    let (main_model_data, small_model_data) = if opencode_enabled {
        opencode_tray::get_opencode_tray_model_data(app).await?
    } else {
        (
            opencode_tray::TrayModelData { title: "主模型".to_string(), current_display: String::new(), items: vec![] },
            opencode_tray::TrayModelData { title: "小模型".to_string(), current_display: String::new(), items: vec![] },
        )
    };
    let opencode_plugin_data = if opencode_plugins_enabled {
        opencode_tray::get_opencode_tray_plugin_data(app).await?
    } else {
        opencode_tray::TrayPluginData { title: "──── OpenCode 插件 ────".to_string(), items: vec![] }
    };
    let omo_data = if omo_enabled {
        omo_tray::get_oh_my_opencode_tray_data(app).await?
    } else {
        omo_tray::TrayConfigData { title: "──── Oh My OpenCode ────".to_string(), items: vec![] }
    };
    let omo_slim_data = if omo_slim_enabled {
        omo_slim_tray::get_oh_my_opencode_slim_tray_data(app).await?
    } else {
        omo_slim_tray::TrayConfigData { title: "──── Oh My OpenCode Slim ────".to_string(), items: vec![] }
    };
    let claude_data = if claude_enabled {
        claude_tray::get_claude_code_tray_data(app).await?
    } else {
        claude_tray::TrayProviderData { title: "──── Claude Code ────".to_string(), items: vec![] }
    };
    let codex_data = if codex_enabled {
        codex_tray::get_codex_tray_data(app).await?
    } else {
        codex_tray::TrayProviderData { title: "──── Codex ────".to_string(), items: vec![] }
    };

    // Build flat menu - all menu items created in same scope to ensure valid lifetime
    let quit_item = PredefinedMenuItem::quit(app, Some("退出")).map_err(|e| e.to_string())?;
    let show_item = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let separator1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;

    // OpenCode Model section (only if enabled)
    let opencode_model_header = if opencode_enabled {
        Some(MenuItem::with_id(app, "opencode_model_header", "──── OpenCode 模型 ────", false, None::<&str>)
            .map_err(|e| e.to_string())?)
    } else {
        None
    };

    let main_model_submenu = if opencode_enabled {
        Some(build_model_submenu(app, &main_model_data, "main").await?)
    } else {
        None
    };

    let small_model_submenu = if opencode_enabled {
        Some(build_model_submenu(app, &small_model_data, "small").await?)
    } else {
        None
    };

    // OpenCode Plugin section (only if enabled)
    let opencode_plugin_header = if opencode_plugins_enabled && !opencode_plugin_data.items.is_empty() {
        Some(MenuItem::with_id(app, "opencode_plugin_header", &opencode_plugin_data.title, false, None::<&str>)
            .map_err(|e| e.to_string())?)
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
                    !item.is_disabled,  // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,   // checked: 是否已启用
                    None::<&str>
                )
                .map_err(|e| e.to_string())?,
            );
            opencode_plugin_items.push(menu_item);
        }
    }

    // Oh My OpenCode section (only if enabled)
    let omo_header = if omo_enabled {
        Some(MenuItem::with_id(app, "omo_header", &omo_data.title, false, None::<&str>)
            .map_err(|e| e.to_string())?)
    } else {
        None
    };

    // Build Oh My OpenCode items
    let mut omo_items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if omo_enabled && omo_data.items.is_empty() {
        let empty_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
            MenuItem::with_id(app, "omo_empty", "  暂无配置", false, None::<&str>)
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
                    !item.is_disabled,  // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,   // checked: 是否已应用
                    None::<&str>
                )
                .map_err(|e| e.to_string())?,
            );
            omo_items.push(menu_item);
        }
    }

    // Oh My OpenCode Slim section (only if enabled)
    let omo_slim_header = if omo_slim_enabled {
        Some(MenuItem::with_id(app, "omo_slim_header", &omo_slim_data.title, false, None::<&str>)
            .map_err(|e| e.to_string())?)
    } else {
        None
    };

    // Build Oh My OpenCode Slim items
    let mut omo_slim_items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    if omo_slim_enabled && omo_slim_data.items.is_empty() {
        let empty_item: Box<dyn tauri::menu::IsMenuItem<R>> = Box::new(
            MenuItem::with_id(app, "omo_slim_empty", "  暂无配置", false, None::<&str>)
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
                    !item.is_disabled,  // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,   // checked: 是否已应用
                    None::<&str>
                )
                .map_err(|e| e.to_string())?,
            );
            omo_slim_items.push(menu_item);
        }
    }

    // Check if modules have items (must be done before consuming items in for loops)
    let claude_has_items = claude_enabled && !claude_data.items.is_empty();
    let codex_has_items = codex_enabled && !codex_data.items.is_empty();

    // Claude Code section (only if enabled and has items)
    let claude_header = if claude_has_items {
        Some(MenuItem::with_id(app, "claude_header", &claude_data.title, false, None::<&str>)
            .map_err(|e| e.to_string())?)
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
                    !item.is_disabled,  // enabled: 如果 is_disabled=true，则 enabled=false
                    item.is_selected,   // checked: 是否已应用
                    None::<&str>
                )
                .map_err(|e| e.to_string())?,
            );
            claude_items.push(menu_item);
        }
    }

    let codex_header = if codex_has_items {
        Some(MenuItem::with_id(app, "codex_header", &codex_data.title, false, None::<&str>)
            .map_err(|e| e.to_string())?)
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
                    None::<&str>
                )
                .map_err(|e| e.to_string())?,
            );
            codex_items.push(menu_item);
        }
    }

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
    if let Some(ref header) = opencode_plugin_header {
        all_items.push(header);
    }
    for item in &opencode_plugin_items {
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
    for item in &claude_items {
        all_items.push(item.as_ref());
    }
    // Add Codex section if enabled
    if let Some(ref header) = codex_header {
        all_items.push(header);
    }
    for item in &codex_items {
        all_items.push(item.as_ref());
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
) -> Result<Submenu<R>, String> {
    // Build title with current selection in parentheses
    let title = if data.current_display.is_empty() {
        data.title.clone()
    } else {
        format!("{} ({})", data.title, data.current_display)
    };
    let submenu_id = format!("{}_submenu", data.title);
    let submenu = Submenu::with_id(app, &submenu_id, &title, true)
        .map_err(|e| e.to_string())?;

    if data.items.is_empty() {
        let empty_item = MenuItem::with_id(app, &format!("{}_empty", data.title), "  暂无模型", false, None::<&str>)
            .map_err(|e| e.to_string())?;
        submenu.append(&empty_item).map_err(|e| e.to_string())?;
    } else {
        for item in &data.items {
            let item_id = format!("opencode_model_{}_{}", model_type, item.id);
            let menu_item = CheckMenuItem::with_id(app, &item_id, &item.display_name, true, item.is_selected, None::<&str>)
                .map_err(|e| e.to_string())?;
            submenu.append(&menu_item).map_err(|e| e.to_string())?;
        }
    }

    Ok(submenu)
}
