use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn sibling_path(config_path: &Path, stem: &str) -> PathBuf {
    let file_name = match config_path.extension().and_then(|extension| extension.to_str()) {
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

fn move_key(map: &mut Map<String, Value>, from: &str, to: &str) -> Result<(), String> {
    let Some(value) = map.remove(from) else {
        return Ok(());
    };

    if let Some(existing) = map.get(to) {
        if existing != &value {
            return Err(format!("Cannot migrate both `{from}` and `{to}` because their values differ"));
        }
        return Ok(());
    }

    map.insert(to.to_string(), value);
    Ok(())
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

fn take_object(map: &mut Map<String, Value>, key: &str) -> Result<Map<String, Value>, String> {
    match map.remove(key) {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(object)) => Ok(object),
        Some(_) => Err(format!("Expected `{key}` to be an object while migrating OpenCode config")),
    }
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
        let canonical_reference = format!("{canonical_id}/{model_id}");
        *value = Value::String(canonical_reference);
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
                let canonical_id = canonical_id.to_string();
                *provider_id = Value::String(canonical_id);
            }
        }
    }
}

fn merge_request_fields(
    target: &mut Map<String, Value>,
    source: &mut Map<String, Value>,
) -> Result<(), String> {
    for field in ["headers", "body"] {
        let values = take_object(source, field)?;
        merge_object_value(target, field, values)?;
    }
    Ok(())
}

fn convert_plugin_list_v1_to_v2(value: Value) -> Result<Value, String> {
    let plugins = value
        .as_array()
        .ok_or_else(|| "Expected V1 `plugin` to be an array".to_string())?;
    let mut converted = Vec::with_capacity(plugins.len());
    for plugin in plugins {
        match plugin {
            Value::String(_) => converted.push(plugin.clone()),
            Value::Array(entry) if entry.len() == 2 => {
                let package = entry[0]
                    .as_str()
                    .ok_or_else(|| "Expected V1 plugin tuple package to be a string".to_string())?;
                let options = entry[1]
                    .as_object()
                    .ok_or_else(|| "Expected V1 plugin tuple options to be an object".to_string())?;
                converted.push(serde_json::json!({ "package": package, "options": options }));
            }
            other => return Err(format!("Unsupported V1 plugin entry: {other}")),
        }
    }
    Ok(Value::Array(converted))
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
                let package = entry
                    .get("package")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Expected V2 plugin entry to have a string `package`".to_string())?;
                let options = entry
                    .get("options")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                converted.push(Value::Array(vec![
                    Value::String(package.to_string()),
                    Value::Object(options),
                ]));
            }
            other => return Err(format!("Unsupported V2 plugin entry: {other}")),
        }
    }
    Ok(Value::Array(converted))
}

fn convert_variant_map_to_array(value: Value) -> Result<Value, String> {
    let variants = value
        .as_object()
        .ok_or_else(|| "Expected V1 model variants to be an object".to_string())?;
    let mut result = Vec::with_capacity(variants.len());

    for (id, value) in variants {
        let mut variant = value
            .as_object()
            .cloned()
            .ok_or_else(|| format!("Expected V1 variant `{id}` to be an object"))?;
        let mut converted = Map::new();
        converted.insert("id".to_string(), Value::String(id.clone()));
        let mut settings = take_object(&mut variant, "settings")?;
        merge_request_fields(&mut converted, &mut variant)?;
        for (key, value) in variant {
            if let Some(existing) = settings.get(&key) {
                if existing != &value {
                    return Err(format!("Conflicting variant `{id}` setting `{key}`"));
                }
            } else {
                settings.insert(key, value);
            }
        }
        if !settings.is_empty() {
            converted.insert("settings".to_string(), Value::Object(settings));
        }
        result.push(Value::Object(converted));
    }

    Ok(Value::Array(result))
}

fn convert_variant_array_to_map(value: Value) -> Result<Value, String> {
    let variants = value
        .as_array()
        .ok_or_else(|| "Expected V2 model variants to be an array".to_string())?;
    let mut result = Map::new();

    for value in variants {
        let mut variant = value
            .as_object()
            .cloned()
            .ok_or_else(|| "Expected each V2 model variant to be an object".to_string())?;
        let id = variant
            .remove("id")
            .and_then(|id| id.as_str().map(str::to_string))
            .ok_or_else(|| "Expected each V2 model variant to have a string `id`".to_string())?;
        let mut settings = take_object(&mut variant, "settings")?;
        for (key, value) in variant {
            if let Some(existing) = settings.get(&key) {
                if existing != &value {
                    return Err(format!("Conflicting V2 variant `{id}` field `{key}`"));
                }
            } else {
                settings.insert(key, value);
            }
        }
        if result.insert(id.clone(), Value::Object(settings)).is_some() {
            return Err(format!("Duplicate V2 model variant id `{id}`"));
        }
    }

    Ok(Value::Object(result))
}

fn convert_model_v1_to_v2(value: &mut Value) -> Result<(), String> {
    let Some(model) = value.as_object_mut() else {
        return Ok(());
    };

    move_key(model, "id", "modelID")?;
    let mut options = take_object(model, "options")?;
    let mut settings = take_object(model, "settings")?;
    let mut headers = take_object(model, "headers")?;
    let mut body = take_object(model, "body")?;
    let option_headers = take_object(&mut options, "headers")?;
    let option_body = take_object(&mut options, "body")?;
    for (key, value) in option_headers {
        if let Some(existing) = headers.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting model header `{key}`"));
            }
        } else {
            headers.insert(key, value);
        }
    }
    for (key, value) in option_body {
        if let Some(existing) = body.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting model body field `{key}`"));
            }
        } else {
            body.insert(key, value);
        }
    }
    for (key, value) in options {
        if let Some(existing) = settings.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting model setting `{key}`"));
            }
        } else {
            settings.insert(key, value);
        }
    }
    if !settings.is_empty() {
        model.insert("settings".to_string(), Value::Object(settings));
    }
    if !headers.is_empty() {
        model.insert("headers".to_string(), Value::Object(headers));
    }
    if !body.is_empty() {
        model.insert("body".to_string(), Value::Object(body));
    }

    if let Some(modalities) = model.remove("modalities") {
        let modalities = modalities
            .as_object()
            .ok_or_else(|| "Expected V1 model `modalities` to be an object".to_string())?;
        let mut capabilities = take_object(model, "capabilities")?;
        for field in ["input", "output"] {
            if let Some(value) = modalities.get(field) {
                if let Some(existing) = capabilities.get(field) {
                    if existing != value {
                        return Err(format!("Conflicting model capability `{field}`"));
                    }
                } else {
                    capabilities.insert(field.to_string(), value.clone());
                }
            }
        }
        model.insert("capabilities".to_string(), Value::Object(capabilities));
    }

    if let Some(tool_call) = model.remove("tool_call") {
        let capabilities = object_at_mut(model, "capabilities")?;
        if let Some(existing) = capabilities.get("tools") {
            if existing != &tool_call {
                return Err("Conflicting model capability `tools`".to_string());
            }
        } else {
            capabilities.insert("tools".to_string(), tool_call);
        }
    }

    if let Some(Value::Object(cost)) = model.get_mut("cost") {
        let mut cache = take_object(cost, "cache")?;
        for (old_key, new_key) in [("cache_read", "read"), ("cache_write", "write")] {
            if let Some(value) = cost.remove(old_key) {
                if let Some(existing) = cache.get(new_key) {
                    if existing != &value {
                        return Err(format!("Conflicting model cost `cache.{new_key}`"));
                    }
                } else {
                    cache.insert(new_key.to_string(), value);
                }
            }
        }
        if !cache.is_empty() {
            cost.insert("cache".to_string(), Value::Object(cache));
        }
    }

    if let Some(variants) = model.remove("variants") {
        let variants = match variants {
            Value::Object(_) => convert_variant_map_to_array(variants)?,
            Value::Array(_) => variants,
            other => return Err(format!("Unsupported V1 model variants shape: {other}")),
        };
        model.insert("variants".to_string(), variants);
    }

    if model.get("status").and_then(Value::as_str) == Some("deprecated") {
        model.remove("status");
        model.insert("disabled".to_string(), Value::Bool(true));
    } else {
        model.remove("status");
    }

    for unsupported in ["release_date", "attachment", "reasoning", "temperature", "experimental", "interleaved"] {
        model.remove(unsupported);
    }

    Ok(())
}

fn convert_model_v2_to_v1(value: &mut Value) -> Result<(), String> {
    let Some(model) = value.as_object_mut() else {
        return Ok(());
    };

    move_key(model, "modelID", "id")?;
    let mut options = take_object(model, "options")?;
    let settings = take_object(model, "settings")?;
    let headers = take_object(model, "headers")?;
    let body = take_object(model, "body")?;
    for (key, value) in settings {
        if let Some(existing) = options.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting V2 model option `{key}`"));
            }
        } else {
            options.insert(key, value);
        }
    }
    if !headers.is_empty() {
        options.insert("headers".to_string(), Value::Object(headers));
    }
    if !body.is_empty() {
        options.insert("body".to_string(), Value::Object(body));
    }
    if !options.is_empty() {
        model.insert("options".to_string(), Value::Object(options));
    }

    if let Some(capabilities) = model.remove("capabilities") {
        let capabilities = capabilities
            .as_object()
            .ok_or_else(|| "Expected V2 model `capabilities` to be an object".to_string())?;
        let mut modalities = Map::new();
        for field in ["input", "output"] {
            if let Some(value) = capabilities.get(field) {
                modalities.insert(field.to_string(), value.clone());
            }
        }
        if !modalities.is_empty() {
            model.insert("modalities".to_string(), Value::Object(modalities));
        }
        if let Some(tools) = capabilities.get("tools") {
            model.insert("tool_call".to_string(), tools.clone());
        }
        let remaining: Map<String, Value> = capabilities
            .iter()
            .filter(|(key, _)| !["input", "output", "tools"].contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if !remaining.is_empty() {
            model.insert("capabilities".to_string(), Value::Object(remaining));
        }
    }

    if let Some(Value::Object(cost)) = model.get_mut("cost") {
        let cache = take_object(cost, "cache")?;
        if !cache.is_empty() {
            for (new_key, old_key) in [("read", "cache_read"), ("write", "cache_write")] {
                if let Some(value) = cache.get(new_key) {
                    cost.insert(old_key.to_string(), value.clone());
                }
            }
            let remaining: Map<String, Value> = cache
                .into_iter()
                .filter(|(key, _)| !["read", "write"].contains(&key.as_str()))
                .collect();
            if !remaining.is_empty() {
                cost.insert("cache".to_string(), Value::Object(remaining));
            }
        }
    }

    if let Some(variants) = model.remove("variants") {
        let variants = match variants {
            Value::Array(_) => convert_variant_array_to_map(variants)?,
            Value::Object(_) => variants,
            other => return Err(format!("Unsupported V2 model variants shape: {other}")),
        };
        model.insert("variants".to_string(), variants);
    }

    Ok(())
}

fn merge_provider_options(provider: &mut Map<String, Value>) -> Result<(), String> {
    let mut options = take_object(provider, "options")?;
    let mut settings = take_object(provider, "settings")?;
    let mut headers = take_object(provider, "headers")?;
    let mut body = take_object(provider, "body")?;

    let option_headers = take_object(&mut options, "headers")?;
    for (key, value) in option_headers {
        if let Some(existing) = headers.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting provider header `{key}`"));
            }
        } else {
            headers.insert(key, value);
        }
    }
    let option_body = take_object(&mut options, "body")?;
    for (key, value) in option_body {
        if let Some(existing) = body.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting provider body field `{key}`"));
            }
        } else {
            body.insert(key, value);
        }
    }

    for (key, value) in options {
        if let Some(existing) = settings.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting provider setting `{key}`"));
            }
        } else {
            settings.insert(key, value);
        }
    }

    if let Some(api) = provider.remove("api") {
        if let Some(existing) = settings.get("baseURL") {
            if existing != &api {
                return Err("Conflicting provider endpoint values in `api` and `settings.baseURL`".to_string());
            }
        } else {
            settings.insert("baseURL".to_string(), api);
        }
    }

    if !settings.is_empty() {
        provider.insert("settings".to_string(), Value::Object(settings));
    }
    if !headers.is_empty() {
        provider.insert("headers".to_string(), Value::Object(headers));
    }
    if !body.is_empty() {
        provider.insert("body".to_string(), Value::Object(body));
    }
    Ok(())
}

fn convert_provider_v1_to_v2(value: &mut Value) -> Result<(), String> {
    let Some(provider) = value.as_object_mut() else {
        return Err("Expected OpenCode provider entries to be objects".to_string());
    };

    if let Some(npm) = provider.remove("npm") {
        if let Some(npm) = npm.as_str() {
            if !provider.contains_key("package") {
                provider.insert("package".to_string(), Value::String(format!("aisdk:{npm}")));
            }
        } else {
            return Err("Expected V1 provider `npm` to be a string".to_string());
        }
    }

    merge_provider_options(provider)?;

    if let Some(Value::Object(models)) = provider.get_mut("models") {
        for model in models.values_mut() {
            convert_model_v1_to_v2(model)?;
        }
    }
    Ok(())
}

fn merge_v2_settings_into_options(provider: &mut Map<String, Value>) -> Result<(), String> {
    let mut options = take_object(provider, "options")?;
    let mut settings = take_object(provider, "settings")?;
    let headers = take_object(provider, "headers")?;
    let body = take_object(provider, "body")?;

    if let Some(base_url) = settings.remove("baseURL") {
        if let Some(existing) = provider.get("api") {
            if existing != &base_url {
                return Err("Conflicting V2 provider endpoint values in `api` and `settings.baseURL`".to_string());
            }
        } else {
            provider.insert("api".to_string(), base_url);
        }
    }
    for (key, value) in settings {
        if let Some(existing) = options.get(&key) {
            if existing != &value {
                return Err(format!("Conflicting provider option `{key}`"));
            }
        } else {
            options.insert(key, value);
        }
    }
    if !headers.is_empty() {
        options.insert("headers".to_string(), Value::Object(headers));
    }
    if !body.is_empty() {
        options.insert("body".to_string(), Value::Object(body));
    }
    if !options.is_empty() {
        provider.insert("options".to_string(), Value::Object(options));
    }

    if let Some(Value::String(package)) = provider.get("package") {
        if let Some(npm) = package.strip_prefix("aisdk:") {
            provider.insert("npm".to_string(), Value::String(npm.to_string()));
            provider.remove("package");
        }
    }

    if let Some(Value::Object(models)) = provider.get_mut("models") {
        for model in models.values_mut() {
            convert_model_v2_to_v1(model)?;
        }
    }
    Ok(())
}

fn convert_provider_v1_to_v2_map(value: Value) -> Result<Value, String> {
    let providers = value
        .as_object()
        .ok_or_else(|| "Expected V1 `provider` to be an object".to_string())?;
    let mut result = Map::new();

    for (provider_id, provider) in providers {
        let canonical_id = canonical_provider_id(provider_id);
        let mut provider = provider.clone();
        convert_provider_v1_to_v2(&mut provider)?;
        if result.insert(canonical_id.to_string(), provider).is_some() {
            return Err(format!("Provider ID collision while migrating `{provider_id}` to `{canonical_id}`"));
        }
    }
    Ok(Value::Object(result))
}

fn convert_provider_v2_to_v1_map(value: Value) -> Result<Value, String> {
    let providers = value
        .as_object()
        .ok_or_else(|| "Expected V2 `providers` to be an object".to_string())?;
    let mut result = Map::new();
    for (provider_id, provider) in providers {
        let mut provider = provider.clone();
        let Some(provider_object) = provider.as_object_mut() else {
            return Err(format!("Expected V2 provider `{provider_id}` to be an object"));
        };
        merge_v2_settings_into_options(provider_object)?;
        result.insert(provider_id.clone(), provider);
    }
    Ok(Value::Object(result))
}

fn convert_server_v1_to_v2(value: &mut Value) -> Result<(), String> {
    let Some(server) = value.as_object_mut() else {
        return Ok(());
    };
    if let Some(enabled) = server.remove("enabled") {
        if let Some(enabled) = enabled.as_bool() {
            let disabled = Value::Bool(!enabled);
            if let Some(existing) = server.get("disabled") {
                if existing != &disabled {
                    return Err("Conflicting MCP server `enabled` and `disabled` values".to_string());
                }
            } else {
                server.insert("disabled".to_string(), disabled);
            }
        } else {
            return Err("Expected V1 MCP server `enabled` to be a boolean".to_string());
        }
    }
    let timeout = server.get("timeout").and_then(Value::as_u64);
    if let Some(timeout) = timeout {
        server.insert(
            "timeout".to_string(),
            serde_json::json!({ "catalog": timeout, "execution": timeout }),
        );
    }
    for (legacy, native) in [
        ("clientId", "client_id"),
        ("clientSecret", "client_secret"),
        ("callbackPort", "callback_port"),
        ("redirectUri", "redirect_uri"),
    ] {
        move_key(server, legacy, native)?;
    }
    Ok(())
}

fn convert_server_v2_to_v1(value: &mut Value) -> Result<(), String> {
    let Some(server) = value.as_object_mut() else {
        return Ok(());
    };
    if let Some(disabled) = server.remove("disabled") {
        if let Some(disabled) = disabled.as_bool() {
            let enabled = Value::Bool(!disabled);
            if let Some(existing) = server.get("enabled") {
                if existing != &enabled {
                    return Err("Conflicting MCP server `enabled` and `disabled` values".to_string());
                }
            } else {
                server.insert("enabled".to_string(), enabled);
            }
        } else {
            return Err("Expected V2 MCP server `disabled` to be a boolean".to_string());
        }
    }
    let legacy_timeout = match server.get("timeout") {
        Some(Value::Object(timeout)) if timeout.get("catalog") == timeout.get("execution") => {
            timeout.get("catalog").cloned()
        }
        _ => None,
    };
    if let Some(timeout) = legacy_timeout {
        server.insert("timeout".to_string(), timeout);
    }
    for (native, legacy) in [
        ("client_id", "clientId"),
        ("client_secret", "clientSecret"),
        ("callback_port", "callbackPort"),
        ("redirect_uri", "redirectUri"),
    ] {
        move_key(server, native, legacy)?;
    }
    Ok(())
}

fn convert_mcp_v1_to_v2(value: &mut Value) -> Result<(), String> {
    let Some(mcp) = value.as_object_mut() else {
        return Ok(());
    };

    let mut servers = match mcp.remove("servers") {
        None => Map::new(),
        Some(Value::Object(servers)) => servers,
        Some(_) => return Err("Expected V2 MCP `servers` to be an object".to_string()),
    };
    let mut legacy_timeout = None;
    let keys: Vec<String> = mcp.keys().cloned().collect();
    for key in keys {
        if key == "timeout" {
            legacy_timeout = mcp.remove(&key);
            continue;
        }
        let Some(mut server) = mcp.remove(&key) else {
            continue;
        };
        if servers.contains_key(&key) {
            return Err(format!("MCP server `{key}` is defined in both legacy and V2 locations"));
        }
        convert_server_v1_to_v2(&mut server)?;
        servers.insert(key, server);
    }
    for server in servers.values_mut() {
        convert_server_v1_to_v2(server)?;
    }
    if !servers.is_empty() {
        mcp.insert("servers".to_string(), Value::Object(servers));
    }
    if let Some(timeout) = legacy_timeout {
        if let Some(timeout_value) = timeout.as_u64() {
            let converted = serde_json::json!({ "catalog": timeout_value, "execution": timeout_value });
            if let Some(existing) = mcp.get("timeout") {
                if existing != &converted {
                    return Err("Conflicting values for `mcp.timeout`".to_string());
                }
            } else {
                mcp.insert("timeout".to_string(), converted);
            }
        } else if !mcp.contains_key("timeout") {
            mcp.insert("timeout".to_string(), timeout);
        } else if mcp.get("timeout") != Some(&timeout) {
            return Err("Conflicting values for `mcp.timeout`".to_string());
        }
    }
    Ok(())
}

fn convert_mcp_v2_to_v1(value: &mut Value) -> Result<(), String> {
    let Some(mcp) = value.as_object_mut() else {
        return Ok(());
    };
    if let Some(servers) = mcp.remove("servers") {
        let servers = servers
            .as_object()
            .cloned()
            .ok_or_else(|| "Expected V2 MCP `servers` to be an object".to_string())?;
        for (name, mut server) in servers {
            convert_server_v2_to_v1(&mut server)?;
            if mcp.insert(name.clone(), server).is_some() {
                return Err(format!("Cannot restore MCP server `{name}` because that key already exists"));
            }
        }
    }
    Ok(())
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
    if let Some(providers) = root.remove("provider") {
        root.insert("providers".to_string(), convert_provider_v1_to_v2_map(providers)?);
    }
    if let Some(plugins) = root.remove("plugin") {
        root.insert("plugins".to_string(), convert_plugin_list_v1_to_v2(plugins)?);
    }
    for key in ["disabled_providers", "enabled_providers"] {
        if let Some(provider_ids) = root.get_mut(key) {
            canonicalize_provider_list(provider_ids);
        }
    }
    if let Some(small_model) = root.remove("small_model") {
        let mut small_model = small_model;
        canonicalize_model_reference(&mut small_model);
        let agents = object_at_mut(root, "agents")?;
        let title_agent = object_at_mut(agents, "title")?;
        if let Some(existing) = title_agent.get("model") {
            if existing != &small_model {
                return Err("Conflicting model selections in `small_model` and `agents.title.model`".to_string());
            }
        } else {
            title_agent.insert("model".to_string(), small_model);
        }
    }
    if let Some(mcp) = root.get_mut("mcp") {
        convert_mcp_v1_to_v2(mcp)?;
    }

    Ok(value)
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
    if let Some(providers) = root.remove("providers") {
        root.insert("provider".to_string(), convert_provider_v2_to_v1_map(providers)?);
    }
    if let Some(plugins) = root.remove("plugins") {
        root.insert("plugin".to_string(), convert_plugin_list_v2_to_v1(plugins)?);
    }
    for key in ["disabled_providers", "enabled_providers"] {
        if let Some(provider_ids) = root.get_mut(key) {
            canonicalize_provider_list(provider_ids);
        }
    }
    let small_model = root
        .get_mut("agents")
        .and_then(Value::as_object_mut)
        .and_then(|agents| agents.get_mut("title"))
        .and_then(Value::as_object_mut)
        .and_then(|title_agent| title_agent.remove("model"));
    if let Some(small_model) = small_model {
        root.insert("small_model".to_string(), small_model);
    }
    if let Some(mcp) = root.get_mut("mcp") {
        convert_mcp_v2_to_v1(mcp)?;
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
    let base = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let extension = config_path.extension().and_then(|value| value.to_str());
    let suffix = if attempt == 0 {
        chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
    } else {
        format!("{}_{}", chrono::Local::now().format("%Y%m%d_%H%M%S"), attempt)
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
        return Err("OpenCode config path conflicts with a reserved migration backup name".to_string());
    }

    if enabled {
        if backup_v1.exists() {
            return Ok(true);
        }
        if !config_path.exists() {
            return Err(format!("OpenCode config file does not exist: {}", config_path.display()));
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

        fs::rename(config_path, &backup_v1)
            .map_err(|error| format!("Failed to back up V1 config to {}: {error}", backup_v1.display()))?;
        if let Err(error) = temporary.persist(config_path) {
            let restore_error = fs::rename(&backup_v1, config_path).err();
            let mut detail = format!("Failed to install migrated V2 config: {}", error.error);
            if let Some(restore_error) = restore_error {
                detail.push_str(&format!("; also failed to restore V1 config: {restore_error}"));
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
            format!("Failed to archive previous V2 backup {}: {error}", backup_v2.display())
        })?;
        archived_previous_v2 = Some(archive_path);
    }

    if has_active_config {
        if let Err(error) = fs::rename(config_path, &backup_v2) {
            if let Some(archive_path) = archived_previous_v2.as_ref() {
                let _ = fs::rename(archive_path, &backup_v2);
            }
            return Err(format!("Failed to save V2 config to {}: {error}", backup_v2.display()));
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
        let mut detail = format!("Failed to restore V1 config from {}: {error}", backup_v1.display());
        if let Some(rollback_error) = rollback_v2_error {
            detail.push_str(&format!("; also failed to restore active V2 config: {rollback_error}"));
        }
        if let Some(rollback_error) = rollback_archive_error {
            detail.push_str(&format!("; also failed to restore older V2 backup: {rollback_error}"));
        }
        return Err(detail);
    }

    Ok(false)
}
