//! Streamable HTTP MCP server for the tools that provider plugins export.
//!
//! A plugin's `tool` hook (OpenCode's `{ tool: { name: tool({ description,
//! args, execute }) } }`) is offered to every agent through this server, so a
//! CLI that only speaks MCP can call it too. The gateway serves it at
//! [`MCP_PATH`] (and the Magpie-compatible alias [`MCP_ALIAS_PATH`]).
//!
//! The server speaks the subset of MCP the gateway needs: `initialize`,
//! `ping`, `tools/list`, `tools/call`, and notifications answered with 202.
//! Every tool call runs in the Bun host through its `tool` RPC; a tool error
//! is reported inside a normal result with `isError: true`, as MCP expects.

use std::collections::HashSet;

use serde_json::{json, Value};

use super::bun_host::PluginToolSet;
use super::PluginManagerState;

/// Where the plugin tools MCP server is served.
pub const MCP_PATH: &str = "/mcp/plugins";
/// Magpie's spelling of the same server.
pub const MCP_ALIAS_PATH: &str = "/mcp/magpie/tools";

/// The longest body the server takes: a tool's arguments may carry a file.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "ai-toolbox-plugin-tools";
/// Tool calls run in the host; the MCP call waits a little longer than the
/// host's own tool deadline so the host's error wins.
const TOOL_TIMEOUT_MS: u64 = 55_000;
const MAX_TOOL_NAME_BYTES: usize = 64;
const MAX_PREFIX_BYTES: usize = 24;

/// One named tool: `<plugin>_<tool>` plus where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedTool {
    pub spec: String,
    /// The tool's own name as the plugin exported it.
    pub tool_name: String,
    pub description: String,
    pub parameters: Value,
    /// The MCP name clients see and call.
    pub mcp_name: String,
}

/// A package spec's own name: no scope, no version, no git ref.
fn package_name(spec: &str) -> &str {
    let bare = spec.rsplit(['/', '\\']).next().unwrap_or(spec);
    let bare = bare.split('#').next().unwrap_or(bare);
    match bare.rfind('@') {
        // The leading `@` of a scope is never a version separator.
        Some(index) if index > 0 => &bare[..index],
        _ => bare,
    }
}

/// The plugin's part of its tools' names: its short name in the characters
/// MCP clients take, at most 24 bytes.
pub fn tool_prefix(spec: &str) -> String {
    let mut name = package_name(spec);
    for kind in ["middleware-", "opencode-", "tools-", "page-", "agent-"] {
        if let Some(rest) = name.strip_prefix(kind) {
            if !rest.is_empty() {
                name = rest;
            }
            break;
        }
    }
    let name = name.strip_suffix("-auth").unwrap_or(name);

    let mut prefix = String::new();
    let mut after_separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            prefix.push(character);
            after_separator = false;
        } else if !after_separator {
            prefix.push('-');
            after_separator = true;
        }
    }
    let prefix = prefix.trim_matches(|character| character == '-' || character == '_');
    if prefix.is_empty() {
        "plugin".to_string()
    } else {
        // The sanitized prefix is ASCII, so slicing at 24 bytes is safe.
        prefix.chars().take(MAX_PREFIX_BYTES).collect()
    }
}

/// The tool's own name in the characters MCP clients take. Illegal runs
/// collapse to one dash, the edges are trimmed, and a name with nothing usable
/// falls back to `tool` so the MCP name always stays valid.
fn sanitize_tool_name(name: &str) -> String {
    let mut sanitized = String::new();
    let mut after_separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            sanitized.push(character);
            after_separator = false;
        } else if !after_separator {
            sanitized.push('-');
            after_separator = true;
        }
    }
    let sanitized = sanitized.trim_matches(|character| character == '-' || character == '_');
    if sanitized.is_empty() {
        "tool".to_string()
    } else {
        sanitized.to_string()
    }
}

/// `text` cut to at most `max_bytes` where a character begins.
fn cut_name(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Name every tool `<plugin>_<tool>`, at most 64 bytes, with a numeric
/// suffix when two would collide.
pub fn named_tools(sets: &[PluginToolSet]) -> Vec<NamedTool> {
    let mut named = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for set in sets {
        let prefix = tool_prefix(&set.spec);
        for tool in &set.tools {
            let base = format!("{prefix}_{}", sanitize_tool_name(&tool.name));
            let mut name = cut_name(&base, MAX_TOOL_NAME_BYTES);
            let mut counter = 2;
            while seen.contains(&name) {
                let suffix = format!("_{counter}");
                name = format!(
                    "{}{}",
                    cut_name(&base, MAX_TOOL_NAME_BYTES - suffix.len()),
                    suffix
                );
                counter += 1;
            }
            seen.insert(name.clone());
            named.push(NamedTool {
                spec: set.spec.clone(),
                tool_name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.parameters.clone(),
                mcp_name: name,
            });
        }
    }
    named
}

fn json_headers() -> Vec<(String, String)> {
    vec![("Content-Type".to_string(), "application/json".to_string())]
}

fn json_response(status: u16, value: Value) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    (status, json_headers(), body)
}

fn jsonrpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn jsonrpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

/// The caller agent the `x-magpie-caller-agent` header names, if any.
fn caller_agent(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-magpie-caller-agent"))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// Answer one Streamable HTTP MCP request. Returns the status, headers and
/// body the gateway should relay; provider routing and token accounting never
/// run for these paths.
pub async fn handle_mcp_http_request(
    manager: &PluginManagerState,
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    if body.len() > MAX_BODY_BYTES {
        return json_response(
            413,
            jsonrpc_error(
                &Value::Null,
                -32700,
                "the request body is longer than this server takes",
            ),
        );
    }
    match method {
        "DELETE" => return (204, Vec::new(), Vec::new()),
        "POST" => {}
        _ => {
            let mut headers = json_headers();
            headers.push(("Allow".to_string(), "POST, DELETE".to_string()));
            let body = serde_json::to_vec(&jsonrpc_error(
                &Value::Null,
                -32600,
                "POST JSON-RPC messages here (MCP Streamable HTTP)",
            ))
            .unwrap_or_default();
            return (405, headers, body);
        }
    }

    let message: Value = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(_) => {
            return json_response(
                400,
                jsonrpc_error(
                    &Value::Null,
                    -32700,
                    "parse error: the body isn't a JSON-RPC message",
                ),
            )
        }
    };
    let Some(object) = message.as_object() else {
        return json_response(
            400,
            jsonrpc_error(
                &Value::Null,
                -32700,
                "parse error: the body isn't a JSON-RPC message",
            ),
        );
    };
    // A message without an id is a notification: answer it with 202 and no
    // body, as Streamable HTTP requires.
    if object.get("id").is_none_or(Value::is_null) {
        return (202, Vec::new(), Vec::new());
    }
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let Some(method_name) = object.get("method").and_then(Value::as_str) else {
        return json_response(
            200,
            jsonrpc_error(&id, -32600, "invalid request: no method"),
        );
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);

    match method_name {
        "initialize" => json_response(
            200,
            jsonrpc_result(
                &id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            ),
        ),
        "ping" => json_response(200, jsonrpc_result(&id, json!({}))),
        "tools/list" => match manager.host().list_tools().await {
            Ok(sets) => json_response(200, jsonrpc_result(&id, tools_list_result(&sets))),
            Err(error) => json_response(
                200,
                jsonrpc_error(
                    &id,
                    -32603,
                    &format!("AI Toolbox couldn't load its plugins: {error}"),
                ),
            ),
        },
        "tools/call" => json_response(200, handle_tools_call(manager, &id, &params, headers).await),
        _ => json_response(
            200,
            jsonrpc_error(&id, -32601, &format!("method not found: {method_name}")),
        ),
    }
}

fn tools_list_result(sets: &[PluginToolSet]) -> Value {
    let tools = named_tools(sets)
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.mcp_name,
                "description": tool.description,
                "inputSchema": tool.parameters,
            })
        })
        .collect::<Vec<_>>();
    json!({ "tools": tools })
}

async fn handle_tools_call(
    manager: &PluginManagerState,
    id: &Value,
    params: &Value,
    headers: &[(String, String)],
) -> Value {
    let Some(name) = params
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return jsonrpc_error(id, -32602, "invalid params: tools/call needs a tool name");
    };
    let arguments = params
        .get("arguments")
        .filter(|arguments| arguments.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));

    let sets = match manager.host().list_tools().await {
        Ok(sets) => sets,
        Err(error) => {
            return jsonrpc_result(
                id,
                tool_result(
                    &format!("AI Toolbox couldn't load its plugins: {error}"),
                    true,
                ),
            )
        }
    };
    let Some(tool) = named_tools(&sets)
        .into_iter()
        .find(|tool| tool.mcp_name == name)
    else {
        return jsonrpc_error(
            id,
            -32602,
            &format!("unknown tool {name}: no enabled plugin gives it"),
        );
    };

    match manager
        .host()
        .run_tool(
            &tool.spec,
            &tool.tool_name,
            arguments,
            TOOL_TIMEOUT_MS,
            caller_agent(headers),
        )
        .await
    {
        Ok(value) => jsonrpc_result(id, tool_result(&tool_text(&value), false)),
        Err(error) => jsonrpc_result(id, tool_result(&error, true)),
    }
}

/// `host.js::runTool` answers `{ text }`; anything else is serialized so a
/// surprising shape still reaches the agent.
fn tool_text(value: &Value) -> String {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    value.to_string()
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDbState;
    use crate::plugins::bun_host::PluginToolDef;
    use crate::plugins::PluginManager;

    fn tool_set(spec: &str, names: &[&str]) -> PluginToolSet {
        PluginToolSet {
            spec: spec.to_string(),
            tools: names
                .iter()
                .map(|name| PluginToolDef {
                    name: name.to_string(),
                    description: format!("{name} tool"),
                    parameters: json!({ "type": "object", "properties": {} }),
                })
                .collect(),
        }
    }

    fn test_manager() -> (tempfile::TempDir, PluginManagerState) {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let manager = PluginManager::new(db, root.path().to_path_buf());
        (root, manager)
    }

    #[test]
    fn tool_names_use_the_plugins_short_name() {
        assert_eq!(tool_prefix("@scope/weather"), "weather");
        assert_eq!(tool_prefix("@scope/weather@1.2.3"), "weather");
        assert_eq!(tool_prefix("weather@1.2.3"), "weather");
        assert_eq!(tool_prefix("github:owner/weather#main"), "weather");
        assert_eq!(tool_prefix("opencode-weather-tools"), "weather-tools");
        assert_eq!(tool_prefix("tools-page-agent-auth"), "page-agent");
        assert_eq!(tool_prefix("C:\\plugins\\demo\\pkg"), "pkg");
        // Every illegal run becomes one dash, and the edges are trimmed.
        assert_eq!(tool_prefix("weird!!  plugin"), "weird-plugin");
        assert_eq!(tool_prefix("@!@"), "plugin");
        assert_eq!(tool_prefix(""), "plugin");
        assert!(tool_prefix(&"a".repeat(40)).len() <= MAX_PREFIX_BYTES);

        let named = named_tools(&[tool_set("opencode-weather-tools", &["forecast"])]);
        assert_eq!(named[0].mcp_name, "weather-tools_forecast");
        assert_eq!(named[0].tool_name, "forecast");
    }

    #[test]
    fn tool_names_are_cut_to_64_bytes_at_a_character_boundary() {
        let long_tool = "a".repeat(60);
        let named = named_tools(&[tool_set("demo", &[&long_tool])]);
        assert_eq!(named[0].mcp_name.len(), MAX_TOOL_NAME_BYTES);

        // Multi-byte characters must never be split by the byte cap.
        let unicode_name = "工具名称".repeat(20);
        let cut = cut_name(&unicode_name, MAX_TOOL_NAME_BYTES);
        assert!(cut.len() <= MAX_TOOL_NAME_BYTES);
        assert!(cut.is_char_boundary(cut.len()));

        // A name with no ASCII-safe characters still yields a valid MCP name.
        let named = named_tools(&[tool_set("demo", &[&unicode_name])]);
        assert_eq!(named[0].mcp_name, "demo_tool");
        assert_eq!(named[0].tool_name, unicode_name);
    }

    #[test]
    fn unsafe_tool_names_are_sanitized_into_mcp_names() {
        let named = named_tools(&[tool_set("demo", &["read file", "path/to", "!!!"])]);
        assert_eq!(
            named
                .iter()
                .map(|tool| tool.mcp_name.as_str())
                .collect::<Vec<_>>(),
            vec!["demo_read-file", "demo_path-to", "demo_tool"]
        );
        // The original names stay untouched for the host-side tool call.
        assert_eq!(named[0].tool_name, "read file");
        assert_eq!(named[1].tool_name, "path/to");
    }

    #[test]
    fn duplicate_tool_names_get_a_numeric_suffix() {
        let named = named_tools(&[
            tool_set("demo", &["run"]),
            tool_set("demo", &["run", "run"]),
        ]);
        assert_eq!(
            named
                .iter()
                .map(|tool| tool.mcp_name.as_str())
                .collect::<Vec<_>>(),
            vec!["demo_run", "demo_run_2", "demo_run_3"]
        );
    }

    #[tokio::test]
    async fn initialize_and_ping_follow_the_mcp_contract() {
        let (_root, manager) = test_manager();
        let (status, headers, body) = handle_mcp_http_request(
            &manager,
            "POST",
            &[],
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        )
        .await;
        assert_eq!(status, 200);
        assert!(headers
            .iter()
            .any(|(name, value)| name == "Content-Type" && value == "application/json"));
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["capabilities"]["tools"], json!({}));
        assert_eq!(reply["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(
            reply["result"]["serverInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );

        let (status, _headers, body) = handle_mcp_http_request(
            &manager,
            "POST",
            &[],
            br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
        )
        .await;
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["result"], json!({}));
    }

    #[tokio::test]
    async fn notifications_are_accepted_with_no_body() {
        let (_root, manager) = test_manager();
        let (status, headers, body) = handle_mcp_http_request(
            &manager,
            "POST",
            &[],
            br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await;
        assert_eq!(status, 202);
        assert!(headers.is_empty());
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn tools_lists_no_tools_without_a_provider_plugin() {
        let (_root, manager) = test_manager();
        let (status, _headers, body) = handle_mcp_http_request(
            &manager,
            "POST",
            &[],
            br#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
        )
        .await;
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["result"]["tools"], json!([]));
    }

    #[tokio::test]
    async fn calling_an_unknown_tool_is_invalid_params() {
        let (_root, manager) = test_manager();
        let (status, _headers, body) = handle_mcp_http_request(
            &manager,
            "POST",
            &[],
            br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"missing","arguments":{}}}"#,
        )
        .await;
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["error"]["code"], -32602);
        assert!(reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing"));

        // A call without a name is refused the same way.
        let (_status, _headers, body) = handle_mcp_http_request(
            &manager,
            "POST",
            &[],
            br#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"arguments":{}}}"#,
        )
        .await;
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn unknown_methods_and_bad_bodies_are_jsonrpc_errors() {
        let (_root, manager) = test_manager();
        let (status, _headers, body) = handle_mcp_http_request(
            &manager,
            "POST",
            &[],
            br#"{"jsonrpc":"2.0","id":6,"method":"bogus"}"#,
        )
        .await;
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["error"]["code"], -32601);

        let (status, _headers, body) =
            handle_mcp_http_request(&manager, "POST", &[], b"not json").await;
        assert_eq!(status, 400);
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn http_method_and_size_limits_are_enforced() {
        let (_root, manager) = test_manager();

        let (status, headers, _body) = handle_mcp_http_request(&manager, "DELETE", &[], b"").await;
        assert_eq!(status, 204);
        assert!(headers.is_empty());

        let (status, headers, body) = handle_mcp_http_request(&manager, "GET", &[], b"").await;
        assert_eq!(status, 405);
        assert!(headers
            .iter()
            .any(|(name, value)| name == "Allow" && value == "POST, DELETE"));
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["error"]["code"], -32600);

        let oversized = vec![b'a'; MAX_BODY_BYTES + 1];
        let (status, _headers, body) =
            handle_mcp_http_request(&manager, "POST", &[], &oversized).await;
        assert_eq!(status, 413);
        let reply: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn listing_tools_never_starts_bun_without_a_provider_plugin() {
        let (_root, manager) = test_manager();
        assert!(manager.host().list_tools().await.unwrap().is_empty());
        assert!(!manager.host().status().await.running);
    }
}
