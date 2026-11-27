use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde_json::Value;
use tauri::Manager;

use crate::db::SqliteDbState;

use super::listen::validate_listen_host;
use super::runtime::{codex_base_url_from_config, ProxyGatewayState};
use super::settings::load_settings_from_sqlite_state;
use super::types::{GatewayCliKey, ProxyGatewaySettings, ProxyGatewayStatus};

fn is_loopback_ip(ip: IpAddr) -> bool {
    ip.is_loopback()
        || matches!(
            ip,
            IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        )
}

/// `0.0.0.0`, `::` and the IPv4-mapped `::ffff:0.0.0.0` all mean "unspecified".
/// On Windows (and most stacks) connecting to one of them reaches the local
/// listener, so they must be treated like loopback when the gateway itself is
/// bound to a loopback address.
fn is_unspecified_ip(ip: IpAddr) -> bool {
    ip.is_unspecified()
        || matches!(
            ip,
            IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some_and(|v4| v4.is_unspecified())
        )
}

fn url_matches_listener(url: &reqwest::Url, listen_addr: SocketAddr) -> bool {
    let Some(target_port) = url.port_or_known_default() else {
        return false;
    };
    if target_port != listen_addr.port() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let normalized = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let listener_ip = listen_addr.ip();
    if normalized == "localhost" || normalized.ends_with(".localhost") {
        is_loopback_ip(listener_ip) || is_unspecified_ip(listener_ip)
    } else if let Ok(ip) = normalized.parse::<IpAddr>() {
        if is_loopback_ip(listener_ip) || is_unspecified_ip(listener_ip) {
            is_loopback_ip(ip) || is_unspecified_ip(ip)
        } else {
            ip == listener_ip
        }
    } else {
        false
    }
}

fn decode_percent_segment(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[index + 1] as char).to_digit(16)?;
            let lo = (bytes[index + 2] as char).to_digit(16)?;
            out.push(((hi << 4) | lo) as u8);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Extract `<provider_id>` when `url` targets this gateway's own listener at
/// `/plugin/<provider_id>(/...)`. Returns `None` for any other host, port or
/// route so external URLs and true self-loop routes are never misclassified.
pub(crate) fn extract_plugin_provider_id_from_gateway_url(
    url: &reqwest::Url,
    listen_addr: SocketAddr,
) -> Option<String> {
    if !url_matches_listener(url, listen_addr) {
        return None;
    }
    let mut segments = url.path_segments()?.filter(|segment| !segment.is_empty());
    if segments.next()? != "plugin" {
        return None;
    }
    let raw_id = segments.next()?;
    let decoded = decode_percent_segment(raw_id)?.trim().to_string();
    if decoded.is_empty() || decoded == "." || decoded == ".." || decoded.contains(['/', '\\']) {
        return None;
    }
    Some(decoded)
}

/// Extract `<provider_id>` when a configured `base_url` points to any of this
/// gateway's own listening addresses at `/plugin/<provider_id>(/...)`.
pub(crate) fn extract_local_gateway_plugin_provider_id(
    base_url: &str,
    gateway_addrs: &[SocketAddr],
) -> Option<String> {
    let trimmed = base_url.trim().trim_end_matches("##").trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = reqwest::Url::parse(trimmed).ok()?;
    for &addr in gateway_addrs {
        if let Some(provider_id) = extract_plugin_provider_id_from_gateway_url(&parsed, addr) {
            return Some(provider_id);
        }
    }
    None
}

/// Reject targeting this proxy gateway's own listening socket to prevent
/// infinite loopback recursion and OOM. The error message deliberately omits
/// the URL so query-string API keys never leak.
pub(crate) fn ensure_url_not_self_loopback(
    url: &reqwest::Url,
    listen_addr: SocketAddr,
) -> Result<(), String> {
    if url_matches_listener(url, listen_addr) {
        return Err(format!(
            "Upstream URL targets the proxy gateway's own listening port ({}); refusing to forward to prevent a self-loop",
            listen_addr.port()
        ));
    }
    Ok(())
}

fn parse_gateway_socket_addr(host: &str, port: u16) -> Option<SocketAddr> {
    if port == 0 {
        return None;
    }
    let normalized = validate_listen_host(host).unwrap_or_else(|_| host.trim().to_string());
    let ip = match normalized.to_ascii_lowercase().as_str() {
        "localhost" => IpAddr::V4(Ipv4Addr::LOCALHOST),
        other => other
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .ok()?,
    };
    Some(SocketAddr::new(ip, port))
}

fn push_unique_addr(addrs: &mut Vec<SocketAddr>, addr: Option<SocketAddr>) {
    if let Some(addr) = addr {
        if !addrs.contains(&addr) {
            addrs.push(addr);
        }
    }
}

pub(crate) fn gateway_addrs_from_status_and_settings(
    status: Option<&ProxyGatewayStatus>,
    settings: Option<&ProxyGatewaySettings>,
) -> Vec<SocketAddr> {
    let mut addrs = Vec::new();
    if let Some(status) = status {
        if status.running {
            if let Some(port) = status.listen_port {
                push_unique_addr(&mut addrs, parse_gateway_socket_addr(&status.listen_host, port));
            }
        }
    }
    if let Some(settings) = settings {
        push_unique_addr(
            &mut addrs,
            parse_gateway_socket_addr(&settings.listen_host, settings.listen_port),
        );
    }
    addrs
}

pub(crate) fn configured_and_active_gateway_addrs<R: tauri::Runtime>(
    db: &SqliteDbState,
    app: Option<&tauri::AppHandle<R>>,
) -> Vec<SocketAddr> {
    let active_status = app
        .and_then(|handle| handle.try_state::<ProxyGatewayState>())
        .and_then(|state| state.manager.lock().ok().map(|manager| manager.status()));
    let configured = load_settings_from_sqlite_state(db).ok();
    gateway_addrs_from_status_and_settings(active_status.as_ref(), configured.as_ref())
}

pub(crate) fn ensure_base_url_not_gateway_self_loop(
    base_url: &str,
    gateway_addrs: &[SocketAddr],
) -> Result<(), String> {
    let trimmed = base_url.trim().trim_end_matches("##").trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let Ok(parsed) = reqwest::Url::parse(trimmed) else {
        return Ok(());
    };
    for &addr in gateway_addrs {
        // Pointing at `/plugin/<provider_id>` on this gateway is recognized at
        // runtime and routed directly to the in-process Bun plugin bridge
        // without an HTTP loopback hop, so it is safe to save and engage.
        if extract_plugin_provider_id_from_gateway_url(&parsed, addr).is_some() {
            continue;
        }
        ensure_url_not_self_loopback(&parsed, addr).map_err(|_| {
            format!(
                "Base URL 不能指向本机代理网关端口（{}），否则会导致回环死循环。如需接入插件渠道，请填写 /plugin/<渠道ID> 或使用「从插件导入」。",
                addr.port()
            )
        })?;
    }
    Ok(())
}

fn has_plugin_provider_meta(meta: Option<&Value>) -> bool {
    meta.and_then(|value| {
        value
            .get("pluginProvider")
            .or_else(|| value.get("plugin_provider"))
    })
    .is_some_and(|value| !value.is_null())
}

fn push_non_empty(urls: &mut Vec<String>, candidate: Option<&str>) {
    if let Some(value) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
        urls.push(value.to_string());
    }
}

fn extract_grok_toml_base_urls(config_toml: &str, urls: &mut Vec<String>) {
    let Ok(parsed) = toml::from_str::<toml::Value>(config_toml) else {
        return;
    };
    push_non_empty(urls, parsed.get("base_url").and_then(toml::Value::as_str));
    if let Some(models) = parsed.get("models").and_then(toml::Value::as_array) {
        for model in models {
            push_non_empty(urls, model.get("base_url").and_then(toml::Value::as_str));
        }
    }
}

fn extract_cli_provider_base_urls(cli_key: GatewayCliKey, settings_config: &str) -> Vec<String> {
    let Ok(settings) = serde_json::from_str::<Value>(settings_config) else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    match cli_key {
        GatewayCliKey::Claude | GatewayCliKey::ClaudeDesktop => {
            push_non_empty(
                &mut urls,
                settings.pointer("/env/ANTHROPIC_BASE_URL").and_then(Value::as_str),
            );
        }
        GatewayCliKey::Codex => {
            if let Some(config_toml) = settings.get("config").and_then(Value::as_str) {
                if let Some(url) = codex_base_url_from_config(config_toml) {
                    push_non_empty(&mut urls, Some(&url));
                }
            }
        }
        GatewayCliKey::Grok => {
            if let Some(models) = settings
                .pointer("/modelCatalog/models")
                .and_then(Value::as_array)
            {
                for model in models {
                    push_non_empty(
                        &mut urls,
                        model
                            .get("baseUrl")
                            .or_else(|| model.get("base_url"))
                            .and_then(Value::as_str),
                    );
                }
            }
            if let Some(config_toml) = settings.get("config").and_then(Value::as_str) {
                extract_grok_toml_base_urls(config_toml, &mut urls);
            }
        }
        GatewayCliKey::Kimi => {
            if let Some(configs) = settings.get("providerConfigs").and_then(Value::as_object) {
                for config in configs.values() {
                    push_non_empty(
                        &mut urls,
                        config
                            .get("base_url")
                            .or_else(|| config.get("baseUrl"))
                            .and_then(Value::as_str),
                    );
                }
            }
        }
        GatewayCliKey::Gemini | GatewayCliKey::Antigravity => {
            push_non_empty(
                &mut urls,
                settings
                    .pointer("/env/GOOGLE_GEMINI_BASE_URL")
                    .and_then(Value::as_str),
            );
            push_non_empty(
                &mut urls,
                settings
                    .pointer("/env/GOOGLE_VERTEX_BASE_URL")
                    .and_then(Value::as_str),
            );
        }
        GatewayCliKey::Workbuddy | GatewayCliKey::Codebuddy | GatewayCliKey::OpenCode => {}
    }
    urls
}

/// Configuration-layer guard for gateway-takeover CLI providers.
///
/// Official accounts and plugin-backed providers (`meta.pluginProvider`) are
/// exempt because they never forward HTTP requests to `settings_config`'s base
/// URL; custom providers targeting this gateway's own listening port are
/// rejected before persisting so self-loops fail at save time.
pub(crate) fn ensure_cli_provider_not_gateway_self_loop<R: tauri::Runtime>(
    db: &SqliteDbState,
    app: Option<&tauri::AppHandle<R>>,
    cli_key: GatewayCliKey,
    category: &str,
    settings_config: &str,
    meta: Option<&Value>,
) -> Result<(), String> {
    if category.eq_ignore_ascii_case("official") || has_plugin_provider_meta(meta) {
        return Ok(());
    }
    let gateway_addrs = configured_and_active_gateway_addrs(db, app);
    if gateway_addrs.is_empty() {
        return Ok(());
    }
    for url in extract_cli_provider_base_urls(cli_key, settings_config) {
        ensure_base_url_not_gateway_self_loop(&url, &gateway_addrs)?;
    }
    Ok(())
}

/// Takeover-layer guard ensuring a non-plugin candidate provider cannot route
/// back into the running or configured proxy gateway port.
pub(crate) fn ensure_takeover_provider_not_gateway_self_loop(
    base_url: &str,
    is_plugin_provider: bool,
    gateway_status: &ProxyGatewayStatus,
    settings: &ProxyGatewaySettings,
) -> Result<(), String> {
    if is_plugin_provider {
        return Ok(());
    }
    let gateway_addrs = gateway_addrs_from_status_and_settings(Some(gateway_status), Some(settings));
    ensure_base_url_not_gateway_self_loop(base_url, &gateway_addrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::settings::save_settings_to_sqlite_state;
    use serde_json::json;

    fn gateway_settings_on(port: u16) -> ProxyGatewaySettings {
        ProxyGatewaySettings {
            listen_host: "127.0.0.1".to_string(),
            listen_port: port,
            ..ProxyGatewaySettings::default()
        }
    }

    #[test]
    fn rejects_claude_provider_targeting_configured_gateway_port() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        save_settings_to_sqlite_state(&db, gateway_settings_on(37123)).unwrap();

        let err = ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Claude,
            "custom",
            &json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/anthropic" } })
                .to_string(),
            None,
        )
        .unwrap_err();
        assert!(err.contains("37123"));
        assert!(!err.contains("/anthropic"));

        // Localhost alias and AxonHub `##` suffix are also rejected.
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Claude,
            "custom",
            &json!({ "env": { "ANTHROPIC_BASE_URL": "http://localhost:37123/v1/messages##" } })
                .to_string(),
            None,
        )
        .is_err());

        // Other local ports and external URLs remain allowed.
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Claude,
            "custom",
            &json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:11434/v1" } }).to_string(),
            None,
        )
        .is_ok());
    }

    #[test]
    fn exempts_plugin_backed_and_official_providers() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        save_settings_to_sqlite_state(&db, gateway_settings_on(37123)).unwrap();
        let settings = json!({
            "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/plugin/qoder/v1" }
        })
        .to_string();

        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Claude,
            "custom",
            &settings,
            Some(&json!({ "pluginProvider": { "providerId": "qoder" } })),
        )
        .is_ok());

        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Claude,
            "official",
            &settings,
            None,
        )
        .is_ok());
    }

    #[test]
    fn rejects_codex_grok_kimi_gemini_and_claude_desktop_self_loops() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        save_settings_to_sqlite_state(&db, gateway_settings_on(37123)).unwrap();

        let codex_settings = json!({
            "config": "model_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"http://localhost:37123/openai/v1\"\n"
        })
        .to_string();
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Codex,
            "custom",
            &codex_settings,
            None,
        )
        .is_err());

        let grok_settings = json!({
            "modelCatalog": {
                "models": [{ "id": "grok-3", "baseUrl": "http://[::1]:37123/grok/v1" }]
            }
        })
        .to_string();
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Grok,
            "custom",
            &grok_settings,
            None,
        )
        .is_err());

        let kimi_settings = json!({
            "providerConfigs": {
                "managed:kimi-code": { "base_url": "http://127.0.0.1:37123/kimi/v1" }
            }
        })
        .to_string();
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Kimi,
            "custom",
            &kimi_settings,
            None,
        )
        .is_err());

        let gemini_settings = json!({
            "env": { "GOOGLE_GEMINI_BASE_URL": "http://127.0.0.1:37123/gemini/v1beta" }
        })
        .to_string();
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Gemini,
            "custom",
            &gemini_settings,
            None,
        )
        .is_err());

        let desktop_settings = json!({
            "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/anthropic" }
        })
        .to_string();
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::ClaudeDesktop,
            "custom",
            &desktop_settings,
            None,
        )
        .is_err());
    }

    #[test]
    fn takeover_guard_checks_active_auto_selected_port() {
        let settings = gateway_settings_on(37123);
        let status = ProxyGatewayStatus {
            running: true,
            listen_host: "127.0.0.1".to_string(),
            listen_port: Some(41234),
            base_url: Some("http://127.0.0.1:41234".to_string()),
            ..ProxyGatewayStatus::stopped(&settings, None)
        };

        assert!(ensure_takeover_provider_not_gateway_self_loop(
            "http://127.0.0.1:41234/openai/v1",
            false,
            &status,
            &settings,
        )
        .is_err());
        assert!(ensure_takeover_provider_not_gateway_self_loop(
            "http://127.0.0.1:41234/plugin/qoder/v1",
            true,
            &status,
            &settings,
        )
        .is_ok());
    }

    #[test]
    fn recognizes_and_allows_local_gateway_plugin_routes_without_allowing_other_self_loops() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let settings = gateway_settings_on(37123);
        save_settings_to_sqlite_state(&db, settings.clone()).unwrap();
        let addrs = gateway_addrs_from_status_and_settings(None, Some(&settings));

        assert_eq!(
            extract_local_gateway_plugin_provider_id("http://127.0.0.1:37123/plugin/qoder", &addrs)
                .as_deref(),
            Some("qoder")
        );
        assert_eq!(
            extract_local_gateway_plugin_provider_id(
                "http://localhost:37123/plugin/qoder-cn/v1/messages##",
                &addrs
            )
            .as_deref(),
            Some("qoder-cn")
        );
        assert_eq!(
            extract_local_gateway_plugin_provider_id("http://127.0.0.1:37123/plugin", &addrs),
            None
        );
        assert_eq!(
            extract_local_gateway_plugin_provider_id("https://example.com/plugin/qoder", &addrs),
            None
        );

        // Custom providers pointing at `/plugin/<id>` on the local gateway pass
        // config-layer validation (runtime routes them in-process to the plugin host).
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Claude,
            "custom",
            &json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/plugin/qoder" } })
                .to_string(),
            None,
        )
        .is_ok());

        // Missing `<provider_id>` or non-plugin gateway routes are still rejected.
        assert!(ensure_cli_provider_not_gateway_self_loop::<tauri::Wry>(
            &db,
            None,
            GatewayCliKey::Claude,
            "custom",
            &json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/plugin" } })
                .to_string(),
            None,
        )
        .is_err());
    }

    #[test]
    fn treats_unspecified_hosts_as_the_loopback_listener() {
        let listen_addr: SocketAddr = "127.0.0.1:37123".parse().unwrap();
        for url in [
            "http://0.0.0.0:37123/anthropic",
            "http://[::]:37123/anthropic",
            "http://[::ffff:0.0.0.0]:37123/anthropic",
        ] {
            let parsed = reqwest::Url::parse(url).unwrap();
            assert!(
                ensure_url_not_self_loopback(&parsed, listen_addr).is_err(),
                "{url} reaches the loopback listener and must be rejected"
            );
            assert!(extract_plugin_provider_id_from_gateway_url(&parsed, listen_addr).is_none());
        }

        // A `/plugin/<id>` route on the same unspecified hosts is still
        // recognized so it can be dispatched in-process to the Bun host.
        for url in [
            "http://0.0.0.0:37123/plugin/demo",
            "http://[::]:37123/plugin/demo",
            "http://[::ffff:0.0.0.0]:37123/plugin/demo",
        ] {
            let parsed = reqwest::Url::parse(url).unwrap();
            assert_eq!(
                extract_plugin_provider_id_from_gateway_url(&parsed, listen_addr).as_deref(),
                Some("demo"),
                "{url} must be recognized as a plugin route"
            );
        }
    }
}
