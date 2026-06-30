use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<Function>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google: Option<GoogleTools>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_custom_tool: Option<ResponseCustomTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(
        default,
        rename = "parametersJsonSchema",
        skip_serializing_if = "Option::is_none"
    )]
    pub parameters_json_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, rename = "type", skip_serializing_if = "String::is_empty")]
    pub tool_type: String,
    pub function: FunctionCall,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_custom_tool_call: Option<ResponseCustomToolCall>,
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Value>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub transformer_metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    String(String),
    Named(NamedToolChoice),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedToolChoice {
    #[serde(rename = "type")]
    pub choice_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoogleTools {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_execution: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_context: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResponseCustomTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ResponseCustomToolFormat>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResponseCustomToolFormat {
    #[serde(rename = "type")]
    pub format_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub syntax: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub definition: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResponseCustomToolCall {
    pub call_id: String,
    pub name: String,
    pub input: String,
}
