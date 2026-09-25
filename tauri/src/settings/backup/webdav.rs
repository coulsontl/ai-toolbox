use log::{error, info};
use regex::Regex;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use super::filename::backup_sort_key;
use super::generate::generate_backup_file;
use super::restore::{prepare_backup_bytes, restore_from_archive};
use super::utils::RestoreResult;
use crate::db::SqliteDbState;
use crate::http_client;

/// Backup file info structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFileInfo {
    pub filename: String,
    pub size: u64,
    pub encrypted: bool,
}

/// WebDAV 错误类型
#[derive(Debug, Clone)]
pub struct WebDAVError {
    pub error_type: String,
    pub message: String,
    pub suggestion: String,
}

impl WebDAVError {
    fn new(error_type: &str, message: &str, suggestion: &str) -> Self {
        Self {
            error_type: error_type.to_string(),
            message: message.to_string(),
            suggestion: suggestion.to_string(),
        }
    }

    fn to_json(&self) -> String {
        serde_json::json!({
            "type": self.error_type,
            "message": self.message,
            "suggestion": self.suggestion
        })
        .to_string()
    }
}

/// 分析 HTTP 错误并返回详细信息
fn analyze_http_error(status: reqwest::StatusCode, url: &str) -> WebDAVError {
    match status.as_u16() {
        401 => WebDAVError::new(
            "AUTH_FAILED",
            "Authentication failed",
            "settings.webdav.errors.authFailed",
        ),
        403 => WebDAVError::new(
            "FORBIDDEN",
            "Access forbidden",
            "settings.webdav.errors.authFailed",
        ),
        404 => WebDAVError::new(
            "PATH_NOT_FOUND",
            "Remote path not found",
            "settings.webdav.errors.pathNotFound",
        ),
        405 => WebDAVError::new(
            "NOT_SUPPORTED",
            "Server does not support WebDAV",
            "settings.webdav.errors.notSupported",
        ),
        500 | 502 | 503 => WebDAVError::new(
            "SERVER_ERROR",
            &format!("Server error: {}", status),
            "settings.webdav.errors.serverError",
        ),
        _ => WebDAVError::new(
            "HTTP_ERROR",
            &format!("HTTP error: {} ({})", status, url),
            "settings.webdav.suggestions.contactAdmin",
        ),
    }
}

/// 分析 WebDAV 下载（GET）失败响应，并识别「下载被重定向到外部 host」的场景。
///
/// 某些 WebDAV 服务端（如 OpenList/AList 开启「302 重定向」下载策略）对 GET 返回 3xx，
/// 把下载重定向到上游网盘的 CDN 签名地址（如 115 防盗链链接）。通用客户端拿不到 CDN
/// 要求的 Cookie/IP 绑定信息，会得到 403；这种 403 跟“账号认证失败”无关，直接映射成
/// `authFailed` 会误导用户。当最终响应 host 与配置的 WebDAV host 不同且返回 403 时，
/// 返回专门针对该场景的诊断提示；其余情况沿用通用 `analyze_http_error` 语义。
fn analyze_download_error(resp: &reqwest::Response, original_url: &str) -> WebDAVError {
    let original_host = reqwest::Url::parse(original_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned));
    let redirected_external = original_host
        .as_deref()
        .map(|origin| resp.url().host_str() != Some(origin))
        .unwrap_or(false);

    if resp.status().as_u16() == 403 && redirected_external {
        WebDAVError::new(
            "DOWNLOAD_REDIRECT",
            "Download was redirected to an external CDN that refused access",
            "settings.webdav.errors.downloadRedirect",
        )
    } else {
        analyze_http_error(resp.status(), original_url)
    }
}

/// 分析 reqwest 错误并返回详细信息
fn analyze_reqwest_error(err: &reqwest::Error, url: &str) -> WebDAVError {
    if err.is_timeout() {
        WebDAVError::new(
            "TIMEOUT",
            "Connection timeout",
            "settings.webdav.errors.timeout",
        )
    } else if err.is_connect() {
        WebDAVError::new(
            "NETWORK_ERROR",
            "Network connection failed",
            "settings.webdav.errors.networkError",
        )
    } else if err.to_string().contains("certificate") || err.to_string().contains("SSL") {
        WebDAVError::new(
            "SSL_ERROR",
            "SSL/TLS certificate error",
            "settings.webdav.errors.sslError",
        )
    } else {
        WebDAVError::new(
            "UNKNOWN_ERROR",
            &format!("Request failed: {} ({})", err, url),
            "settings.webdav.suggestions.contactAdmin",
        )
    }
}

/// Test WebDAV connection
#[tauri::command]
pub async fn test_webdav_connection(
    state: tauri::State<'_, SqliteDbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
) -> Result<(), String> {
    info!("Testing WebDAV connection to: {}", url);

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let folder_url = if remote.is_empty() {
        format!("{}/", base_url)
    } else {
        format!("{}/{}/", base_url, remote)
    };

    // Send PROPFIND request to test connection
    let client = http_client::client(&state).await.map_err(|e| {
        error!("Failed to create HTTP client: {}", e);
        e
    })?;

    let response = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
            &folder_url,
        )
        .basic_auth(&username, Some(&password))
        .header("Depth", "0")
        .send()
        .await;

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                info!("WebDAV connection test successful");
                Ok(())
            } else {
                let error = analyze_http_error(resp.status(), &folder_url);
                error!("WebDAV connection test failed: {:?}", error);
                Err(error.to_json())
            }
        }
        Err(e) => {
            let error = analyze_reqwest_error(&e, &folder_url);
            error!("WebDAV connection test failed: {:?}", error);
            Err(error.to_json())
        }
    }
}

/// Backup database to WebDAV server
#[tauri::command]
pub async fn backup_to_webdav(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
    host_label: String,
) -> Result<String, String> {
    info!("Starting WebDAV backup to: {}", url);

    // Shared generation layer: zip + optional encryption + shared filename.
    let generated = generate_backup_file(&app_handle, Some(&host_label)).await?;

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, generated.filename)
    } else {
        format!("{}/{}/{}", base_url, remote, generated.filename)
    };

    info!("Uploading backup to: {}", full_url);

    // Upload to WebDAV using PUT request with proxy support
    let client = http_client::client_with_timeout(&state, 300)
        .await
        .map_err(|e| {
            error!("Failed to create HTTP client: {}", e);
            e
        })?;

    let response = client
        .put(&full_url)
        .basic_auth(&username, Some(&password))
        .body(generated.bytes)
        .send()
        .await;

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                info!("WebDAV backup successful: {}", full_url);
                Ok(full_url)
            } else {
                let error = analyze_http_error(resp.status(), &full_url);
                error!("WebDAV backup failed: {:?}", error);
                Err(error.to_json())
            }
        }
        Err(e) => {
            let error = analyze_reqwest_error(&e, &full_url);
            error!("WebDAV backup failed: {:?}", error);
            Err(error.to_json())
        }
    }
}

/// Internal function: List backup files from WebDAV server
pub(crate) async fn list_webdav_backups_internal(
    db_state: &SqliteDbState,
    url: &str,
    username: &str,
    password: &str,
    remote_path: &str,
) -> Result<Vec<BackupFileInfo>, String> {
    info!("Listing WebDAV backups from: {}", url);

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let folder_url = if remote.is_empty() {
        format!("{}/", base_url)
    } else {
        format!("{}/{}/", base_url, remote)
    };

    // Send PROPFIND request to list files with proxy support
    let client = http_client::client(db_state).await.map_err(|e| {
        error!("Failed to create HTTP client: {}", e);
        e
    })?;

    let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:allprop/>
</d:propfind>"#;

    let response = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
            &folder_url,
        )
        .basic_auth(username, Some(password))
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(propfind_body)
        .send()
        .await;

    let body = match response {
        Ok(resp) => {
            if resp.status().is_success() {
                resp.text().await.map_err(|e| {
                    error!("Failed to read response: {}", e);
                    format!("Failed to read response: {}", e)
                })?
            } else {
                let error = analyze_http_error(resp.status(), &folder_url);
                error!("Failed to list WebDAV backups: {:?}", error);
                return Err(error.to_json());
            }
        }
        Err(e) => {
            let error = analyze_reqwest_error(&e, &folder_url);
            error!("Failed to list WebDAV backups: {:?}", error);
            return Err(error.to_json());
        }
    };

    // Parse XML response to extract backup files with sizes
    // WebDAV servers use different namespace prefixes: <D:response>, <d:response>, or <response>
    // e.g. 坚果云 (Jianguoyun) uses lowercase <d:response>
    // The filename regex must capture the full `.zip` / `.zip.enc` suffix — a bare
    // `\.zip` would silently truncate encrypted backups and hide them from restore.
    let filename_re = Regex::new(r"ai-toolbox-backup-.*?\d{8}-\d{6}[^.]*\.zip(?:\.enc)?").unwrap();
    let response_re = Regex::new(r"(?i)<[\w]*:?response[>\s]").unwrap();
    let size_re =
        Regex::new(r"(?i)<[\w]*:?getcontentlength>(\d+)</[\w]*:?getcontentlength>").unwrap();

    let mut backups = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Split body into response blocks using case-insensitive matching
    let response_starts: Vec<usize> = response_re.find_iter(&body).map(|m| m.start()).collect();
    for (i, &start) in response_starts.iter().enumerate() {
        let end = response_starts.get(i + 1).copied().unwrap_or(body.len());
        let response_block = &body[start..end];

        // Try to find a filename in this block
        if let Some(filename_match) = filename_re.find(response_block) {
            let filename = filename_match.as_str().to_string();

            // Skip if already seen
            if !seen.insert(filename.clone()) {
                continue;
            }

            // Try to find size in the same block
            let size = if let Some(size_match) = size_re.captures(response_block) {
                size_match
                    .get(1)
                    .and_then(|m| m.as_str().parse::<u64>().ok())
                    .unwrap_or(0)
            } else {
                0
            };

            let encrypted = filename.ends_with(".zip.enc");
            backups.push(BackupFileInfo {
                filename,
                size,
                encrypted,
            });
        }
    }

    // Sort by shared filename contract (descending = most recent first)
    backups.sort_by(|a, b| backup_sort_key(&b.filename).cmp(&backup_sort_key(&a.filename)));

    info!("Found {} backup files", backups.len());
    Ok(backups)
}

/// List backup files from WebDAV server
#[tauri::command]
pub async fn list_webdav_backups(
    state: tauri::State<'_, SqliteDbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
) -> Result<Vec<BackupFileInfo>, String> {
    list_webdav_backups_internal(&state, &url, &username, &password, &remote_path).await
}

/// Internal function: Delete a backup file from WebDAV server
pub(crate) async fn delete_webdav_backup_internal(
    db_state: &SqliteDbState,
    url: &str,
    username: &str,
    password: &str,
    remote_path: &str,
    filename: &str,
) -> Result<(), String> {
    info!("Deleting WebDAV backup: {}", filename);

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, filename)
    } else {
        format!("{}/{}/{}", base_url, remote, filename)
    };

    // Send DELETE request
    let client = http_client::client(db_state).await.map_err(|e| {
        error!("Failed to create HTTP client: {}", e);
        e
    })?;

    let response = client
        .delete(&full_url)
        .basic_auth(username, Some(password))
        .send()
        .await;

    match response {
        Ok(resp) => {
            if resp.status().is_success() || resp.status().as_u16() == 204 {
                info!("WebDAV backup deleted successfully: {}", filename);
                Ok(())
            } else {
                let error = analyze_http_error(resp.status(), &full_url);
                error!("Failed to delete WebDAV backup: {:?}", error);
                Err(error.to_json())
            }
        }
        Err(e) => {
            let error = analyze_reqwest_error(&e, &full_url);
            error!("Failed to delete WebDAV backup: {:?}", error);
            Err(error.to_json())
        }
    }
}

/// Delete a backup file from WebDAV server
#[tauri::command]
pub async fn delete_webdav_backup(
    state: tauri::State<'_, SqliteDbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
    filename: String,
) -> Result<(), String> {
    delete_webdav_backup_internal(&state, &url, &username, &password, &remote_path, &filename).await
}

/// Restore database from WebDAV server
#[tauri::command]
pub async fn restore_from_webdav(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
    filename: String,
    skip_cli_custom_roots: Option<bool>,
    restore_password: Option<String>,
) -> Result<RestoreResult, String> {
    let skip_cli_custom_roots = skip_cli_custom_roots.unwrap_or(false);
    info!("Starting WebDAV restore from: {}/{}", url, filename);

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, filename)
    } else {
        format!("{}/{}/{}", base_url, remote, filename)
    };

    info!("Downloading backup from: {}", full_url);

    // Download from WebDAV with proxy support
    let client = http_client::client_with_timeout(&state, 300)
        .await
        .map_err(|e| {
            error!("Failed to create HTTP client: {}", e);
            e
        })?;

    let response = client
        .get(&full_url)
        .basic_auth(&username, Some(&password))
        .send()
        .await;

    let zip_data = match response {
        Ok(resp) => {
            if resp.status().is_success() {
                resp.bytes().await.map_err(|e| {
                    error!("Failed to read response: {}", e);
                    format!("Failed to read response: {}", e)
                })?
            } else {
                let error = analyze_download_error(&resp, &full_url);
                error!("WebDAV download failed: {:?}", error);
                return Err(error.to_json());
            }
        }
        Err(e) => {
            let error = analyze_reqwest_error(&e, &full_url);
            error!("WebDAV download failed: {:?}", error);
            return Err(error.to_json());
        }
    };

    info!("Extracting backup archive...");

    // Decrypt if needed (header-based detection) before any restore write happens.
    let zip_data = tauri::async_runtime::spawn_blocking(move || {
        prepare_backup_bytes(zip_data.to_vec(), restore_password.as_deref())
    })
    .await
    .map_err(|error| error.to_string())??;

    let cursor = std::io::Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor).map_err(|e| {
        error!("Failed to read zip archive: {}", e);
        format!("Failed to read zip archive: {}", e)
    })?;

    let result = restore_from_archive(&app_handle, &mut archive, skip_cli_custom_roots)?;

    info!("WebDAV restore completed successfully");
    Ok(result)
}
