//! 本机专属的 WebKit 启动偏好：必须在 GTK 和 SQLite 初始化前可读，
//! 不随跨机器恢复的设置数据库迁移。
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::Mutex;
use std::sync::OnceLock;

/// 应用会为兼容级别设置的 WebKit/GDK 开关。用户若在启动环境里预先设了其中
/// 任何一个，本进程就不会覆盖它，`性能优先` 也就无法恢复原生渲染。
#[cfg(any(target_os = "linux", test))]
pub(crate) const MANAGED_ENVIRONMENT_VARIABLES: [&str; 4] = [
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_DISABLE_GPU_PROCESS",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
    "GDK_BACKEND",
];

/// 用户预置的开关：变量存在，但本进程从未注入过它。应用只覆盖自己注入的值，
/// 所以这些变量会一直压过用户选择的渲染模式。
#[cfg(any(target_os = "linux", test))]
fn preset_variables_from(
    injected: &[(String, std::ffi::OsString)],
    exists: impl Fn(&str) -> bool,
) -> Vec<&'static str> {
    MANAGED_ENVIRONMENT_VARIABLES
        .into_iter()
        .filter(|name| exists(name) && !injected.iter().any(|(key, _)| key == name))
        .collect()
}

#[cfg(target_os = "linux")]
fn preset_variables() -> Vec<&'static str> {
    let injected = INJECTED_ENV
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    preset_variables_from(&injected, |name| std::env::var_os(name).is_some())
}

/// 非 Linux 平台不会应用这些开关，渲染设置本身也不受支持。
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn preset_variables() -> Vec<&'static str> {
    Vec::new()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderingMode {
    #[default]
    Compatible,
    Performance,
}

#[derive(Default, Serialize, Deserialize)]
struct Preference {
    // 旧版本或未来版本写下的记录可能缺这个字段，缺省按兼容优先处理。
    #[serde(default)]
    mode: RenderingMode,
}

#[derive(Clone, Copy)]
struct Startup {
    mode: RenderingMode,
    level: u8,
}

static STARTUP: OnceLock<Startup> = OnceLock::new();
#[cfg(target_os = "linux")]
static INJECTED_ENV: Mutex<Vec<(String, std::ffi::OsString)>> = Mutex::new(Vec::new());

#[cfg(target_os = "linux")]
pub(crate) fn record_injected_environment(name: &str, value: &str) {
    INJECTED_ENV
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push((name.to_string(), value.into()));
}

#[cfg(any(target_os = "linux", test))]
fn clean_restart_environment(
    command: &mut std::process::Command,
    injected: &[(String, std::ffi::OsString)],
    current: impl Fn(&str) -> Option<std::ffi::OsString>,
) {
    for (name, value) in injected {
        if current(name).as_ref() == Some(value) {
            command.env_remove(name);
        }
    }
}

/// 重启时重新应用用户选择的策略，清除当前进程注入的降级开关，保留用户显式变量。
#[cfg(target_os = "linux")]
pub(crate) fn configure_restart_environment(command: &mut std::process::Command) {
    let injected = INJECTED_ENV
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    clean_restart_environment(command, &injected, |name| std::env::var_os(name));
}

#[derive(Serialize)]
pub struct RenderingInfo {
    supported: bool,
    mode: RenderingMode,
    active_mode: RenderingMode,
    workaround_level: Option<u8>,
    environment_override: bool,
    restart_required: bool,
}

fn preference_path() -> std::path::PathBuf {
    // 自定义业务数据目录不会搬迁本机启动偏好；隔离启动仍由 DATA_DIR 重定向。
    crate::app_paths::default_data_dir().join("linux_webview_rendering.json")
}

fn load_mode_at(path: &Path) -> Result<RenderingMode, String> {
    match fs::read_to_string(path) {
        // 损坏或无法识别的记录只影响“保存的偏好”，读取失败不应让设置界面永久报错。
        Ok(raw) => Ok(serde_json::from_str::<Preference>(&raw)
            .map(|value| value.mode)
            .unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RenderingMode::default()),
        Err(error) => Err(format!(
            "Failed to read Linux rendering preference: {error}"
        )),
    }
}

fn save_mode_at(path: &Path, mode: RenderingMode) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Invalid Linux rendering preference path")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let raw = serde_json::to_vec(&Preference { mode }).map_err(|error| error.to_string())?;
    // 先写临时文件再改名，避免保存期间被强杀留下截断的 JSON。
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, raw).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn load_startup_mode() -> RenderingMode {
    load_mode_at(&preference_path()).unwrap_or_else(|error| {
        log::warn!("{error}; using compatible rendering");
        RenderingMode::default()
    })
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn initial_level(
    debug: bool,
    wayland: bool,
    appimage: bool,
    wsl: bool,
    mode: RenderingMode,
    persisted: u8,
) -> u8 {
    if wsl || (debug && mode == RenderingMode::Compatible) {
        return 4;
    }
    let minimum = u8::from(mode == RenderingMode::Compatible && (wayland || appimage));
    persisted.min(4).max(minimum)
}

#[cfg(target_os = "linux")]
pub(crate) fn record_startup(mode: RenderingMode, level: u8) {
    let _ = STARTUP.set(Startup { mode, level });
    let presets = preset_variables();
    if !presets.is_empty() {
        log::warn!(
            "Linux WebView environment: user preset {} keeps the selected rendering mode from taking effect",
            presets.join(", ")
        );
    }
    for name in [
        "WEBKIT_DISABLE_DMABUF_RENDERER",
        "WEBKIT_DISABLE_GPU_PROCESS",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
        "WEBKIT_SKIA_GPU_PAINTING_THREADS",
        "GDK_BACKEND",
    ] {
        log::info!(
            "Linux WebView environment: {name}={}",
            std::env::var(name).unwrap_or_else(|_| "<unset>".to_string())
        );
    }
    // 读取实际加载的库，避免把宿主包版本误当成 AppImage 内置的 WebKitGTK 版本。
    unsafe {
        let names = [
            c"webkit_get_major_version",
            c"webkit_get_minor_version",
            c"webkit_get_micro_version",
        ];
        let symbols = names.map(|name| libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()));
        if symbols.iter().all(|symbol| !symbol.is_null()) {
            let versions = symbols.map(|symbol| {
                let version: unsafe extern "C" fn() -> libc::c_uint = std::mem::transmute(symbol);
                version()
            });
            let mut info: libc::Dl_info = std::mem::zeroed();
            let library = if libc::dladdr(symbols[0], &mut info) != 0 && !info.dli_fname.is_null() {
                std::ffi::CStr::from_ptr(info.dli_fname)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "unknown".to_string()
            };
            log::info!("Linux WebView: mode={mode:?}, workaround_level={level}, WebKitGTK={}.{}.{}, library={library}", versions[0], versions[1], versions[2]);
        } else {
            log::info!("Linux WebView: mode={mode:?}, workaround_level={level}, WebKitGTK version unavailable");
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn active_mode() -> RenderingMode {
    STARTUP.get().map(|state| state.mode).unwrap_or_default()
}

fn rendering_info() -> Result<RenderingInfo, String> {
    let mode = load_mode_at(&preference_path())?;
    let startup = STARTUP.get();
    let active_mode = startup.map(|state| state.mode).unwrap_or_default();
    Ok(RenderingInfo {
        supported: cfg!(target_os = "linux"),
        mode,
        active_mode,
        workaround_level: startup.map(|state| state.level),
        environment_override: std::env::var("AI_TOOLBOX_WAYLAND_WEBVIEW_WORKAROUND_LEVEL")
            .ok()
            .and_then(|value| value.trim().parse::<u8>().ok())
            .is_some()
            || std::env::var_os("AI_TOOLBOX_DISABLE_WAYLAND_WEBVIEW_WORKAROUND").is_some()
            // 用户预置的 WebKit 开关不会被应用覆盖，性能模式因此无法恢复到原生渲染。
            || !preset_variables().is_empty(),
        restart_required: mode != active_mode,
    })
}

#[tauri::command]
pub async fn get_linux_webview_rendering() -> Result<RenderingInfo, String> {
    tauri::async_runtime::spawn_blocking(rendering_info)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn set_linux_webview_rendering(mode: RenderingMode) -> Result<RenderingInfo, String> {
    if !cfg!(target_os = "linux") {
        return Err("Linux rendering preferences are only supported on Linux".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        save_mode_at(&preference_path(), mode)?;
        rendering_info()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_removes_only_unchanged_app_injected_flags() {
        let injected = vec![
            ("WEBKIT_DISABLE_DMABUF_RENDERER".to_string(), "1".into()),
            ("GDK_BACKEND".to_string(), "x11".into()),
        ];
        let mut command = std::process::Command::new("test");
        command.env("EXPLICIT_USER_SETTING", "1");
        clean_restart_environment(&mut command, &injected, |name| match name {
            "WEBKIT_DISABLE_DMABUF_RENDERER" => Some("1".into()),
            "GDK_BACKEND" => Some("wayland".into()),
            _ => None,
        });
        let changes = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_str().unwrap(),
                    value.and_then(|value| value.to_str()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(changes.get("WEBKIT_DISABLE_DMABUF_RENDERER"), Some(&None));
        assert!(!changes.contains_key("GDK_BACKEND"));
        assert_eq!(changes.get("EXPLICIT_USER_SETTING"), Some(&Some("1")));
    }

    #[test]
    fn rendering_preference_round_trips_without_changing_the_active_process() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bootstrap/linux_webview_rendering.json");
        assert_eq!(load_mode_at(&path).unwrap(), RenderingMode::Compatible);
        for mode in [RenderingMode::Performance, RenderingMode::Compatible] {
            save_mode_at(&path, mode).unwrap();
            assert_eq!(load_mode_at(&path).unwrap(), mode);
        }
        // 截断、未知枚举值或缺字段的记录都只是“没有偏好”，不能让设置界面永久报错。
        for corrupt in [
            r#"{"mode":"perf"#,
            r#"{"mode":"invalid"}"#,
            r#"{}"#,
            "not-json",
        ] {
            fs::write(&path, corrupt).unwrap();
            assert_eq!(load_mode_at(&path).unwrap(), RenderingMode::Compatible);
        }
        save_mode_at(&path, RenderingMode::Performance).unwrap();
        assert!(!root.path().join("bootstrap/linux_webview_rendering.json.tmp").exists());
    }

    #[test]
    fn preset_variables_counts_only_values_the_app_never_injected() {
        let injected = vec![
            ("WEBKIT_DISABLE_DMABUF_RENDERER".to_string(), "1".into()),
            ("GDK_BACKEND".to_string(), "x11".into()),
        ];
        // 环境里只有应用注入的这两个变量时，没有用户预置项。
        let only_injected = |name: &str| {
            injected.iter().any(|(key, _)| key == name)
        };
        assert!(preset_variables_from(&injected, only_injected).is_empty());
        // 用户自己设的 WEBKIT 开关会压过所选渲染模式。
        assert_eq!(
            preset_variables_from(&injected, |name| {
                only_injected(name) || name == "WEBKIT_DISABLE_GPU_PROCESS"
            }),
            vec!["WEBKIT_DISABLE_GPU_PROCESS"]
        );
        // 应用注入过的变量即使存在，也不算用户预置。
        assert_eq!(
            preset_variables_from(&injected, |name| {
                only_injected(name) || name == "WEBKIT_DISABLE_DMABUF_RENDERER"
            }),
            Vec::<&str>::new()
        );
        assert!(preset_variables_from(&[], |_| false).is_empty());
    }

    #[test]
    fn explicit_performance_keeps_native_rendering_and_preserves_failure_fallbacks() {
        assert_eq!(
            initial_level(false, true, true, false, RenderingMode::Compatible, 0),
            1
        );
        assert_eq!(
            initial_level(false, true, true, false, RenderingMode::Performance, 0),
            0
        );
        assert_eq!(
            initial_level(false, true, true, false, RenderingMode::Performance, 2),
            2
        );
        assert_eq!(
            initial_level(true, true, true, false, RenderingMode::Compatible, 0),
            4
        );
        assert_eq!(
            initial_level(true, true, true, false, RenderingMode::Performance, 0),
            0
        );
        assert_eq!(
            initial_level(false, true, true, true, RenderingMode::Performance, 0),
            4
        );
        assert_eq!(
            initial_level(false, false, false, false, RenderingMode::Compatible, 0),
            0
        );
        assert_eq!(
            initial_level(false, false, true, false, RenderingMode::Compatible, 0),
            1
        );
    }
}
