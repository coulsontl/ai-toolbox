//! Unified HTTP Client Module
//!
//! Provides HTTP client creation with automatic proxy configuration.
//! All HTTP requests in the application should use this module to ensure
//! they respect the user's proxy settings.
//!
//! # Usage
//!
//! ```rust
//! use crate::http_client;
//! use crate::db::DbState;
//!
//! // Create client with automatic proxy configuration (30s timeout)
//! let client = http_client::client(&state).await?;
//!
//! // Create client with custom timeout
//! let client = http_client::client_with_timeout(&state, 60).await?;
//!
//! // Bypass proxy (special cases only)
//! let client = http_client::client_no_proxy(30)?;
//! ```

use reqwest::{Client, Proxy};
use std::time::Duration;

use crate::db::DbState;

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
/// ```rust
/// let client = http_client::client(&state).await?;
/// let response = client.get("https://api.example.com").send().await?;
/// ```
pub async fn client(db_state: &DbState) -> Result<Client, String> {
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
/// ```rust
/// let client = http_client::client_with_timeout(&state, 60).await?;
/// ```
pub async fn client_with_timeout(
    db_state: &DbState,
    timeout_secs: u64,
) -> Result<Client, String> {
    let proxy_url = get_proxy_from_settings(db_state).await?;
    build_client(&proxy_url, timeout_secs)
}

/// Build an HTTP client with explicit proxy URL.
///
/// This is an internal function. Business code should use `client()` or `client_with_timeout()`.
///
/// # Arguments
/// * `proxy_url` - Proxy URL (e.g., "http://proxy.com:8080" or "socks5://proxy.com:1080")
///                 Empty string means use system proxy (Windows/macOS) or env vars (Linux)
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A configured reqwest::Client
///
/// # Proxy Priority
/// 1. User-configured proxy (if proxy_url is not empty)
/// 2. System proxy (Windows/macOS) or environment variables (Linux)
/// 3. Direct connection (if no proxy available)
fn build_client(proxy_url: &str, timeout_secs: u64) -> Result<Client, String> {
    let mut builder = Client::builder().timeout(Duration::from_secs(timeout_secs));

    if !proxy_url.is_empty() {
        // User-configured proxy takes priority over system proxy
        if let Some(proxy) = build_proxy(proxy_url)? {
            builder = builder.proxy(proxy);
        }
    }
    // If proxy_url is empty, system-proxy feature automatically detects system proxy

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
        .timeout(Duration::from_secs(timeout_secs))
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

    // Create client with proxy
    let client = build_client(proxy_url, 10)?;

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

/// Read proxy URL from database settings.
///
/// This is an internal function used by `client()` and `client_with_timeout()`.
async fn get_proxy_from_settings(db_state: &DbState) -> Result<String, String> {
    let db = db_state.0.lock().await;

    let mut result = db
        .query("SELECT proxy_url OMIT id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query proxy settings: {}", e))?;

    let records: Vec<serde_json::Value> = result
        .take(0)
        .map_err(|e| format!("Failed to parse proxy settings: {}", e))?;

    if let Some(record) = records.first() {
        Ok(record
            .get("proxy_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    } else {
        Ok(String::new())
    }
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
    let proxy = Proxy::all(&normalized_url)
        .map_err(|e| format!("Invalid proxy URL '{}': {}", url, e))?;

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
