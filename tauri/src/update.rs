use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri_plugin_updater::UpdaterExt;
use tauri::Emitter;

use crate::db::DbState;
use crate::http_client;

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
}

/// Check for updates from GitHub releases
#[tauri::command]
pub async fn check_for_updates(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
) -> Result<UpdateCheckResult, String> {
    const GITHUB_REPO: &str = "coulsontl/ai-toolbox";
    let latest_json_url = format!(
        "https://github.com/{}/releases/latest/download/latest.json",
        GITHUB_REPO
    );

    // Get current version from package info
    let current_version = app_handle.package_info().version.to_string();

    // Detect current platform
    let current_platform = detect_current_platform();

    // Fetch latest.json using http_client with proxy support
    let client = http_client::client(&state).await?;
    let response = client
        .get(&latest_json_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch latest.json: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch latest.json: HTTP {}",
            response.status()
        ));
    }

    let release: LatestRelease = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse latest.json: {}", e))?;

    let latest_version = release.version.trim_start_matches('v').to_string();

    let has_update = compare_versions(&latest_version, &current_version) > 0;

    // Get signature and url for current platform
    let platform_info = release.platforms.get(&current_platform);
    let signature = platform_info.and_then(|p| p.signature.clone()).filter(|s| !s.is_empty());
    let url = platform_info.and_then(|p| p.url.clone()).filter(|s| !s.is_empty());

    Ok(UpdateCheckResult {
        has_update,
        current_version,
        latest_version: latest_version.clone(),
        release_url: format!(
            "https://github.com/{}/releases/tag/v{}",
            GITHUB_REPO, latest_version
        ),
        release_notes: release.notes.unwrap_or_default(),
        signature,
        url,
    })
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

/// Install the update
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<bool, String> {
    // Check for updates using the updater plugin
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            // Emit download started event
            let _ = app.emit("update-download-progress", serde_json::json!({
                "status": "started",
                "progress": 0,
                "downloaded": 0,
                "total": 0,
                "speed": 0
            }));

            // Download and install with speed calculation
            let downloaded = AtomicU64::new(0);
            let mut last_downloaded = 0u64;
            let mut last_time = Instant::now();
            let mut speed: f64 = 0.0;

            let result = update.download_and_install(
                |chunk_length, content_length| {
                    downloaded.fetch_add(chunk_length as u64, Ordering::SeqCst);
                    let current_downloaded = downloaded.load(Ordering::SeqCst);

                    // Calculate download speed
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_time);

                    if elapsed >= Duration::from_millis(200) {
                        let bytes_since_last = current_downloaded.saturating_sub(last_downloaded);
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
                        let percentage = (current_downloaded as f64 / total as f64 * 100.0) as u32;
                        // Emit progress event with speed
                        let _ = app.emit("update-download-progress", serde_json::json!({
                            "status": "downloading",
                            "progress": percentage,
                            "downloaded": current_downloaded,
                            "total": total,
                            "speed": speed as u64
                        }));
                    }
                },
                || {
                    let current_downloaded = downloaded.load(Ordering::SeqCst);
                    // Emit installing event
                    let _ = app.emit("update-download-progress", serde_json::json!({
                        "status": "installing",
                        "progress": 100,
                        "downloaded": current_downloaded,
                        "total": current_downloaded,
                        "speed": 0
                    }));
                },
            ).await;

            match result {
                Ok(_) => {
                    println!("Update installed successfully");
                    Ok(true)
                }
                Err(e) => {
                    let error_msg = format!("Failed to install update: {}", e);
                    eprintln!("{}", error_msg);
                    Err(error_msg)
                }
            }
        }
        Ok(None) => Err("No update available".to_string()),
        Err(e) => Err(format!("Failed to check for updates: {}", e)),
    }
}

/// Compare two version strings (e.g., "1.2.3" vs "1.2.4")
/// Returns: 1 if v1 > v2, -1 if v1 < v2, 0 if equal
fn compare_versions(v1: &str, v2: &str) -> i32 {
    let parts1: Vec<i32> = v1.split('.').filter_map(|s| s.parse().ok()).collect();
    let parts2: Vec<i32> = v2.split('.').filter_map(|s| s.parse().ok()).collect();

    let max_len = parts1.len().max(parts2.len());

    for i in 0..max_len {
        let num1 = parts1.get(i).copied().unwrap_or(0);
        let num2 = parts2.get(i).copied().unwrap_or(0);

        if num1 > num2 {
            return 1;
        }
        if num1 < num2 {
            return -1;
        }
    }

    0
}
