use super::types::{ProxyGatewayPortCheckInput, ProxyGatewayPortCheckResult, ProxyGatewaySettings};
use std::net::TcpListener;

const MIN_USER_PORT: u16 = 1024;
const AUTO_SELECT_SCAN_LIMIT: u16 = 100;

#[derive(Debug)]
pub struct BoundGatewayListener {
    pub listener: TcpListener,
    pub listen_host: String,
    pub listen_port: u16,
    pub base_url: String,
}

pub fn validate_listen_host(input: &str) -> Result<String, String> {
    let host = input.trim();
    if host.is_empty() {
        return Err("Gateway listen host cannot be empty".to_string());
    }
    if host.contains("://")
        || host.contains('/')
        || host.contains('\\')
        || host.chars().any(char::is_whitespace)
    {
        return Err("Gateway listen host must be a host only, not a URL or path".to_string());
    }

    match host {
        "127.0.0.1" | "localhost" => Ok("127.0.0.1".to_string()),
        _ => Err("Gateway MVP only supports local loopback host 127.0.0.1".to_string()),
    }
}

pub fn validate_listen_port(port: u16) -> Result<u16, String> {
    if port < MIN_USER_PORT {
        return Err("Gateway listen port must be 1024 or higher".to_string());
    }
    Ok(port)
}

pub fn validate_settings(settings: &ProxyGatewaySettings) -> Result<(String, u16), String> {
    let host = validate_listen_host(&settings.listen_host)?;
    let port = validate_listen_port(settings.listen_port)?;
    if settings.per_provider_retry_count > settings.max_retry_count {
        return Err("Gateway per-provider retry count cannot exceed max retry count".to_string());
    }
    // Streaming/non-streaming timeouts must be positive so the runtime never
    // builds a zero Duration from persisted settings. The runtime also clamps
    // via `.max(1)`, but rejecting at save time gives users a clear error
    // instead of silently degrading to a 1s per-chunk idle.
    for (label, secs) in [
        (
            "streaming first byte timeout",
            settings.streaming_first_byte_timeout_secs,
        ),
        (
            "streaming idle timeout",
            settings.streaming_idle_timeout_secs,
        ),
        ("non-streaming timeout", settings.non_streaming_timeout_secs),
    ] {
        if secs == 0 {
            return Err(format!("Gateway {label} must be at least 1 second, got 0"));
        }
    }
    // Validate expression early so bad UI input fails at save time.
    super::retryable_status::parse_retryable_status_codes(&settings.retryable_status_codes)?;
    for (cli_key, app_config) in &settings.app_configs {
        for (label, secs) in [
            (
                "streaming first byte timeout",
                app_config.streaming_first_byte_timeout_secs,
            ),
            (
                "streaming idle timeout",
                app_config.streaming_idle_timeout_secs,
            ),
            (
                "non-streaming timeout",
                app_config.non_streaming_timeout_secs,
            ),
        ] {
            if matches!(secs, Some(0)) {
                return Err(format!(
                    "Gateway {} per-app {label} must be at least 1 second, got 0",
                    cli_key.as_str()
                ));
            }
        }
        if let (Some(per_provider), Some(max_retry)) = (
            app_config.per_provider_retry_count,
            app_config.max_retry_count,
        ) {
            if per_provider > max_retry {
                return Err(format!(
                    "Gateway {} per-app per-provider retry count cannot exceed max retry count",
                    cli_key.as_str()
                ));
            }
        }
    }
    Ok((host, port))
}

pub fn format_listen_base_url(host: &str, port: u16) -> String {
    let trimmed = host.trim();
    if trimmed.contains(':') && !trimmed.starts_with('[') && !trimmed.ends_with(']') {
        format!("http://[{trimmed}]:{port}")
    } else {
        format!("http://{trimmed}:{port}")
    }
}

pub fn bind_gateway_listener(
    settings: &ProxyGatewaySettings,
) -> Result<BoundGatewayListener, String> {
    let (host, start_port) = validate_settings(settings)?;
    let max_port = if settings.port_auto_select {
        start_port.saturating_add(AUTO_SELECT_SCAN_LIMIT)
    } else {
        start_port
    };

    let mut last_error = None;
    for port in start_port..=max_port {
        match TcpListener::bind((host.as_str(), port)) {
            Ok(listener) => {
                listener.set_nonblocking(true).map_err(|error| {
                    format!("Failed to set gateway listener nonblocking: {error}")
                })?;
                let local_port = listener
                    .local_addr()
                    .map_err(|error| format!("Failed to read gateway listener address: {error}"))?
                    .port();
                return Ok(BoundGatewayListener {
                    listener,
                    listen_host: host.clone(),
                    listen_port: local_port,
                    base_url: format_listen_base_url(&host, local_port),
                });
            }
            Err(error) => {
                last_error = Some(error.to_string());
                if !settings.port_auto_select {
                    break;
                }
            }
        }
    }

    Err(format!(
        "Failed to bind gateway listener on {}:{}{}",
        host,
        start_port,
        last_error
            .map(|error| format!(" ({error})"))
            .unwrap_or_default()
    ))
}

pub fn check_port_available(
    input: ProxyGatewayPortCheckInput,
) -> Result<ProxyGatewayPortCheckResult, String> {
    let host = validate_listen_host(&input.listen_host)?;
    let port = validate_listen_port(input.listen_port)?;
    let available = TcpListener::bind((host.as_str(), port)).is_ok();
    Ok(ProxyGatewayPortCheckResult {
        available,
        listen_host: host,
        listen_port: port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserve_port() -> TcpListener {
        TcpListener::bind(("127.0.0.1", 0)).expect("reserve port")
    }

    #[test]
    fn localhost_is_normalized_to_loopback() {
        assert_eq!(validate_listen_host("localhost").unwrap(), "127.0.0.1");
    }

    #[test]
    fn loopback_host_is_accepted() {
        assert_eq!(validate_listen_host("127.0.0.1").unwrap(), "127.0.0.1");
    }

    #[test]
    fn listen_host_rejects_url() {
        assert!(validate_listen_host("http://127.0.0.1").is_err());
    }

    #[test]
    fn listen_host_rejects_path() {
        assert!(validate_listen_host("127.0.0.1/gateway").is_err());
    }

    #[test]
    fn listen_host_rejects_lan_for_mvp() {
        assert!(validate_listen_host("0.0.0.0").is_err());
    }

    #[test]
    fn listen_port_rejects_privileged_port() {
        assert!(validate_listen_port(80).is_err());
    }

    #[test]
    fn check_port_available_reports_occupied_port() {
        let listener = reserve_port();
        let port = listener.local_addr().unwrap().port();
        let result = check_port_available(ProxyGatewayPortCheckInput {
            listen_host: "127.0.0.1".to_string(),
            listen_port: port,
        })
        .unwrap();
        assert!(!result.available);
    }

    #[test]
    fn auto_select_skips_occupied_port() {
        let listener = reserve_port();
        let occupied_port = listener.local_addr().unwrap().port();
        let settings = ProxyGatewaySettings {
            listen_port: occupied_port,
            port_auto_select: true,
            ..ProxyGatewaySettings::default()
        };

        let bound = bind_gateway_listener(&settings).unwrap();

        assert_ne!(bound.listen_port, occupied_port);
        assert_eq!(bound.listen_host, "127.0.0.1");
    }

    #[test]
    fn format_listen_base_url_brackets_bare_ipv6_literals() {
        assert_eq!(
            format_listen_base_url("127.0.0.1", 37123),
            "http://127.0.0.1:37123"
        );
        assert_eq!(format_listen_base_url("::1", 37123), "http://[::1]:37123");
        assert_eq!(
            format_listen_base_url("[::1]", 37123),
            "http://[::1]:37123"
        );
    }
}
