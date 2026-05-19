//! Unified HTTP Client Module
//!
//! Provides HTTP client creation with automatic proxy configuration.
//! All HTTP requests in the application should use this module to ensure
//! they respect the user's proxy settings.
//!
//! # Usage
//!
//! ```rust,no_run
//! use ai_toolbox_lib::{http_client, SqliteDbState};
//!
//! # async fn demo(state: &SqliteDbState) -> Result<(), String> {
//! // Create client with automatic proxy configuration (30s timeout)
//! let client = http_client::client(state).await?;
//!
//! // Create client with custom timeout
//! let client = http_client::client_with_timeout(state, 60).await?;
//!
//! // Bypass proxy (special cases only)
//! let direct_client = http_client::create_client_no_proxy(30)?;
//! # let _ = (client, direct_client);
//! # Ok(())
//! # }
//! ```

use reqwest::{Client, Proxy};
use std::time::Duration;

use crate::db::SqliteDbState;
use crate::settings::store::load_settings_from_sqlite_state;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMode {
    Direct,
    Custom,
    System,
}

impl ProxyMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "direct" => Self::Direct,
            "custom" => Self::Custom,
            _ => Self::System,
        }
    }
}

/// Create an HTTP client with automatic proxy configuration.
///
/// This is the primary function for making HTTP requests.
/// Proxy settings are automatically applied from user settings.
///
/// # Arguments
/// * `db_state` - Database state to read proxy settings from
///
/// # Returns
/// A configured reqwest::Client with 30 second timeout
///
/// # Example
/// ```rust,no_run
/// use ai_toolbox_lib::{http_client, SqliteDbState};
///
/// # async fn demo(state: &SqliteDbState) -> Result<(), String> {
/// let client = http_client::client(state).await?;
/// let response = client
///     .get("https://api.example.com")
///     .send()
///     .await
///     .map_err(|error| error.to_string())?;
/// # let _ = response;
/// # Ok(())
/// # }
/// ```
pub async fn client(db_state: &SqliteDbState) -> Result<Client, String> {
    client_with_timeout(db_state, 30).await
}

/// Create an HTTP client with custom timeout.
///
/// # Arguments
/// * `db_state` - Database state to read proxy settings from
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A configured reqwest::Client
///
/// # Example
/// ```rust,no_run
/// use ai_toolbox_lib::{http_client, SqliteDbState};
///
/// # async fn demo(state: &SqliteDbState) -> Result<(), String> {
/// let client = http_client::client_with_timeout(state, 60).await?;
/// # let _ = client;
/// # Ok(())
/// # }
/// ```
pub async fn client_with_timeout(
    db_state: &SqliteDbState,
    timeout_secs: u64,
) -> Result<Client, String> {
    let (proxy_mode, proxy_url) = get_proxy_from_settings(db_state).await?;
    build_client(proxy_mode, &proxy_url, timeout_secs, false)
}

/// Create an HTTP client with custom timeout and disabled automatic response decompression.
///
/// Use this for compatibility workarounds when an upstream gateway mishandles
/// compressed responses for specific clients or platforms.
pub async fn client_with_timeout_no_compression(
    db_state: &SqliteDbState,
    timeout_secs: u64,
) -> Result<Client, String> {
    let (proxy_mode, proxy_url) = get_proxy_from_settings(db_state).await?;
    build_client(proxy_mode, &proxy_url, timeout_secs, true)
}

/// Build an HTTP client with explicit proxy URL.
///
/// This is an internal function. Business code should use `client()` or `client_with_timeout()`.
///
/// # Arguments
/// * `proxy_mode` - Proxy mode selected by user
/// * `proxy_url` - Proxy URL (e.g., "http://proxy.com:8080" or "socks5://proxy.com:1080")
///                 Only used when proxy_mode is custom
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A configured reqwest::Client
///
/// # Proxy Priority
/// 1. direct: explicitly disable all proxies (including system proxy)
/// 2. custom: use user-configured proxy
/// 3. system: use system proxy (Windows/macOS) or env vars (Linux)
fn build_client(
    proxy_mode: ProxyMode,
    proxy_url: &str,
    timeout_secs: u64,
    disable_content_decoding: bool,
) -> Result<Client, String> {
    let mut builder = Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(timeout_secs));

    if disable_content_decoding {
        builder = builder.no_gzip().no_brotli().no_zstd().no_deflate();
    }

    match proxy_mode {
        ProxyMode::Direct => {
            // User explicitly chose direct connection - bypass all proxies including system proxy
            builder = builder.no_proxy();
        }
        ProxyMode::Custom => {
            if proxy_url.is_empty() {
                return Err("Custom proxy mode requires a proxy URL".to_string());
            }
            if let Some(proxy) = build_proxy(proxy_url)? {
                builder = builder.proxy(proxy);
            }
        }
        ProxyMode::System => {}
    }
    // In system mode, reqwest automatically detects system proxy or environment proxies

    builder
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Create an HTTP client without proxy (bypass proxy settings).
///
/// Use this only when you explicitly need to bypass proxy settings.
///
/// # Arguments
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A reqwest::Client without proxy
pub fn create_client_no_proxy(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(timeout_secs))
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .no_proxy()
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Test proxy connectivity by making a request to a test URL.
///
/// This function is used by the settings page to validate proxy configuration.
///
/// # Arguments
/// * `proxy_url` - Proxy URL to test
///
/// # Returns
/// Ok(()) if connection successful, Err with message otherwise
pub async fn test_proxy(proxy_url: &str) -> Result<(), String> {
    if proxy_url.is_empty() {
        return Err("Proxy URL is empty".to_string());
    }

    // Create client with custom proxy mode
    let client = build_client(ProxyMode::Custom, proxy_url, 10, false)?;

    // Test with httpbin.org - it's designed for testing HTTP clients
    let response = client
        .get("https://httpbin.org/ip")
        .send()
        .await
        .map_err(|e| format!("Proxy connection failed: {}", e))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "Proxy test failed with status: {}",
            response.status()
        ))
    }
}

/// Read proxy settings from database.
///
/// This is a public function that can be used by any module needing proxy configuration.
/// Returns (proxy_mode, proxy_url) tuple.
///
/// # Arguments
/// * `db_state` - Database state to read proxy settings from
///
/// # Returns
/// Tuple of (proxy_mode, proxy_url)
pub async fn get_proxy_from_settings(
    db_state: &SqliteDbState,
) -> Result<(ProxyMode, String), String> {
    let settings = load_settings_from_sqlite_state(db_state)?;
    Ok((ProxyMode::parse(&settings.proxy_mode), settings.proxy_url))
}

/// Build a reqwest::Proxy from URL string.
///
/// Supports:
/// - HTTP proxy: http://[user:pass@]host:port
/// - HTTPS proxy: https://[user:pass@]host:port
/// - SOCKS5 proxy: socks5://[user:pass@]host:port
///
/// Auto-detects protocol from URL scheme.
fn build_proxy(url: &str) -> Result<Option<Proxy>, String> {
    if url.is_empty() {
        return Ok(None);
    }

    let normalized_url = normalize_proxy_url(url);

    // Use Proxy::all() to apply proxy to all protocols (HTTP and HTTPS)
    let proxy =
        Proxy::all(&normalized_url).map_err(|e| format!("Invalid proxy URL '{}': {}", url, e))?;

    Ok(Some(proxy))
}

/// Normalize proxy URL by ensuring it has a scheme.
///
/// If no scheme is provided, defaults to http://
fn normalize_proxy_url(url: &str) -> String {
    let url_lower = url.to_lowercase();

    if url_lower.starts_with("http://")
        || url_lower.starts_with("https://")
        || url_lower.starts_with("socks5://")
        || url_lower.starts_with("socks5h://")
    {
        url.to_string()
    } else {
        // Default to http:// if no scheme provided
        format!("http://{}", url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_proxy_url() {
        assert_eq!(
            normalize_proxy_url("proxy.example.com:8080"),
            "http://proxy.example.com:8080"
        );
        assert_eq!(
            normalize_proxy_url("http://proxy.example.com:8080"),
            "http://proxy.example.com:8080"
        );
        assert_eq!(
            normalize_proxy_url("HTTP://proxy.example.com:8080"),
            "HTTP://proxy.example.com:8080"
        );
        assert_eq!(
            normalize_proxy_url("socks5://proxy.example.com:1080"),
            "socks5://proxy.example.com:1080"
        );
        assert_eq!(
            normalize_proxy_url("user:pass@proxy.example.com:8080"),
            "http://user:pass@proxy.example.com:8080"
        );
    }

    #[test]
    fn test_build_proxy_empty() {
        let result = build_proxy("");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_build_proxy_http() {
        let result = build_proxy("http://proxy.example.com:8080");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_build_proxy_socks5() {
        let result = build_proxy("socks5://proxy.example.com:1080");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_build_proxy_with_auth() {
        let result = build_proxy("http://user:password@proxy.example.com:8080");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }
}
