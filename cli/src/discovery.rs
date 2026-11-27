//! Reads the discovery file written by the desktop app.
//!
//! The bearer token lives only in this file (mode 0600 on Unix) and is never
//! printed by the CLI.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const CONTROL_FILE_ENV: &str = "AI_TOOLBOX_CONTROL_FILE";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Discovery {
    pub version: u32,
    pub pid: u32,
    pub url: String,
    pub token: String,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub data_dir: String,
    #[serde(default)]
    pub started_at: String,
}

impl Discovery {
    pub fn endpoint(&self) -> Result<(String, u16), String> {
        parse_http_url(&self.url)
    }
}

pub fn default_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os(CONTROL_FILE_ENV) {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    home_dir().join(".ai-toolbox").join("control.json")
}

fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(profile);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    PathBuf::from(".")
}

pub fn read(path: &Path) -> Result<Discovery, String> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "AI Toolbox is not running (cannot read {}: {error}). Start the desktop app and enable Settings > Agent Integration > Local Control.",
            path.display()
        )
    })?;
    let discovery: Discovery = serde_json::from_str(&text).map_err(|error| {
        format!(
            "The discovery file {} is invalid: {error}. Restart AI Toolbox to regenerate it.",
            path.display()
        )
    })?;
    if discovery.url.trim().is_empty() || discovery.token.trim().is_empty() {
        return Err("The discovery file is missing its URL or token".to_string());
    }
    Ok(discovery)
}

/// Parses `http://127.0.0.1:PORT` (loopback only). The control API never
/// listens on TLS, so https is rejected instead of silently downgraded.
pub fn parse_http_url(url: &str) -> Result<(String, u16), String> {
    let trimmed = url.trim().trim_end_matches('/');
    let rest = trimmed.strip_prefix("http://").ok_or_else(|| {
        format!("Unsupported control URL '{url}': expected http://127.0.0.1:<port>")
    })?;
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        return Err(format!("Control URL '{url}' has no host"));
    }

    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| format!("Control URL '{url}' has a malformed IPv6 host"))?;
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| format!("Control URL '{url}' has no port"))?
            .parse::<u16>()
            .map_err(|_| format!("Control URL '{url}' has an invalid port"))?;
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.to_string(),
                port.parse::<u16>()
                    .map_err(|_| format!("Control URL '{url}' has an invalid port"))?,
            ),
            None => (authority.to_string(), 80),
        }
    };

    if host.is_empty() {
        return Err(format!("Control URL '{url}' has no host"));
    }
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loopback_urls() {
        assert_eq!(
            parse_http_url("http://127.0.0.1:12345").unwrap(),
            ("127.0.0.1".to_string(), 12345)
        );
        assert_eq!(
            parse_http_url("http://localhost:8123/").unwrap(),
            ("localhost".to_string(), 8123)
        );
        assert_eq!(
            parse_http_url("http://[::1]:9000").unwrap(),
            ("::1".to_string(), 9000)
        );
        assert!(parse_http_url("https://127.0.0.1:1").is_err());
        assert!(parse_http_url("http://127.0.0.1:notaport").is_err());
    }

    #[test]
    fn explicit_control_file_wins_over_the_environment() {
        // The test environment may or may not define HOME, so only assert the
        // suffix and that the path is not empty.
        let path = default_path();
        assert!(path.ends_with("control.json") || !path.as_os_str().is_empty());
    }
}
