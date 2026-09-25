use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn sibling_path(config_path: &Path, stem: &str) -> PathBuf {
    let file_name = match config_path
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some(extension) if !extension.is_empty() => format!("{stem}.{extension}"),
        _ => stem.to_string(),
    };
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(file_name)
}

pub fn v1_backup_path(config_path: &Path) -> PathBuf {
    sibling_path(config_path, "openvode_v1")
}

pub fn v2_backup_path(config_path: &Path) -> PathBuf {
    sibling_path(config_path, "opencode_v2")
}

pub fn is_active(config_path: &Path) -> bool {
    v1_backup_path(config_path).is_file()
}

fn object_from(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn take_object(map: &mut Map<String, Value>, key: &str) -> Result<Map<String, Value>, String> {
    match map.remove(key) {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(object)) => Ok(object),
        Some(_) => Err(format!(
            "Expected `{key}` to be an object while migrating OpenCode config"
        )),
    }
}

fn object_at_mut<'a>(
    map: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let value = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    value
        .as_object_mut()
        .ok_or_else(|| format!("Expected `{key}` to be an object while migrating OpenCode config"))
}

fn merge_object_value(
    target: &mut Map<String, Value>,
    key: &str,
    source: Map<String, Value>,
) -> Result<(), String> {
    let target_object = object_at_mut(target, key)?;
    for (source_key, source_value) in source {
        if let Some(existing) = target_object.get(&source_key) {
            if existing != &source_value {
                return Err(format!(
                    "Cannot migrate conflicting `{key}.{source_key}` values"
                ));
            }
        } else {
            target_object.insert(source_key, source_value);
        }
    }
    Ok(())
}

fn move_key(map: &mut Map<String, Value>, from: &str, to: &str) -> Result<(), String> {
    let Some(value) = map.remove(from) else {
        return Ok(());
    };
    if let Some(existing) = map.get(to) {
        if existing != &value {
            return Err(format!(
                "Cannot migrate both `{from}` and `{to}` because their values differ"
            ));
        }
        return Ok(());
    }
    map.insert(to.to_string(), value);
    Ok(())
}

fn remove_keys(map: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        map.remove(*key);
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn header_map(value: Option<&Value>) -> Map<String, Value> {
    object_from(value)
        .into_iter()
        .filter(|(_, value)| value.is_string())
        .collect()
}

fn canonical_provider_id(provider_id: &str) -> &str {
    match provider_id {
        "azure-cognitive-services" => "azure",
        "google-vertex-anthropic" => "google-vertex",
        _ => provider_id,
    }
}

fn canonicalize_model_reference(value: &mut Value) {
    let Some(reference) = value.as_str() else {
        return;
    };
    let Some((provider_id, model_id)) = reference.split_once('/') else {
        return;
    };
    let canonical_id = canonical_provider_id(provider_id);
    if canonical_id != provider_id {
        *value = Value::String(format!("{canonical_id}/{model_id}"));
    }
}

fn canonicalize_provider_list(value: &mut Value) {
    let Some(provider_ids) = value.as_array_mut() else {
        return;
    };
    for provider_id in provider_ids {
        if let Some(current_id) = provider_id.as_str() {
            let canonical_id = canonical_provider_id(current_id);
            if canonical_id != current_id {
                *provider_id = Value::String(canonical_id.to_string());
            }
        }
    }
}

fn snake_key(key: &str) -> String {
    let mut result = String::with_capacity(key.len());
    for character in key.chars() {
        if character.is_ascii_uppercase() {
            result.push('_');
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

fn snake_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(snake_value).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (snake_key(key), snake_value(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn snake_object(object: &Map<String, Value>) -> Map<String, Value> {
    object
        .iter()
        .map(|(key, value)| (snake_key(key), snake_value(value)))
        .collect()
}

fn bearer(value: Option<&Value>) -> Option<String> {
    string_value(value).map(|value| format!("Bearer {value}"))
}

fn insert_optional_string(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        map.insert(key.to_string(), Value::String(value));
    }
}

/// Provider-level option lowering, mirroring the official
/// `@opencode-ai/core/v1/config/provider-options` lowerers.
///
/// The result is stored under `api.url`, `api.settings` and `request`.
#[derive(Default)]
struct RenderedProviderOptions {
    url: Option<Value>,
    headers: Map<String, Value>,
    body: Map<String, Value>,
    settings: Map<String, Value>,
}

fn render_provider_options(
    package_name: Option<&str>,
    options: &Map<String, Value>,
    explicit_api: Option<&str>,
) -> RenderedProviderOptions {
    let package_name = package_name.unwrap_or("");
    let mut settings = options.clone();
    // Official lowerers let an explicit `options.headers` entry win over the
    // generated Authorization/x-api-key/api-key header, so merge it last.
    let custom_headers = header_map(options.get("headers"));
    let mut headers = Map::new();
    let mut body = object_from(options.get("body"));
    let mut url = string_value(options.get("baseURL")).map(Value::String);
    if url.is_none() {
        url = explicit_api.map(|value| Value::String(value.to_string()));
    }

    match package_name {
        "@ai-sdk/openai" => {
            insert_optional_string(
                &mut headers,
                "Authorization",
                string_value(options.get("apiKey")).map(|key| format!("Bearer {key}")),
            );
            insert_optional_string(
                &mut headers,
                "OpenAI-Organization",
                string_value(options.get("organization")),
            );
            insert_optional_string(
                &mut headers,
                "OpenAI-Project",
                string_value(options.get("project")),
            );
            remove_keys(
                &mut settings,
                &[
                    "apiKey",
                    "baseURL",
                    "organization",
                    "project",
                    "headers",
                    "body",
                ],
            );
        }
        "@ai-sdk/anthropic" | "@ai-sdk/google-vertex/anthropic" => {
            insert_optional_string(
                &mut headers,
                "x-api-key",
                string_value(options.get("apiKey")),
            );
            insert_optional_string(
                &mut headers,
                "Authorization",
                bearer(options.get("authToken")),
            );
            remove_keys(
                &mut settings,
                &["apiKey", "authToken", "baseURL", "headers", "body"],
            );
        }
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => {
            insert_optional_string(
                &mut headers,
                "x-goog-api-key",
                string_value(options.get("apiKey")),
            );
            remove_keys(&mut settings, &["apiKey", "baseURL", "headers", "body"]);
        }
        "@ai-sdk/azure" => {
            insert_optional_string(&mut headers, "api-key", string_value(options.get("apiKey")));
            remove_keys(&mut settings, &["apiKey", "baseURL", "headers", "body"]);
        }
        "@ai-sdk/amazon-bedrock" => {
            remove_keys(&mut settings, &["headers", "body"]);
        }
        "@ai-sdk/openai-compatible"
        | "@ai-sdk/cerebras"
        | "@ai-sdk/deepinfra"
        | "@ai-sdk/groq"
        | "@ai-sdk/mistral"
        | "@ai-sdk/togetherai"
        | "@ai-sdk/xai"
        | "@openrouter/ai-sdk-provider"
        | "ai-gateway-provider"
        | "venice-ai-sdk-provider" => {
            remove_keys(&mut settings, &["baseURL", "headers", "body"]);
        }
        _ => {
            // Official `raw` lowerer: the entire option object becomes the body.
            body = options.clone();
            settings.clear();
        }
    }

    for (key, value) in custom_headers {
        headers.insert(key, value);
    }

    RenderedProviderOptions {
        url,
        headers,
        body,
        settings,
    }
}

fn openai_request(options: &Map<String, Value>) -> Map<String, Value> {
    let mut result = snake_object(options);
    if options.get("reasoningEffort").is_some() || options.get("reasoningSummary").is_some() {
        let mut reasoning = result
            .remove("reasoning")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if let Some(effort) = options.get("reasoningEffort") {
            reasoning.insert("effort".to_string(), effort.clone());
        }
        if let Some(summary) = options.get("reasoningSummary") {
            reasoning.insert("summary".to_string(), summary.clone());
        }
        result.remove("reasoning_effort");
        result.remove("reasoning_summary");
        result.insert("reasoning".to_string(), Value::Object(reasoning));
    }
    if let Some(verbosity) = options.get("textVerbosity") {
        let mut text = result
            .remove("text")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        text.insert("verbosity".to_string(), verbosity.clone());
        result.remove("text_verbosity");
        result.insert("text".to_string(), Value::Object(text));
    }
    result
}

fn anthropic_request(options: &Map<String, Value>) -> Map<String, Value> {
    let mut result = snake_object(options);
    if options.get("effort").is_some() || options.get("taskBudget").is_some() {
        let mut output_config = Map::new();
        if let Some(effort) = options.get("effort") {
            output_config.insert("effort".to_string(), effort.clone());
        }
        if let Some(task_budget) = options.get("taskBudget") {
            output_config.insert("task_budget".to_string(), task_budget.clone());
        }
        result.remove("effort");
        result.remove("task_budget");
        result.insert("output_config".to_string(), Value::Object(output_config));
    }
    result
}

fn google_request(options: &Map<String, Value>) -> Map<String, Value> {
    const GENERATION_CONFIG_KEYS: [&str; 4] = [
        "thinkingConfig",
        "responseModalities",
        "mediaResolution",
        "imageConfig",
    ];
    let mut result = options.clone();
    let mut generation_config = Map::new();
    for key in GENERATION_CONFIG_KEYS {
        if let Some(value) = result.remove(key) {
            generation_config.insert(key.to_string(), value);
        }
    }
    if !generation_config.is_empty() {
        result.insert(
            "generationConfig".to_string(),
            Value::Object(generation_config),
        );
    }
    result
}

fn compatible_request(options: &Map<String, Value>) -> Map<String, Value> {
    let mut result = options.clone();
    if let Some(effort) = result.remove("reasoningEffort") {
        result.insert("reasoning_effort".to_string(), effort);
    }
    result
}

fn render_model_request(
    package_name: Option<&str>,
    options: &Map<String, Value>,
) -> Map<String, Value> {
    match package_name.unwrap_or("") {
        "@ai-sdk/openai" | "@ai-sdk/azure" => openai_request(options),
        "@ai-sdk/anthropic" | "@ai-sdk/google-vertex/anthropic" => anthropic_request(options),
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => google_request(options),
        "@ai-sdk/amazon-bedrock" => {
            let mut result = Map::new();
            result.insert(
                "additionalModelRequestFields".to_string(),
                Value::Object(options.clone()),
            );
            result
        }
        "@ai-sdk/openai-compatible"
        | "@ai-sdk/cerebras"
        | "@ai-sdk/deepinfra"
        | "@ai-sdk/groq"
        | "@ai-sdk/mistral"
        | "@ai-sdk/togetherai"
        | "@ai-sdk/xai"
        | "@openrouter/ai-sdk-provider"
        | "ai-gateway-provider"
        | "venice-ai-sdk-provider" => compatible_request(options),
        _ => options.clone(),
    }
}

fn permission_map_to_rules(value: &Value) -> Vec<Value> {
    let mut rules = Vec::new();
    let Some(permissions) = value.as_object() else {
        return rules;
    };
    for (action, rule) in permissions {
        match rule {
            Value::String(effect) => rules.push(json!({
                "action": action,
                "resource": "*",
                "effect": effect,
            })),
            Value::Object(resources) => {
                for (resource, effect) in resources {
                    rules.push(json!({
                        "action": action,
                        "resource": resource,
                        "effect": effect,
                    }));
                }
            }
            _ => {}
        }
    }
    rules
}

fn permission_rules_to_map(rules: &[Value]) -> Value {
    let mut grouped: std::collections::BTreeMap<String, Map<String, Value>> =
        std::collections::BTreeMap::new();
    for rule in rules {
        let Some(rule) = rule.as_object() else {
            continue;
        };
        let (Some(action), Some(resource), Some(effect)) = (
            rule.get("action").and_then(Value::as_str),
            rule.get("resource").and_then(Value::as_str),
            rule.get("effect"),
        ) else {
            continue;
        };
        grouped
            .entry(action.to_string())
            .or_default()
            .insert(resource.to_string(), effect.clone());
    }
    let mut result = Map::new();
    for (action, resources) in grouped {
        if resources.len() == 1 {
            if let Some(resource) = resources.keys().next().cloned() {
                if resource == "*" {
                    if let Some(effect) = resources.get(&resource).cloned() {
                        result.insert(action, effect);
                        continue;
                    }
                } else {
                    let mut object = Map::new();
                    if let Some(effect) = resources.get(&resource).cloned() {
                        object.insert(resource, effect);
                        result.insert(action, Value::Object(object));
                        continue;
                    }
                }
            }
        }
        result.insert(action, Value::Object(resources));
    }
    Value::Object(result)
}

fn migrate_agent(value: Value) -> Result<Value, String> {
    let mut agent = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected OpenCode agent entries to be objects".to_string())?;
    let mut out = Map::new();
    if let Some(model) = agent.remove("model") {
        out.insert("model".to_string(), model);
    }
    if let Some(variant) = agent.remove("variant") {
        out.insert("variant".to_string(), variant);
    }
    let mut body = take_object(&mut agent, "options")?;
    if let Some(temperature) = agent.remove("temperature") {
        body.insert("temperature".to_string(), temperature);
    }
    if let Some(top_p) = agent.remove("top_p") {
        body.insert("top_p".to_string(), top_p);
    }
    if !body.is_empty() {
        out.insert("request".to_string(), json!({ "body": body }));
    }
    if let Some(prompt) = agent.remove("prompt") {
        out.insert("system".to_string(), prompt);
    }
    for key in ["description", "mode", "hidden", "color", "steps"] {
        if let Some(value) = agent.remove(key) {
            out.insert(key.to_string(), value);
        }
    }
    if let Some(disable) = agent.remove("disable") {
        out.insert("disabled".to_string(), disable);
    }
    if let Some(permission) = agent.remove("permission") {
        let rules = permission_map_to_rules(&permission);
        if !rules.is_empty() {
            out.insert("permissions".to_string(), Value::Array(rules));
        }
    }
    // Official v2 migration does not port the legacy boolean tool map or
    // `maxSteps`; permissions and `steps` are the v2 surfaces.
    agent.remove("tools");
    agent.remove("maxSteps");
    // Preserve unknown fields so an editor round trip does not drop provider
    // specific agent keys the V1 model itself cannot express.
    for (key, value) in agent {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn migrate_agents(root: &mut Map<String, Value>) -> Result<(), String> {
    let mut entries = Map::new();
    if let Some(agent) = root.remove("agent") {
        entries.extend(
            agent
                .as_object()
                .cloned()
                .ok_or_else(|| "Expected V1 `agent` to be an object".to_string())?,
        );
    }
    if let Some(mode) = root.remove("mode") {
        for (name, mut agent) in mode
            .as_object()
            .cloned()
            .ok_or_else(|| "Expected V1 `mode` to be an object".to_string())?
        {
            if let Some(object) = agent.as_object_mut() {
                object
                    .entry("mode".to_string())
                    .or_insert_with(|| Value::String("primary".to_string()));
            }
            entries.insert(name, agent);
        }
    }
    if !entries.is_empty() {
        let mut agents = entries
            .into_iter()
            .filter_map(|(name, value)| {
                value
                    .is_null()
                    .then_some(None)
                    .unwrap_or_else(|| Some((name, value)))
            })
            .collect::<Map<_, _>>();
        for value in agents.values_mut() {
            *value = migrate_agent(std::mem::take(value))?;
        }
        merge_object_value(root, "agents", agents)?;
    }

    if let Some(mut small_model) = root.remove("small_model") {
        canonicalize_model_reference(&mut small_model);
        let agents = object_at_mut(root, "agents")?;
        let title_agent = object_at_mut(agents, "title")?;
        if let Some(existing) = title_agent.get("model") {
            if existing != &small_model {
                return Err(
                    "Conflicting model selections in `small_model` and `agents.title.model`"
                        .to_string(),
                );
            }
        } else {
            title_agent.insert("model".to_string(), small_model);
        }
    }
    Ok(())
}

fn migrate_mcp_server(value: Value) -> Result<Value, String> {
    let mut server = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected MCP server entries to be objects".to_string())?;
    if let Some(enabled) = server.remove("enabled") {
        let disabled = !enabled
            .as_bool()
            .ok_or_else(|| "Expected V1 MCP server `enabled` to be a boolean".to_string())?;
        server.insert("disabled".to_string(), Value::Bool(disabled));
    }
    if let Some(timeout) = server.remove("timeout") {
        if let Some(timeout) = timeout.as_u64() {
            server.insert("timeout".to_string(), json!({ "request": timeout }));
        } else {
            server.insert("timeout".to_string(), timeout);
        }
    }
    if let Some(oauth) = server.remove("oauth") {
        if let Some(oauth) = oauth.as_object() {
            let mut converted = Map::new();
            for (legacy, native) in [
                ("clientId", "client_id"),
                ("clientSecret", "client_secret"),
                ("scope", "scope"),
                ("callbackPort", "callback_port"),
                ("redirectUri", "redirect_uri"),
            ] {
                if let Some(value) = oauth.get(legacy) {
                    converted.insert(native.to_string(), value.clone());
                }
            }
            server.insert("oauth".to_string(), Value::Object(converted));
        } else {
            server.insert("oauth".to_string(), oauth);
        }
    }
    Ok(Value::Object(server))
}

fn migrate_mcp(root: &mut Map<String, Value>) -> Result<(), String> {
    let Some(mcp) = root.remove("mcp") else {
        return Ok(());
    };
    let mut mcp = mcp
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V1 `mcp` to be an object".to_string())?;
    let mut servers = take_object(&mut mcp, "servers")?;
    let global_timeout = mcp.remove("timeout");
    for (name, server) in std::mem::take(&mut mcp) {
        if servers.contains_key(&name) {
            return Err(format!(
                "MCP server `{name}` is defined in both legacy and V2 locations"
            ));
        }
        servers.insert(name, migrate_mcp_server(server)?);
    }
    let mut out = Map::new();
    if let Some(experimental) = root.get_mut("experimental").and_then(Value::as_object_mut) {
        if let Some(timeout) = experimental.remove("mcp_timeout") {
            if let Some(timeout) = timeout.as_u64() {
                out.insert("timeout".to_string(), json!({ "request": timeout }));
            } else {
                out.insert("timeout".to_string(), timeout);
            }
        }
        if experimental.is_empty() {
            root.remove("experimental");
        }
    }
    if let Some(timeout) = global_timeout {
        if let Some(timeout) = timeout.as_u64() {
            out.insert("timeout".to_string(), json!({ "request": timeout }));
        } else {
            out.entry("timeout".to_string()).or_insert(timeout);
        }
    }
    if !servers.is_empty() {
        out.insert("servers".to_string(), Value::Object(servers));
    }
    if !out.is_empty() {
        root.insert("mcp".to_string(), Value::Object(out));
    }
    Ok(())
}

fn migrate_model(value: Value, parent_package: Option<&str>) -> Result<Value, String> {
    let mut model = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected OpenCode model entries to be objects".to_string())?;
    let model_provider = model
        .remove("provider")
        .and_then(|value| value.as_object().cloned());
    let model_package = model_provider
        .as_ref()
        .and_then(|provider| string_value(provider.get("npm")));
    let package_name = model_package
        .clone()
        .or_else(|| parent_package.map(str::to_string));

    let id = string_value(model.get("id"));
    let family = model.remove("family");
    let name = model.remove("name");
    let limit = model.remove("limit");
    let modalities = model.remove("modalities");
    let tool_call = model.remove("tool_call");
    let cost = model.remove("cost");
    let options = take_object(&mut model, "options")?;
    let headers = model.remove("headers");
    let variants = model.remove("variants");
    let status = string_value(model.get("status"));

    let mut out = Map::new();
    if let Some(family) = family {
        out.insert("family".to_string(), family);
    }
    if let Some(name) = name {
        out.insert("name".to_string(), name);
    }
    // The released 2.0 shape names these flat: `modelID` is the API model id
    // (`api.id` in the preview shape), `package` overrides the provider package
    // for this model, and `settings` carries the V1 options verbatim because the
    // official `model()` mapping is an identity function.
    if let Some(id) = id.clone() {
        out.insert("modelID".to_string(), Value::String(id));
    }
    if let Some(package) = model_package.as_ref() {
        out.insert("package".to_string(), Value::String(package.clone()));
    }
    if !options.is_empty() {
        out.insert("settings".to_string(), Value::Object(options.clone()));
    }
    if let Some(headers) = headers
        .as_ref()
        .and_then(|value| value.as_object().cloned())
    {
        out.insert("headers".to_string(), Value::Object(headers));
    }
    if let Some(package) = model_package.as_ref() {
        let mut api = Map::new();
        if let Some(id) = id.clone() {
            api.insert("id".to_string(), Value::String(id));
        }
        api.insert("type".to_string(), Value::String("aisdk".to_string()));
        api.insert("package".to_string(), Value::String(package.clone()));
        if let Some(url) = model_provider
            .as_ref()
            .and_then(|provider| string_value(provider.get("api")))
        {
            api.insert("url".to_string(), Value::String(url));
        }
        api.insert("settings".to_string(), Value::Object(Map::new()));
        out.insert("api".to_string(), Value::Object(api));
    } else if let Some(id) = id {
        out.insert("api".to_string(), json!({ "id": id }));
    }

    let capabilities = if tool_call.is_some()
        || modalities
            .as_ref()
            .and_then(|value| value.get("input"))
            .is_some()
        || modalities
            .as_ref()
            .and_then(|value| value.get("output"))
            .is_some()
    {
        Some(json!({
            "tools": tool_call.as_ref().and_then(Value::as_bool).unwrap_or(false),
            "input": modalities.as_ref().and_then(|value| value.get("input")).cloned().unwrap_or_else(|| json!([])),
            "output": modalities.as_ref().and_then(|value| value.get("output")).cloned().unwrap_or_else(|| json!([])),
        }))
    } else {
        None
    };
    if let Some(capabilities) = capabilities {
        out.insert("capabilities".to_string(), capabilities);
    }

    let rendered_request = package_name
        .as_deref()
        .map(|package| render_model_request(Some(package), &options))
        .unwrap_or_else(|| render_model_request(None, &options));
    let mut request = Map::new();
    if let Some(headers) = headers.and_then(|value| value.as_object().cloned()) {
        request.insert("headers".to_string(), Value::Object(headers));
    }
    if !options.is_empty() {
        request.insert("body".to_string(), Value::Object(rendered_request));
    }
    if !request.is_empty() {
        out.insert("request".to_string(), Value::Object(request));
    }

    if let Some(variants) = variants.and_then(|value| value.as_object().cloned()) {
        let converted = variants
            .into_iter()
            .map(|(id, options)| {
                let body = options
                    .as_object()
                    .map(|options| render_model_request(package_name.as_deref(), options))
                    .unwrap_or_default();
                json!({ "id": id, "body": body })
            })
            .collect::<Vec<_>>();
        out.insert("variants".to_string(), Value::Array(converted));
    }

    if let Some(cost) = cost.and_then(|value| value.as_object().cloned()) {
        let cache = json!({
            "read": cost.get("cache_read").cloned().unwrap_or_else(|| json!(0)),
            "write": cost.get("cache_write").cloned().unwrap_or_else(|| json!(0)),
        });
        let mut entry = Map::new();
        entry.insert(
            "input".to_string(),
            cost.get("input").cloned().unwrap_or_else(|| json!(0)),
        );
        entry.insert(
            "output".to_string(),
            cost.get("output").cloned().unwrap_or_else(|| json!(0)),
        );
        entry.insert("cache".to_string(), cache);
        let mut costs = vec![Value::Object(entry)];
        if let Some(context) = cost.get("context_over_200k").and_then(Value::as_object) {
            costs.push(json!({
                "tier": { "type": "context", "size": 200000 },
                "input": context.get("input").cloned().unwrap_or_else(|| json!(0)),
                "output": context.get("output").cloned().unwrap_or_else(|| json!(0)),
                "cache": {
                    "read": context.get("cache_read").cloned().unwrap_or_else(|| json!(0)),
                    "write": context.get("cache_write").cloned().unwrap_or_else(|| json!(0)),
                },
            }));
        }
        out.insert("cost".to_string(), Value::Array(costs));
    }

    if status.as_deref() == Some("deprecated") {
        out.insert("disabled".to_string(), Value::Bool(true));
    }
    if let Some(limit) = limit.and_then(|value| value.as_object().cloned()) {
        let mut converted = Map::new();
        for key in ["context", "input", "output"] {
            if let Some(value) = limit.get(key) {
                if let Some(value) = value
                    .as_i64()
                    .or_else(|| value.as_f64().map(|value| value.trunc() as i64))
                {
                    converted.insert(key.to_string(), Value::Number(value.into()));
                }
            }
        }
        if !converted.is_empty() {
            out.insert("limit".to_string(), Value::Object(converted));
        }
    }

    // Preserve unknown model fields that are not part of the V1 schema this
    // converter understands. Known legacy flags are intentionally not ported
    // (official migrate drops reasoning/temperature/attachment/interleaved).
    for key in [
        "release_date",
        "attachment",
        "reasoning",
        "temperature",
        "experimental",
        "interleaved",
        "status",
        "id",
    ] {
        model.remove(key);
    }
    for (key, value) in model {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn migrate_provider(value: Value) -> Result<Value, String> {
    let mut provider = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected OpenCode provider entries to be objects".to_string())?;
    let package_name = string_value(provider.remove("npm").as_ref());
    let explicit_api = string_value(provider.remove("api").as_ref());
    let options = take_object(&mut provider, "options")?;
    let models = provider.remove("models");
    let name = provider.remove("name");
    let env = provider.remove("env");
    let rendered =
        render_provider_options(package_name.as_deref(), &options, explicit_api.as_deref());

    let mut out = Map::new();
    if let Some(name) = name {
        out.insert("name".to_string(), name);
    }
    if let Some(env) = env {
        out.insert("env".to_string(), env);
    }
    if let Some(package) = package_name.clone() {
        let mut api = Map::new();
        api.insert("type".to_string(), Value::String("aisdk".to_string()));
        api.insert("package".to_string(), Value::String(package.clone()));
        if let Some(url) = rendered.url.clone() {
            api.insert("url".to_string(), url);
        }
        if !rendered.settings.is_empty() {
            api.insert(
                "settings".to_string(),
                Value::Object(rendered.settings.clone()),
            );
        }
        out.insert("api".to_string(), Value::Object(api));
        // OpenCode 2.0 reads the provider flat (`package` plus `settings`/
        // `headers`/`body` overlays, with the base URL inside `settings`) and
        // rejects a provider whose package it cannot resolve, so both shapes are
        // written: the preview shape keeps the URL at `api.url`, the released
        // shape needs `settings.baseURL`. Each decoder ignores the other's keys.
        out.insert("package".to_string(), Value::String(package));
        let mut settings = rendered.settings.clone();
        if let Some(url) = rendered.url.clone() {
            settings.insert("baseURL".to_string(), url);
        }
        if !settings.is_empty() {
            out.insert("settings".to_string(), Value::Object(settings));
        }
    } else if let Some(url) = rendered.url.clone() {
        out.insert("api".to_string(), json!({ "type": "native", "url": url }));
    }
    if !rendered.headers.is_empty() || !rendered.body.is_empty() {
        let mut request = Map::new();
        if !rendered.headers.is_empty() {
            request.insert(
                "headers".to_string(),
                Value::Object(rendered.headers.clone()),
            );
        }
        if !rendered.body.is_empty() {
            request.insert("body".to_string(), Value::Object(rendered.body.clone()));
        }
        out.insert("request".to_string(), Value::Object(request));
    }
    if !rendered.headers.is_empty() {
        out.insert(
            "headers".to_string(),
            Value::Object(rendered.headers.clone()),
        );
    }
    if !rendered.body.is_empty() {
        out.insert("body".to_string(), Value::Object(rendered.body.clone()));
    }
    if let Some(models) = models.and_then(|value| value.as_object().cloned()) {
        let converted = models
            .into_iter()
            .map(|(id, model)| Ok((id, migrate_model(model, package_name.as_deref())?)))
            .collect::<Result<Map<_, _>, String>>()?;
        out.insert("models".to_string(), Value::Object(converted));
    }
    // Preserve provider fields V2 does not model (e.g. whitelist/blacklist) so a
    // read/write round trip through this editor does not silently delete them.
    for (key, value) in provider {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn migrate_providers(value: Value) -> Result<Value, String> {
    let providers = value
        .as_object()
        .ok_or_else(|| "Expected V1 `provider` to be an object".to_string())?;
    let mut result = Map::new();
    for (provider_id, provider) in providers {
        let canonical_id = canonical_provider_id(provider_id);
        if result
            .insert(
                canonical_id.to_string(),
                migrate_provider(provider.clone())?,
            )
            .is_some()
        {
            return Err(format!(
                "Provider ID collision while migrating `{provider_id}` to `{canonical_id}`"
            ));
        }
    }
    Ok(Value::Object(result))
}

fn migrate_plugin_list(value: Value) -> Result<Value, String> {
    let plugins = value
        .as_array()
        .ok_or_else(|| "Expected V1 `plugin` to be an array".to_string())?;
    let mut converted = Vec::with_capacity(plugins.len());
    for plugin in plugins {
        match plugin {
            Value::String(_) => converted.push(plugin.clone()),
            Value::Array(entry) if entry.len() == 2 => {
                converted.push(json!({
                    "package": entry[0].clone(),
                    "options": entry[1].clone(),
                }));
            }
            other => return Err(format!("Unsupported V1 plugin entry: {other}")),
        }
    }
    Ok(Value::Array(converted))
}

fn normalize_tool_action(action: &str) -> &str {
    match action {
        "write" | "patch" => "edit",
        _ => action,
    }
}

fn migrate_permissions(root: &mut Map<String, Value>) -> Result<(), String> {
    let mut rules = Vec::new();
    if let Some(permission) = root.remove("permission") {
        rules.extend(permission_map_to_rules(&permission));
    }
    if let Some(tools) = root
        .remove("tools")
        .and_then(|value| value.as_object().cloned())
    {
        for (action, enabled) in tools {
            rules.push(json!({
                "action": normalize_tool_action(&action),
                "resource": "*",
                "effect": if enabled.as_bool().unwrap_or(false) { "allow" } else { "deny" },
            }));
        }
    }
    if rules.is_empty() {
        return Ok(());
    }
    let mut permissions = match root.remove("permissions") {
        None => Vec::new(),
        Some(Value::Array(existing)) => existing,
        Some(_) => return Err("Expected V2 `permissions` to be an array".to_string()),
    };
    permissions.extend(rules);
    root.insert("permissions".to_string(), Value::Array(permissions));
    Ok(())
}

fn append_policies(root: &mut Map<String, Value>, rules: Vec<Value>) -> Result<(), String> {
    if rules.is_empty() {
        return Ok(());
    }
    let experimental = object_at_mut(root, "experimental")?;
    let policies = match experimental.remove("policies") {
        None => Vec::new(),
        Some(Value::Array(existing)) => existing,
        Some(_) => return Err("Expected V2 `experimental.policies` to be an array".to_string()),
    };
    let mut policies = policies;
    policies.extend(rules);
    experimental.insert("policies".to_string(), Value::Array(policies));
    Ok(())
}

fn migrate_disabled_and_enabled_providers(root: &mut Map<String, Value>) -> Result<(), String> {
    let disabled = root.remove("disabled_providers");
    let enabled = root.remove("enabled_providers");
    let mut rules = Vec::new();
    if let Some(disabled) = disabled {
        let ids = string_array(Some(&disabled));
        for id in ids {
            rules.push(json!({ "effect": "deny", "action": "provider.use", "resource": id }));
        }
    }
    if let Some(enabled) = enabled {
        let ids = string_array(Some(&enabled));
        if !ids.is_empty() {
            rules.push(json!({ "effect": "deny", "action": "provider.use", "resource": "*" }));
            for id in ids {
                rules.push(json!({ "effect": "allow", "action": "provider.use", "resource": id }));
            }
        }
    }
    append_policies(root, rules)
}

pub fn v1_to_v2_value(mut value: Value) -> Result<Value, String> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| "OpenCode config root must be an object".to_string())?;

    if root.contains_key("provider") && root.contains_key("providers") {
        return Err("OpenCode config contains both V1 `provider` and V2 `providers`".to_string());
    }
    if root.contains_key("plugin") && root.contains_key("plugins") {
        return Err("OpenCode config contains both V1 `plugin` and V2 `plugins`".to_string());
    }
    if root.contains_key("agent") && root.contains_key("agents") {
        return Err("OpenCode config contains both V1 `agent` and V2 `agents`".to_string());
    }

    if let Some(providers) = root.remove("provider") {
        root.insert("providers".to_string(), migrate_providers(providers)?);
    }
    if let Some(plugins) = root.remove("plugin") {
        root.insert("plugins".to_string(), migrate_plugin_list(plugins)?);
    }
    migrate_agents(root)?;
    migrate_permissions(root)?;
    migrate_mcp(root)?;
    migrate_disabled_and_enabled_providers(root)?;

    move_key(root, "snapshot", "snapshots")?;
    move_key(root, "attachment", "attachments")?;
    move_key(root, "command", "commands")?;
    move_key(root, "reference", "references")?;

    if let Some(skills) = root.remove("skills") {
        if let Some(skills) = skills.as_object() {
            let mut converted = Vec::new();
            converted.extend(string_array(skills.get("paths")));
            converted.extend(string_array(skills.get("urls")));
            root.insert(
                "skills".to_string(),
                Value::Array(converted.into_iter().map(Value::String).collect()),
            );
        } else if !skills.is_null() {
            root.insert("skills".to_string(), skills);
        }
    }

    if let Some(autoshare) = root.remove("autoshare") {
        if root.get("share").is_none() && autoshare.as_bool() == Some(true) {
            root.insert("share".to_string(), Value::String("auto".to_string()));
        }
    }

    if let Some(compaction) = root.get_mut("compaction").and_then(Value::as_object_mut) {
        let auto = compaction.remove("auto");
        let prune = compaction.remove("prune");
        let preserve_recent_tokens = compaction.remove("preserve_recent_tokens");
        let reserved = compaction.remove("reserved");
        if let Some(auto) = auto {
            compaction.insert("auto".to_string(), auto);
        }
        if let Some(prune) = prune {
            compaction.insert("prune".to_string(), prune);
        }
        if let Some(tokens) = preserve_recent_tokens {
            compaction.insert("keep".to_string(), json!({ "tokens": tokens }));
        }
        if let Some(buffer) = reserved {
            compaction.insert("buffer".to_string(), buffer);
        }
    }

    Ok(value)
}

fn convert_provider_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut provider = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 provider entries to be objects".to_string())?;
    let mut out = Map::new();
    if let Some(name) = provider.remove("name") {
        out.insert("name".to_string(), name);
    }
    if let Some(env) = provider.remove("env") {
        out.insert("env".to_string(), env);
    }
    let mut options = Map::new();
    if let Some(api) = provider
        .remove("api")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(package) = string_value(api.get("package")) {
            out.insert("npm".to_string(), Value::String(package));
        }
        if let Some(url) = string_value(api.get("url")) {
            out.insert("api".to_string(), Value::String(url.clone()));
            options.insert("baseURL".to_string(), Value::String(url));
        }
        if let Some(settings) = api.get("settings").and_then(Value::as_object) {
            for (key, value) in settings {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(request) = provider
        .remove("request")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(headers) = request.get("headers") {
            options.insert("headers".to_string(), headers.clone());
        }
        if let Some(body) = request.get("body") {
            options.insert("body".to_string(), body.clone());
        }
    }
    // A provider written by OpenCode 2.0 itself only carries the flat shape, and
    // a file written by this editor carries both; `or_insert` keeps the preview
    // shape authoritative when the two overlap without dropping flat-only keys.
    let flat_settings = provider
        .remove("settings")
        .and_then(|value| value.as_object().cloned());
    let flat_package = string_value(provider.remove("package").as_ref());
    if !out.contains_key("npm") {
        if let Some(package) = flat_package {
            out.insert("npm".to_string(), Value::String(package));
        }
    }
    if let Some(url) = flat_settings
        .as_ref()
        .and_then(|settings| string_value(settings.get("baseURL")))
    {
        out.entry("api".to_string())
            .or_insert(Value::String(url.clone()));
        options
            .entry("baseURL".to_string())
            .or_insert(Value::String(url));
    }
    if let Some(settings) = flat_settings {
        for (key, value) in settings {
            options.entry(key).or_insert(value);
        }
    }
    for key in ["headers", "body"] {
        if !options.contains_key(key) {
            if let Some(value) = provider.remove(key) {
                options.insert(key.to_string(), value);
            }
        } else {
            provider.remove(key);
        }
    }
    if !options.is_empty() {
        out.insert("options".to_string(), Value::Object(options));
    }
    if let Some(models) = provider
        .remove("models")
        .and_then(|value| value.as_object().cloned())
    {
        let converted = models
            .into_iter()
            .map(|(id, model)| Ok((id, convert_model_v2_to_v1(model)?)))
            .collect::<Result<Map<_, _>, String>>()?;
        out.insert("models".to_string(), Value::Object(converted));
    }
    for (key, value) in provider {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn convert_model_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut model = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 model entries to be objects".to_string())?;
    let mut options = Map::new();
    if let Some(api) = model
        .remove("api")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(id) = api.get("id") {
            model.insert("id".to_string(), id.clone());
        }
        if let Some(package) = string_value(api.get("package")) {
            let url = string_value(api.get("url"));
            let mut provider = Map::new();
            provider.insert("npm".to_string(), Value::String(package));
            if let Some(url) = url {
                provider.insert("api".to_string(), Value::String(url));
            }
            model.insert("provider".to_string(), Value::Object(provider));
        }
        if let Some(settings) = api.get("settings").and_then(Value::as_object) {
            for (key, value) in settings {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(request) = model
        .remove("request")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(headers) = request.get("headers") {
            model.insert("headers".to_string(), headers.clone());
        }
        if let Some(body) = request.get("body").and_then(Value::as_object) {
            for (key, value) in body {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    // Same dual-shape rule as providers: the flat keys are what OpenCode 2.0
    // writes, they only fill what the preview shape left unset, and they are
    // consumed here so a save cannot echo them back next to the preview fields.
    if !model.contains_key("id") {
        if let Some(id) = string_value(model.get("modelID")) {
            model.insert("id".to_string(), Value::String(id));
        }
    }
    model.remove("modelID");
    if !model.contains_key("provider") {
        if let Some(package) = string_value(model.get("package")) {
            model.insert("provider".to_string(), json!({ "npm": package }));
        }
    }
    model.remove("package");
    if let Some(settings) = model
        .remove("settings")
        .and_then(|value| value.as_object().cloned())
    {
        for (key, value) in settings {
            options.entry(key).or_insert(value);
        }
    }
    if !options.is_empty() {
        model.insert("options".to_string(), Value::Object(options));
    }
    if let Some(capabilities) = model
        .remove("capabilities")
        .and_then(|value| value.as_object().cloned())
    {
        if capabilities.get("input").is_some() || capabilities.get("output").is_some() {
            let mut modalities = Map::new();
            if let Some(input) = capabilities.get("input") {
                modalities.insert("input".to_string(), input.clone());
            }
            if let Some(output) = capabilities.get("output") {
                modalities.insert("output".to_string(), output.clone());
            }
            model.insert("modalities".to_string(), Value::Object(modalities));
        }
        if let Some(tools) = capabilities.get("tools") {
            model.insert("tool_call".to_string(), tools.clone());
        }
    }
    if let Some(variants) = model
        .remove("variants")
        .and_then(|value| value.as_array().cloned())
    {
        let mut converted = Map::new();
        for variant in variants {
            let Some(variant) = variant.as_object() else {
                continue;
            };
            let Some(id) = string_value(variant.get("id")) else {
                continue;
            };
            let mut value = variant
                .get("body")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (key, item) in variant {
                if key != "id" && key != "body" {
                    value.entry(key.clone()).or_insert_with(|| item.clone());
                }
            }
            converted.insert(id, Value::Object(value));
        }
        model.insert("variants".to_string(), Value::Object(converted));
    }
    if let Some(costs) = model
        .remove("cost")
        .and_then(|value| value.as_array().cloned())
    {
        if let Some(first) = costs.first().and_then(Value::as_object) {
            let mut cost = Map::new();
            if let Some(input) = first.get("input") {
                cost.insert("input".to_string(), input.clone());
            }
            if let Some(output) = first.get("output") {
                cost.insert("output".to_string(), output.clone());
            }
            if let Some(cache) = first.get("cache").and_then(Value::as_object) {
                if let Some(read) = cache.get("read") {
                    cost.insert("cache_read".to_string(), read.clone());
                }
                if let Some(write) = cache.get("write") {
                    cost.insert("cache_write".to_string(), write.clone());
                }
            }
            if let Some(second) = costs.get(1).and_then(Value::as_object) {
                cost.insert(
                    "context_over_200k".to_string(),
                    Value::Object(second.clone()),
                );
            }
            model.insert("cost".to_string(), Value::Object(cost));
        }
    }
    if model.remove("disabled").and_then(|value| value.as_bool()) == Some(true) {
        model.insert(
            "status".to_string(),
            Value::String("deprecated".to_string()),
        );
    }
    Ok(Value::Object(model))
}

fn convert_providers_v2_to_v1(value: Value) -> Result<Value, String> {
    let providers = value
        .as_object()
        .ok_or_else(|| "Expected V2 `providers` to be an object".to_string())?;
    let mut result = Map::new();
    for (provider_id, provider) in providers {
        result.insert(
            provider_id.clone(),
            convert_provider_v2_to_v1(provider.clone())?,
        );
    }
    Ok(Value::Object(result))
}

fn convert_plugin_list_v2_to_v1(value: Value) -> Result<Value, String> {
    let plugins = value
        .as_array()
        .ok_or_else(|| "Expected V2 `plugins` to be an array".to_string())?;
    let mut converted = Vec::with_capacity(plugins.len());
    for plugin in plugins {
        match plugin {
            Value::String(_) => converted.push(plugin.clone()),
            Value::Object(entry) => {
                let package = entry.get("package").cloned().ok_or_else(|| {
                    "Expected V2 plugin entry to have a string `package`".to_string()
                })?;
                let options = entry.get("options").cloned().unwrap_or_else(|| json!({}));
                converted.push(Value::Array(vec![package, options]));
            }
            other => return Err(format!("Unsupported V2 plugin entry: {other}")),
        }
    }
    Ok(Value::Array(converted))
}

fn convert_agent_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut agent = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 agent entries to be objects".to_string())?;
    let mut out = Map::new();
    if let Some(model) = agent.remove("model") {
        match model {
            Value::String(reference) => {
                if let Some((model, variant)) = reference.split_once('#') {
                    out.insert("model".to_string(), Value::String(model.to_string()));
                    out.insert("variant".to_string(), Value::String(variant.to_string()));
                } else {
                    out.insert("model".to_string(), Value::String(reference));
                }
            }
            Value::Object(selection) => {
                let provider_id = string_value(selection.get("providerID"));
                let model_id = string_value(selection.get("model"));
                if let (Some(provider_id), Some(model_id)) = (provider_id, model_id) {
                    out.insert(
                        "model".to_string(),
                        Value::String(format!("{provider_id}/{model_id}")),
                    );
                }
                if let Some(variant) = selection.get("variant") {
                    out.insert("variant".to_string(), variant.clone());
                }
            }
            other => {
                out.insert("model".to_string(), other);
            }
        }
    }
    if let Some(system) = agent.remove("system") {
        out.insert("prompt".to_string(), system);
    }
    if let Some(disabled) = agent.remove("disabled") {
        out.insert("disable".to_string(), disabled);
    }
    let mut options = Map::new();
    if let Some(request) = agent
        .remove("request")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(headers) = request.get("headers") {
            options.insert("headers".to_string(), headers.clone());
        }
        if let Some(body) = request.get("body").and_then(Value::as_object) {
            for (key, value) in body {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    if !options.is_empty() {
        out.insert("options".to_string(), Value::Object(options));
    }
    if let Some(permissions) = agent
        .remove("permissions")
        .and_then(|value| value.as_array().cloned())
    {
        out.insert(
            "permission".to_string(),
            permission_rules_to_map(&permissions),
        );
    }
    for (key, value) in agent {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn convert_agents_v2_to_v1(root: &mut Map<String, Value>) -> Result<(), String> {
    let Some(agents) = root.remove("agents") else {
        return Ok(());
    };
    let mut converted = Map::new();
    if let Some(agents) = agents.as_object() {
        for (name, agent) in agents {
            converted.insert(name.clone(), convert_agent_v2_to_v1(agent.clone())?);
        }
    } else {
        return Err("Expected V2 `agents` to be an object".to_string());
    }
    let small_model = converted
        .get("title")
        .and_then(Value::as_object)
        .and_then(|title| title.get("model"))
        .and_then(|model| match model {
            Value::String(reference) => {
                Some(reference.split('#').next().unwrap_or(reference).to_string())
            }
            Value::Object(selection) => {
                let provider_id = string_value(selection.get("providerID"))?;
                let model_id = string_value(selection.get("model"))?;
                Some(format!("{provider_id}/{model_id}"))
            }
            _ => None,
        });
    root.insert("agent".to_string(), Value::Object(converted));
    if let Some(small_model) = small_model {
        root.insert("small_model".to_string(), Value::String(small_model));
    }
    Ok(())
}

fn convert_mcp_server_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut server = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 MCP server entries to be objects".to_string())?;
    if let Some(disabled) = server.remove("disabled") {
        server.insert(
            "enabled".to_string(),
            Value::Bool(!disabled.as_bool().unwrap_or(false)),
        );
    }
    if let Some(timeout) = server
        .remove("timeout")
        .and_then(|value| value.as_object().cloned())
    {
        if timeout.get("startup").is_none() {
            if let Some(request) = timeout.get("request") {
                server.insert("timeout".to_string(), request.clone());
            } else {
                server.insert("timeout".to_string(), Value::Object(timeout));
            }
        } else {
            server.insert("timeout".to_string(), Value::Object(timeout));
        }
    }
    if let Some(oauth) = server
        .remove("oauth")
        .and_then(|value| value.as_object().cloned())
    {
        let mut converted = Map::new();
        for (native, legacy) in [
            ("client_id", "clientId"),
            ("client_secret", "clientSecret"),
            ("scope", "scope"),
            ("callback_port", "callbackPort"),
            ("redirect_uri", "redirectUri"),
        ] {
            if let Some(value) = oauth.get(native) {
                converted.insert(legacy.to_string(), value.clone());
            }
        }
        server.insert("oauth".to_string(), Value::Object(converted));
    }
    Ok(Value::Object(server))
}

fn convert_mcp_v2_to_v1(root: &mut Map<String, Value>) -> Result<(), String> {
    let Some(mcp) = root.remove("mcp") else {
        return Ok(());
    };
    let mut mcp = mcp
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 `mcp` to be an object".to_string())?;
    let mut servers = take_object(&mut mcp, "servers")?;
    if let Some(timeout) = mcp
        .remove("timeout")
        .and_then(|value| value.as_object().cloned())
    {
        if timeout.get("startup").is_none() {
            if let Some(request) = timeout.get("request") {
                let experimental = object_at_mut(root, "experimental")?;
                experimental.insert("mcp_timeout".to_string(), request.clone());
            }
        }
    }
    for (name, server) in std::mem::take(&mut servers) {
        mcp.insert(name, convert_mcp_server_v2_to_v1(server)?);
    }
    if !mcp.is_empty() {
        root.insert("mcp".to_string(), Value::Object(mcp));
    }
    Ok(())
}

fn take_provider_selection_policies(
    root: &mut Map<String, Value>,
) -> (Option<Vec<String>>, Option<Vec<String>>) {
    let Some(experimental) = root.get_mut("experimental").and_then(Value::as_object_mut) else {
        return (None, None);
    };
    let Some(policies) = experimental
        .remove("policies")
        .and_then(|value| value.as_array().cloned())
    else {
        return (None, None);
    };
    let provider_rules = policies.iter().all(|rule| {
        rule.as_object()
            .is_some_and(|rule| rule.get("action").and_then(Value::as_str) == Some("provider.use"))
    });
    if !provider_rules || policies.is_empty() {
        experimental.insert("policies".to_string(), Value::Array(policies));
        return (None, None);
    }
    let every_deny = policies.iter().all(|rule| {
        rule.as_object().is_some_and(|rule| {
            rule.get("effect").and_then(Value::as_str) == Some("deny")
                && rule
                    .get("resource")
                    .and_then(Value::as_str)
                    .is_some_and(|resource| resource != "*")
        })
    });
    let (disabled, enabled) = if every_deny {
        (
            Some(
                policies
                    .iter()
                    .filter_map(|rule| {
                        rule.get("resource")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect::<Vec<_>>(),
            ),
            None,
        )
    } else {
        let mut allow = policies.iter();
        let first = allow.next().and_then(|rule| rule.as_object());
        let first_deny_all = first.is_some_and(|rule| {
            rule.get("effect").and_then(Value::as_str) == Some("deny")
                && rule.get("resource").and_then(Value::as_str) == Some("*")
        });
        if first_deny_all
            && allow.clone().all(|rule| {
                rule.get("effect").and_then(Value::as_str) == Some("allow")
                    && rule.get("resource").and_then(Value::as_str).is_some()
            })
        {
            (
                None,
                Some(
                    allow
                        .filter_map(|rule| {
                            rule.get("resource")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .collect::<Vec<_>>(),
                ),
            )
        } else {
            experimental.insert("policies".to_string(), Value::Array(policies));
            return (None, None);
        }
    };
    if experimental.get("policies").is_none() && experimental.is_empty() {
        root.remove("experimental");
    }
    (disabled, enabled)
}

pub fn v2_to_v1_value(mut value: Value) -> Result<Value, String> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| "OpenCode config root must be an object".to_string())?;

    if root.contains_key("provider") && root.contains_key("providers") {
        return Err("OpenCode config contains both V1 `provider` and V2 `providers`".to_string());
    }
    if root.contains_key("plugin") && root.contains_key("plugins") {
        return Err("OpenCode config contains both V1 `plugin` and V2 `plugins`".to_string());
    }
    if root.contains_key("agent") && root.contains_key("agents") {
        return Err("OpenCode config contains both V1 `agent` and V2 `agents`".to_string());
    }

    if let Some(providers) = root.remove("providers") {
        root.insert(
            "provider".to_string(),
            convert_providers_v2_to_v1(providers)?,
        );
    }
    if let Some(plugins) = root.remove("plugins") {
        root.insert("plugin".to_string(), convert_plugin_list_v2_to_v1(plugins)?);
    }
    convert_agents_v2_to_v1(root)?;
    convert_mcp_v2_to_v1(root)?;

    let (disabled, enabled) = take_provider_selection_policies(root);
    if let Some(disabled) = disabled {
        if root.get("disabled_providers").is_none() {
            root.insert(
                "disabled_providers".to_string(),
                Value::Array(disabled.into_iter().map(Value::String).collect()),
            );
        }
    }
    if let Some(enabled) = enabled {
        if root.get("enabled_providers").is_none() {
            root.insert(
                "enabled_providers".to_string(),
                Value::Array(enabled.into_iter().map(Value::String).collect()),
            );
        }
    }

    for key in ["disabled_providers", "enabled_providers"] {
        if let Some(provider_ids) = root.get_mut(key) {
            canonicalize_provider_list(provider_ids);
        }
    }
    Ok(value)
}

pub fn parse_v1_as_v2_json(content: &str) -> Result<String, String> {
    let value = json5::from_str::<Value>(content)
        .map_err(|error| format!("Failed to parse OpenCode V1 config: {error}"))?;
    let migrated = v1_to_v2_value(value)?;
    serde_json::to_string_pretty(&migrated)
        .map_err(|error| format!("Failed to serialize OpenCode V2 config: {error}"))
}

fn archived_v2_path(config_path: &Path, attempt: usize) -> PathBuf {
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let extension = config_path.extension().and_then(|value| value.to_str());
    let suffix = if attempt == 0 {
        chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
    } else {
        format!(
            "{}_{}",
            chrono::Local::now().format("%Y%m%d_%H%M%S"),
            attempt
        )
    };
    let file_name = match extension {
        Some(extension) if !extension.is_empty() => format!("opencode_v2.{suffix}.{extension}"),
        _ => format!("opencode_v2.{suffix}"),
    };
    base.join(file_name)
}

fn unique_archived_v2_path(config_path: &Path) -> PathBuf {
    (0..1000)
        .map(|attempt| archived_v2_path(config_path, attempt))
        .find(|path| !path.exists())
        .unwrap_or_else(|| archived_v2_path(config_path, 1000))
}

pub fn set_mode(config_path: &Path, enabled: bool) -> Result<bool, String> {
    let backup_v1 = v1_backup_path(config_path);
    let backup_v2 = v2_backup_path(config_path);
    let config_path_text = config_path.to_string_lossy();
    if config_path_text.eq_ignore_ascii_case(&backup_v1.to_string_lossy())
        || config_path_text.eq_ignore_ascii_case(&backup_v2.to_string_lossy())
    {
        return Err(
            "OpenCode config path conflicts with a reserved migration backup name".to_string(),
        );
    }

    if enabled {
        if backup_v1.exists() {
            return Ok(true);
        }
        if !config_path.exists() {
            return Err(format!(
                "OpenCode config file does not exist: {}",
                config_path.display()
            ));
        }
        let content = fs::read_to_string(config_path)
            .map_err(|error| format!("Failed to read OpenCode config: {error}"))?;
        let migrated = parse_v1_as_v2_json(&content)?;
        let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("Failed to prepare migrated OpenCode config: {error}"))?;
        temporary
            .write_all(migrated.as_bytes())
            .map_err(|error| format!("Failed to write migrated OpenCode config: {error}"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| format!("Failed to flush migrated OpenCode config: {error}"))?;

        fs::rename(config_path, &backup_v1).map_err(|error| {
            format!(
                "Failed to back up V1 config to {}: {error}",
                backup_v1.display()
            )
        })?;
        if let Err(error) = temporary.persist(config_path) {
            let restore_error = fs::rename(&backup_v1, config_path).err();
            let mut detail = format!("Failed to install migrated V2 config: {}", error.error);
            if let Some(restore_error) = restore_error {
                detail.push_str(&format!(
                    "; also failed to restore V1 config: {restore_error}"
                ));
            }
            return Err(detail);
        }
        return Ok(true);
    }

    if !backup_v1.exists() {
        return Ok(false);
    }

    let mut archived_previous_v2 = None;
    let has_active_config = config_path.exists();
    if has_active_config && backup_v2.exists() {
        let archive_path = unique_archived_v2_path(config_path);
        fs::rename(&backup_v2, &archive_path).map_err(|error| {
            format!(
                "Failed to archive previous V2 backup {}: {error}",
                backup_v2.display()
            )
        })?;
        archived_previous_v2 = Some(archive_path);
    }

    if has_active_config {
        if let Err(error) = fs::rename(config_path, &backup_v2) {
            if let Some(archive_path) = archived_previous_v2.as_ref() {
                let _ = fs::rename(archive_path, &backup_v2);
            }
            return Err(format!(
                "Failed to save V2 config to {}: {error}",
                backup_v2.display()
            ));
        }
    }

    if let Err(error) = fs::rename(&backup_v1, config_path) {
        let rollback_v2_error = if has_active_config {
            fs::rename(&backup_v2, config_path).err()
        } else {
            None
        };
        let rollback_archive_error = archived_previous_v2
            .as_ref()
            .and_then(|archive_path| fs::rename(archive_path, &backup_v2).err());
        let mut detail = format!(
            "Failed to restore V1 config from {}: {error}",
            backup_v1.display()
        );
        if let Some(rollback_error) = rollback_v2_error {
            detail.push_str(&format!(
                "; also failed to restore active V2 config: {rollback_error}"
            ));
        }
        if let Some(rollback_error) = rollback_archive_error {
            detail.push_str(&format!(
                "; also failed to restore older V2 backup: {rollback_error}"
            ));
        }
        return Err(detail);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_provider_migrates_to_official_v2_provider_and_model_shapes() {
        let value = json!({
            "provider": {
                "relay": {
                    "name": "Relay",
                    "npm": "@ai-sdk/openai-compatible",
                    "api": "https://relay.example.com/v1",
                    "options": {
                        "apiKey": "sk-test",
                        "baseURL": "https://relay.example.com/v1",
                        "headers": { "X-Test": "1" }
                    },
                    "models": {
                        "gpt-x": {
                            "id": "gpt-x-upstream",
                            "name": "GPT X",
                            "modalities": { "input": ["text"], "output": ["text"] },
                            "tool_call": true,
                            "cost": { "input": 1, "output": 2, "cache_read": 0.1, "cache_write": 0.2 },
                            "limit": { "context": 1000.5, "output": 200 },
                            "options": { "reasoningEffort": "high" },
                            "variants": { "low": { "reasoningEffort": "low" } }
                        }
                    }
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();
        let provider = &migrated["providers"]["relay"];

        // Official provider V2 shape: `api` carries package/url/settings, and
        // the request carries headers/body. The old top-level `api` string and
        // `options` object must not survive.
        assert_eq!(
            provider["api"],
            json!({
                "type": "aisdk",
                "package": "@ai-sdk/openai-compatible",
                "url": "https://relay.example.com/v1",
                "settings": { "apiKey": "sk-test" }
            })
        );
        assert_eq!(provider["request"], json!({ "headers": { "X-Test": "1" } }));
        assert!(provider.get("npm").is_none());
        assert!(provider.get("options").is_none());

        // Released 2.0 shape written alongside it: flat package plus overlays,
        // and the base URL inside `settings` because that shape has no `api.url`.
        // OpenCode 2.0 rejects a provider whose package it cannot resolve, so a
        // file that only carried the preview shape would fail to load at all.
        assert_eq!(provider["package"], "@ai-sdk/openai-compatible");
        assert_eq!(
            provider["settings"],
            json!({ "apiKey": "sk-test", "baseURL": "https://relay.example.com/v1" })
        );
        assert_eq!(provider["headers"], json!({ "X-Test": "1" }));

        let model = &provider["models"]["gpt-x"];
        // The official migration only puts a model package on `api` when the
        // model itself declares a provider override; otherwise `api.id` is
        // enough because the provider catalog supplies the package.
        assert_eq!(model["api"], json!({ "id": "gpt-x-upstream" }));
        assert_eq!(model["modelID"], "gpt-x-upstream");
        assert_eq!(model["settings"], json!({ "reasoningEffort": "high" }));
        assert_eq!(
            model["capabilities"],
            json!({ "tools": true, "input": ["text"], "output": ["text"] })
        );
        assert_eq!(
            model["request"]["body"],
            json!({ "reasoning_effort": "high" })
        );
        assert_eq!(
            model["variants"],
            json!([{ "id": "low", "body": { "reasoning_effort": "low" } }])
        );
        assert_eq!(
            model["cost"],
            json!([{
                "input": 1,
                "output": 2,
                "cache": { "read": 0.1, "write": 0.2 }
            }])
        );
        assert_eq!(model["limit"], json!({ "context": 1000, "output": 200 }));
    }

    #[test]
    fn v1_agent_mode_and_small_model_migrate_to_v2_agents() {
        let value = json!({
            "agent": {
                "build": {
                    "prompt": "You build.",
                    "model": "anthropic/claude-sonnet",
                    "options": { "temperature": 0.2 },
                    "permission": { "bash": "ask" }
                }
            },
            "mode": {
                "plan": { "prompt": "You plan." }
            },
            "small_model": "relay/gpt-x"
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert!(migrated.get("agent").is_none());
        assert!(migrated.get("mode").is_none());
        assert!(migrated.get("small_model").is_none());
        assert_eq!(migrated["agents"]["title"]["model"], json!("relay/gpt-x"));
        assert_eq!(migrated["agents"]["build"]["system"], json!("You build."));
        assert_eq!(
            migrated["agents"]["build"]["model"],
            json!("anthropic/claude-sonnet")
        );
        assert_eq!(
            migrated["agents"]["build"]["request"]["body"],
            json!({ "temperature": 0.2 })
        );
        assert_eq!(
            migrated["agents"]["build"]["permissions"],
            json!([{ "action": "bash", "resource": "*", "effect": "ask" }])
        );
        assert_eq!(migrated["agents"]["plan"]["mode"], json!("primary"));
    }

    #[test]
    fn v1_mcp_nests_oauth_and_uses_request_timeout() {
        let value = json!({
            "mcp": {
                "remote": {
                    "type": "remote",
                    "url": "https://oauth.example.com/mcp",
                    "oauth": {
                        "clientId": "client",
                        "clientSecret": "secret",
                        "callbackPort": 19877
                    },
                    "enabled": false,
                    "timeout": 30
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert_eq!(
            migrated["mcp"]["servers"]["remote"]["oauth"],
            json!({ "client_id": "client", "client_secret": "secret", "callback_port": 19877 })
        );
        assert_eq!(
            migrated["mcp"]["servers"]["remote"]["disabled"],
            json!(true)
        );
        assert_eq!(
            migrated["mcp"]["servers"]["remote"]["timeout"],
            json!({ "request": 30 })
        );
        assert!(migrated["mcp"]["servers"]["remote"]
            .get("enabled")
            .is_none());
    }

    #[test]
    fn v1_top_level_names_and_skills_migrate() {
        let value = json!({
            "snapshot": true,
            "attachment": { "image": { "auto_resize": true } },
            "command": { "deploy": { "template": "deploy" } },
            "reference": { "sdk": { "repository": "github.com/example/sdk" } },
            "skills": { "paths": ["./skills"], "urls": ["https://example.com/skills"] },
            "autoshare": true,
            "permission": { "bash": { "git status": "allow" } },
            "tools": { "write": false }
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert_eq!(migrated["snapshots"], json!(true));
        assert_eq!(migrated["attachments"]["image"]["auto_resize"], json!(true));
        assert_eq!(migrated["commands"]["deploy"]["template"], json!("deploy"));
        assert_eq!(
            migrated["references"]["sdk"]["repository"],
            json!("github.com/example/sdk")
        );
        assert_eq!(
            migrated["skills"],
            json!(["./skills", "https://example.com/skills"])
        );
        assert_eq!(migrated["share"], json!("auto"));
        assert!(migrated.get("snapshot").is_none());
        assert!(migrated.get("attachment").is_none());
        assert!(migrated.get("command").is_none());
        assert!(migrated.get("reference").is_none());
        assert!(migrated.get("autoshare").is_none());
        assert!(migrated.get("permission").is_none());
        assert!(migrated.get("tools").is_none());
        assert_eq!(
            migrated["permissions"],
            json!([
                { "action": "bash", "resource": "git status", "effect": "allow" },
                { "action": "edit", "resource": "*", "effect": "deny" }
            ])
        );
    }

    #[test]
    fn disabled_and_enabled_providers_round_trip_through_policies() {
        let value = json!({
            "disabled_providers": ["provider-a", "provider-b"],
            "provider": {}
        });

        let migrated = v1_to_v2_value(value).unwrap();
        assert_eq!(
            migrated["experimental"]["policies"],
            json!([
                { "effect": "deny", "action": "provider.use", "resource": "provider-a" },
                { "effect": "deny", "action": "provider.use", "resource": "provider-b" }
            ])
        );

        let restored = v2_to_v1_value(migrated).unwrap();
        assert_eq!(
            restored["disabled_providers"],
            json!(["provider-a", "provider-b"])
        );
        assert!(restored.get("experimental").is_none());
    }

    /// Anthropic providers lower their key into an `x-api-key` header instead of
    /// `settings`, so the flat shape has to carry the header overlay for the
    /// released reader to authenticate at all.
    #[test]
    fn v1_anthropic_provider_writes_the_flat_shape_with_its_auth_header() {
        let v1 = json!({
            "provider": {
                "anthropic-relay": {
                    "name": "Anthropic Relay",
                    "npm": "@ai-sdk/anthropic",
                    "options": { "apiKey": "sk-ant-test", "baseURL": "https://relay.example.com" },
                    "models": { "claude-x": { "id": "claude-x-20260101", "name": "Claude X" } }
                }
            }
        });
        let written = v1_to_v2_value(v1).unwrap();
        let provider = &written["providers"]["anthropic-relay"];
        assert_eq!(provider["package"], "@ai-sdk/anthropic");
        assert_eq!(
            provider["settings"],
            json!({ "baseURL": "https://relay.example.com" })
        );
        assert_eq!(provider["headers"], json!({ "x-api-key": "sk-ant-test" }));
        assert_eq!(
            provider["models"]["claude-x"]["modelID"],
            "claude-x-20260101"
        );
        assert_eq!(
            written["providers"]["anthropic-relay"]["api"]["url"],
            "https://relay.example.com"
        );
    }

    /// A file written by OpenCode 2.0 itself carries the flat shape only, so the
    /// editor has to restore it without the preview `api`/`request` keys.
    #[test]
    fn released_v2_flat_provider_restores_the_v1_editor_shape() {
        let value = json!({
            "providers": {
                "relay": {
                    "name": "Relay",
                    "package": "@ai-sdk/openai-compatible",
                    "settings": {
                        "apiKey": "sk-test",
                        "baseURL": "https://relay.example.com/v1"
                    },
                    "headers": { "X-Test": "1" },
                    "body": { "reasoning_effort": "high" },
                    "models": {
                        "gpt-x": {
                            "name": "GPT X",
                            "modelID": "gpt-x-upstream",
                            "package": "@ai-sdk/openai-compatible",
                            "settings": { "reasoningEffort": "high" },
                            "headers": { "X-Model": "2" }
                        }
                    }
                }
            }
        });

        let restored = v2_to_v1_value(value).unwrap();
        let provider = &restored["provider"]["relay"];
        assert_eq!(provider["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(provider["api"], "https://relay.example.com/v1");
        assert_eq!(
            provider["options"]["baseURL"],
            "https://relay.example.com/v1"
        );
        assert_eq!(provider["options"]["apiKey"], "sk-test");
        assert_eq!(provider["options"]["headers"], json!({ "X-Test": "1" }));
        assert_eq!(
            provider["options"]["body"],
            json!({ "reasoning_effort": "high" })
        );
        // The flat keys are consumed, not echoed next to the preview shape.
        assert!(provider.get("package").is_none());
        assert!(provider.get("settings").is_none());
        assert!(provider.get("headers").is_none());
        assert!(provider.get("body").is_none());

        let model = &provider["models"]["gpt-x"];
        assert_eq!(model["id"], "gpt-x-upstream");
        assert_eq!(model["provider"]["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(model["options"]["reasoningEffort"], "high");
        assert_eq!(model["headers"], json!({ "X-Model": "2" }));
        assert!(model.get("modelID").is_none());
        assert!(model.get("settings").is_none());
        assert!(model.get("package").is_none());
    }

    /// The dual-shape file this editor writes must survive a full round trip:
    /// reading it back and saving again cannot drop or duplicate either shape.
    #[test]
    fn dual_shape_provider_survives_a_read_write_round_trip() {
        let v1 = json!({
            "provider": {
                "relay": {
                    "name": "Relay",
                    "npm": "@ai-sdk/openai-compatible",
                    "api": "https://relay.example.com/v1",
                    "options": { "apiKey": "sk-test", "headers": { "X-Test": "1" } },
                    "models": { "gpt-x": { "id": "gpt-x-upstream", "name": "GPT X" } }
                }
            }
        });
        let written = v1_to_v2_value(v1.clone()).unwrap();
        let restored = v2_to_v1_value(written.clone()).unwrap();
        let again = v1_to_v2_value(restored).unwrap();
        assert_eq!(again["providers"]["relay"], written["providers"]["relay"]);
        assert_eq!(
            again["providers"]["relay"]["settings"]["baseURL"],
            "https://relay.example.com/v1"
        );
        assert_eq!(
            again["providers"]["relay"]["api"]["url"],
            "https://relay.example.com/v1"
        );
    }

    #[test]
    fn v2_provider_agent_and_mcp_restore_v1_editor_shapes() {
        let value = json!({
            "providers": {
                "relay": {
                    "name": "Relay",
                    "api": {
                        "type": "aisdk",
                        "package": "@ai-sdk/openai-compatible",
                        "url": "https://relay.example.com/v1",
                        "settings": { "apiKey": "sk-test" }
                    },
                    "request": { "headers": { "X-Test": "1" } },
                    "models": {
                        "gpt-x": {
                            "api": { "id": "gpt-x-upstream" },
                            "capabilities": { "tools": true, "input": ["text"], "output": ["text"] },
                            "request": { "body": { "reasoning_effort": "high" } },
                            "variants": [{ "id": "low", "body": { "reasoning_effort": "low" } }],
                            "cost": [{ "input": 1, "output": 2, "cache": { "read": 0.1, "write": 0.2 } }],
                            "limit": { "context": 1000, "output": 200 }
                        }
                    }
                }
            },
            "agents": {
                "title": { "model": "relay/gpt-x" },
                "build": {
                    "model": { "providerID": "anthropic", "model": "claude-sonnet", "variant": "thinking" },
                    "system": "You build.",
                    "disabled": true,
                    "request": { "body": { "temperature": 0.2 } },
                    "permissions": [{ "action": "bash", "resource": "*", "effect": "ask" }]
                }
            },
            "mcp": {
                "servers": {
                    "remote": {
                        "type": "remote",
                        "url": "https://oauth.example.com/mcp",
                        "oauth": { "client_id": "client", "client_secret": "secret", "callback_port": 19877 },
                        "disabled": false,
                        "timeout": { "request": 30 }
                    }
                }
            }
        });

        let restored = v2_to_v1_value(value).unwrap();

        assert_eq!(
            restored["provider"]["relay"]["npm"],
            json!("@ai-sdk/openai-compatible")
        );
        assert_eq!(
            restored["provider"]["relay"]["api"],
            json!("https://relay.example.com/v1")
        );
        assert_eq!(
            restored["provider"]["relay"]["options"]["apiKey"],
            json!("sk-test")
        );
        assert_eq!(
            restored["provider"]["relay"]["options"]["headers"],
            json!({ "X-Test": "1" })
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["id"],
            json!("gpt-x-upstream")
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["tool_call"],
            json!(true)
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["options"]["reasoning_effort"],
            json!("high")
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["variants"]["low"],
            json!({ "reasoning_effort": "low" })
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["cost"]["cache_read"],
            json!(0.1)
        );
        assert_eq!(restored["small_model"], json!("relay/gpt-x"));
        assert_eq!(restored["agent"]["build"]["prompt"], json!("You build."));
        assert_eq!(
            restored["agent"]["build"]["model"],
            json!("anthropic/claude-sonnet")
        );
        assert_eq!(restored["agent"]["build"]["variant"], json!("thinking"));
        assert_eq!(restored["agent"]["build"]["disable"], json!(true));
        assert_eq!(restored["mcp"]["remote"]["enabled"], json!(true));
        assert_eq!(restored["mcp"]["remote"]["timeout"], json!(30));
        assert_eq!(
            restored["mcp"]["remote"]["oauth"],
            json!({ "clientId": "client", "clientSecret": "secret", "callbackPort": 19877 })
        );
    }

    #[test]
    fn set_mode_restores_the_original_v1_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("opencode.json");
        let original = br#"{"provider":{"relay":{"npm":"@ai-sdk/openai-compatible","models":{}}}}"#;
        fs::write(&config_path, original).unwrap();

        assert!(set_mode(&config_path, true).unwrap());
        let migrated = fs::read_to_string(&config_path).unwrap();
        assert!(migrated.contains("\"providers\""));
        assert!(v1_backup_path(&config_path).is_file());

        assert!(!set_mode(&config_path, false).unwrap());
        assert_eq!(fs::read(&config_path).unwrap(), original);
    }

    #[test]
    fn explicit_provider_header_overrides_generated_auth_header() {
        let value = json!({
            "provider": {
                "relay": {
                    "npm": "@ai-sdk/openai",
                    "options": {
                        "apiKey": "sk-test",
                        "headers": { "Authorization": "Custom" }
                    },
                    "models": {}
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert_eq!(
            migrated["providers"]["relay"]["request"]["headers"]["Authorization"],
            json!("Custom")
        );
    }
}
