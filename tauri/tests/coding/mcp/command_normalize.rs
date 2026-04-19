use ai_toolbox_lib::coding::mcp::command_normalize::{
    process_claude_json, process_codex_toml, process_opencode_json, unwrap_cmd_c,
    unwrap_cmd_c_opencode_array,
};
use serde_json::{json, Value};

#[test]
fn test_unwrap_cmd_c_wrapped() {
    let input = json!({
        "type": "stdio",
        "command": "cmd",
        "args": ["/c", "npx", "-y", "@foo/bar"]
    });
    let result = unwrap_cmd_c(&input);
    assert_eq!(result["command"], "npx");
    assert_eq!(result["args"], json!(["-y", "@foo/bar"]));
}

#[test]
fn test_unwrap_cmd_c_not_wrapped() {
    let input = json!({
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@foo/bar"]
    });
    let result = unwrap_cmd_c(&input);
    assert_eq!(result["command"], "npx");
    assert_eq!(result["args"], json!(["-y", "@foo/bar"]));
}

#[test]
fn test_unwrap_cmd_c_http_unchanged() {
    let input = json!({
        "type": "http",
        "url": "https://example.com/mcp"
    });
    let result = unwrap_cmd_c(&input);
    assert_eq!(result, input);
}

#[test]
fn test_unwrap_cmd_c_sse_unchanged() {
    let input = json!({
        "type": "sse",
        "url": "https://example.com/mcp"
    });
    let result = unwrap_cmd_c(&input);
    assert_eq!(result, input);
}

#[cfg(windows)]
#[test]
fn test_wrap_cmd_c_npx() {
    use ai_toolbox_lib::coding::mcp::command_normalize::wrap_cmd_c;

    let input = json!({
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@foo/bar"]
    });
    let result = wrap_cmd_c(&input);
    assert_eq!(result["command"], "cmd");
    assert_eq!(result["args"], json!(["/c", "npx", "-y", "@foo/bar"]));
}

#[cfg(windows)]
#[test]
fn test_wrap_cmd_c_already_wrapped() {
    use ai_toolbox_lib::coding::mcp::command_normalize::wrap_cmd_c;

    let input = json!({
        "type": "stdio",
        "command": "cmd",
        "args": ["/c", "npx", "-y", "@foo/bar"]
    });
    let result = wrap_cmd_c(&input);
    assert_eq!(result["command"], "cmd");
    assert_eq!(result["args"], json!(["/c", "npx", "-y", "@foo/bar"]));
}

#[cfg(windows)]
#[test]
fn test_wrap_cmd_c_python_skipped() {
    use ai_toolbox_lib::coding::mcp::command_normalize::wrap_cmd_c;

    let input = json!({
        "type": "stdio",
        "command": "python",
        "args": ["server.py"]
    });
    let result = wrap_cmd_c(&input);
    assert_eq!(result["command"], "python");
    assert_eq!(result["args"], json!(["server.py"]));
}

#[test]
fn test_unwrap_opencode_array() {
    let input = vec![
        json!("cmd"),
        json!("/c"),
        json!("npx"),
        json!("-y"),
        json!("@foo/bar"),
    ];
    let result = unwrap_cmd_c_opencode_array(&input);
    assert_eq!(result, vec![json!("npx"), json!("-y"), json!("@foo/bar")]);
}

#[test]
fn test_unwrap_opencode_array_not_wrapped() {
    let input = vec![json!("npx"), json!("-y"), json!("@foo/bar")];
    let result = unwrap_cmd_c_opencode_array(&input);
    assert_eq!(result, input);
}

#[test]
fn test_process_claude_json_unwrap() {
    let content = r#"{
            "mcpServers": {
                "test": {
                    "type": "stdio",
                    "command": "cmd",
                    "args": ["/c", "npx", "-y", "@foo/bar"]
                }
            }
        }"#;
    let result = process_claude_json(content, false).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["mcpServers"]["test"]["command"], "npx");
    assert_eq!(parsed["mcpServers"]["test"]["args"], json!(["-y", "@foo/bar"]));
}

#[test]
fn test_process_opencode_json_unwrap() {
    let content = r#"{
            "mcp": {
                "test": {
                    "type": "local",
                    "command": ["cmd", "/c", "npx", "-y", "@foo/bar"]
                }
            }
        }"#;
    let result = process_opencode_json(content, false).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["mcp"]["test"]["command"], json!(["npx", "-y", "@foo/bar"]));
}

#[test]
fn test_process_codex_toml_unwrap() {
    let content = r#"
[mcp_servers.test]
type = "stdio"
command = "cmd"
args = ["/c", "npx", "-y", "@foo/bar"]
"#;
    let result = process_codex_toml(content, false).unwrap();
    assert!(result.contains(r#"command = "npx""#));
    assert!(result.contains(r#"args = ["-y", "@foo/bar"]"#));
}
