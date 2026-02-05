#[allow(unused_imports)]
use tauri::{Listener, Manager};

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;
use tokio::sync::Mutex;

use log::{error, info, warn};
use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};

#[cfg(target_os = "linux")]
use std::sync::Mutex as StdMutex;

// Module declarations
pub mod auto_launch;
pub mod coding;
pub mod db;
pub mod http_client;
pub mod settings;
pub mod single_instance;
pub mod tray;
pub mod update;

// Re-export DbState for use in other modules
pub use db::DbState;

/// Set window background color (affects macOS titlebar color)
#[tauri::command]
fn set_window_background_color(window: tauri::Window, r: u8, g: u8, b: u8) -> Result<(), String> {
    use tauri::window::Color;
    window
        .set_background_color(Some(Color(r, g, b, 255)))
        .map_err(|e| format!("Failed to set background color: {}", e))
}

/// Open a folder in the system file manager
/// If the path is a file, opens the parent directory
/// Creates the directory if it doesn't exist
#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let path = Path::new(&path);

    // Determine the folder to open
    let folder = if path.is_file() {
        path.parent()
            .ok_or_else(|| "Cannot get parent directory".to_string())?
    } else {
        path
    };

    // Create directory if it doesn't exist
    if !folder.exists() {
        fs::create_dir_all(folder)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    // Open the folder using system default file manager
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}

/// 初始化日志系统，日志文件位于应用数据目录下的 logs 文件夹
/// 同一天的日志会追加到同一个文件中
fn init_logging() -> Option<std::path::PathBuf> {
    // 获取日志目录路径
    let log_dir = dirs::data_dir()
        .map(|p| p.join("com.ai-toolbox").join("logs"))
        .or_else(|| dirs::home_dir().map(|p| p.join(".ai-toolbox").join("logs")));

    let log_dir = match log_dir {
        Some(dir) => dir,
        None => return None,
    };

    // 确保日志目录存在
    if let Err(e) = fs::create_dir_all(&log_dir) {
        eprintln!("无法创建日志目录: {}", e);
        return None;
    }

    // 使用日期命名日志文件，同一天的日志追加到同一个文件
    let date = chrono::Local::now().format("%Y%m%d");
    let log_file = log_dir.join(format!("ai-toolbox_{}.log", date));

    // 以追加模式打开日志文件
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("无法打开日志文件: {}", e);
            return None;
        }
    };

    // 初始化日志系统
    if CombinedLogger::init(vec![WriteLogger::new(
        LevelFilter::Info,
        Config::default(),
        file,
    )])
    .is_err()
    {
        eprintln!("日志系统初始化失败");
        return None;
    }

    // 清理旧日志文件（保留最近 7 天）
    if let Ok(entries) = fs::read_dir(&log_dir) {
        let mut log_files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with("ai-toolbox_"))
                    .unwrap_or(false)
            })
            .collect();

        log_files.sort_by_key(|e| std::cmp::Reverse(e.path()));

        // 删除超过 7 天的旧日志
        for old_log in log_files.into_iter().skip(7) {
            let _ = fs::remove_file(old_log.path());
        }
    }

    Some(log_file)
}

/// 设置 panic hook，将 panic 信息写入日志
fn setup_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // 记录 panic 信息到日志
        let msg = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic".to_string()
        };

        let location = panic_info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        error!("PANIC 发生: {} at {}", msg, location);

        // 尝试将错误写入单独的崩溃日志文件
        if let Some(log_dir) = dirs::data_dir()
            .map(|p| p.join("com.ai-toolbox").join("logs"))
            .or_else(|| dirs::home_dir().map(|p| p.join(".ai-toolbox").join("logs")))
        {
            let crash_file = log_dir.join("CRASH.log");
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let crash_msg = format!("[{}] PANIC: {} at {}\n", timestamp, msg, location);
            let _ = std::fs::write(&crash_file, crash_msg);
        }

        // 调用默认 hook
        default_hook(panic_info);
    }));
}

#[cfg(target_os = "linux")]
fn set_env_if_missing(key: &str, value: &str) -> bool {
    if std::env::var_os(key).is_some() {
        return false;
    }
    std::env::set_var(key, value);
    true
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    if session_type.eq_ignore_ascii_case("wayland") {
        return true;
    }
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(target_os = "linux")]
const WAYLAND_WEBVIEW_WORKAROUND_MAX_LEVEL: u8 = 3;

#[cfg(target_os = "linux")]
fn wayland_webview_workaround_level_path() -> Option<std::path::PathBuf> {
    let base_dir = dirs::data_dir()
        .map(|p| p.join("com.ai-toolbox"))
        .or_else(|| dirs::home_dir().map(|p| p.join(".ai-toolbox")))?;
    Some(
        base_dir
            .join("runtime")
            .join("wayland_webview_workaround_level"),
    )
}

#[cfg(target_os = "linux")]
fn read_wayland_webview_workaround_level() -> u8 {
    let Some(path) = wayland_webview_workaround_level_path() else {
        return 0;
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return 0;
    };
    raw.trim()
        .parse::<u8>()
        .ok()
        .map(|v| v.min(WAYLAND_WEBVIEW_WORKAROUND_MAX_LEVEL))
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn write_wayland_webview_workaround_level(level: u8) {
    let Some(path) = wayland_webview_workaround_level_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, level.to_string());
}

#[cfg(target_os = "linux")]
fn try_acquire_single_instance_lock_with_optional_retry() -> Result<single_instance::SingleInstanceLock, String> {
    if std::env::var_os("AI_TOOLBOX_RESTART_WAIT_LOCK").is_none() {
        return single_instance::try_acquire_lock();
    }

    let mut last_err: Option<String> = None;
    for _ in 0..20 {
        match single_instance::try_acquire_lock() {
            Ok(lock) => return Ok(lock),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(150));
            }
        }
    }

    Err(last_err.unwrap_or_else(|| "Failed to acquire single instance lock".to_string()))
}

#[cfg(target_os = "linux")]
fn setup_linux_wayland_egl_failure_monitor(
    egl_failure_flag: Arc<std::sync::atomic::AtomicBool>,
) -> Arc<std::sync::atomic::AtomicBool> {
    use std::io::{Read, Write};
    use std::os::unix::io::FromRawFd;

    if std::env::var_os("AI_TOOLBOX_DISABLE_WAYLAND_WEBVIEW_WORKAROUND").is_some() {
        return egl_failure_flag;
    }
    if !is_wayland_session() {
        return egl_failure_flag;
    }

    let egl_failure_flag_clone = egl_failure_flag.clone();
    let Ok(thread_builder) = std::thread::Builder::new().name("egl-stderr-monitor".to_string()).spawn(move || unsafe {
        let mut pipe_fds = [0; 2];
        if libc::pipe(pipe_fds.as_mut_ptr()) != 0 {
            return;
        }

        let read_fd = pipe_fds[0];
        let write_fd = pipe_fds[1];

        let original_stderr_fd = libc::dup(libc::STDERR_FILENO);
        if original_stderr_fd < 0 {
            libc::close(read_fd);
            libc::close(write_fd);
            return;
        }

        if libc::dup2(write_fd, libc::STDERR_FILENO) < 0 {
            libc::close(original_stderr_fd);
            libc::close(read_fd);
            libc::close(write_fd);
            return;
        }
        libc::close(write_fd);

        let mut original_stderr = std::fs::File::from_raw_fd(original_stderr_fd);
        let mut reader = std::fs::File::from_raw_fd(read_fd);

        let mut buf = [0u8; 4096];
        let mut carry = String::new();
        loop {
            let Ok(n) = reader.read(&mut buf) else { break };
            if n == 0 {
                break;
            }

            let chunk = &buf[..n];
            let _ = original_stderr.write_all(chunk);
            let text = String::from_utf8_lossy(chunk);

            carry.push_str(&text);
            if carry.contains("Could not create default EGL display")
                || carry.contains("EGL_BAD_PARAMETER")
            {
                egl_failure_flag_clone.store(true, std::sync::atomic::Ordering::Relaxed);
            }

            if carry.len() > 4096 {
                let keep_from = carry.len().saturating_sub(2048);
                carry.drain(..keep_from);
            }
        }
    }) else {
        return egl_failure_flag;
    };

    let _ = thread_builder;
    egl_failure_flag
}

#[cfg(target_os = "linux")]
fn start_linux_wayland_webview_auto_downgrade_watchdog(
    app_handle: tauri::AppHandle,
    current_level: u8,
    egl_failure_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    single_instance_lock_holder: Arc<StdMutex<Option<single_instance::SingleInstanceLock>>>,
) {
    use std::sync::atomic::Ordering;
    use tokio::sync::watch;

    let egl_failure_flag = egl_failure_flag
        .unwrap_or_else(|| Arc::new(std::sync::atomic::AtomicBool::new(false)));

    tauri::async_runtime::spawn(async move {
        let (ready_tx, mut ready_rx) = watch::channel(false);
        let _handler = app_handle.listen("frontend-ready", move |_event| {
            let _ = ready_tx.send(true);
        });

        let timeout_secs = std::env::var("AI_TOOLBOX_FRONTEND_READY_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(20);
        let timeout = Duration::from_secs(timeout_secs);
        let start = std::time::Instant::now();

        loop {
            if *ready_rx.borrow() {
                info!("frontend-ready received; Wayland webview auto-downgrade not needed");
                return;
            }

            let egl_failed = egl_failure_flag.load(Ordering::Relaxed);
            if egl_failed || start.elapsed() >= timeout {
                let next_level = if egl_failed {
                    WAYLAND_WEBVIEW_WORKAROUND_MAX_LEVEL
                } else {
                    (current_level + 1).min(WAYLAND_WEBVIEW_WORKAROUND_MAX_LEVEL)
                };

                if next_level <= current_level {
                    warn!(
                        "Wayland webview auto-downgrade triggered but already at level {}",
                        current_level
                    );
                    return;
                }

                if egl_failed {
                    warn!(
                        "Detected EGL initialization failure; restarting with Wayland webview workaround level {}",
                        next_level
                    );
                } else {
                    warn!(
                        "frontend-ready not received in {:?}; restarting with Wayland webview workaround level {}",
                        timeout, next_level
                    );
                }

                write_wayland_webview_workaround_level(next_level);

                if let Ok(mut guard) = single_instance_lock_holder.lock() {
                    let _ = guard.take();
                }

                let current_exe = match std::env::current_exe() {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to get current executable for restart: {}", e);
                        return;
                    }
                };
                let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

                match std::process::Command::new(&current_exe)
                    .args(args)
                    .env("AI_TOOLBOX_RESTART_WAIT_LOCK", "1")
                    .spawn()
                {
                    Ok(_) => {
                        std::process::exit(0);
                    }
                    Err(e) => {
                        error!("Failed to spawn restarted instance: {}", e);
                        return;
                    }
                }
            }

            tokio::select! {
                _ = ready_rx.changed() => {},
                _ = tokio::time::sleep(Duration::from_millis(200)) => {},
            }
        }
    });
}

/// Workaround: On some Linux Wayland environments (e.g. Plasma 6), WebKitGTK can fail to
/// initialize EGL and the webview shows a white screen.
///
/// We apply WebKitGTK GPU/DMABuf workarounds on Wayland based on a fallback level:
/// - 0: Default (GPU/DMABuf enabled)
/// - 1: Disable DMABuf renderer
/// - 2: Disable GPU process
/// - 3: Disable compositing mode
///
/// Notes:
/// - Debug builds default to level 3 to avoid dev-time white screens.
/// - Release builds default to level 0 and may auto-downgrade on failure.
/// - Set `AI_TOOLBOX_DISABLE_WAYLAND_WEBVIEW_WORKAROUND=1` to opt out.
/// - Set `AI_TOOLBOX_WAYLAND_WEBVIEW_WORKAROUND_LEVEL=0..3` to override.
#[cfg(target_os = "linux")]
fn setup_linux_wayland_webview_workaround() -> u8 {
    if std::env::var_os("AI_TOOLBOX_DISABLE_WAYLAND_WEBVIEW_WORKAROUND").is_some() {
        info!("Wayland webview workaround disabled via AI_TOOLBOX_DISABLE_WAYLAND_WEBVIEW_WORKAROUND");
        return 0;
    }

    if !is_wayland_session() {
        return 0;
    }

    let level = std::env::var("AI_TOOLBOX_WAYLAND_WEBVIEW_WORKAROUND_LEVEL")
        .ok()
        .and_then(|v| v.trim().parse::<u8>().ok())
        .map(|v| v.min(WAYLAND_WEBVIEW_WORKAROUND_MAX_LEVEL))
        .unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                WAYLAND_WEBVIEW_WORKAROUND_MAX_LEVEL
            } else {
                read_wayland_webview_workaround_level()
            }
        });

    let mut changed = false;
    if level >= 1 {
        changed |= set_env_if_missing("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    if level >= 2 {
        changed |= set_env_if_missing("WEBKIT_DISABLE_GPU_PROCESS", "1");
    }
    if level >= 3 {
        changed |= set_env_if_missing("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }

    if level == 0 {
        info!("Detected Wayland session; WebKitGTK GPU/DMABuf is enabled (workaround level 0)");
    } else if changed {
        info!(
            "Detected Wayland session; applied WebKitGTK workarounds (level {}) to avoid white screen",
            level
        );
    } else {
        info!(
            "Detected Wayland session; WebKitGTK workarounds (level {}) already set via environment",
            level
        );
    }

    level
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化日志系统
    let log_file = init_logging();
    if let Some(ref path) = log_file {
        eprintln!("日志文件: {:?}", path);
    }

    // 设置 panic hook
    setup_panic_hook();

    info!("========================================");
    info!("AI Toolbox 启动中...");
    info!("版本: {}", env!("CARGO_PKG_VERSION"));
    info!("操作系统: {}", std::env::consts::OS);
    info!("架构: {}", std::env::consts::ARCH);
    info!("========================================");

    #[cfg(target_os = "linux")]
    let wayland_webview_workaround_level = setup_linux_wayland_webview_workaround();

    #[cfg(target_os = "linux")]
    let auto_downgrade_enabled = is_wayland_session()
        && std::env::var_os("AI_TOOLBOX_DISABLE_WAYLAND_WEBVIEW_WORKAROUND").is_none()
        && std::env::var_os("AI_TOOLBOX_WAYLAND_WEBVIEW_WORKAROUND_LEVEL").is_none()
        && wayland_webview_workaround_level < WAYLAND_WEBVIEW_WORKAROUND_MAX_LEVEL
        && (!cfg!(debug_assertions)
            || std::env::var_os("AI_TOOLBOX_ENABLE_WAYLAND_WEBVIEW_AUTO_DOWNGRADE").is_some());

    #[cfg(target_os = "linux")]
    let egl_failure_flag: Option<Arc<std::sync::atomic::AtomicBool>> = if auto_downgrade_enabled {
        Some(setup_linux_wayland_egl_failure_monitor(Arc::new(
            std::sync::atomic::AtomicBool::new(false),
        )))
    } else {
        None
    };

    // Linux: Try to acquire file-based single instance lock as fallback
    // This is needed because D-Bus based detection may not work in all environments
    #[cfg(target_os = "linux")]
    let single_instance_lock_holder: Arc<StdMutex<Option<single_instance::SingleInstanceLock>>> =
        Arc::new(StdMutex::new(None));

    #[cfg(target_os = "linux")]
    {
        let lock = match try_acquire_single_instance_lock_with_optional_retry() {
            Ok(lock) => {
                info!("文件锁单实例检测成功");
                lock
            }
            Err(e) => {
                error!("单实例检测失败: {}", e);
                eprintln!("AI Toolbox 已经在运行中。");
                eprintln!("{}", e);
                std::process::exit(1);
            }
        };

        if let Ok(mut guard) = single_instance_lock_holder.lock() {
            *guard = Some(lock);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a second instance is launched, show and focus the existing window
            if let Some(window) = app.get_webview_window("main") {
                // macOS: Switch back to Regular mode to show in Dock
                #[cfg(target_os = "macos")]
                {
                    use tauri::ActivationPolicy;
                    let _ = app.set_activation_policy(ActivationPolicy::Regular);
                }
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            info!("开始执行 setup()...");
            let app_handle = app.handle().clone();

            // Create main window with platform-specific configuration
            info!("正在创建主窗口...");
            #[cfg(target_os = "macos")]
            {
                use tauri::{TitleBarStyle, WebviewUrl, WebviewWindowBuilder};
                
                let _window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                    .title("AI Toolbox")
                    .inner_size(1200.0, 800.0)
                    .min_inner_size(800.0, 600.0)
                    .center()
                    .title_bar_style(TitleBarStyle::Overlay)
                    .hidden_title(true)
                    .build()
                    .expect("Failed to create main window");
            }
            
            #[cfg(not(target_os = "macos"))]
            {
                use tauri::{WebviewUrl, WebviewWindowBuilder};
                
                let _window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                    .title("AI Toolbox")
                    .inner_size(1200.0, 800.0)
                    .min_inner_size(800.0, 600.0)
                    .center()
                    .build()
                    .expect("Failed to create main window");
            }

            // Create app data directory
            info!("正在获取应用数据目录...");
            let app_data_dir = match app_handle.path().app_data_dir() {
                Ok(dir) => {
                    info!("应用数据目录: {:?}", dir);
                    dir
                }
                Err(e) => {
                    error!("无法获取应用数据目录: {}", e);
                    panic!("Failed to get app data dir: {}", e);
                }
            };

            if !app_data_dir.exists() {
                info!("创建应用数据目录...");
                if let Err(e) = fs::create_dir_all(&app_data_dir) {
                    error!("无法创建应用数据目录: {}", e);
                    panic!("Failed to create app data dir: {}", e);
                }
            }

            let db_path = app_data_dir.join("database");
            info!("数据库路径: {:?}", db_path);

            // Initialize SurrealDB
            info!("正在初始化 SurrealDB...");
            tauri::async_runtime::block_on(async {
                let db = match Surreal::new::<SurrealKv>(db_path.clone()).await {
                    Ok(db) => {
                        info!("SurrealDB 初始化成功");
                        db
                    }
                    Err(e) => {
                        error!("SurrealDB 初始化失败: {}", e);
                        panic!("Failed to initialize SurrealDB: {}", e);
                    }
                };

                info!("正在选择命名空间和数据库...");
                if let Err(e) = db.use_ns("ai_toolbox").use_db("main").await {
                    error!("选择命名空间/数据库失败: {}", e);
                    panic!("Failed to select namespace and database: {}", e);
                }
                info!("命名空间和数据库选择成功");

                // Run database migrations
                info!("正在运行数据库迁移...");
                if let Err(e) = db::run_migrations(&db).await {
                    error!("数据库迁移失败: {}", e);
                    panic!("Failed to run database migrations: {}", e);
                }
                info!("数据库迁移完成");

                // Initialize default provider models in database
                info!("正在初始化默认提供商模型...");
                let db_state = DbState(Arc::new(Mutex::new(db.clone())));
                if let Err(e) =
                    coding::open_code::free_models::init_default_provider_models(&db_state).await
                {
                    warn!("初始化默认提供商模型失败: {}", e);
                    // 不 panic，这不是致命错误
                }

                // Skip auto-import of local settings into database on startup.
                // Local configs are now loaded on-demand without writing to DB.

                app.manage(db_state);
                info!("数据库状态已注册到应用");
            });

            // Create system tray
            info!("正在创建系统托盘...");
            if let Err(e) = tray::create_tray(&app_handle) {
                error!("创建系统托盘失败: {}", e);
                panic!("Failed to create system tray: {}", e);
            }
            info!("系统托盘创建成功");

            // Listen for config changes to refresh tray menu
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let value = app_handle_clone.clone();
                let value_for_closure = value.clone();
                let _listener = value.listen("config-changed", move |_event| {
                    let app = value_for_closure.app_handle().clone();
                    let _ = tauri::async_runtime::spawn(async move {
                        let _ = tray::refresh_tray_menus(&app).await;
                    });
                });
                
                // Keep this async block alive forever to prevent listener from being dropped
                std::future::pending::<()>().await;
            });
            
            
            // Enable auto-launch if setting is true
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let db_state = app_handle_clone.state::<DbState>();
                let db = db_state.0.lock().await;
                
                let mut result = db
                    .query("SELECT * OMIT id FROM settings:`app` LIMIT 1")
                    .await
                    .ok();
                
                if let Some(ref mut res) = result {
                    let records: Result<Vec<serde_json::Value>, _> = res.take(0);
                    if let Ok(records) = records {
                        if let Some(record) = records.first() {
                            let launch_on_startup = record
                                .get("launch_on_startup")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true);
                            
                            if launch_on_startup {
                                let _ = auto_launch::enable_auto_launch();
                            }
                        }
                    }
                }
            });

            // Listen for WSL sync requests (Windows only)
            #[cfg(target_os = "windows")]
            {
                // OpenCode sync listener
                let app1 = app_handle.clone();
                let app1_clone = app1.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app1.listen("wsl-sync-request-opencode", move |_event| {
                        let app = app1_clone.clone();
                        // Spawn background task without awaiting
                        tauri::async_runtime::spawn(async move {
                            // Re-obtain state inside the spawned task
                            let db_state = app.state::<crate::DbState>();
                            let result = coding::wsl::wsl_sync(db_state, app.clone(), Some("opencode".to_string())).await;
                            // Ignore result - fire and forget
                            let _ = result;
                        });
                    });

                    // Keep this async block alive forever to prevent listener from being dropped
                    std::future::pending::<()>().await;
                });

                // Claude sync listener
                let app2 = app_handle.clone();
                let app2_clone = app2.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app2.listen("wsl-sync-request-claude", move |_event| {
                        let app = app2_clone.clone();
                        // Spawn background task without awaiting
                        tauri::async_runtime::spawn(async move {
                            // Re-obtain state inside the spawned task
                            let db_state = app.state::<crate::DbState>();
                            let result = coding::wsl::wsl_sync(db_state, app.clone(), Some("claude".to_string())).await;
                            // Ignore result - fire and forget
                            let _ = result;
                        });
                    });

                    // Keep this async block alive forever to prevent listener from being dropped
                    std::future::pending::<()>().await;
                });

                // Codex sync listener
                let app3 = app_handle.clone();
                let app3_clone = app3.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app3.listen("wsl-sync-request-codex", move |_event| {
                        let app = app3_clone.clone();
                        // Spawn background task without awaiting
                        tauri::async_runtime::spawn(async move {
                            // Re-obtain state inside the spawned task
                            let db_state = app.state::<crate::DbState>();
                            let result = coding::wsl::wsl_sync(db_state, app.clone(), Some("codex".to_string())).await;
                            // Ignore result - fire and forget
                            let _ = result;
                        });
                    });

                    // Keep this async block alive forever to prevent listener from being dropped
                    std::future::pending::<()>().await;
                });

                // MCP-changed listener - triggers MCP WSL sync
                let app_mcp = app_handle.clone();
                let app_mcp_clone = app_mcp.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app_mcp.listen("mcp-changed", move |_event| {
                        let app = app_mcp_clone.clone();
                        tauri::async_runtime::spawn(async move {
                            let db_state = app.state::<crate::DbState>();
                            let _ = coding::wsl::sync_mcp_to_wsl(&db_state, app.clone()).await;
                        });
                    });

                    std::future::pending::<()>().await;
                });

                // Skills-changed listener - triggers Skills WSL sync
                let app_skills = app_handle.clone();
                let app_skills_clone = app_skills.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app_skills.listen("skills-changed", move |_event| {
                        let app = app_skills_clone.clone();
                        tauri::async_runtime::spawn(async move {
                            let db_state = app.state::<crate::DbState>();
                            let _ = coding::wsl::sync_skills_to_wsl(&db_state, app.clone()).await;
                        });
                    });

                    std::future::pending::<()>().await;
                });
            }

            #[cfg(target_os = "windows")]
            // WSL sync on app startup (delayed to avoid blocking startup)
            {
                info!("正在初始化 WSL 同步任务...");
                let app_clone = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let db_state = app_clone.state::<crate::DbState>();
                    let app = app_clone.clone();
                    let _ = coding::wsl::wsl_sync(db_state, app, None).await;
                });
            }

            // Git cache auto-cleanup task (checks every hour)
            {
                let app_clone = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    // Initial delay before first cleanup
                    tokio::time::sleep(Duration::from_secs(5)).await;

                    loop {
                        let db_state = app_clone.state::<crate::DbState>();
                        let days = coding::skills::cache_cleanup::get_git_cache_cleanup_days(&db_state).await;
                        if days > 0 {
                            let max_age = Duration::from_secs((days as u64) * 86400);
                            match coding::skills::cache_cleanup::cleanup_git_cache_dirs(&app_clone, max_age) {
                                Ok(count) if count > 0 => {
                                    info!("Git cache auto-cleanup: removed {} expired cache(s)", count);
                                }
                                Err(e) => {
                                    warn!("Git cache auto-cleanup failed: {}", e);
                                }
                                _ => {}
                            }
                        }

                        // Check every hour
                        tokio::time::sleep(Duration::from_secs(3600)).await;
                    }
                });
            }

            // Check for resync flag after restore (delayed to ensure DB is ready)
            {
                let app_clone = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    // Delay to ensure database is fully initialized
                    tokio::time::sleep(Duration::from_secs(3)).await;

                    let app_data_dir = match app_clone.path().app_data_dir() {
                        Ok(dir) => dir,
                        Err(_) => return,
                    };
                    let resync_flag = app_data_dir.join(".resync_required");

                    if resync_flag.exists() {
                        info!("Resync flag detected, starting skills and MCP resync...");

                        // Remove the flag file first to prevent repeated resync
                        let _ = fs::remove_file(&resync_flag);

                        let db_state = app_clone.state::<crate::DbState>();

                        // Resync skills
                        match coding::skills::commands::skills_resync_all(app_clone.clone(), db_state.clone()).await {
                            Ok(synced) => {
                                info!("Skills resync completed: {} items synced", synced.len());
                            }
                            Err(e) => {
                                warn!("Skills resync failed: {}", e);
                            }
                        }

                        // Resync MCP servers
                        match coding::mcp::commands::mcp_sync_all(app_clone.clone(), db_state).await {
                            Ok(results) => {
                                let success_count = results.iter().filter(|r| r.success).count();
                                info!("MCP resync completed: {}/{} succeeded", success_count, results.len());
                            }
                            Err(e) => {
                                warn!("MCP resync failed: {}", e);
                            }
                        }

                        info!("Post-restore resync completed");
                    }
                });
            }

            info!("setup() 完成，应用即将启动");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app_handle = window.app_handle().clone();
                
                // Check minimize_to_tray_on_close setting with default value
                let minimize_to_tray = {
                    let db_state = app_handle.state::<DbState>();
                    let db = db_state.0.blocking_lock();
                    
                    // Query settings synchronously using block_on
                    let query_result = tauri::async_runtime::block_on(async {
                        db.query("SELECT * OMIT id FROM settings:`app` LIMIT 1").await
                    });
                    
                    match query_result {
                        Ok(mut res) => {
                            let records: Result<Vec<serde_json::Value>, surrealdb::Error> = res.take(0);
                            match records {
                                Ok(records) => {
                                    if let Some(record) = records.first() {
                                        record
                                            .get("minimize_to_tray_on_close")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(true)
                                    } else {
                                        true
                                    }
                                }
                                Err(_) => true,
                            }
                        }
                        Err(_) => true,
                    }
                };
                
                if minimize_to_tray {
                    // Hide window instead of closing
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.hide();
                        
                        // macOS: Switch to Accessory mode to hide from Dock
                        #[cfg(target_os = "macos")]
                        {
                            use tauri::ActivationPolicy;
                            let _ = app_handle.set_activation_policy(ActivationPolicy::Accessory);
                        }
                    }
                    // Prevent default close behavior
                    api.prevent_close();
                }
                // If minimize_to_tray is false, do nothing - window will close normally
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Common
            open_folder,
            set_window_background_color,
            // Update
            update::check_for_updates,
            update::install_update,
            // Settings
            settings::get_settings,
            settings::save_settings,
            settings::set_auto_launch,
            settings::get_auto_launch_status,
            settings::restart_app,
            settings::test_proxy_connection,
            // Backup - Local
            settings::backup::backup_database,
            settings::backup::restore_database,
            settings::backup::get_database_path,
            settings::backup::open_app_data_dir,
            // Backup - WebDAV
            settings::backup::backup_to_webdav,
            settings::backup::list_webdav_backups,
            settings::backup::restore_from_webdav,
            settings::backup::test_webdav_connection,
            settings::backup::delete_webdav_backup,
            // Claude Code
            coding::claude_code::list_claude_providers,
            coding::claude_code::create_claude_provider,
            coding::claude_code::update_claude_provider,
            coding::claude_code::delete_claude_provider,
            coding::claude_code::reorder_claude_providers,
            coding::claude_code::select_claude_provider,
            coding::claude_code::get_claude_config_path,
            coding::claude_code::reveal_claude_config_folder,
            coding::claude_code::read_claude_settings,
            coding::claude_code::apply_claude_config,
            coding::claude_code::toggle_claude_code_provider_disabled,
            coding::claude_code::get_claude_common_config,
            coding::claude_code::save_claude_common_config,
            coding::claude_code::save_claude_local_config,
            coding::claude_code::get_claude_plugin_status,
            coding::claude_code::apply_claude_plugin_config,
            coding::claude_code::get_claude_onboarding_status,
            coding::claude_code::apply_claude_onboarding_skip,
            coding::claude_code::clear_claude_onboarding_skip,
// OpenCode
            coding::open_code::get_opencode_config_path,
            coding::open_code::get_opencode_config_path_info,
            coding::open_code::read_opencode_config,
            coding::open_code::save_opencode_config,
            coding::open_code::get_opencode_common_config,
            coding::open_code::save_opencode_common_config,
            coding::open_code::fetch_provider_models,
            coding::open_code::get_opencode_free_models,
            coding::open_code::get_provider_models,
            coding::open_code::get_opencode_unified_models,
            coding::open_code::get_opencode_auth_providers,
            coding::open_code::get_opencode_auth_config_path,
            coding::open_code::backup_opencode_config,
            coding::open_code::test_provider_model_connectivity,
            coding::open_code::list_opencode_favorite_plugins,
            coding::open_code::add_opencode_favorite_plugin,
            coding::open_code::delete_opencode_favorite_plugin,
            coding::open_code::list_opencode_favorite_providers,
            coding::open_code::upsert_opencode_favorite_provider,
            coding::open_code::delete_opencode_favorite_provider,
            // Codex
            coding::codex::get_codex_config_dir_path,
            coding::codex::get_codex_config_file_path,
            coding::codex::reveal_codex_config_folder,
            coding::codex::list_codex_providers,
            coding::codex::create_codex_provider,
            coding::codex::update_codex_provider,
            coding::codex::delete_codex_provider,
            coding::codex::repair_codex_providers,
            coding::codex::reorder_codex_providers,
            coding::codex::select_codex_provider,
            coding::codex::apply_codex_config,
            coding::codex::toggle_codex_provider_disabled,
            coding::codex::read_codex_settings,
            coding::codex::get_codex_common_config,
            coding::codex::save_codex_common_config,
            coding::codex::save_codex_local_config,
            // Tray
            tray::refresh_tray_menu,
            // Oh My OpenCode
            coding::oh_my_opencode::list_oh_my_opencode_configs,
            coding::oh_my_opencode::create_oh_my_opencode_config,
            coding::oh_my_opencode::update_oh_my_opencode_config,
            coding::oh_my_opencode::delete_oh_my_opencode_config,
            coding::oh_my_opencode::apply_oh_my_opencode_config,
            coding::oh_my_opencode::reorder_oh_my_opencode_configs,
            coding::oh_my_opencode::toggle_oh_my_opencode_config_disabled,
            coding::oh_my_opencode::get_oh_my_opencode_config_path_info,
            coding::oh_my_opencode::get_oh_my_opencode_global_config,
            coding::oh_my_opencode::save_oh_my_opencode_global_config,
            coding::oh_my_opencode::check_oh_my_opencode_config_exists,
            coding::oh_my_opencode::save_oh_my_opencode_local_config,
            // Oh My OpenCode Slim
            coding::oh_my_opencode_slim::list_oh_my_opencode_slim_configs,
            coding::oh_my_opencode_slim::create_oh_my_opencode_slim_config,
            coding::oh_my_opencode_slim::update_oh_my_opencode_slim_config,
            coding::oh_my_opencode_slim::delete_oh_my_opencode_slim_config,
            coding::oh_my_opencode_slim::apply_oh_my_opencode_slim_config,
            coding::oh_my_opencode_slim::reorder_oh_my_opencode_slim_configs,
            coding::oh_my_opencode_slim::toggle_oh_my_opencode_slim_config_disabled,
            coding::oh_my_opencode_slim::get_oh_my_opencode_slim_config_path_info,
            coding::oh_my_opencode_slim::get_oh_my_opencode_slim_global_config,
            coding::oh_my_opencode_slim::save_oh_my_opencode_slim_global_config,
            coding::oh_my_opencode_slim::check_oh_my_opencode_slim_config_exists,
            coding::oh_my_opencode_slim::save_oh_my_opencode_slim_local_config,
            // WSL Sync
            coding::wsl::wsl_detect,
            coding::wsl::wsl_check_distro,
            coding::wsl::wsl_get_distro_state,
            coding::wsl::wsl_get_config,
            coding::wsl::wsl_save_config,
            coding::wsl::wsl_add_file_mapping,
            coding::wsl::wsl_update_file_mapping,
            coding::wsl::wsl_delete_file_mapping,
            coding::wsl::wsl_reset_file_mappings,
            coding::wsl::wsl_sync,
            coding::wsl::wsl_get_status,
            coding::wsl::wsl_test_path,
            coding::wsl::wsl_get_default_mappings,
            coding::wsl::wsl_open_terminal,
            coding::wsl::wsl_open_folder,
            // Skills Hub
            coding::skills::skills_get_tool_status,
            coding::skills::skills_get_central_repo_path,
            coding::skills::skills_set_central_repo_path,
            coding::skills::skills_get_managed_skills,
            coding::skills::skills_install_local,
            coding::skills::skills_install_git,
            coding::skills::skills_list_git_skills,
            coding::skills::skills_install_git_selection,
            coding::skills::skills_sync_to_tool,
            coding::skills::skills_unsync_from_tool,
            coding::skills::skills_update_managed,
            coding::skills::skills_delete_managed,
            coding::skills::skills_get_onboarding_plan,
            coding::skills::skills_import_existing,
            coding::skills::skills_get_git_cache_cleanup_days,
            coding::skills::skills_set_git_cache_cleanup_days,
            coding::skills::skills_get_git_cache_ttl_secs,
            coding::skills::skills_clear_git_cache,
            coding::skills::skills_get_git_cache_path,
            coding::skills::skills_get_preferred_tools,
            coding::skills::skills_set_preferred_tools,
            coding::skills::skills_get_show_in_tray,
            coding::skills::skills_set_show_in_tray,
            // Skills Hub - Custom Tools
            coding::skills::skills_get_custom_tools,
            coding::skills::skills_add_custom_tool,
            coding::skills::skills_remove_custom_tool,
            coding::skills::skills_check_custom_tool_path,
            coding::skills::skills_create_custom_tool_path,
            // Skills Hub - Skill Repos
            coding::skills::skills_get_repos,
            coding::skills::skills_add_repo,
            coding::skills::skills_remove_repo,
            coding::skills::skills_init_default_repos,
            // Skills Hub - Reorder
            coding::skills::skills_reorder,
            // Skills Hub - Resync
            coding::skills::skills_resync_all,
            // MCP Servers
            coding::mcp::mcp_list_servers,
            coding::mcp::mcp_create_server,
            coding::mcp::mcp_update_server,
            coding::mcp::mcp_delete_server,
            coding::mcp::mcp_toggle_tool,
            coding::mcp::mcp_reorder_servers,
            coding::mcp::mcp_sync_to_tool,
            coding::mcp::mcp_sync_all,
            coding::mcp::mcp_import_from_tool,
            coding::mcp::mcp_get_tools,
            coding::mcp::mcp_scan_servers,
            coding::mcp::mcp_get_show_in_tray,
            coding::mcp::mcp_set_show_in_tray,
            coding::mcp::mcp_get_preferred_tools,
            coding::mcp::mcp_set_preferred_tools,
            coding::mcp::mcp_add_custom_tool,
            coding::mcp::mcp_remove_custom_tool,
            // MCP Favorites
            coding::mcp::mcp_list_favorites,
            coding::mcp::mcp_upsert_favorite,
            coding::mcp::mcp_delete_favorite,
            coding::mcp::mcp_init_default_favorites,
        ])
        .build(tauri::generate_context!())
        .map_err(|e| {
            error!("构建 Tauri 应用失败: {}", e);
            e
        })
        .expect("error while building tauri application")
        .run(move |app_handle, event| {
            match event {
                // Handle macOS dock icon click when app is hidden
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    use tauri::ActivationPolicy;
                    // Switch back to Regular mode to show in Dock
                    let _ = app_handle.set_activation_policy(ActivationPolicy::Regular);
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }

                // Linux/Wayland: If the webview fails to initialize (white screen), restart with a higher
                // workaround level. This keeps GPU/DMABuf enabled by default and only degrades when needed.
                #[cfg(target_os = "linux")]
                tauri::RunEvent::Ready => {
                    if auto_downgrade_enabled {
                        start_linux_wayland_webview_auto_downgrade_watchdog(
                            app_handle.clone(),
                            wayland_webview_workaround_level,
                            egl_failure_flag.clone(),
                            single_instance_lock_holder.clone(),
                        );
                    }
                }

                _ => {}
            }

            // Avoid unused warnings on platforms where the match arms above are empty.
            let _ = app_handle;
        });
}
