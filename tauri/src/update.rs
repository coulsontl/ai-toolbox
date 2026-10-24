use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::db::SqliteDbState;
use crate::http_client;

const GITHUB_REPO: &str = "coulsontl/ai-toolbox";
const STABLE_LATEST_JSON_URL: &str =
    "https://github.com/coulsontl/ai-toolbox/releases/latest/download/latest.json";
const BETA_LATEST_JSON_URL: &str =
    "https://github.com/coulsontl/ai-toolbox/releases/download/beta/latest.json";

/// Response from GitHub latest.json
#[derive(Debug, Serialize, Deserialize)]
struct LatestRelease {
    version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    platforms: HashMap<String, PlatformInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PlatformInfo {
    signature: Option<String>,
    url: Option<String>,
}

/// Update check result
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    pub release_notes: String,
    pub signature: Option<String>,
    pub url: Option<String>,
    /// Whether the offered build comes from the rolling prerelease on the
    /// fixed `beta` tag (only ever true with `receive_beta_updates` on).
    pub is_beta_channel: bool,
    /// Whether the running app is managed by Scoop. When true the in-app
    /// installer payload is dropped (see `check_for_updates`) and the user
    /// must upgrade via `scoop update ai-toolbox`.
    pub scoop_install: bool,
    /// Whether the running app was installed from the release `.deb`. When true
    /// the in-app installer payload is dropped (see `check_for_updates`) and the
    /// user must reinstall the package or switch to the AppImage build.
    pub deb_install: bool,
}

/// Check for updates from GitHub releases.
///
/// Uses the DB-backed proxy client when the database is available (normal
/// startup). When the database is not managed yet (recovery / "schema too
/// new" startup path), falls back to `create_client_with_env_proxy` which
/// honors `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` environment variables.
/// Whether update checks also consider the rolling prerelease published on
/// the fixed `beta` tag (AppSettings `receive_beta_updates`). Unknown or
/// missing settings read as off.
fn beta_channel_enabled(app: &tauri::AppHandle) -> bool {
    app.try_state::<SqliteDbState>()
        .and_then(|state| {
            crate::settings::store::load_settings_from_sqlite_state(&state)
                .map(|settings| settings.receive_beta_updates)
                .ok()
        })
        .unwrap_or(false)
}

async fn fetch_latest_release(app: &tauri::AppHandle, url: &str) -> Result<LatestRelease, String> {
    // Prefer DB-backed proxy client; in recovery mode (DB not managed) use
    // an env-proxy client.
    let response = match app.try_state::<SqliteDbState>() {
        Some(state) => {
            let client = http_client::client(&state).await?;
            client.get(url).send().await
        }
        None => {
            let client = http_client::create_client_with_env_proxy(30)?;
            client.get(url).send().await
        }
    }
    .map_err(|e| format!("Failed to fetch latest.json: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch latest.json: HTTP {}",
            response.status()
        ));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse latest.json: {}", e))
}

/// One channel's latest.json plus which channel it came from.
struct UpdateCandidate {
    release: LatestRelease,
    beta_channel: bool,
}

/// Picks the update candidate for this installation.
///
/// With the beta channel off this is simply the stable release. With it on,
/// the candidate is the semver-newer of the stable release and the rolling
/// prerelease on the fixed `beta` tag, so a shipped stable supersedes an
/// older beta and a fresh beta supersedes the stable again. A missing or
/// unreachable beta release degrades to the stable candidate: the beta
/// channel simply has nothing newer to offer yet.
async fn select_update_candidate(app: &tauri::AppHandle) -> Result<UpdateCandidate, String> {
    let stable = fetch_latest_release(app, STABLE_LATEST_JSON_URL).await?;
    if !beta_channel_enabled(app) {
        return Ok(UpdateCandidate {
            release: stable,
            beta_channel: false,
        });
    }

    match fetch_latest_release(app, BETA_LATEST_JSON_URL).await {
        Ok(beta) => {
            let beta_version = beta.version.trim_start_matches('v');
            let stable_version = stable.version.trim_start_matches('v');
            if compare_versions(beta_version, stable_version) > 0 {
                Ok(UpdateCandidate {
                    release: beta,
                    beta_channel: true,
                })
            } else {
                Ok(UpdateCandidate {
                    release: stable,
                    beta_channel: false,
                })
            }
        }
        Err(_) => Ok(UpdateCandidate {
            release: stable,
            beta_channel: false,
        }),
    }
}

#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateCheckResult, String> {
    // Get current version from package info
    let current_version = app.package_info().version.to_string();

    // Detect current platform
    let current_platform = detect_current_platform();

    let candidate = select_update_candidate(&app).await?;
    let release = candidate.release;
    let is_beta_channel = candidate.beta_channel;

    let latest_version = release.version.trim_start_matches('v').to_string();

    // Defense against a partial manifest (a build round still uploading, or
    // any channel publishing for other platforms only): a version whose
    // platforms map has no entry for this machine is not an update for it,
    // whatever the version number says.
    let platform_ready = release
        .platforms
        .get(&current_platform)
        .is_some_and(|info| info.url.as_deref().is_some_and(|url| !url.is_empty()));

    let has_update = platform_ready && compare_versions(&latest_version, &current_version) > 0;

    // Get signature and url for current platform
    let platform_info = release.platforms.get(&current_platform);
    let mut signature = platform_info
        .and_then(|p| p.signature.clone())
        .filter(|s| !s.is_empty());
    let mut url = platform_info
        .and_then(|p| p.url.clone())
        .filter(|s| !s.is_empty());

    let scoop_install = is_scoop_install();
    let deb_install = is_deb_install_async().await;

    // Package-managed installs cannot be replaced by the in-app updater:
    // - Scoop lives under `<scoop>\apps\...`; running the in-app NSIS updater
    //   would create a second copy outside Scoop's control while the
    //   Scoop-managed copy stays stale.
    // - A dpkg install lives at `/usr/bin/...`, which makes
    //   `tauri-plugin-updater` take its deb branch and reject the AppImage
    //   bytes that `latest.json` publishes under `linux-x86_64`.
    // Drop the installer payload so every frontend path degrades to opening
    // the release page instead.
    if scoop_install || deb_install {
        signature = None;
        url = None;
    }

    Ok(UpdateCheckResult {
        has_update,
        current_version,
        latest_version: latest_version.clone(),
        release_url: if is_beta_channel {
            format!("https://github.com/{}/releases/tag/beta", GITHUB_REPO)
        } else {
            format!(
                "https://github.com/{}/releases/tag/v{}",
                GITHUB_REPO, latest_version
            )
        },
        release_notes: release.notes.unwrap_or_default(),
        signature,
        url,
        is_beta_channel,
        scoop_install,
        deb_install,
    })
}

/// Whether the running executable is managed by Scoop (Windows package manager).
#[allow(unreachable_code)]
pub fn is_scoop_install() -> bool {
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe_path) = std::env::current_exe() {
            let roots: Vec<String> = ["SCOOP", "SCOOP_GLOBAL"]
                .into_iter()
                .filter_map(|name| std::env::var_os(name))
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            return is_scoop_install_path(&exe_path.to_string_lossy(), &roots);
        }
        return false;
    }

    false
}

/// Pure path check for Scoop-managed executables (case-insensitive).
#[cfg(any(target_os = "windows", test))]
fn is_scoop_install_path(exe_path: &str, roots: &[String]) -> bool {
    let normalize = |path: &str| {
        let normalized = path.replace('/', "\\").to_lowercase();
        let normalized = if let Some(suffix) = normalized.strip_prefix(r"\\?\unc\") {
            format!(r"\\{suffix}")
        } else {
            normalized
                .strip_prefix(r"\\?\")
                .unwrap_or(&normalized)
                .to_string()
        };
        normalized.trim_end_matches('\\').to_string()
    };
    let normalized_exe = normalize(exe_path);
    normalized_exe.contains("\\scoop\\apps\\")
        || roots
            .iter()
            .filter(|root| !root.trim().is_empty())
            .any(|root| normalized_exe.starts_with(&format!("{}\\apps\\", normalize(root))))
}

/// Whether the running executable is managed by the system package manager
/// (`dpkg`), i.e. the app was installed from the release `.deb`.
///
/// `tauri-plugin-updater` picks the install strategy from the *running binary*,
/// not from the payload: an executable under `/usr/...` that `dpkg -S` claims
/// selects its `install_deb` branch, and that branch rejects the AppImage bytes
/// published under `linux-x86_64` with `InvalidUpdaterFormat`. Publishing a deb
/// payload instead would not fix it either — `install_deb` needs `pkexec`,
/// `zenity` / `kdialog`, or an interactive terminal `sudo` for elevation, none
/// of which exist in a plain GUI session (and none at all inside WSL). So a deb
/// install can never self-update; report it and let the user reinstall the
/// package or switch to the AppImage build.
#[allow(unreachable_code)]
pub fn is_deb_install() -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(exe_path) = std::env::current_exe() else {
            return false;
        };
        let exe_path = exe_path.to_string_lossy().into_owned();
        if !is_deb_install_path(&exe_path) {
            return false;
        }
        // Only trust `/usr/...` when the package database actually claims the
        // file, so an AppImage dropped into `/usr/local` by hand still keeps the
        // in-app updater.
        return std::process::Command::new("dpkg")
            .args(["-S", exe_path.as_str()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
    }

    false
}

/// Pure path check for dpkg-managed executables: the deb bundle always installs
/// the binary to `/usr/bin/<name>`.
#[cfg(any(target_os = "linux", test))]
fn is_deb_install_path(exe_path: &str) -> bool {
    exe_path.starts_with("/usr/")
}

/// `is_deb_install` shells out to `dpkg`, which can block for a long time behind
/// a package-manager lock; keep it off the async runtime (see "Async Runtime
/// Safety" in `AGENTS.md`).
async fn is_deb_install_async() -> bool {
    tauri::async_runtime::spawn_blocking(is_deb_install)
        .await
        .unwrap_or(false)
}

/// Detect current platform string for matching latest.json
#[allow(unreachable_code)]
fn detect_current_platform() -> String {
    #[cfg(target_os = "windows")]
    {
        return "windows-x86_64".to_string();
    }

    #[cfg(target_os = "linux")]
    {
        return "linux-x86_64".to_string();
    }

    #[cfg(target_os = "macos")]
    {
        #[cfg(target_arch = "aarch64")]
        {
            return "darwin-aarch64".to_string();
        }
        #[cfg(target_arch = "x86_64")]
        {
            return "darwin-x86_64".to_string();
        }
    }

    "unknown".to_string()
}

/// Run the updater plugin: check for an available update and, if present,
/// download + install it while emitting `update-download-progress` events.
///
/// This is DB-free; proxy handling is the caller's responsibility (the
/// `tauri-plugin-updater` reads `HTTP_PROXY` / `HTTPS_PROXY` env vars).
async fn run_updater_download(app: &tauri::AppHandle) -> Result<bool, String> {
    // Re-run the channel selection instead of caching the check result:
    // another channel may have published between check and install. The
    // plugin's own endpoint list is failover, not newest-wins, so exactly one
    // endpoint — the winning channel's — is configured here.
    let candidate = select_update_candidate(app).await?;
    let updater = if candidate.beta_channel {
        let endpoint = BETA_LATEST_JSON_URL
            .parse()
            .map_err(|e| format!("Invalid beta update endpoint: {e}"))?;
        app.updater_builder()
            .endpoints(vec![endpoint])
            .map_err(|e| format!("Failed to configure the beta update endpoint: {e}"))?
            .build()
            .map_err(|e| format!("Failed to build the updater: {e}"))?
    } else {
        app.updater().map_err(|e| e.to_string())?
    };
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Failed to check for updates: {}", e))?;

    match update {
        Some(update) => {
            // Emit download started event
            let _ = app.emit(
                "update-download-progress",
                serde_json::json!({
                    "status": "started",
                    "progress": 0,
                    "downloaded": 0,
                    "total": 0,
                    "speed": 0
                }),
            );

            // Download and install with speed calculation
            let downloaded = AtomicU64::new(0);
            let mut last_downloaded = 0u64;
            let mut last_time = Instant::now();
            let mut speed: f64 = 0.0;

            let install_result = update
                .download_and_install(
                    |chunk_length, content_length| {
                        downloaded.fetch_add(chunk_length as u64, Ordering::SeqCst);
                        let current_downloaded = downloaded.load(Ordering::SeqCst);

                        // Calculate download speed
                        let now = Instant::now();
                        let elapsed = now.duration_since(last_time);

                        if elapsed >= Duration::from_millis(200) {
                            let bytes_since_last =
                                current_downloaded.saturating_sub(last_downloaded);
                            if bytes_since_last > 0 {
                                // Speed in bytes per second
                                let speed_calc = bytes_since_last as f64 / elapsed.as_secs_f64();
                                // Use exponential moving average for smoother display
                                if speed == 0.0 {
                                    speed = speed_calc;
                                } else {
                                    speed = speed * 0.7 + speed_calc * 0.3;
                                }
                            }
                            last_downloaded = current_downloaded;
                            last_time = now;
                        }

                        if let Some(total) = content_length {
                            let percentage =
                                (current_downloaded as f64 / total as f64 * 100.0) as u32;
                            // Emit progress event with speed
                            let _ = app.emit(
                                "update-download-progress",
                                serde_json::json!({
                                    "status": "downloading",
                                    "progress": percentage,
                                    "downloaded": current_downloaded,
                                    "total": total,
                                    "speed": speed as u64
                                }),
                            );
                        }
                    },
                    || {
                        let current_downloaded = downloaded.load(Ordering::SeqCst);
                        // Emit installing event
                        let _ = app.emit(
                            "update-download-progress",
                            serde_json::json!({
                                "status": "installing",
                                "progress": 100,
                                "downloaded": current_downloaded,
                                "total": current_downloaded,
                                "speed": 0
                            }),
                        );
                    },
                )
                .await;

            match install_result {
                Ok(_) => {
                    println!("Update installed successfully");
                    Ok(true)
                }
                Err(e) => {
                    let error_msg = format!("Failed to install update: {}", e);
                    // `tauri-plugin-updater`'s own records are filtered out of the
                    // app log file (see `setup_logging`), so keep the reason on a
                    // target the file accepts. Without this a failed auto-update
                    // leaves no trace anywhere the user can retrieve.
                    log::warn!("{}", error_msg);
                    eprintln!("{}", error_msg);
                    Err(error_msg)
                }
            }
        }
        None => Err("No update available".to_string()),
    }
}

/// Install the update.
///
/// When the database is available (normal startup), the in-app proxy config is
/// read from the DB and applied to `HTTP_PROXY` / `HTTPS_PROXY` env vars so
/// the updater plugin picks it up. In recovery mode (DB not managed), env
/// vars are left untouched and the updater inherits system / environment
/// proxy settings.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<bool, String> {
    if is_scoop_install() {
        return Err(
            "Scoop-managed installations must be updated with scoop update ai-toolbox".to_string(),
        );
    }
    if is_deb_install_async().await {
        return Err(
            "deb-managed installations must be updated by reinstalling the .deb package or by switching to the AppImage build".to_string(),
        );
    }
    // Snapshot proxy-related env vars so we can always restore them, regardless
    // of whether we touched them (no-op restore in recovery mode).
    let old_http_proxy = std::env::var("HTTP_PROXY").ok();
    let old_https_proxy = std::env::var("HTTPS_PROXY").ok();
    let old_http_proxy_lower = std::env::var("http_proxy").ok();
    let old_https_proxy_lower = std::env::var("https_proxy").ok();

    // Only apply DB-backed proxy when the database is actually available.
    if let Some(state) = app.try_state::<SqliteDbState>() {
        let (proxy_mode, proxy_url) = http_client::get_proxy_from_settings(&state).await?;

        // Set proxy environment variables for the updater plugin
        // (tauri-plugin-updater reads these env vars for proxy configuration)
        match proxy_mode {
            http_client::ProxyMode::Direct => {
                std::env::remove_var("HTTP_PROXY");
                std::env::remove_var("HTTPS_PROXY");
                std::env::remove_var("http_proxy");
                std::env::remove_var("https_proxy");
            }
            http_client::ProxyMode::Custom => {
                if !proxy_url.is_empty() {
                    std::env::set_var("HTTP_PROXY", &proxy_url);
                    std::env::set_var("HTTPS_PROXY", &proxy_url);
                    std::env::set_var("http_proxy", &proxy_url);
                    std::env::set_var("https_proxy", &proxy_url);
                }
            }
            http_client::ProxyMode::System => {}
        }
    }
    // else: recovery mode — leave env vars as-is (system / env proxy).

    let result = run_updater_download(&app).await;

    // Restore original environment variables
    if let old @ Some(_) = old_http_proxy {
        std::env::set_var("HTTP_PROXY", old.unwrap());
    } else {
        std::env::remove_var("HTTP_PROXY");
    }
    if let old @ Some(_) = old_https_proxy {
        std::env::set_var("HTTPS_PROXY", old.unwrap());
    } else {
        std::env::remove_var("HTTPS_PROXY");
    }
    if let old @ Some(_) = old_http_proxy_lower {
        std::env::set_var("http_proxy", old.unwrap());
    } else {
        std::env::remove_var("http_proxy");
    }
    if let old @ Some(_) = old_https_proxy_lower {
        std::env::set_var("https_proxy", old.unwrap());
    } else {
        std::env::remove_var("https_proxy");
    }

    result
}

/// Compare two version strings (e.g., "1.2.3" vs "1.2.4")
/// Returns: 1 if v1 > v2, -1 if v1 < v2, 0 if equal
/// One dot- or hyphen-separated semver identifier. Numeric identifiers
/// compare numerically and rank below alphanumeric ones (semver §11).
enum VersionIdentifier {
    Numeric(u64),
    Alphanumeric(String),
}

fn parse_identifiers(text: &str) -> Vec<VersionIdentifier> {
    text.split('.')
        .map(|part| match part.parse::<u64>() {
            Ok(number) if !part.starts_with('0') || part == "0" => {
                VersionIdentifier::Numeric(number)
            }
            _ => VersionIdentifier::Alphanumeric(part.to_string()),
        })
        .collect()
}

fn compare_identifiers(left: &[VersionIdentifier], right: &[VersionIdentifier]) -> i32 {
    for index in 0..left.len().max(right.len()) {
        let ordering = match (left.get(index), right.get(index)) {
            (Some(VersionIdentifier::Numeric(a)), Some(VersionIdentifier::Numeric(b))) => a.cmp(b),
            (Some(VersionIdentifier::Numeric(_)), Some(VersionIdentifier::Alphanumeric(_))) => {
                std::cmp::Ordering::Less
            }
            (Some(VersionIdentifier::Alphanumeric(_)), Some(VersionIdentifier::Numeric(_))) => {
                std::cmp::Ordering::Greater
            }
            (
                Some(VersionIdentifier::Alphanumeric(a)),
                Some(VersionIdentifier::Alphanumeric(b)),
            ) => a.cmp(b),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        };
        match ordering {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => continue,
        }
    }
    0
}

/// Full semver comparison, prerelease identifiers included.
///
/// Beyond the plain numeric core this encodes the two rules the rolling beta
/// channel relies on: `1.2.1 > 1.2.1-beta.<stamp>` (a shipped stable
/// supersedes its betas) and `beta.261024b > beta.261024a` (alphanumeric
/// identifiers compare per ASCII, which keeps dictionary order equal to
/// chronological order for the `YYMMDD<letter>` stamp). Build metadata after
/// `+` is ignored, as semver prescribes.
fn split_version(value: &str) -> (&str, Option<&str>) {
    match value.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (value, None),
    }
}

fn strip_version(value: &str) -> &str {
    value.trim().trim_start_matches('v')
}

fn compare_versions(v1: &str, v2: &str) -> i32 {
    let core_numbers = |core: &str| -> Vec<u64> {
        core.split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };

    let (core1, prerelease1) = split_version(strip_version(v1));
    let (core2, prerelease2) = split_version(strip_version(v2));
    let numbers1 = core_numbers(core1);
    let numbers2 = core_numbers(core2);

    for index in 0..numbers1.len().max(numbers2.len()) {
        let a = numbers1.get(index).copied().unwrap_or(0);
        let b = numbers2.get(index).copied().unwrap_or(0);
        if a != b {
            return if a > b { 1 } else { -1 };
        }
    }

    match (prerelease1, prerelease2) {
        (None, None) => 0,
        // A version without a prerelease outranks the same version with one.
        (None, Some(_)) => 1,
        (Some(_), None) => -1,
        (Some(left), Some(right)) => {
            compare_identifiers(&parse_identifiers(left), &parse_identifiers(right))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_deb_install_path, is_scoop_install_path};

    #[test]
    fn detects_per_user_scoop_install() {
        assert!(is_scoop_install_path(
            "C:\\Users\\foo\\scoop\\apps\\ai-toolbox\\1.1.4\\ai-toolbox.exe",
            &[],
        ));
    }

    #[test]
    fn detects_global_and_verbatim_paths_case_insensitively() {
        assert!(is_scoop_install_path(
            "C:\\Program Files\\Scoop\\apps\\ai-toolbox\\1.1.4\\ai-toolbox.exe",
            &[],
        ));
        assert!(is_scoop_install_path(
            "\\\\?\\D:\\Scoop\\Apps\\ai-toolbox\\1.1.4\\ai-toolbox.exe",
            &[],
        ));
    }

    #[test]
    fn rejects_non_scoop_paths() {
        assert!(!is_scoop_install_path(
            "C:\\Program Files\\AI Toolbox\\ai-toolbox.exe",
            &[],
        ));
        assert!(!is_scoop_install_path(
            "C:\\Users\\foo\\AppData\\Local\\AI Toolbox\\ai-toolbox.exe",
            &[],
        ));
        // "scoopless\apps" must not match "\scoop\apps\"
        assert!(!is_scoop_install_path(
            "C:\\Users\\foo\\scoopless\\apps\\ai-toolbox\\ai-toolbox.exe",
            &[],
        ));
    }

    #[test]
    fn detects_deb_install_paths() {
        // The deb bundle installs the binary to `/usr/bin/<name>`.
        assert!(is_deb_install_path("/usr/bin/ai-toolbox"));
        assert!(is_deb_install_path("/usr/lib/AI Toolbox/ai-toolbox"));
    }

    #[test]
    fn rejects_non_deb_install_paths() {
        // An AppImage anywhere outside `/usr` keeps the in-app updater.
        assert!(!is_deb_install_path(
            "/home/foo/Downloads/AI.Toolbox_1.1.7_amd64.AppImage"
        ));
        assert!(!is_deb_install_path(
            "/mnt/c/Users/foo/Downloads/AI.Toolbox_1.1.7_amd64.AppImage"
        ));
        // `/usrx` must not match the `/usr/` prefix.
        assert!(!is_deb_install_path("/usrx/bin/ai-toolbox"));
        // Windows paths never match.
        assert!(!is_deb_install_path(
            "C:\\Program Files\\AI Toolbox\\ai-toolbox.exe"
        ));
    }

    #[test]
    fn detects_custom_scoop_roots_without_matching_adjacent_directories() {
        let roots = vec![r"D:\Tools\".to_string(), "E:/Global Packages".to_string()];
        assert!(is_scoop_install_path(
            r"\\?\D:\Tools\apps\ai-toolbox\current\ai-toolbox.exe",
            &roots
        ));
        assert!(is_scoop_install_path(
            "E:/Global Packages/apps/ai-toolbox/1.1.4/ai-toolbox.exe",
            &roots
        ));
        assert!(!is_scoop_install_path(
            r"D:\Tools-old\apps\ai-toolbox\ai-toolbox.exe",
            &roots
        ));
        assert!(!is_scoop_install_path(
            r"D:\Tools\apps-backup\ai-toolbox.exe",
            &roots
        ));
        assert!(is_scoop_install_path(
            r"\\?\UNC\server\tools\apps\ai-toolbox\current\ai-toolbox.exe",
            &[r"\\server\tools".to_string()],
        ));
    }

    /// The rolling beta timeline every channel decision depends on: a beta
    /// outranks the older stable, a shipped stable outranks its own betas,
    /// and a same-day rebuild outranks the earlier build by stamp.
    #[test]
    fn semver_comparison_covers_the_rolling_beta_timeline() {
        use super::compare_versions as cmp;

        // Plain core ordering still decides first.
        assert_eq!(cmp("1.2.1-beta.261024a", "1.2.0"), 1);
        assert_eq!(cmp("1.2.0", "1.2.1-beta.261024a"), -1);
        // A shipped stable supersedes its own betas: no prerelease wins ties.
        assert_eq!(cmp("1.2.1", "1.2.1-beta.261025b"), 1);
        assert_eq!(cmp("1.2.1-beta.261025b", "1.2.1"), -1);
        // Same base, later stamp wins (ASCII keeps dictionary order equal to
        // chronological order across days, months, and years).
        assert_eq!(cmp("1.2.1-beta.261024b", "1.2.1-beta.261024a"), 1);
        assert_eq!(cmp("1.2.1-beta.261024z", "1.2.1-beta.261024a"), 1);
        assert_eq!(cmp("1.2.1-beta.261024za", "1.2.1-beta.261024z"), 1);
        assert_eq!(cmp("1.2.1-beta.261025a", "1.2.1-beta.261024z"), 1);
        assert_eq!(cmp("1.2.1-beta.261101a", "1.2.1-beta.261031a"), 1);
        assert_eq!(cmp("1.2.2-beta.270301a", "1.2.1-beta.261231z"), 1);
        // Equal is equal.
        assert_eq!(cmp("1.2.1-beta.261024a", "1.2.1-beta.261024a"), 0);
        assert_eq!(cmp("1.2.0", "1.2.0"), 0);
        // A newer core on the beta side still loses to a newer stable core.
        assert_eq!(cmp("1.2.2-beta.261024a", "1.2.3"), -1);
        // The `v` prefix and stray whitespace are tolerated.
        assert_eq!(cmp("v1.2.1", " 1.2.0 "), 1);
    }
}
