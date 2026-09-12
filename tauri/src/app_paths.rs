//! Application data-directory resolution with an optional user override.
//!
//! App-owned data (SQLite DB, caches, the default Skills central repo, image
//! assets, backup flags, …) is anchored to a single resolved directory. By
//! default that directory is Tauri's `app_data_dir()` (equivalently
//! `dirs::data_dir().join("com.ai-toolbox")` — see `default_data_dir`). A user
//! can redirect it to a custom location via the settings UI; the chosen path is
//! stored in a small **bootstrap JSON file** that lives at the *default*
//! location (never inside the override target), so it can be read before the
//! main SQLite DB is opened and never moves with the override value. This
//! mirrors cc-switch's `app_store.rs` pattern, but without `tauri-plugin-store`
//! (a hand-rolled JSON file matches this repo's SQLite-only persistence style).
//!
//! Lifecycle:
//! 1. [`init_resolved_data_dir`] is called once at the very top of `run()`,
//!    before logging is even set up. It reads the bootstrap file and caches the
//!    data and cache directories together in [`RESOLVED_PATHS`].
//! 2. [`resolved_data_dir`] and [`resolved_cache_dir`] use that immutable
//!    snapshot, initializing it on first access if explicit init has not run.
//! 3. [`set_override`] only writes the bootstrap file; it deliberately does
//!    **not** refresh the live cache, because the DB and caches are already
//!    open at the current path mid-session. The new path takes effect on the
//!    next restart — the UI tells the user to restart after applying.

use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Bootstrap file name, stored at the *default* data dir (stable, override-independent).
const BOOTSTRAP_FILENAME: &str = "app_paths.json";
/// JSON key holding the override path inside the bootstrap file.
const OVERRIDE_KEY: &str = "data_dir_override";

static RESOLVED_PATHS: OnceLock<ResolvedAppPaths> = OnceLock::new();

// Serializes path changes with gateway engagement, including tray actions.
pub(crate) static DATA_DIR_CHANGE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, PartialEq, Eq)]
struct ResolvedAppPaths {
    data: PathBuf,
    cache: PathBuf,
}

impl ResolvedAppPaths {
    fn from_bootstrap(file: &Path, default_data: &Path, default_cache: &Path) -> Self {
        match read_override_from(file).filter(|path| !same_directory(path, default_data)) {
            Some(data) => Self {
                cache: data.join("cache"),
                data,
            },
            None => Self {
                data: default_data.to_path_buf(),
                cache: default_cache.to_path_buf(),
            },
        }
    }
}

/// Info returned to the UI about the data-directory configuration.
#[derive(Debug, Clone, Serialize)]
pub struct AppDataDirInfo {
    /// The override path stored in the bootstrap file, if any (`None` = default).
    pub r#override: Option<String>,
    /// The directory actually in effect for the current running session.
    pub effective: String,
    /// The platform-default directory (no override).
    pub default: String,
    /// Whether the running session uses a custom data directory.
    pub is_custom: bool,
    /// The saved target for the next start, including a pending reset.
    pub next_start: String,
    pub restart_required: bool,
}

/// Compute the platform-default app data dir.
///
/// This must stay equivalent to Tauri's `app.path().app_data_dir()` on every
/// platform (Windows `%APPDATA%\com.ai-toolbox`, macOS
/// `~/Library/Application Support/com.ai-toolbox`, Linux
/// `~/.local/share/com.ai-toolbox`). `lib.rs` already relies on this
/// equivalence for its log directory. The home fallback mirrors the existing
/// log/crash path fallback.
///
/// Unlike Tauri's path resolver this needs no `AppHandle`, so it can run before
/// the Tauri app is built.
pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .map(|p| p.join("com.ai-toolbox"))
        .or_else(|| dirs::home_dir().map(|p| p.join(".ai-toolbox")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Location of the bootstrap override file (always at the default dir).
pub fn bootstrap_file_path() -> PathBuf {
    default_data_dir().join(BOOTSTRAP_FILENAME)
}

/// Read the override path from a given bootstrap file (pure, testable).
/// Returns `None` when the file is missing, the key is absent/empty, or the
/// stored value is not an absolute path.
fn read_override_from(file: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(file).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let raw = value.get(OVERRIDE_KEY)?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let expanded = expand_home(raw);
    if expanded.is_absolute() {
        Some(expanded)
    } else {
        None
    }
}

/// Read the override path from the default bootstrap file.
pub fn read_override() -> Option<PathBuf> {
    read_override_from(&bootstrap_file_path())
}

/// Initialize both session paths together, once per process.
fn resolved_paths() -> &'static ResolvedAppPaths {
    RESOLVED_PATHS.get_or_init(|| {
        ResolvedAppPaths::from_bootstrap(
            &bootstrap_file_path(),
            &default_data_dir(),
            &default_cache_dir(),
        )
    })
}

/// Cache the resolved data dir. Called once at the top of `run()`.
pub fn init_resolved_data_dir() {
    let _ = resolved_paths();
}

/// The data dir in effect for the current session.
pub fn resolved_data_dir() -> PathBuf {
    resolved_paths().data.clone()
}

/// The platform-default cache dir (equivalent to Tauri's `app_cache_dir()`:
/// `LOCALAPPDATA\com.ai-toolbox` on Windows, `~/Library/Caches/com.ai-toolbox`
/// on macOS, `~/.cache/com.ai-toolbox` on Linux). Kept separate from the data
/// dir so transient caches (git clones) stay out of roaming profiles by
/// default.
fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .map(|p| p.join("com.ai-toolbox"))
        .unwrap_or_else(|| default_data_dir().join("cache"))
}

/// The cache dir in effect for the current session. Follows the data-dir
/// override when one is set (so a portable/custom data folder also holds the
/// caches); otherwise returns the OS cache dir to preserve the original
/// non-roaming cache location.
pub fn resolved_cache_dir() -> PathBuf {
    resolved_paths().cache.clone()
}

fn same_directory(left: &Path, right: &Path) -> bool {
    left == right
        || match (fs::canonicalize(left), fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
}

pub(crate) fn configure_image_asset_scope<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    data_dir: &Path,
) -> tauri::Result<()> {
    use tauri::Manager;
    app.asset_protocol_scope()
        .allow_directory(data_dir.join("image-studio/assets"), true)
}

/// Expand a leading `~` to the user's home directory. Non-`~` paths are
/// returned unchanged (callers validate absoluteness separately).
fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(stripped) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(raw)
}

/// Write (or clear) the override path into a given bootstrap file.
/// `path = None` / empty string clears the override (revert to
/// default). Relative paths (after `~` expansion) are rejected.
fn write_override_to_file(file: &Path, path: Option<&str>) -> Result<(), String> {
    let default_dir = file
        .parent()
        .ok_or("Bootstrap file has no parent directory")?;
    let resolved = match path.map(str::trim) {
        Some(p) if !p.is_empty() => {
            let expanded = expand_home(p);
            if !expanded.is_absolute() {
                return Err(format!("数据目录必须是绝对路径: {p}"));
            }
            // Validate before replacing the only pointer to the user's data.
            fs::create_dir_all(&expanded)
                .map_err(|error| format!("无法创建数据目录 {}: {error}", expanded.display()))?;
            tempfile::NamedTempFile::new_in(&expanded)
                .map_err(|error| format!("数据目录不可写 {}: {error}", expanded.display()))?;
            (!same_directory(&expanded, default_dir)).then_some(expanded)
        }
        _ => None,
    };

    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("无法创建 bootstrap 目录 {parent:?}: {e}"))?;
    }

    let mut root = serde_json::Map::new();
    match &resolved {
        Some(p) => {
            root.insert(
                OVERRIDE_KEY.to_string(),
                serde_json::Value::String(p.to_string_lossy().to_string()),
            );
        }
        None => {
            root.insert(OVERRIDE_KEY.to_string(), serde_json::Value::Null);
        }
    }
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .map_err(|e| format!("序列化 bootstrap 失败: {e}"))?;
    // A failed/truncated write must never silently send the next boot to the
    // default DB. Persist atomically on the same filesystem (also on Windows).
    let mut temporary = tempfile::NamedTempFile::new_in(default_dir)
        .map_err(|error| format!("无法创建 bootstrap 临时文件: {error}"))?;
    temporary
        .write_all(text.as_bytes())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("写入 bootstrap 临时文件失败: {error}"))?;
    temporary
        .persist(file)
        .map_err(|error| format!("保存 bootstrap 文件 {} 失败: {error}", file.display()))?;

    log::info!("数据目录覆盖已写入 bootstrap（重启后生效）: {:?}", resolved);
    Ok(())
}

/// Write (or clear) the override path into the bootstrap file.
///
/// Only the on-disk file is updated; the live session cache is intentionally
/// untouched so the already-open DB and caches stay consistent. The new path
/// takes effect on the next restart.
///
/// `path = None` / empty string clears the override (revert to default).
/// Relative paths (after `~` expansion) are rejected.
pub fn set_override(path: Option<&str>) -> Result<(), String> {
    // Initialize the session snapshot before writing even for callers outside run().
    let current = resolved_data_dir();
    let requested = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_home)
        .unwrap_or_else(default_data_dir);
    if !same_directory(&current, &requested) {
        crate::coding::proxy_gateway::cli_proxy::ensure_data_dir_can_change(
            &crate::coding::proxy_gateway::paths::ProxyGatewayPaths::new(current),
        )?;
    }
    write_override_to_file(&bootstrap_file_path(), path)
}

/// Build the info payload for the UI.
pub fn get_override_info() -> AppDataDirInfo {
    build_override_info(
        &bootstrap_file_path(),
        &resolved_data_dir(),
        &default_data_dir(),
    )
}

fn build_override_info(file: &Path, effective: &Path, default: &Path) -> AppDataDirInfo {
    let saved = read_override_from(file).filter(|path| !same_directory(path, default));
    let next_start = saved.as_deref().unwrap_or(default);
    AppDataDirInfo {
        r#override: saved
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        effective: effective.to_string_lossy().into_owned(),
        default: default.to_string_lossy().into_owned(),
        is_custom: !same_directory(effective, default),
        next_start: next_start.to_string_lossy().into_owned(),
        restart_required: !same_directory(effective, next_start),
    }
}

pub(crate) fn ensure_no_pending_data_dir_change() -> Result<(), String> {
    if get_override_info().restart_required {
        return Err("数据目录变更尚未生效，请先重启应用或取消目录变更，再启用网关接管".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Read the current data-directory configuration for the settings UI.
#[tauri::command]
pub fn get_app_data_dir_info() -> AppDataDirInfo {
    get_override_info()
}

/// Set or clear the custom data directory. Requires a restart to take effect.
#[tauri::command]
pub async fn set_app_data_dir_override(path: Option<String>) -> Result<AppDataDirInfo, String> {
    let _transition = DATA_DIR_CHANGE_LOCK.lock().await;
    tauri::async_runtime::spawn_blocking(move || {
        set_override(path.as_deref())?;
        Ok(get_override_info())
    })
    .await
    .map_err(|error| error.to_string())?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_data_dir_under_data_dir() {
        let base = dirs::data_dir().map(|p| p.join("com.ai-toolbox"));
        assert_eq!(default_data_dir(), base.unwrap_or_default());
    }

    #[test]
    fn default_cache_dir_under_cache_dir() {
        let base = dirs::cache_dir().map(|p| p.join("com.ai-toolbox"));
        assert_eq!(default_cache_dir(), base.unwrap_or_default());
    }

    #[test]
    fn resolved_cache_dir_defaults_to_os_cache_without_override() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = ResolvedAppPaths::from_bootstrap(
            &temporary.path().join(BOOTSTRAP_FILENAME),
            temporary.path(),
            &default_cache_dir(),
        );
        assert_eq!(paths.cache, default_cache_dir());
    }

    #[test]
    fn read_override_missing_file_is_none() {
        let tmp = std::env::temp_dir().join(format!(
            "aitb-paths-missing-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        assert!(read_override_from(&tmp).is_none());
    }

    #[test]
    fn read_override_absolute_path() {
        let dir = tmp_override_target();
        let file = write_bootstrap(Some(&dir));
        assert_eq!(read_override_from(&file), Some(dir));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn read_override_empty_is_none() {
        let file = write_bootstrap(Some(Path::new("")));
        assert!(read_override_from(&file).is_none());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn read_override_relative_is_none() {
        let file = write_bootstrap(Some(Path::new("relative/sub/dir")));
        assert!(read_override_from(&file).is_none());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn read_override_tilde_expanded_to_absolute() {
        // `~` expands to home which is absolute, so it should resolve.
        let file = write_bootstrap_tilde();
        let got = read_override_from(&file);
        assert!(got.is_some());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn write_override_round_trips_absolute() {
        let dir = tmp_override_target();
        let file = dir.join("default").join(BOOTSTRAP_FILENAME);
        let target = dir.join("custom");
        write_override_to_file(&file, target.to_str()).unwrap();
        assert_eq!(read_override_from(&file), Some(target.clone()));
        assert!(target.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_override_rejects_relative() {
        let dir = tmp_override_target();
        let file = dir.join(BOOTSTRAP_FILENAME);
        assert!(write_override_to_file(&file, Some("relative/path")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_override_none_clears() {
        let dir = tmp_override_target();
        let file = dir.join("default").join(BOOTSTRAP_FILENAME);
        write_override_to_file(&file, dir.join("custom").to_str()).unwrap();
        assert!(read_override_from(&file).is_some());
        write_override_to_file(&file, None).unwrap();
        assert!(read_override_from(&file).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_override_rejects_relative() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(write_override_to_file(
            &temporary.path().join(BOOTSTRAP_FILENAME),
            Some("relative/path")
        )
        .is_err());
    }

    #[test]
    fn saved_path_and_reset_only_change_the_next_session() {
        let temporary = tempfile::tempdir().unwrap();
        let default = temporary.path().join("default");
        let cache = temporary.path().join("os-cache");
        let file = default.join(BOOTSTRAP_FILENAME);
        let custom = temporary.path().join("custom");
        let session = ResolvedAppPaths::from_bootstrap(&file, &default, &cache);
        write_override_to_file(&file, custom.to_str()).unwrap();
        let info = build_override_info(&file, &session.data, &default);
        assert_eq!(session.data, default);
        assert_eq!(session.cache, cache);
        assert!(!info.is_custom);
        assert!(info.restart_required);
        assert_eq!(info.next_start, custom.to_string_lossy());

        let restarted = ResolvedAppPaths::from_bootstrap(&file, &default, &cache);
        assert_eq!(restarted.data, custom);
        assert_eq!(restarted.cache, custom.join("cache"));
        assert!(!build_override_info(&file, &restarted.data, &default).restart_required);

        write_override_to_file(&file, None).unwrap();
        let info = build_override_info(&file, &restarted.data, &default);
        assert!(info.is_custom);
        assert!(info.restart_required);
        assert_eq!(info.next_start, default.to_string_lossy());
        assert_eq!(restarted.cache, custom.join("cache"));
        assert_eq!(
            ResolvedAppPaths::from_bootstrap(&file, &default, &cache),
            session
        );
    }

    #[test]
    fn selecting_default_or_an_alias_clears_override_and_keeps_os_cache() {
        let temporary = tempfile::tempdir().unwrap();
        let default = temporary.path().join("default");
        let file = default.join(BOOTSTRAP_FILENAME);
        let cache = temporary.path().join("os-cache");
        fs::create_dir_all(default.join("child")).unwrap();
        write_override_to_file(&file, default.join("child/..").to_str()).unwrap();
        assert_eq!(read_override_from(&file), None);
        assert_eq!(
            ResolvedAppPaths::from_bootstrap(&file, &default, &cache).cache,
            cache
        );
        assert!(!build_override_info(&file, &default, &default).restart_required);
    }

    #[test]
    fn invalid_target_preserves_the_previous_bootstrap() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("default").join(BOOTSTRAP_FILENAME);
        let custom = temporary.path().join("custom");
        write_override_to_file(&file, custom.to_str()).unwrap();
        let previous = fs::read(&file).unwrap();
        let invalid = temporary.path().join("not-a-directory");
        fs::write(&invalid, "keep this file").unwrap();
        for target in [
            invalid.clone(),
            invalid.join("child"),
            PathBuf::from("relative/path"),
        ] {
            assert!(write_override_to_file(&file, target.to_str()).is_err());
            assert_eq!(fs::read(&file).unwrap(), previous);
        }
        assert_eq!(fs::read_to_string(invalid).unwrap(), "keep this file");
    }

    #[test]
    fn malformed_bootstrap_uses_defaults_without_overwriting_it() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join(BOOTSTRAP_FILENAME);
        fs::write(&file, "{broken").unwrap();
        assert!(read_override_from(&file).is_none());
        assert_eq!(fs::read_to_string(file).unwrap(), "{broken");
    }

    #[test]
    fn empty_override_resets_and_repeated_writes_replace_atomically() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("default").join(BOOTSTRAP_FILENAME);
        for name in ["first", "second"] {
            let target = temporary.path().join(name);
            write_override_to_file(&file, target.to_str()).unwrap();
            assert_eq!(read_override_from(&file), Some(target));
        }
        write_override_to_file(&file, Some("  ")).unwrap();
        assert!(read_override_from(&file).is_none());
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn default_paths_match_tauri_and_custom_assets_are_readable_without_exposing_db() {
        use tauri::{
            test::{mock_builder, mock_context, noop_assets},
            Manager,
        };
        let mut context = mock_context(noop_assets());
        *context.config_mut() = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let app = mock_builder().build(context).unwrap();
        assert_eq!(default_data_dir(), app.path().app_data_dir().unwrap());
        assert_eq!(default_cache_dir(), app.path().app_cache_dir().unwrap());
        let temporary = tempfile::tempdir().unwrap();
        let assets = temporary.path().join("image-studio/assets/job");
        fs::create_dir_all(&assets).unwrap();
        let image = assets.join("output.png");
        fs::write(&image, "fixture").unwrap();
        let database = temporary.path().join("ai-toolbox.db");
        fs::write(&database, "private fixture").unwrap();
        let scope = app.asset_protocol_scope();
        assert!(!scope.is_allowed(&image));
        configure_image_asset_scope(app.handle(), temporary.path()).unwrap();
        assert!(scope.is_allowed(&image));
        assert!(!scope.is_allowed(&database));
        assert!(!scope.is_allowed(bootstrap_file_path()));
    }

    fn tmp_override_target() -> PathBuf {
        std::env::temp_dir().join(format!(
            "aitb-override-target-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    fn write_bootstrap(path: Option<&Path>) -> PathBuf {
        let file = std::env::temp_dir().join(format!(
            "aitb-bootstrap-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let value = match path {
            Some(p) => {
                let s = p.to_string_lossy().to_string();
                if s.is_empty() {
                    serde_json::Value::String(String::new())
                } else {
                    serde_json::Value::String(s)
                }
            }
            None => serde_json::Value::Null,
        };
        let root = serde_json::json!({ OVERRIDE_KEY: value });
        std::fs::write(&file, serde_json::to_string_pretty(&root).unwrap()).unwrap();
        file
    }

    fn write_bootstrap_tilde() -> PathBuf {
        let file = std::env::temp_dir().join(format!(
            "aitb-bootstrap-tilde-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let root = serde_json::json!({ OVERRIDE_KEY: "~/ai-toolbox-data" });
        std::fs::write(&file, serde_json::to_string_pretty(&root).unwrap()).unwrap();
        file
    }
}
