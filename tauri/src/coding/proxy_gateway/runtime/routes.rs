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
    match strip_cli_prefix(&path, "/anthropic") {
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
                _ => match strip_cli_prefix(&path, "/gemini") {
                    Some(forwarded_path) if is_gemini_versioned_path(&forwarded_path) => {
                        Some(GatewayRoute {
                            cli_key: GatewayCliKey::Gemini,
                            route_name: "gemini",
                            forwarded_path,
                            query,
                        })
                    }
                    _ => None,
                },
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
