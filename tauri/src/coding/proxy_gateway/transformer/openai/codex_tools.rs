use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const TOOL_SEARCH_PROXY_NAME: &str = "tool_search";
const CUSTOM_TOOL_INPUT_FIELD: &str = "input";
const CHAT_TOOL_NAME_MAX_LEN: usize = 64;
const CUSTOM_TOOL_INPUT_DESCRIPTION: &str = "Raw string input for the original custom tool. Preserve formatting exactly and follow the original tool definition embedded in the description.";
const CUSTOM_TOOL_PRESERVED_METADATA_HEADING: &str = "Original tool definition:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexToolKind {
    Function,
    Namespace,
    Custom,
    ToolSearch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexToolSpec {
    pub kind: CodexToolKind,
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CodexToolContext {
    chat_tools: Vec<Value>,
    seen_chat_names: HashSet<String>,
    chat_name_to_spec: HashMap<String, CodexToolSpec>,
    namespace_name_to_chat_name: HashMap<(String, String), String>,
}

impl CodexToolContext {
    pub fn is_empty(&self) -> bool {
        self.chat_name_to_spec.is_empty()
    }

    pub fn chat_tools(&self) -> &[Value] {
        &self.chat_tools
    }

    pub fn lookup_chat_name(&self, chat_name: &str) -> Option<&CodexToolSpec> {
        self.chat_name_to_spec.get(chat_name)
    }

    pub fn chat_name_for_response_function(&self, name: &str, namespace: Option<&str>) -> String {
        if let Some(namespace) = namespace.filter(|value| !value.is_empty()) {
            if let Some(chat_name) = self
                .namespace_name_to_chat_name
                .get(&(namespace.to_string(), name.to_string()))
            {
                return chat_name.clone();
            }
            return flatten_namespace_tool_name(namespace, name);
        }

        name.to_string()
    }

    fn add_chat_tool(&mut self, chat_name: String, spec: CodexToolSpec, chat_tool: Value) {
        if chat_name.trim().is_empty() || self.seen_chat_names.contains(&chat_name) {
            return;
        }
        self.seen_chat_names.insert(chat_name.clone());
        if let Some(namespace) = spec.namespace.as_ref() {
            self.namespace_name_to_chat_name
                .insert((namespace.clone(), spec.name.clone()), chat_name.clone());
        }
        self.chat_name_to_spec.insert(chat_name, spec);
        self.chat_tools.push(chat_tool);
    }

    fn add_function_tool(&mut self, tool: &Value, namespace: Option<&str>) {
        let Some(original_name) = responses_tool_name(tool) else {
            return;
        };
        let chat_name = namespace
            .map(|namespace| flatten_namespace_tool_name(namespace, &original_name))
            .unwrap_or_else(|| original_name.clone());
        let Some(chat_tool) = responses_function_tool_to_chat_tool(tool, &chat_name) else {
            return;
        };
        let spec = CodexToolSpec {
            kind: if namespace.is_some() {
                CodexToolKind::Namespace
            } else {
                CodexToolKind::Function
            },
            name: original_name,
            namespace: namespace.map(ToString::to_string),
        };
        self.add_chat_tool(chat_name, spec, chat_tool);
    }

    fn add_custom_tool(&mut self, tool: &Value) {
        let Some(name) = responses_tool_name(tool) else {
            return;
        };
        let chat_tool = json!({
            "type": "function",
            "function": {
                "name": name,
                "description": responses_custom_tool_description(tool),
                "parameters": {
                    "type": "object",
                    "properties": {
                        CUSTOM_TOOL_INPUT_FIELD: {
                            "type": "string",
                            "description": CUSTOM_TOOL_INPUT_DESCRIPTION
                        }
                    },
                    "required": [CUSTOM_TOOL_INPUT_FIELD]
                }
            }
        });
        let spec = CodexToolSpec {
            kind: CodexToolKind::Custom,
            name: name.clone(),
            namespace: None,
        };
        self.add_chat_tool(name, spec, chat_tool);
    }

    fn add_tool_search_tool(&mut self) {
        let chat_tool = json!({
            "type": "function",
            "function": {
                "name": TOOL_SEARCH_PROXY_NAME,
                "description": "Search and load Codex tools, plugins, connectors, and MCP namespaces for the current task.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query for tools or connectors to load."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of tool groups to return."
                        }
                    },
                    "required": ["query"]
                }
            }
        });
        let spec = CodexToolSpec {
            kind: CodexToolKind::ToolSearch,
            name: TOOL_SEARCH_PROXY_NAME.to_string(),
            namespace: None,
        };
        self.add_chat_tool(TOOL_SEARCH_PROXY_NAME.to_string(), spec, chat_tool);
    }

    fn add_namespace_tool(&mut self, namespace_tool: &Value) {
        let Some(namespace) = namespace_tool.get("name").and_then(Value::as_str) else {
            return;
        };
        let Some(children) = namespace_tool
            .get("tools")
            .or_else(|| namespace_tool.get("children"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for child in children {
            if child.get("type").and_then(Value::as_str) == Some("function") {
                self.add_function_tool(child, Some(namespace));
            }
        }
    }

    fn add_response_tool(&mut self, tool: &Value) {
        match tool {
            Value::String(name) => {
                self.add_custom_tool(&json!({
                    "type": "custom",
                    "name": name
                }));
            }
            Value::Object(_) => match tool.get("type").and_then(Value::as_str) {
                Some("function") => self.add_function_tool(tool, None),
                Some("custom") => self.add_custom_tool(tool),
                Some("tool_search") => self.add_tool_search_tool(),
                Some("namespace") => self.add_namespace_tool(tool),
                _ => {}
            },
            _ => {}
        }
    }
}

pub fn build_codex_tool_context_from_request(body: &Value) -> CodexToolContext {
    let mut context = CodexToolContext::default();

    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            context.add_response_tool(tool);
        }
    }
    if let Some(input) = body.get("input") {
        collect_tool_search_output_tools(input, &mut context);
    }

    context
}

pub fn rewrite_responses_request_for_chat_context(
    mut body: Value,
    context: &CodexToolContext,
) -> Value {
    if context.is_empty() {
        return body;
    }
    if let Some(input) = body.get_mut("input") {
        rewrite_responses_input_item_for_chat(input, context);
    }
    body
}

pub fn apply_codex_tool_context_to_chat_request(
    chat_body: &mut Value,
    responses_request: &Value,
    context: &CodexToolContext,
) {
    if context.is_empty() {
        return;
    }
    if let Value::Object(object) = chat_body {
        if !context.chat_tools().is_empty() {
            object.insert(
                "tools".to_string(),
                Value::Array(context.chat_tools().to_vec()),
            );
        }
        if let Some(tool_choice) = responses_request
            .get("tool_choice")
            .and_then(|choice| responses_tool_choice_to_chat(choice, context))
        {
            object.insert("tool_choice".to_string(), tool_choice);
        }
    }
}

pub fn rewrite_response_with_codex_tool_context(response: &mut Value, context: &CodexToolContext) {
    if context.is_empty() {
        return;
    }
    let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return;
    };
    for item in output {
        rewrite_response_tool_item_with_context(item, context);
    }
}

pub fn response_tool_item_id_from_chat_name(
    call_id: &str,
    chat_name: &str,
    context: Option<&CodexToolContext>,
) -> String {
    if context
        .and_then(|context| context.lookup_chat_name(chat_name))
        .is_some_and(|spec| spec.kind == CodexToolKind::Custom)
    {
        responses_custom_tool_call_item_id(call_id)
    } else {
        responses_function_call_item_id(call_id)
    }
}

pub fn response_tool_added_item_from_chat_name(
    item_id: &str,
    status: &str,
    call_id: &str,
    chat_name: &str,
    context: Option<&CodexToolContext>,
) -> Value {
    let Some(spec) = context.and_then(|context| context.lookup_chat_name(chat_name)) else {
        return json!({
            "id": item_id,
            "type": "function_call",
            "status": status,
            "call_id": call_id,
            "name": chat_name,
            "arguments": ""
        });
    };

    match spec.kind {
        CodexToolKind::ToolSearch => json!({
            "type": "tool_search_call",
            "call_id": call_id,
            "status": status,
            "execution": "client",
            "arguments": {}
        }),
        CodexToolKind::Custom => json!({
            "id": item_id,
            "type": "custom_tool_call",
            "status": status,
            "call_id": call_id,
            "name": spec.name,
            "input": ""
        }),
        CodexToolKind::Namespace | CodexToolKind::Function => {
            let mut item = json!({
                "id": item_id,
                "type": "function_call",
                "status": status,
                "call_id": call_id,
                "name": spec.name,
                "arguments": ""
            });
            if let Some(namespace) = spec.namespace.as_ref() {
                item["namespace"] = json!(namespace);
            }
            item
        }
    }
}

pub fn response_tool_done_item_from_chat_name(
    item_id: &str,
    status: &str,
    call_id: &str,
    chat_name: &str,
    arguments: &str,
    context: Option<&CodexToolContext>,
) -> Value {
    let Some(spec) = context.and_then(|context| context.lookup_chat_name(chat_name)) else {
        return json!({
            "id": item_id,
            "type": "function_call",
            "status": status,
            "call_id": call_id,
            "name": chat_name,
            "arguments": arguments
        });
    };

    match spec.kind {
        CodexToolKind::ToolSearch => json!({
            "type": "tool_search_call",
            "call_id": call_id,
            "status": status,
            "execution": "client",
            "arguments": parse_tool_arguments_object(arguments)
        }),
        CodexToolKind::Custom => json!({
            "id": item_id,
            "type": "custom_tool_call",
            "status": status,
            "call_id": call_id,
            "name": spec.name,
            "input": custom_tool_input_from_chat_arguments(arguments)
        }),
        CodexToolKind::Namespace | CodexToolKind::Function => {
            let mut item = json!({
                "id": item_id,
                "type": "function_call",
                "status": status,
                "call_id": call_id,
                "name": spec.name,
                "arguments": arguments
            });
            if let Some(namespace) = spec.namespace.as_ref() {
                item["namespace"] = json!(namespace);
            }
            item
        }
    }
}

pub fn is_custom_tool_chat_name(chat_name: &str, context: Option<&CodexToolContext>) -> bool {
    context
        .and_then(|context| context.lookup_chat_name(chat_name))
        .is_some_and(|spec| spec.kind == CodexToolKind::Custom)
}

fn rewrite_responses_input_item_for_chat(value: &mut Value, context: &CodexToolContext) {
    match value {
        Value::Array(items) => {
            for item in items {
                rewrite_responses_input_item_for_chat(item, context);
            }
        }
        Value::Object(object) => {
            let item_type = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match item_type {
                "tool_search_call" => {
                    let arguments = object
                        .get("arguments")
                        .map(canonical_json_string)
                        .unwrap_or_else(|| "{}".to_string());
                    object.insert("type".to_string(), json!("function_call"));
                    object.insert("name".to_string(), json!(TOOL_SEARCH_PROXY_NAME));
                    object.insert("arguments".to_string(), json!(arguments));
                }
                "tool_search_output" => {
                    let call_id = object.get("call_id").cloned().unwrap_or_else(|| json!(""));
                    let output = canonical_json_string(&Value::Object(object.clone()));
                    object.clear();
                    object.insert("type".to_string(), json!("function_call_output"));
                    object.insert("call_id".to_string(), call_id);
                    object.insert("output".to_string(), json!(output));
                }
                "function_call" => {
                    let name = object
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let namespace = object.get("namespace").and_then(Value::as_str);
                    if namespace.is_some() {
                        object.insert(
                            "name".to_string(),
                            json!(context.chat_name_for_response_function(name, namespace)),
                        );
                    }
                }
                "custom_tool_call" => {
                    let name = object
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let input = object.get("input").cloned().unwrap_or_else(|| json!(""));
                    object.insert("type".to_string(), json!("function_call"));
                    object.insert("name".to_string(), json!(name));
                    object.insert(
                        "arguments".to_string(),
                        json!(canonical_json_string(
                            &json!({ CUSTOM_TOOL_INPUT_FIELD: input })
                        )),
                    );
                }
                "custom_tool_call_output" => {
                    let call_id = object.get("call_id").cloned().unwrap_or_else(|| json!(""));
                    let output = canonical_json_string(&Value::Object(object.clone()));
                    object.clear();
                    object.insert("type".to_string(), json!("function_call_output"));
                    object.insert("call_id".to_string(), call_id);
                    object.insert("output".to_string(), json!(output));
                }
                _ => {
                    for value in object.values_mut() {
                        rewrite_responses_input_item_for_chat(value, context);
                    }
                }
            }
        }
        _ => {}
    }
}

fn rewrite_response_tool_item_with_context(item: &mut Value, context: &CodexToolContext) {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return;
    }
    let chat_name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let Some(spec) = context.lookup_chat_name(&chat_name).cloned() else {
        return;
    };
    let call_id = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed")
        .to_string();
    let arguments = item
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    *item = match spec.kind {
        CodexToolKind::ToolSearch => json!({
            "type": "tool_search_call",
            "call_id": call_id,
            "status": status,
            "execution": "client",
            "arguments": parse_tool_arguments_object(&arguments)
        }),
        CodexToolKind::Custom => json!({
            "id": responses_custom_tool_call_item_id(&call_id),
            "type": "custom_tool_call",
            "status": status,
            "call_id": call_id,
            "name": spec.name,
            "input": custom_tool_input_from_chat_arguments(&arguments)
        }),
        CodexToolKind::Namespace | CodexToolKind::Function => {
            let mut object = Map::new();
            object.insert(
                "id".to_string(),
                json!(responses_function_call_item_id(&call_id)),
            );
            object.insert("type".to_string(), json!("function_call"));
            object.insert("status".to_string(), json!(status));
            object.insert("call_id".to_string(), json!(call_id));
            object.insert("name".to_string(), json!(spec.name));
            if let Some(namespace) = spec.namespace {
                object.insert("namespace".to_string(), json!(namespace));
            }
            object.insert("arguments".to_string(), json!(arguments));
            Value::Object(object)
        }
    };
}

fn collect_tool_search_output_tools(value: &Value, context: &mut CodexToolContext) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_tool_search_output_tools(item, context);
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("tool_search_output") {
                if let Some(tools) = object.get("tools").and_then(Value::as_array) {
                    for tool in tools {
                        context.add_response_tool(tool);
                    }
                }
            }
            for value in object.values() {
                collect_tool_search_output_tools(value, context);
            }
        }
        _ => {}
    }
}

fn responses_tool_choice_to_chat(tool_choice: &Value, context: &CodexToolContext) -> Option<Value> {
    match tool_choice {
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("function") => {
            let name = object.get("name").and_then(Value::as_str).unwrap_or("");
            let namespace = object.get("namespace").and_then(Value::as_str);
            Some(json!({
                "type": "function",
                "function": {
                    "name": context.chat_name_for_response_function(name, namespace)
                }
            }))
        }
        Value::Object(object)
            if object.get("type").and_then(Value::as_str) == Some("tool_search") =>
        {
            Some(json!({
                "type": "function",
                "function": {
                    "name": TOOL_SEARCH_PROXY_NAME
                }
            }))
        }
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("custom") => {
            let name = object.get("name").and_then(Value::as_str).unwrap_or("");
            Some(json!({
                "type": "function",
                "function": {
                    "name": name
                }
            }))
        }
        _ => None,
    }
}

fn responses_tool_name(tool: &Value) -> Option<String> {
    tool.get("function")
        .and_then(|function| function.get("name"))
        .or_else(|| tool.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn responses_function_tool_to_chat_tool(tool: &Value, chat_name: &str) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }

    if let Some(function) = tool.get("function") {
        let mut chat_tool = json!({
            "type": "function",
            "function": function.clone()
        });
        if let Some(object) = chat_tool.get_mut("function").and_then(Value::as_object_mut) {
            object.insert("name".to_string(), json!(chat_name));
            if let Some(strict) = tool.get("strict").cloned() {
                object.entry("strict".to_string()).or_insert(strict);
            }
        }
        return Some(chat_tool);
    }

    let mut function = json!({
        "name": chat_name,
        "description": tool.get("description").cloned().unwrap_or(Value::Null),
        "parameters": tool.get("parameters").cloned().unwrap_or_else(|| json!({}))
    });
    if let Some(strict) = tool.get("strict") {
        function["strict"] = strict.clone();
    }
    Some(json!({
        "type": "function",
        "function": function
    }))
}

fn responses_custom_tool_description(tool: &Value) -> String {
    format!(
        "{CUSTOM_TOOL_PRESERVED_METADATA_HEADING}\n```json\n{}\n```",
        canonical_json_string(tool)
    )
}

pub fn custom_tool_input_from_chat_arguments(arguments: &str) -> String {
    if arguments.trim().is_empty() {
        return String::new();
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(object)) => object
            .get(CUSTOM_TOOL_INPUT_FIELD)
            .and_then(Value::as_str)
            .unwrap_or(arguments)
            .to_string(),
        _ => arguments.to_string(),
    }
}

fn parse_tool_arguments_object(arguments: &str) -> Value {
    if arguments.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str::<Value>(arguments)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "query": arguments }))
}

fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

fn responses_function_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("fc") {
        call_id.to_string()
    } else if call_id.is_empty() {
        "fc_0".to_string()
    } else {
        format!("fc_{call_id}")
    }
}

fn responses_custom_tool_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("ctc") {
        call_id.to_string()
    } else if call_id.is_empty() {
        "ctc_0".to_string()
    } else {
        format!("ctc_{call_id}")
    }
}

fn flatten_namespace_tool_name(namespace: &str, name: &str) -> String {
    let full_name = format!("{namespace}__{name}");
    if full_name.len() <= CHAT_TOOL_NAME_MAX_LEN {
        return full_name;
    }

    let hash = short_sha256_hex(full_name.as_bytes());
    let suffix = format!("__{hash}");
    let prefix_len = CHAT_TOOL_NAME_MAX_LEN.saturating_sub(suffix.len());
    let mut prefix = String::new();
    for ch in full_name.chars() {
        if prefix.len() + ch.len_utf8() > prefix_len {
            break;
        }
        prefix.push(ch);
    }
    format!("{prefix}{suffix}")
}

fn short_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
