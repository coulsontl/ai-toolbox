use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::coding::proxy_gateway::types::GatewayCliKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GatewayRoute {
    pub(super) cli_key: GatewayCliKey,
    pub(super) route_name: &'static str,
    pub(super) forwarded_path: String,
    pub(super) query: Option<String>,
}

pub(super) fn match_gateway_route(request_target: &str) -> Option<GatewayRoute> {
    let (path, query) = split_request_target(request_target);
    match strip_cli_prefix(&path, "/claude-desktop") {
        Some(forwarded_path) => Some(GatewayRoute {
            cli_key: GatewayCliKey::ClaudeDesktop,
            route_name: "claude_desktop",
            forwarded_path,
            query,
        }),
        None => match strip_cli_prefix(&path, "/anthropic") {
            Some(forwarded_path) => Some(GatewayRoute {
                cli_key: GatewayCliKey::Claude,
                route_name: "anthropic",
                forwarded_path,
                query,
            }),
            None => match strip_cli_prefix(&path, "/openai") {
                Some(forwarded_path)
                    if forwarded_path == "/v1" || forwarded_path.starts_with("/v1/") =>
                {
                    Some(GatewayRoute {
                        cli_key: GatewayCliKey::Codex,
                        route_name: "openai-compatible",
                        forwarded_path,
                        query,
                    })
                }
                _ => match strip_cli_prefix(&path, "/grok") {
                    Some(forwarded_path)
                        if matches!(forwarded_path.as_str(), "/v1" | "/v1/responses") =>
                    {
                        Some(GatewayRoute {
                            cli_key: GatewayCliKey::Grok,
                            route_name: "grok",
                            forwarded_path,
                            query,
                        })
                    }
                    _ => match strip_cli_prefix(&path, "/kimi") {
                        Some(forwarded_path)
                            if matches!(
                                forwarded_path.as_str(),
                                "/v1" | "/v1/chat/completions"
                            ) =>
                        {
                            Some(GatewayRoute {
                                cli_key: GatewayCliKey::Kimi,
                                route_name: "kimi",
                                forwarded_path,
                                query,
                            })
                        }
                        _ => match strip_cli_prefix(&path, "/gemini") {
                            Some(forwarded_path) if is_gemini_versioned_path(&forwarded_path) => {
                                Some(GatewayRoute {
                                    cli_key: GatewayCliKey::Gemini,
                                    route_name: "gemini",
                                    forwarded_path,
                                    query,
                                })
                            }
                            _ => match strip_cli_prefix(&path, "/antigravity") {
                                Some(forwarded_path)
                                    if is_gemini_versioned_path(&forwarded_path) =>
                                {
                                    Some(GatewayRoute {
                                        cli_key: GatewayCliKey::Antigravity,
                                        route_name: "antigravity",
                                        forwarded_path,
                                        query,
                                    })
                                }
                                _ => match_openai_chat_route(
                                    &path,
                                    "/workbuddy",
                                    GatewayCliKey::Workbuddy,
                                    "workbuddy",
                                    query.clone(),
                                )
                                .or_else(|| {
                                    match_openai_chat_route(
                                        &path,
                                        "/codebuddy",
                                        GatewayCliKey::Codebuddy,
                                        "codebuddy",
                                        query.clone(),
                                    )
                                }),
                            },
                        },
                    },
                },
            },
        },
    }
}

/// OpenAI-Chat-only takeover routes: the WorkBuddy family serves the same two
/// paths as Kimi (a bare `/v1` probe and `/v1/chat/completions`).
fn match_openai_chat_route(
    path: &str,
    prefix: &str,
    cli_key: GatewayCliKey,
    route_name: &'static str,
    query: Option<String>,
) -> Option<GatewayRoute> {
    let forwarded_path = strip_cli_prefix(path, prefix)?;
    if !matches!(forwarded_path.as_str(), "/v1" | "/v1/chat/completions") {
        return None;
    }
    Some(GatewayRoute {
        cli_key,
        route_name,
        forwarded_path,
        query,
    })
}

/// A `/plugin/<provider_id>/...` request. These routes bypass CLI takeover
/// and provider selection entirely: the path names the plugin provider, and
/// the gateway only converts protocols and forwards through the plugin host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PluginRoute {
    pub(super) provider_id: String,
    pub(super) forwarded_path: String,
    pub(super) query: Option<String>,
    pub(super) kind: PluginRouteKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginRouteKind {
    /// Local GET/HEAD health probe (`/plugin/<id>`, `/v1`, `/v1beta`).
    Probe,
    /// A model catalog: OpenAI's `GET /v1/models` or Gemini's
    /// `GET /v1beta/models` shape.
    Models { gemini: bool },
    /// An inference path. `source_protocol` is what the client speaks and
    /// `cli_key` is the gateway CLI family whose conversion pipeline matches
    /// that source protocol.
    Inference {
        source_protocol: AiProtocol,
        cli_key: GatewayCliKey,
    },
}

pub(super) fn match_plugin_route(request_target: &str) -> Option<PluginRoute> {
    let (path, query) = split_request_target(request_target);
    let rest = path.strip_prefix("/plugin/")?;
    let (provider_id, tail) = match rest.split_once('/') {
        Some((provider_id, tail)) => (provider_id, format!("/{tail}")),
        None => (rest, String::new()),
    };
    if provider_id.trim().is_empty() {
        return None;
    }
    let (kind, forwarded_path) = match tail.as_str() {
        "" | "/" | "/v1" | "/v1/" | "/v1beta" | "/v1beta/" => {
            (PluginRouteKind::Probe, tail.clone())
        }
        "/v1/models" => (PluginRouteKind::Models { gemini: false }, tail.clone()),
        "/v1beta/models" => (PluginRouteKind::Models { gemini: true }, tail.clone()),
        "/v1/chat/completions" => (
            PluginRouteKind::Inference {
                source_protocol: AiProtocol::OpenAiChat,
                cli_key: GatewayCliKey::Kimi,
            },
            tail.clone(),
        ),
        "/v1/responses" | "/v1/responses/compact" => (
            PluginRouteKind::Inference {
                source_protocol: AiProtocol::OpenAiResponses,
                cli_key: GatewayCliKey::Grok,
            },
            tail.clone(),
        ),
        "/v1/messages" | "/v1/messages/count_tokens" => (
            PluginRouteKind::Inference {
                source_protocol: AiProtocol::AnthropicMessages,
                cli_key: GatewayCliKey::Claude,
            },
            tail.clone(),
        ),
        tail if is_gemini_versioned_path(tail)
            && (tail.contains(":generateContent") || tail.contains(":streamGenerateContent")) =>
        {
            (
                PluginRouteKind::Inference {
                    source_protocol: AiProtocol::GeminiNative,
                    cli_key: GatewayCliKey::Gemini,
                },
                tail.to_string(),
            )
        }
        _ => return None,
    };
    Some(PluginRoute {
        provider_id: provider_id.to_string(),
        forwarded_path,
        query,
        kind,
    })
}

#[cfg(test)]
mod openai_chat_route_tests {
    use super::*;

    #[test]
    fn family_routes_accept_probe_and_chat_only() {
        for (prefix, cli_key, name) in [
            ("/workbuddy", GatewayCliKey::Workbuddy, "workbuddy"),
            ("/codebuddy", GatewayCliKey::Codebuddy, "codebuddy"),
        ] {
            // Drive the public entry point: it is what splits the query off
            // before `strip_cli_prefix` sees the path.
            let probe = match_gateway_route(&format!("{prefix}/v1")).expect("probe route");
            assert_eq!(probe.cli_key, cli_key);
            assert_eq!(probe.route_name, name);
            assert_eq!(probe.forwarded_path, "/v1");

            let chat = match_gateway_route(&format!("{prefix}/v1/chat/completions?trace=1"))
                .expect("chat route");
            assert_eq!(chat.cli_key, cli_key);
            assert_eq!(chat.forwarded_path, "/v1/chat/completions");
            assert_eq!(chat.query.as_deref(), Some("trace=1"));

            assert!(match_gateway_route(&format!("{prefix}/v1/responses")).is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_desktop_route_strips_prefix_and_keeps_messages_path() {
        let route = match_gateway_route("/claude-desktop/v1/messages").expect("desktop route");
        assert_eq!(route.cli_key, GatewayCliKey::ClaudeDesktop);
        assert_eq!(route.route_name, "claude_desktop");
        assert_eq!(route.forwarded_path, "/v1/messages");

        let probe = match_gateway_route("/claude-desktop").expect("desktop probe route");
        assert_eq!(probe.cli_key, GatewayCliKey::ClaudeDesktop);
        assert_eq!(probe.forwarded_path, "/");
    }

    #[test]
    fn grok_route_accepts_probe_and_responses_only() {
        let probe = match_gateway_route("/grok/v1").expect("probe route");
        assert_eq!(probe.cli_key, GatewayCliKey::Grok);
        assert_eq!(probe.forwarded_path, "/v1");

        let responses = match_gateway_route("/grok/v1/responses?trace=1").expect("responses route");
        assert_eq!(responses.cli_key, GatewayCliKey::Grok);
        assert_eq!(responses.forwarded_path, "/v1/responses");
        assert_eq!(responses.query.as_deref(), Some("trace=1"));

        assert!(match_gateway_route("/grok/v1/chat/completions").is_none());
        assert!(match_gateway_route("/grok/v1/responses/compact").is_none());
    }

    #[test]
    fn plugin_routes_cover_the_common_client_endpoints() {
        let models = match_plugin_route("/plugin/demo/v1/models").expect("openai models");
        assert_eq!(models.kind, PluginRouteKind::Models { gemini: false });
        let gemini_models = match_plugin_route("/plugin/demo/v1beta/models?key=x")
            .expect("gemini models");
        assert_eq!(gemini_models.kind, PluginRouteKind::Models { gemini: true });
        assert_eq!(gemini_models.query.as_deref(), Some("key=x"));

        for (path, protocol, cli_key) in [
            (
                "/plugin/demo/v1/responses/compact",
                AiProtocol::OpenAiResponses,
                GatewayCliKey::Grok,
            ),
            (
                "/plugin/demo/v1/messages/count_tokens",
                AiProtocol::AnthropicMessages,
                GatewayCliKey::Claude,
            ),
        ] {
            match match_plugin_route(path).expect(path).kind {
                PluginRouteKind::Inference {
                    source_protocol,
                    cli_key: route_cli,
                } => {
                    assert_eq!(source_protocol, protocol, "{path}");
                    assert_eq!(route_cli, cli_key, "{path}");
                }
                other => panic!("{path} matched {other:?}"),
            }
        }

        assert!(match_plugin_route("/plugin/demo/v1/messages/unknown").is_none());
    }

    #[test]
    fn kimi_route_accepts_probe_and_chat_only() {
        let probe = match_gateway_route("/kimi/v1").expect("probe route");
        assert_eq!(probe.cli_key, GatewayCliKey::Kimi);
        assert_eq!(probe.forwarded_path, "/v1");

        let chat = match_gateway_route("/kimi/v1/chat/completions?trace=1").expect("chat route");
        assert_eq!(chat.cli_key, GatewayCliKey::Kimi);
        assert_eq!(chat.forwarded_path, "/v1/chat/completions");
        assert_eq!(chat.query.as_deref(), Some("trace=1"));

        assert!(match_gateway_route("/kimi/v1/responses").is_none());
        assert!(match_gateway_route("/kimi/v1/messages").is_none());
    }
}

fn is_gemini_versioned_path(path: &str) -> bool {
    matches!(path, "/v1" | "/v1beta" | "/v1alpha")
        || path.starts_with("/v1/")
        || path.starts_with("/v1beta/")
        || path.starts_with("/v1alpha/")
}

pub(super) fn split_request_target(request_target: &str) -> (String, Option<String>) {
    if let Ok(url) = reqwest::Url::parse(request_target) {
        return (url.path().to_string(), url.query().map(str::to_string));
    }

    match request_target.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (request_target.to_string(), None),
    }
}

fn strip_cli_prefix(path: &str, prefix: &str) -> Option<String> {
    if path == prefix {
        return Some("/".to_string());
    }
    let rest = path.strip_prefix(prefix)?;
    if !rest.starts_with('/') {
        return None;
    }
    Some(rest.to_string())
}

pub(super) fn build_target_url(
    base_url: &str,
    forwarded_path: &str,
    query: Option<&str>,
) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|error| format!("Invalid upstream base URL '{}': {error}", base_url))?;
    let base_path = url.path().trim_end_matches('/');
    let forwarded_path = if base_path.ends_with("/v1")
        && (forwarded_path == "/v1" || forwarded_path.starts_with("/v1/"))
    {
        forwarded_path.strip_prefix("/v1").unwrap_or(forwarded_path)
    } else if base_path.ends_with("/v1beta") {
        strip_leading_gemini_api_version(forwarded_path).unwrap_or(forwarded_path)
    } else if base_path.ends_with("/v1alpha") {
        strip_leading_gemini_api_version(forwarded_path).unwrap_or(forwarded_path)
    } else {
        forwarded_path
    };

    let mut combined_path = String::new();
    combined_path.push_str(base_path);
    combined_path.push_str(forwarded_path);
    if combined_path.is_empty() {
        combined_path.push('/');
    }
    if !combined_path.starts_with('/') {
        combined_path.insert(0, '/');
    }
    url.set_path(&combined_path);
    url.set_query(query);
    Ok(url)
}

fn strip_leading_gemini_api_version(path: &str) -> Option<&str> {
    for version in ["/v1beta", "/v1alpha", "/v1"] {
        if path == version {
            return Some("");
        }
        if let Some(rest) = path.strip_prefix(version) {
            if rest.starts_with('/') {
                return Some(rest);
            }
        }
    }
    None
}
