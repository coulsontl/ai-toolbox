//! `/v1/clis/*` routes: capability matrix, providers, models, global config
//! files, global prompt presets and runtime paths.
//!
//! Every mutating arm runs through `approve_write` first; every read arm
//! returns redacted output. The actual work is delegated to the existing
//! Tauri commands via `cli_dispatch`.

use std::collections::HashMap;

use hyper::Method;
use serde_json::{json, Map, Value};
use tauri::Manager;

use super::approval::RiskLevel;
use super::cli_dispatch;
use super::error::{ApiError, ApiResult};
use super::redact::{
    is_redacted_value, merge_patch_preserving_secrets, redact_json_string, redact_text,
    redact_value,
};
use super::routes::{approve_write, body_optional_string, redacted};
use super::ServerContext;
use crate::coding::{config_patch, runtime_location};
use crate::db::SqliteDbState;

pub fn normalize_cli_key(raw: &str) -> Option<&'static str> {
    cli_dispatch::normalize_cli_key(raw)
}

fn db_state(context: &ServerContext) -> tauri::State<'_, SqliteDbState> {
    context.app.state::<SqliteDbState>()
}

fn app(context: &ServerContext) -> tauri::AppHandle {
    context.app.clone()
}

fn internal(error: String) -> ApiError {
    ApiError::internal(error)
}

fn invalid(error: String) -> ApiError {
    ApiError::bad_request(error)
}

fn resolve_cli(segments: &[String]) -> ApiResult<&'static str> {
    let raw = segments
        .get(1)
        .map(String::as_str)
        .ok_or_else(|| ApiError::bad_request("A CLI key is required"))?;
    normalize_cli_key(raw).ok_or_else(|| ApiError::bad_request(format!("Unknown CLI key '{raw}'")))
}

pub async fn dispatch(
    context: &ServerContext,
    method: &Method,
    segments: &[String],
    query: &HashMap<String, String>,
    body: Value,
) -> ApiResult<Value> {
    if segments.len() == 1 {
        return match method {
            &Method::GET => Ok(cli_dispatch::capability_matrix()),
            _ => Err(ApiError::new(405, "method_not_allowed", "Use GET /v1/clis")),
        };
    }

    let cli = resolve_cli(segments)?;
    let rest: Vec<&str> = segments.iter().skip(2).map(String::as_str).collect();
    let scope = query.get("scope").map(String::as_str).unwrap_or_default();

    match (method, rest.as_slice()) {
        (&Method::GET, ["providers"]) => cli_dispatch::list_providers(context, cli).await,
        (&Method::GET, ["providers", id]) => cli_dispatch::get_provider(context, cli, id).await,
        (&Method::POST, ["providers"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.provider.create"),
                format!("Create a provider in '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            cli_dispatch::create_provider(context, cli, body).await
        }
        (&Method::PATCH, ["providers", id]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.provider.update"),
                format!("Update provider '{id}' in '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            cli_dispatch::update_provider(context, cli, id, body).await
        }
        (&Method::DELETE, ["providers", id]) => {
            approve_write(
                context,
                RiskLevel::L2,
                format!("cli.{cli}.provider.delete"),
                format!("Delete provider '{id}' from '{cli}' and every model it owns"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            cli_dispatch::delete_provider(context, cli, id).await
        }
        (&Method::POST, ["providers", id, "apply"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.provider.apply"),
                format!("Apply provider '{id}' to the '{cli}' runtime configuration"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            let model_id = body_optional_string(&body, "modelId")
                .or_else(|| body_optional_string(&body, "model"));
            cli_dispatch::apply_provider(context, cli, id, model_id.as_deref()).await
        }
        (&Method::POST, ["providers", id, "test"]) => {
            let result = cli_dispatch::test_provider(context, cli, id, &body).await?;
            Ok(redacted(result))
        }
        (&Method::POST, ["providers", id, "fetch-models"]) => {
            let result = cli_dispatch::fetch_models(context, cli, id, &body).await?;
            Ok(redacted(result))
        }
        (&Method::GET, ["providers", id, "models"]) => {
            cli_dispatch::list_models(context, cli, id).await
        }
        (&Method::POST, ["providers", id, "models"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.model.create"),
                format!("Add a model to provider '{id}' in '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            cli_dispatch::add_model(context, cli, id, body).await
        }
        (&Method::DELETE, ["providers", id, "models", model_id]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.model.delete"),
                format!("Delete model '{model_id}' from provider '{id}' in '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            cli_dispatch::delete_model(context, cli, id, model_id).await
        }
        (&Method::PUT, ["providers", id, "default-model"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.model.default"),
                format!("Change the default model of provider '{id}' in '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            let model_id = body
                .get("modelId")
                .or_else(|| body.get("model"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::bad_request("'modelId' is required"))?;
            cli_dispatch::set_default_model(context, cli, id, model_id).await
        }
        (&Method::GET, ["config"]) => get_config(context, cli, scope).await,
        (&Method::PUT, ["config"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.config.update"),
                format!("Write the global configuration file of '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            put_config(context, cli, scope, body).await
        }
        (&Method::POST, ["config", "extract"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.config.extract"),
                format!("Import global configuration for '{cli}' from its current file"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            extract_config(context, cli, scope).await
        }
        (&Method::GET, ["prompts"]) => list_prompts(context, cli).await,
        (&Method::POST, ["prompts"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.prompt.create"),
                format!("Create a global prompt preset in '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            create_prompt(context, cli, body).await
        }
        (&Method::POST, ["prompts", "disable"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.prompt.disable"),
                format!("Remove the applied global prompt of '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            disable_prompt(context, cli, body).await
        }
        (&Method::PUT, ["prompts", id]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.prompt.update"),
                format!("Update global prompt preset '{id}' in '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            update_prompt(context, cli, id, body).await
        }
        (&Method::DELETE, ["prompts", id]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.prompt.delete"),
                format!("Delete global prompt preset '{id}' from '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            delete_prompt(context, cli, id).await
        }
        (&Method::POST, ["prompts", id, "apply"]) => {
            approve_write(
                context,
                RiskLevel::L1,
                format!("cli.{cli}.prompt.apply"),
                format!("Apply global prompt preset '{id}' to '{cli}'"),
                Some(cli.to_string()),
                false,
            )
            .await?;
            apply_prompt(context, cli, id).await
        }
        (&Method::GET, ["paths"]) => paths_for_cli(context, cli).await,
        _ => Err(ApiError::not_found(format!(
            "Unknown route for CLI '{cli}': /{}",
            segments.join("/")
        ))),
    }
}

// ---------------------------------------------------------------------------
// Global configuration
// ---------------------------------------------------------------------------

fn config_format(cli: &str) -> &'static str {
    match cli {
        "codex" | "grok" | "kimi" | "zcode" => "toml",
        "oh_my_pi" | "hermes" | "dsh" => "yaml",
        "omo_native" => "jsonc",
        _ => "json",
    }
}

/// `zcode --scope cli` reads the product-owned `cli/config.json`, which is JSON
/// even though the common config is TOML. Reporting TOML made agents send TOML
/// to a JSON-only setter.
fn config_format_for_scope(cli: &str, scope: &str) -> &'static str {
    if cli == "zcode" && scope == "cli" {
        "json"
    } else {
        config_format(cli)
    }
}

fn config_kind(cli: &str) -> &'static str {
    match cli {
        "pi" | "oh_my_pi" | "omo_native" | "hermes" | "dsh" | "codebuddy" => "other",
        _ => "common",
    }
}

async fn get_config(context: &ServerContext, cli: &str, scope: &str) -> ApiResult<Value> {
    if cli == "workbuddy" {
        return Err(cli_dispatch::unsupported(
            cli,
            "Global config file editing",
        ));
    }
    let format = config_format_for_scope(cli, scope);
    let mut config_path: Option<String> = None;
    let (config, root_dir, updated_at) = match cli {
        "claudecode" => {
            let existing =
                crate::coding::claude_code::commands::get_claude_common_config(db_state(context))
                    .await
                    .map_err(internal)?;
            match existing {
                Some(common) => (
                    parse_config_text(&common.config, format),
                    common.root_dir,
                    common.updated_at,
                ),
                None => (json!({}), None, String::new()),
            }
        }
        "claudedesktop" => {
            let existing =
                crate::coding::claude_desktop::commands::get_claude_desktop_common_config(
                    db_state(context),
                )
                .await
                .map_err(internal)?;
            match existing {
                Some(common) => (
                    parse_config_text(&common.config, format),
                    None,
                    common.updated_at,
                ),
                None => (json!({}), None, String::new()),
            }
        }
        "codex" => {
            let existing =
                crate::coding::codex::commands::get_codex_common_config(db_state(context))
                    .await
                    .map_err(internal)?;
            match existing {
                Some(common) => (
                    parse_config_text(&common.config, format),
                    common.root_dir,
                    common.updated_at,
                ),
                None => (Value::String(String::new()), None, String::new()),
            }
        }
        "grok" => {
            let existing = crate::coding::grok::commands::get_grok_common_config(db_state(context))
                .await
                .map_err(internal)?;
            match existing {
                Some(common) => (
                    parse_config_text(&common.config, format),
                    common.root_dir,
                    common.updated_at,
                ),
                None => (Value::String(String::new()), None, String::new()),
            }
        }
        "kimi" => {
            let existing = crate::coding::kimi::commands::get_kimi_common_config(db_state(context))
                .await
                .map_err(internal)?;
            match existing {
                Some(common) => (
                    parse_config_text(&common.config, format),
                    common.root_dir,
                    common.updated_at,
                ),
                None => (Value::String(String::new()), None, String::new()),
            }
        }
        "geminicli" => {
            let existing = crate::coding::gemini_cli::commands::get_gemini_cli_common_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            match existing {
                Some(common) => (
                    parse_config_text(&common.config, format),
                    common.root_dir,
                    common.updated_at,
                ),
                None => (json!({}), None, String::new()),
            }
        }
        "antigravity" => {
            let common = crate::coding::antigravity::commands::get_antigravity_common_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            (
                parse_config_text(&common.config, format),
                common.root_dir,
                common.updated_at,
            )
        }
        "zcode" if scope == "cli" => {
            // `cli/config.json` is a separate product-owned file (MCP servers
            // and CLI settings) from the provider registry common config.
            let text =
                crate::coding::zcode::commands::read_zcode_cli_config(db_state(context))
                    .await
                    .map_err(internal)?;
            if text.trim().is_empty() {
                (json!({}), None, String::new())
            } else {
                let parsed = serde_json::from_str::<Value>(&text)
                    .map_err(|error| invalid(format!("ZCode cli/config.json is not valid JSON: {error}")))?;
                (parse_config_text(&parsed.to_string(), format), None, String::new())
            }
        }
        "zcode" => {
            let common = crate::coding::zcode::commands::get_zcode_common_config(db_state(context))
                .await
                .map_err(internal)?;
            (
                parse_config_text(
                    common.get("config").and_then(Value::as_str).unwrap_or("{}"),
                    format,
                ),
                common
                    .get("rootDir")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                common
                    .get("updatedAt")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        }
        "opencode" => {
            config_path = crate::coding::open_code::commands::get_opencode_config_path(
                db_state(context),
            )
            .await
            .ok();
            let config = cli_dispatch::read_opencode_config_value(context).await?;
            (redacted_value(config), None, String::new())
        }
        "openclaw" => {
            config_path = crate::coding::open_claw::commands::get_openclaw_config_path(
                db_state(context),
            )
            .await
            .ok();
            let config = cli_dispatch::read_openclaw_config_value(context).await?;
            (redacted_value(config), None, String::new())
        }
        "pi" => {
            let runtime = crate::coding::pi::commands::read_pi_runtime_config(db_state(context))
                .await
                .map_err(internal)?;
            (redacted_value(runtime.other_settings), None, String::new())
        }
        "oh_my_pi" => {
            let runtime =
                crate::coding::oh_my_pi::commands::read_omp_runtime_config(db_state(context))
                    .await
                    .map_err(internal)?;
            (redacted_value(runtime.other_settings), None, String::new())
        }
        "omo_native" => {
            let runtime = crate::coding::omo_native::commands::read_omo_native_runtime_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            if scope == "omo" {
                // `omo.jsonc` shared top-level keys (the file the engine reads).
                (redacted_value(runtime.shared_base), None, String::new())
            } else {
                // `settings.json` keys owned by neither model settings nor
                // extensions (the default scope kept for compatibility).
                (redacted_value(runtime.other_settings), None, String::new())
            }
        }
        "hermes" => {
            let runtime =
                crate::coding::hermes::commands::read_hermes_runtime_config(db_state(context))
                    .await
                    .map_err(internal)?;
            (redacted_value(runtime.other_settings), None, String::new())
        }
        "dsh" => {
            let runtime = crate::coding::dsh::commands::read_dsh_runtime_config(db_state(context))
                .await
                .map_err(internal)?;
            (redacted_value(runtime.other_settings), None, String::new())
        }
        "codebuddy" => {
            let raw =
                crate::coding::codebuddy::commands::read_codebuddy_settings_file(db_state(context))
                    .await
                    .map_err(internal)?;
            match raw {
                Some(text) => {
                    let redacted = redact_text(&text);
                    let parsed = serde_json::from_str::<Value>(&redacted)
                        .map(|mut value| {
                            redact_value(&mut value);
                            value
                        })
                        .unwrap_or(Value::String(redacted));
                    (parsed, None, String::new())
                }
                None => (Value::Object(Map::new()), None, String::new()),
            }
        }
        "workbuddy" => {
            return Err(cli_dispatch::unsupported(
                "workbuddy",
                "A global settings file (models.json is exposed through /providers)",
            ))
        }
        other => return Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    };

    let mut payload = json!({
        "cli": cli,
        "scope": if scope.is_empty() { "default" } else { scope },
        "kind": config_kind(cli),
        "format": format,
        "readOnly": cli == "codebuddy",
        "config": config,
        "rootDir": root_dir,
        "configPath": config_path,
        "updatedAt": updated_at,
    });
    redact_value(&mut payload);
    Ok(payload)
}

/// Re-imports the CLI's current runtime file into the stored common config.
///
/// Only the six CLIs that own a real `extract_*_common_config_from_current_file`
/// helper can do this. Everything else returns 405 instead of pretending the
/// GET response was an import.
async fn extract_config(context: &ServerContext, cli: &str, scope: &str) -> ApiResult<Value> {
    // Extract reads the CLI's live file and saves it as the common config. When
    // the gateway has taken the CLI over that file is the proxy projection, so
    // reading it outside the restore -> read -> re-engage window would persist
    // the gateway address as the user's base URL.
    cli_dispatch::with_gateway_reattach_api(context, cli, || async {
        extract_config_inner(context, cli, scope).await
    })
    .await
}

async fn extract_config_inner(
    context: &ServerContext,
    cli: &str,
    scope: &str,
) -> ApiResult<Value> {
    match cli {
        "claudecode" => {
            let extracted =
                crate::coding::claude_code::commands::extract_claude_common_config_from_current_file(
                    db_state(context),
                )
                .await
                .map_err(internal)?;
            crate::coding::claude_code::commands::save_claude_common_config(
                db_state(context),
                app(context),
                crate::coding::claude_code::types::ClaudeCommonConfigInput {
                    config: extracted.config,
                    root_dir: extracted.root_dir,
                    clear_root_dir: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        "codex" => {
            let extracted =
                crate::coding::codex::commands::extract_codex_common_config_from_current_file(
                    db_state(context),
                )
                .await
                .map_err(internal)?;
            crate::coding::codex::commands::save_codex_common_config(
                db_state(context),
                app(context),
                crate::coding::codex::types::CodexCommonConfigInput {
                    config: extracted.config,
                    root_dir: extracted.root_dir,
                    clear_root_dir: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        "grok" => {
            let extracted =
                crate::coding::grok::commands::extract_grok_common_config_from_current_file(
                    db_state(context),
                )
                .await
                .map_err(internal)?;
            crate::coding::grok::commands::save_grok_common_config(
                db_state(context),
                app(context),
                crate::coding::grok::types::GrokCommonConfigInput {
                    config: extracted.config,
                    root_dir: extracted.root_dir,
                    clear_root_dir: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        "kimi" => {
            let extracted =
                crate::coding::kimi::commands::extract_kimi_common_config_from_current_file(
                    db_state(context),
                )
                .await
                .map_err(internal)?;
            crate::coding::kimi::commands::save_kimi_common_config(
                db_state(context),
                app(context),
                crate::coding::kimi::types::KimiCommonConfigInput {
                    config: extracted.config,
                    root_dir: extracted.root_dir,
                    clear_root_dir: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        "geminicli" => {
            let extracted = crate::coding::gemini_cli::commands::
                extract_gemini_cli_common_config_from_current_file(db_state(context))
                .await
                .map_err(internal)?;
            crate::coding::gemini_cli::commands::save_gemini_cli_common_config(
                db_state(context),
                app(context),
                crate::coding::gemini_cli::types::GeminiCliCommonConfigInput {
                    config: extracted.config,
                    root_dir: extracted.root_dir,
                    clear_root_dir: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        "antigravity" => {
            let extracted = crate::coding::antigravity::commands::
                extract_antigravity_common_config_from_current_file(db_state(context))
                .await
                .map_err(internal)?;
            crate::coding::antigravity::commands::save_antigravity_common_config(
                app(context),
                db_state(context),
                crate::coding::antigravity::types::AntigravityCommonConfigInput {
                    config: extracted.config,
                    root_dir: extracted.root_dir,
                    clear_root_dir: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        other => {
            return Err(ApiError::new(
                405,
                "method_not_allowed",
                format!("Config extract is not supported for CLI '{other}'"),
            ))
        }
    }
    get_config(context, cli, scope).await
}

/// JSON-shaped configs are parsed (and therefore redacted structurally);
/// TOML/YAML configs are returned as text so the agent sees the exact file.
fn parse_config_text(raw: &str, format: &str) -> Value {
    if format == "toml" {
        // Early builds stored an empty TOML config as the JSON literal `{}`,
        // which is not valid TOML and made every later patch fail. Present it
        // as an empty document instead.
        let raw = if raw.trim() == "{}" { "" } else { raw };
        return Value::String(redact_text(raw));
    }
    if format == "yaml" {
        return Value::String(redact_text(raw));
    }
    redact_json_string(raw)
}

fn redacted_value(mut value: Value) -> Value {
    redact_value(&mut value);
    value
}

async fn put_config(
    context: &ServerContext,
    cli: &str,
    scope: &str,
    body: Value,
) -> ApiResult<Value> {
    cli_dispatch::with_gateway_reattach_api(context, cli, || async {
        put_config_inner(context, cli, scope, body).await
    })
    .await
}

async fn put_config_inner(
    context: &ServerContext,
    cli: &str,
    scope: &str,
    body: Value,
) -> ApiResult<Value> {
    if cli == "codebuddy" {
        return Err(ApiError::read_only(
            "CodeBuddy's settings.json is owned by the product and is read-only in the control API",
        ));
    }
    if cli == "workbuddy" {
        return Err(cli_dispatch::unsupported(
            "workbuddy",
            "Global config file editing",
        ));
    }
    let format = config_format_for_scope(cli, scope);
    let mode = body_optional_string(&body, "mode").unwrap_or_else(|| "patch".to_string());
    if mode != "patch" && mode != "replace" {
        return Err(ApiError::bad_request("'mode' must be 'patch' or 'replace'"));
    }
    let incoming = body
        .get("config")
        .cloned()
        .ok_or_else(|| ApiError::bad_request("'config' is required"))?;
    // JSON configs round-trip `***` through `fill_redacted`/the merge layer.
    // TOML/YAML cannot: there the mask must be omitted, not sent back.
    if matches!(format, "toml" | "yaml") && super::redact::contains_redacted(&incoming) {
        return Err(ApiError::bad_request(
            "Masked secrets ('***') cannot be written back; send the full value or omit the key",
        ));
    }
    let root_dir = body_optional_string(&body, "rootDir");
    let clear_root_dir = body
        .get("clearRootDir")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    match cli {
        "claudecode" => {
            let existing =
                crate::coding::claude_code::commands::get_claude_common_config(db_state(context))
                    .await
                    .map_err(internal)?;
            let config = apply_config_patch(
                existing
                    .as_ref()
                    .map(|common| common.config.as_str())
                    .unwrap_or("{}"),
                &incoming,
                format,
                &mode,
            )?;
            crate::coding::claude_code::commands::save_claude_common_config(
                db_state(context),
                app(context),
                crate::coding::claude_code::types::ClaudeCommonConfigInput {
                    config,
                    root_dir,
                    clear_root_dir,
                },
            )
            .await
            .map_err(internal)?;
        }
        "claudedesktop" => {
            let existing =
                crate::coding::claude_desktop::commands::get_claude_desktop_common_config(
                    db_state(context),
                )
                .await
                .map_err(internal)?;
            let config = apply_config_patch(
                existing
                    .as_ref()
                    .map(|common| common.config.as_str())
                    .unwrap_or("{}"),
                &incoming,
                format,
                &mode,
            )?;
            crate::coding::claude_desktop::commands::save_claude_desktop_common_config(
                db_state(context),
                app(context),
                crate::coding::claude_desktop::types::ClaudeDesktopCommonConfigInput { config },
            )
            .await
            .map_err(internal)?;
        }
        "codex" => {
            let existing =
                crate::coding::codex::commands::get_codex_common_config(db_state(context))
                    .await
                    .map_err(internal)?;
            let config = apply_config_patch(
                existing
                    .as_ref()
                    .map(|common| common.config.as_str())
                    .unwrap_or(""),
                &incoming,
                format,
                &mode,
            )?;
            crate::coding::codex::commands::save_codex_common_config(
                db_state(context),
                app(context),
                crate::coding::codex::types::CodexCommonConfigInput {
                    config,
                    root_dir,
                    clear_root_dir,
                },
            )
            .await
            .map_err(internal)?;
        }
        "grok" => {
            let existing = crate::coding::grok::commands::get_grok_common_config(db_state(context))
                .await
                .map_err(internal)?;
            let config = apply_config_patch(
                existing
                    .as_ref()
                    .map(|common| common.config.as_str())
                    .unwrap_or(""),
                &incoming,
                format,
                &mode,
            )?;
            crate::coding::grok::commands::save_grok_common_config(
                db_state(context),
                app(context),
                crate::coding::grok::types::GrokCommonConfigInput {
                    config,
                    root_dir,
                    clear_root_dir,
                },
            )
            .await
            .map_err(internal)?;
        }
        "kimi" => {
            let existing = crate::coding::kimi::commands::get_kimi_common_config(db_state(context))
                .await
                .map_err(internal)?;
            let config = apply_config_patch(
                existing
                    .as_ref()
                    .map(|common| common.config.as_str())
                    .unwrap_or(""),
                &incoming,
                format,
                &mode,
            )?;
            crate::coding::kimi::commands::save_kimi_common_config(
                db_state(context),
                app(context),
                crate::coding::kimi::types::KimiCommonConfigInput {
                    config,
                    root_dir,
                    clear_root_dir,
                },
            )
            .await
            .map_err(internal)?;
        }
        "geminicli" => {
            let existing = crate::coding::gemini_cli::commands::get_gemini_cli_common_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            let config = apply_config_patch(
                existing
                    .as_ref()
                    .map(|common| common.config.as_str())
                    .unwrap_or("{}"),
                &incoming,
                format,
                &mode,
            )?;
            crate::coding::gemini_cli::commands::save_gemini_cli_common_config(
                db_state(context),
                app(context),
                crate::coding::gemini_cli::types::GeminiCliCommonConfigInput {
                    config,
                    root_dir,
                    clear_root_dir,
                },
            )
            .await
            .map_err(internal)?;
        }
        "antigravity" => {
            let existing = crate::coding::antigravity::commands::get_antigravity_common_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            let config = apply_config_patch(&existing.config, &incoming, format, &mode)?;
            crate::coding::antigravity::commands::save_antigravity_common_config(
                app(context),
                db_state(context),
                crate::coding::antigravity::types::AntigravityCommonConfigInput {
                    config,
                    root_dir,
                    clear_root_dir,
                },
            )
            .await
            .map_err(internal)?;
        }
        "zcode" if scope == "cli" => {
            let existing =
                crate::coding::zcode::commands::read_zcode_cli_config(db_state(context))
                    .await
                    .map_err(internal)?;
            let existing_text = if existing.trim().is_empty() {
                "{}"
            } else {
                existing.as_str()
            };
            let config = apply_config_patch(existing_text, &incoming, "json", &mode)?;
            crate::coding::zcode::commands::save_zcode_cli_config(
                db_state(context),
                app(context),
                config,
            )
            .await
            .map_err(internal)?;
        }
        "zcode" => {
            let existing =
                crate::coding::zcode::commands::get_zcode_common_config(db_state(context))
                    .await
                    .map_err(internal)?;
            let config = apply_config_patch(
                existing
                    .get("config")
                    .and_then(Value::as_str)
                    .unwrap_or("{}"),
                &incoming,
                format,
                &mode,
            )?;
            crate::coding::zcode::commands::save_zcode_common_config(
                db_state(context),
                app(context),
                crate::coding::zcode::types::ZcodeCommonConfigInput {
                    config,
                    root_dir,
                    clear_root_dir: Some(clear_root_dir),
                },
            )
            .await
            .map_err(internal)?;
        }
        "opencode" => {
            let (incoming, requested_path) = split_config_path(&incoming);
            if let Some(path) = requested_path {
                let mut config = crate::coding::open_code::commands::get_opencode_common_config(
                    db_state(context),
                )
                .await
                .map_err(internal)?
                .unwrap_or(crate::coding::open_code::types::OpenCodeCommonConfig {
                    config_path: None,
                    show_plugins_in_tray: false,
                    updated_at: chrono::Local::now().to_rfc3339(),
                });
                config.config_path = Some(path);
                crate::coding::open_code::commands::save_opencode_common_config(
                    db_state(context),
                    app(context),
                    config,
                )
                .await
                .map_err(internal)?;
            }
            if has_config_body(&incoming) {
                let existing = cli_dispatch::read_opencode_config_value(context).await?;
                let merged = apply_config_patch(&existing.to_string(), &incoming, "json", &mode)?;
                let merged: Value = serde_json::from_str(&merged).map_err(|error| {
                    invalid(format!("Merged OpenCode config is invalid: {error}"))
                })?;
                cli_dispatch::save_opencode_config_value(context, merged).await?;
            }
        }
        "openclaw" => {
            let (incoming, requested_path) = split_config_path(&incoming);
            if let Some(path) = requested_path {
                let mut config = crate::coding::open_claw::commands::get_openclaw_common_config(
                    db_state(context),
                )
                .await
                .map_err(internal)?
                .unwrap_or(crate::coding::open_claw::types::OpenClawCommonConfig {
                    config_path: None,
                    updated_at: chrono::Local::now().to_rfc3339(),
                });
                config.config_path = Some(path);
                crate::coding::open_claw::commands::save_openclaw_common_config(
                    db_state(context),
                    app(context),
                    config,
                )
                .await
                .map_err(internal)?;
            }
            if has_config_body(&incoming) {
                let existing = cli_dispatch::read_openclaw_config_value(context).await?;
                let merged = apply_config_patch(&existing.to_string(), &incoming, "json", &mode)?;
                let merged: Value = serde_json::from_str(&merged).map_err(|error| {
                    invalid(format!("Merged OpenClaw config is invalid: {error}"))
                })?;
                cli_dispatch::save_openclaw_config_value(context, merged).await?;
            }
        }
        "pi" => {
            let next = merge_other_settings(context, cli, &incoming, &mode).await?;
            crate::coding::pi::commands::save_pi_other_settings(
                db_state(context),
                app(context),
                next,
            )
            .await
            .map_err(internal)?;
        }
        "oh_my_pi" => {
            let next = merge_other_settings(context, cli, &incoming, &mode).await?;
            crate::coding::oh_my_pi::commands::save_omp_other_settings(
                db_state(context),
                app(context),
                next,
            )
            .await
            .map_err(internal)?;
        }
        "hermes" => {
            let next = merge_other_settings(context, cli, &incoming, &mode).await?;
            crate::coding::hermes::commands::save_hermes_other_settings(
                db_state(context),
                app(context),
                next,
            )
            .await
            .map_err(internal)?;
        }
        "dsh" => {
            let next = merge_other_settings(context, cli, &incoming, &mode).await?;
            crate::coding::dsh::commands::save_dsh_other_settings(
                db_state(context),
                app(context),
                next,
            )
            .await
            .map_err(dsh_other_settings_error)?;
        }
        "omo_native" if scope == "omo" => {
            let runtime = crate::coding::omo_native::commands::read_omo_native_runtime_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            let merged = apply_config_patch(
                &runtime.shared_base.to_string(),
                &incoming,
                "json",
                &mode,
            )?;
            let merged: Value = serde_json::from_str(&merged).map_err(|error| {
                invalid(format!("Merged omo.jsonc shared config is invalid: {error}"))
            })?;
            crate::coding::omo_native::commands::save_omo_native_other_config(
                db_state(context),
                app(context),
                merged,
            )
            .await
            .map_err(internal)?;
        }
        "omo_native" => {
            let existing = crate::coding::omo_native::commands::read_omo_native_runtime_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            let next = merge_other_settings(context, cli, &incoming, &mode).await?;
            let mut set = Map::new();
            let mut remove = Vec::new();
            if let Some(next_object) = next.as_object() {
                for (key, value) in next_object {
                    set.insert(key.clone(), value.clone());
                }
            }
            if let Some(existing_object) = existing.other_settings.as_object() {
                for key in existing_object.keys() {
                    if !set.contains_key(key) {
                        remove.push(key.clone());
                    }
                }
            }
            crate::coding::omo_native::commands::save_omo_native_other_settings(
                db_state(context),
                app(context),
                crate::coding::omo_native::types::OmoNativeOtherSettingsInput { set, remove },
            )
            .await
            .map_err(internal)?;
        }
        other => return Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }

    get_config(context, cli, scope).await
}

async fn merge_other_settings(
    context: &ServerContext,
    cli: &str,
    incoming: &Value,
    mode: &str,
) -> ApiResult<Value> {
    let current = match cli {
        "pi" => {
            crate::coding::pi::commands::read_pi_runtime_config(db_state(context))
                .await
                .map_err(internal)?
                .other_settings
        }
        "oh_my_pi" => {
            crate::coding::oh_my_pi::commands::read_omp_runtime_config(db_state(context))
                .await
                .map_err(internal)?
                .other_settings
        }
        "hermes" => {
            crate::coding::hermes::commands::read_hermes_runtime_config(db_state(context))
                .await
                .map_err(internal)?
                .other_settings
        }
        "dsh" => {
            crate::coding::dsh::commands::read_dsh_runtime_config(db_state(context))
                .await
                .map_err(internal)?
                .other_settings
        }
        "omo_native" => {
            crate::coding::omo_native::commands::read_omo_native_runtime_config(db_state(context))
                .await
                .map_err(internal)?
                .other_settings
        }
        _ => Value::Object(Map::new()),
    };
    // These CLIs store YAML on disk, so accepting a YAML document is the
    // natural input shape. `aitb config set --file x.yml` sends the file as a
    // string.
    let incoming = config_patch::coerce_object_input(
        incoming,
        matches!(cli, "oh_my_pi" | "hermes" | "dsh"),
    )
    .map_err(invalid)?;
    let incoming = Value::Object(incoming);
    if mode == "replace" {
        Ok(fill_redacted(&current, incoming))
    } else {
        let mut merged = current;
        merge_patch_preserving_secrets(&mut merged, incoming);
        Ok(merged)
    }
}

fn split_config_path(incoming: &Value) -> (Value, Option<String>) {
    match incoming {
        Value::Object(object) => {
            let mut object = object.clone();
            let path = object
                .remove("configPath")
                .and_then(|value| value.as_str().map(str::to_string))
                .filter(|path| !path.trim().is_empty());
            (Value::Object(object), path)
        }
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(parsed @ Value::Object(_)) => (parsed, None),
            Ok(_) => (Value::String(text.clone()), None),
            Err(_) if text.trim().starts_with('{') => (Value::String(text.clone()), None),
            // Legacy shape: a bare string meant "use this config path".
            Err(_) => (Value::Null, Some(text.clone())),
        },
        other => (other.clone(), None),
    }
}

fn has_config_body(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Object(object) => !object.is_empty(),
        Value::String(text) => !text.trim().is_empty(),
        _ => true,
    }
}

/// dsh 0.1.7+ keeps live config in `profiles/<profile>/cordis.patch.yml`; the
/// module deliberately refuses to rewrite its unrelated plugin rows. That is a
/// capability boundary, not a 500, so `aitb` must report `unsupported`.
fn dsh_other_settings_error(error: String) -> ApiError {
    if error.contains("cordis.patch.yml") {
        ApiError::new(400, "unsupported", error)
    } else {
        internal(error)
    }
}

/// Dispatches whole-file patches to the backend's structured merge. JSON stays
/// local because its masked-patch contract is the control API's own API shape,
/// not a CLI config-file format.
fn apply_config_patch(
    existing: &str,
    incoming: &Value,
    format: &str,
    mode: &str,
) -> ApiResult<String> {
    match format {
        "toml" => {
            // See `parse_config_text`: legacy empty rows are `{}`, not TOML.
            let existing = if existing.trim() == "{}" { "" } else { existing };
            config_patch::merge_toml_config_text(existing, incoming, mode).map_err(invalid)
        }
        "yaml" => {
            config_patch::merge_yaml_config_text(existing, incoming, mode).map_err(invalid)
        }
        _ => merge_json_config_text(existing, incoming, mode),
    }
}

fn merge_json_config_text(existing: &str, incoming: &Value, mode: &str) -> ApiResult<String> {
    let incoming = match incoming {
        Value::String(text) => serde_json::from_str::<Value>(text).map_err(|error| {
            invalid(format!("'config' string is not valid JSON: {error}"))
        })?,
        other => other.clone(),
    };
    if !incoming.is_object() {
        return Err(ApiError::bad_request("'config' must be an object"));
    }
    let mut current = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(existing)
            .map_err(|error| invalid(format!("Existing config is not valid JSON: {error}")))?
    };
    if !current.is_object() {
        current = json!({});
    }
    if mode == "replace" {
        current = fill_redacted(&current, incoming);
    } else {
        merge_patch_preserving_secrets(&mut current, incoming);
    }
    Ok(serde_json::to_string_pretty(&current)?)
}

/// Replace mode must not destroy credentials that the GET response masked.
/// Values equal to the mask are replaced by the previous value at the same path.
fn fill_redacted(existing: &Value, incoming: Value) -> Value {
    match (existing, incoming) {
        (Value::Object(existing_map), Value::Object(mut incoming_map)) => {
            for (key, value) in incoming_map.iter_mut() {
                if is_redacted_value(value) {
                    if let Some(previous) = existing_map.get(key) {
                        *value = previous.clone();
                    }
                } else if let Some(previous) = existing_map.get(key) {
                    *value = fill_redacted(previous, value.clone());
                }
            }
            Value::Object(incoming_map)
        }
        (Value::Array(existing_items), Value::Array(mut incoming_items)) => {
            for (index, value) in incoming_items.iter_mut().enumerate() {
                if is_redacted_value(value) {
                    if let Some(previous) = existing_items.get(index) {
                        *value = previous.clone();
                    }
                } else if let Some(previous) = existing_items.get(index) {
                    *value = fill_redacted(previous, value.clone());
                }
            }
            Value::Array(incoming_items)
        }
        (_, other) => other,
    }
}

// ---------------------------------------------------------------------------
// Global prompt presets
// ---------------------------------------------------------------------------

async fn list_prompts(context: &ServerContext, cli: &str) -> ApiResult<Value> {
    macro_rules! list {
        ($path:path) => {{
            let prompts = $path(db_state(context)).await.map_err(internal)?;
            let prompts = serde_json::to_value(prompts)?;
            Ok(json!({ "prompts": prompts }))
        }};
    }
    match cli {
        "claudecode" => list!(crate::coding::claude_code::commands::list_claude_prompt_configs),
        "claudedesktop" => {
            list!(crate::coding::claude_desktop::prompt::list_claude_desktop_prompt_configs)
        }
        "codex" => list!(crate::coding::codex::commands::list_codex_prompt_configs),
        "grok" => list!(crate::coding::grok::commands::list_grok_prompt_configs),
        "kimi" => list!(crate::coding::kimi::commands::list_kimi_prompt_configs),
        "geminicli" => list!(crate::coding::gemini_cli::commands::list_gemini_cli_prompt_configs),
        "antigravity" => {
            list!(crate::coding::antigravity::commands::list_antigravity_prompt_configs)
        }
        "opencode" => list!(crate::coding::open_code::commands::list_opencode_prompt_configs),
        "pi" => list!(crate::coding::pi::commands::list_pi_prompt_configs),
        "oh_my_pi" => list!(crate::coding::oh_my_pi::commands::list_omp_prompt_configs),
        "omo_native" => list!(crate::coding::omo_native::prompt::list_omo_native_prompt_configs),
        "hermes" => list!(crate::coding::hermes::commands::list_hermes_prompt_configs),
        "dsh" => list!(crate::coding::dsh::commands::list_dsh_prompt_configs),
        "zcode" => list!(crate::coding::zcode::commands::list_zcode_prompt_configs),
        "codebuddy" => list!(crate::coding::codebuddy::commands::list_codebuddy_prompt_configs),
        "openclaw" | "workbuddy" => Err(cli_dispatch::unsupported(cli, "Global prompt presets")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

async fn create_prompt(context: &ServerContext, cli: &str, body: Value) -> ApiResult<Value> {
    macro_rules! create {
        ($module:path, $input:ty) => {{
            let mut input: $input = serde_json::from_value(body)
                .map_err(|error| invalid(format!("Invalid prompt preset: {error}")))?;
            input.id = None;
            let created = $module(db_state(context), app(context), input)
                .await
                .map_err(internal)?;
            Ok(serde_json::to_value(created)?)
        }};
    }
    match cli {
        "claudecode" => create!(
            crate::coding::claude_code::commands::create_claude_prompt_config,
            crate::coding::claude_code::types::ClaudePromptConfigInput
        ),
        "claudedesktop" => create!(
            crate::coding::claude_desktop::prompt::create_claude_desktop_prompt_config,
            crate::coding::claude_desktop::types::ClaudeDesktopPromptConfigInput
        ),
        "codex" => create!(
            crate::coding::codex::commands::create_codex_prompt_config,
            crate::coding::codex::types::CodexPromptConfigInput
        ),
        "grok" => create!(
            crate::coding::grok::commands::create_grok_prompt_config,
            crate::coding::grok::types::GrokPromptConfigInput
        ),
        "kimi" => create!(
            crate::coding::kimi::commands::create_kimi_prompt_config,
            crate::coding::kimi::types::KimiPromptConfigInput
        ),
        "geminicli" => create!(
            crate::coding::gemini_cli::commands::create_gemini_cli_prompt_config,
            crate::coding::gemini_cli::types::GeminiCliPromptConfigInput
        ),
        "antigravity" => create!(
            crate::coding::antigravity::commands::create_antigravity_prompt_config,
            crate::coding::antigravity::types::AntigravityPromptConfigInput
        ),
        "opencode" => create!(
            crate::coding::open_code::commands::create_opencode_prompt_config,
            crate::coding::open_code::types::OpenCodePromptConfigInput
        ),
        "pi" => create!(
            crate::coding::pi::commands::create_pi_prompt_config,
            crate::coding::pi::types::PiPromptConfigInput
        ),
        "oh_my_pi" => create!(
            crate::coding::oh_my_pi::commands::create_omp_prompt_config,
            crate::coding::oh_my_pi::types::OmpPromptConfigInput
        ),
        "omo_native" => create!(
            crate::coding::omo_native::prompt::create_omo_native_prompt_config,
            crate::coding::omo_native::types::OmoNativePromptConfigInput
        ),
        "hermes" => create!(
            crate::coding::hermes::commands::create_hermes_prompt_config,
            crate::coding::hermes::types::HermesPromptConfigInput
        ),
        "dsh" => create!(
            crate::coding::dsh::commands::create_dsh_prompt_config,
            crate::coding::dsh::types::DshPromptConfigInput
        ),
        "zcode" => create!(
            crate::coding::zcode::commands::create_zcode_prompt_config,
            crate::coding::zcode::types::ZcodePromptConfigInput
        ),
        "codebuddy" => create!(
            crate::coding::codebuddy::commands::create_codebuddy_prompt_config,
            crate::coding::codebuddy::types::CodebuddyPromptConfigInput
        ),
        "openclaw" | "workbuddy" => Err(cli_dispatch::unsupported(cli, "Global prompt presets")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

async fn update_prompt(
    context: &ServerContext,
    cli: &str,
    id: &str,
    mut body: Value,
) -> ApiResult<Value> {
    if !body.is_object() {
        return Err(ApiError::bad_request(
            "Prompt preset body must be an object",
        ));
    }
    let has_name = body.get("name").and_then(Value::as_str).is_some();
    let has_content = body.get("content").and_then(Value::as_str).is_some();
    if !has_name || !has_content {
        // The per-CLI prompt inputs require both fields. A partial update must
        // read the stored preset first instead of failing deserialization or
        // blanking the missing half.
        let listing = list_prompts(context, cli).await?;
        let existing = listing
            .get("prompts")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            })
            .cloned()
            .ok_or_else(|| {
                ApiError::not_found(format!("Prompt preset '{id}' was not found in '{cli}'"))
            })?;
        if !has_name {
            body["name"] = existing.get("name").cloned().unwrap_or(Value::Null);
        }
        if !has_content {
            body["content"] = existing.get("content").cloned().unwrap_or(Value::Null);
        }
    }
    macro_rules! update {
        ($module:path, $input:ty) => {{
            let mut value = body;
            if !value.is_object() {
                return Err(ApiError::bad_request(
                    "Prompt preset body must be an object",
                ));
            }
            value["id"] = json!(id);
            let input: $input = serde_json::from_value(value)
                .map_err(|error| invalid(format!("Invalid prompt preset: {error}")))?;
            let updated = $module(db_state(context), app(context), input)
                .await
                .map_err(internal)?;
            Ok(serde_json::to_value(updated)?)
        }};
    }
    match cli {
        "claudecode" => update!(
            crate::coding::claude_code::commands::update_claude_prompt_config,
            crate::coding::claude_code::types::ClaudePromptConfigInput
        ),
        "claudedesktop" => update!(
            crate::coding::claude_desktop::prompt::update_claude_desktop_prompt_config,
            crate::coding::claude_desktop::types::ClaudeDesktopPromptConfigInput
        ),
        "codex" => update!(
            crate::coding::codex::commands::update_codex_prompt_config,
            crate::coding::codex::types::CodexPromptConfigInput
        ),
        "grok" => update!(
            crate::coding::grok::commands::update_grok_prompt_config,
            crate::coding::grok::types::GrokPromptConfigInput
        ),
        "kimi" => update!(
            crate::coding::kimi::commands::update_kimi_prompt_config,
            crate::coding::kimi::types::KimiPromptConfigInput
        ),
        "geminicli" => update!(
            crate::coding::gemini_cli::commands::update_gemini_cli_prompt_config,
            crate::coding::gemini_cli::types::GeminiCliPromptConfigInput
        ),
        "antigravity" => update!(
            crate::coding::antigravity::commands::update_antigravity_prompt_config,
            crate::coding::antigravity::types::AntigravityPromptConfigInput
        ),
        "opencode" => update!(
            crate::coding::open_code::commands::update_opencode_prompt_config,
            crate::coding::open_code::types::OpenCodePromptConfigInput
        ),
        "pi" => update!(
            crate::coding::pi::commands::update_pi_prompt_config,
            crate::coding::pi::types::PiPromptConfigInput
        ),
        "oh_my_pi" => update!(
            crate::coding::oh_my_pi::commands::update_omp_prompt_config,
            crate::coding::oh_my_pi::types::OmpPromptConfigInput
        ),
        "omo_native" => update!(
            crate::coding::omo_native::prompt::update_omo_native_prompt_config,
            crate::coding::omo_native::types::OmoNativePromptConfigInput
        ),
        "hermes" => update!(
            crate::coding::hermes::commands::update_hermes_prompt_config,
            crate::coding::hermes::types::HermesPromptConfigInput
        ),
        "dsh" => update!(
            crate::coding::dsh::commands::update_dsh_prompt_config,
            crate::coding::dsh::types::DshPromptConfigInput
        ),
        "zcode" => update!(
            crate::coding::zcode::commands::update_zcode_prompt_config,
            crate::coding::zcode::types::ZcodePromptConfigInput
        ),
        "codebuddy" => update!(
            crate::coding::codebuddy::commands::update_codebuddy_prompt_config,
            crate::coding::codebuddy::types::CodebuddyPromptConfigInput
        ),
        "openclaw" | "workbuddy" => Err(cli_dispatch::unsupported(cli, "Global prompt presets")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

async fn delete_prompt(context: &ServerContext, cli: &str, id: &str) -> ApiResult<Value> {
    macro_rules! delete {
        ($path:path) => {{
            $path(db_state(context), app(context), id.to_string())
                .await
                .map_err(internal)?;
            Ok(json!({ "deleted": id }))
        }};
    }
    match cli {
        "claudecode" => delete!(crate::coding::claude_code::commands::delete_claude_prompt_config),
        "claudedesktop" => {
            delete!(crate::coding::claude_desktop::prompt::delete_claude_desktop_prompt_config)
        }
        "codex" => delete!(crate::coding::codex::commands::delete_codex_prompt_config),
        "grok" => delete!(crate::coding::grok::commands::delete_grok_prompt_config),
        "kimi" => delete!(crate::coding::kimi::commands::delete_kimi_prompt_config),
        "geminicli" => {
            delete!(crate::coding::gemini_cli::commands::delete_gemini_cli_prompt_config)
        }
        "antigravity" => {
            crate::coding::antigravity::commands::delete_antigravity_prompt_config(
                app(context),
                db_state(context),
                id.to_string(),
            )
            .await
            .map_err(internal)?;
            Ok(json!({ "deleted": id }))
        }
        "opencode" => delete!(crate::coding::open_code::commands::delete_opencode_prompt_config),
        "pi" => delete!(crate::coding::pi::commands::delete_pi_prompt_config),
        "oh_my_pi" => delete!(crate::coding::oh_my_pi::commands::delete_omp_prompt_config),
        "omo_native" => delete!(crate::coding::omo_native::prompt::delete_omo_native_prompt_config),
        "hermes" => delete!(crate::coding::hermes::commands::delete_hermes_prompt_config),
        "dsh" => delete!(crate::coding::dsh::commands::delete_dsh_prompt_config),
        "zcode" => delete!(crate::coding::zcode::commands::delete_zcode_prompt_config),
        "codebuddy" => delete!(crate::coding::codebuddy::commands::delete_codebuddy_prompt_config),
        "openclaw" | "workbuddy" => Err(cli_dispatch::unsupported(cli, "Global prompt presets")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

async fn apply_prompt(context: &ServerContext, cli: &str, id: &str) -> ApiResult<Value> {
    macro_rules! apply {
        ($path:path) => {{
            $path(db_state(context), app(context), id.to_string())
                .await
                .map_err(internal)?;
            Ok(json!({ "applied": id }))
        }};
    }
    match cli {
        "claudecode" => apply!(crate::coding::claude_code::commands::apply_claude_prompt_config),
        "claudedesktop" => {
            apply!(crate::coding::claude_desktop::prompt::apply_claude_desktop_prompt_config)
        }
        "codex" => apply!(crate::coding::codex::commands::apply_codex_prompt_config),
        "grok" => apply!(crate::coding::grok::commands::apply_grok_prompt_config),
        "kimi" => apply!(crate::coding::kimi::commands::apply_kimi_prompt_config),
        "geminicli" => apply!(crate::coding::gemini_cli::commands::apply_gemini_cli_prompt_config),
        "antigravity" => {
            crate::coding::antigravity::commands::apply_antigravity_prompt_config(
                app(context),
                db_state(context),
                id.to_string(),
            )
            .await
            .map_err(internal)?;
            Ok(json!({ "applied": id }))
        }
        "opencode" => apply!(crate::coding::open_code::commands::apply_opencode_prompt_config),
        "pi" => apply!(crate::coding::pi::commands::apply_pi_prompt_config),
        "oh_my_pi" => apply!(crate::coding::oh_my_pi::commands::apply_omp_prompt_config),
        "omo_native" => apply!(crate::coding::omo_native::prompt::apply_omo_native_prompt_config),
        "hermes" => apply!(crate::coding::hermes::commands::apply_hermes_prompt_config),
        "dsh" => apply!(crate::coding::dsh::commands::apply_dsh_prompt_config),
        "zcode" => apply!(crate::coding::zcode::commands::apply_zcode_prompt_config),
        "codebuddy" => apply!(crate::coding::codebuddy::commands::apply_codebuddy_prompt_config),
        "openclaw" | "workbuddy" => Err(cli_dispatch::unsupported(cli, "Global prompt presets")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

async fn disable_prompt(context: &ServerContext, cli: &str, body: Value) -> ApiResult<Value> {
    let explicit_id = body.get("id").and_then(Value::as_str).map(str::to_string);
    let id = match explicit_id {
        Some(id) => id,
        None => {
            let listing = list_prompts(context, cli).await?;
            listing
                .get("prompts")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items.iter().find(|item| {
                        item.get("isApplied").and_then(Value::as_bool) == Some(true)
                            || item.get("is_applied").and_then(Value::as_bool) == Some(true)
                    })
                })
                .and_then(|item| item.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    ApiError::bad_request(
                        "No prompt preset is currently applied; pass 'id' to disable a specific one",
                    )
                })?
        }
    };

    macro_rules! disable {
        ($path:path) => {{
            $path(db_state(context), app(context), id.clone())
                .await
                .map_err(internal)?;
            Ok(json!({ "disabled": id }))
        }};
    }
    match cli {
        "claudecode" => {
            disable!(crate::coding::claude_code::commands::disable_claude_prompt_config)
        }
        "claudedesktop" => {
            disable!(crate::coding::claude_desktop::prompt::disable_claude_desktop_prompt_config)
        }
        "codex" => disable!(crate::coding::codex::commands::disable_codex_prompt_config),
        "grok" => disable!(crate::coding::grok::commands::disable_grok_prompt_config),
        "kimi" => disable!(crate::coding::kimi::commands::disable_kimi_prompt_config),
        "geminicli" => {
            disable!(crate::coding::gemini_cli::commands::disable_gemini_cli_prompt_config)
        }
        "antigravity" => {
            crate::coding::antigravity::commands::disable_antigravity_prompt_config(
                app(context),
                db_state(context),
                id.clone(),
            )
            .await
            .map_err(internal)?;
            Ok(json!({ "disabled": id }))
        }
        "opencode" => disable!(crate::coding::open_code::commands::disable_opencode_prompt_config),
        "pi" => disable!(crate::coding::pi::commands::disable_pi_prompt_config),
        "oh_my_pi" => disable!(crate::coding::oh_my_pi::commands::disable_omp_prompt_config),
        "omo_native" => {
            disable!(crate::coding::omo_native::prompt::disable_omo_native_prompt_config)
        }
        "hermes" => disable!(crate::coding::hermes::commands::disable_hermes_prompt_config),
        "dsh" => disable!(crate::coding::dsh::commands::disable_dsh_prompt_config),
        "zcode" => disable!(crate::coding::zcode::commands::disable_zcode_prompt_config),
        "codebuddy" => {
            disable!(crate::coding::codebuddy::commands::disable_codebuddy_prompt_config)
        }
        "openclaw" | "workbuddy" => Err(cli_dispatch::unsupported(cli, "Global prompt presets")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

// ---------------------------------------------------------------------------
// Runtime paths
// ---------------------------------------------------------------------------

async fn paths_for_cli(context: &ServerContext, cli: &str) -> ApiResult<Value> {
    let root_info = |value: &Value| -> (Option<String>, Option<String>) {
        (
            value
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_string),
            value
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_string),
        )
    };
    let string_path = |result: Result<std::path::PathBuf, String>| {
        result
            .map(|path| Some(path.to_string_lossy().to_string()))
            .unwrap_or(None)
    };

    let (root_dir, config_path, prompt_path, source) = match cli {
        "claudecode" => {
            let info = serde_json::to_value(
                crate::coding::claude_code::commands::get_claude_root_path_info(db_state(context))
                    .await
                    .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            (
                root,
                string_path(
                    runtime_location::get_claude_settings_path_async(&db_state(context)).await,
                ),
                string_path(
                    runtime_location::get_claude_prompt_path_async(&db_state(context)).await,
                ),
                source,
            )
        }
        "claudedesktop" => {
            let paths = crate::coding::claude_desktop::commands::get_claude_desktop_paths()
                .await
                .map_err(internal)?;
            (
                paths.profile_path.clone(),
                paths
                    .threep_config_path
                    .clone()
                    .or(paths.normal_config_path.clone()),
                None,
                Some("resolved".to_string()),
            )
        }
        "codex" => {
            let info = serde_json::to_value(
                crate::coding::codex::commands::get_codex_root_path_info(db_state(context))
                    .await
                    .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            (
                root,
                string_path(
                    runtime_location::get_codex_config_path_async(&db_state(context)).await,
                ),
                string_path(
                    runtime_location::get_codex_prompt_path_async(&db_state(context)).await,
                ),
                source,
            )
        }
        "grok" => {
            let info = serde_json::to_value(
                crate::coding::grok::commands::get_grok_root_path_info(db_state(context))
                    .await
                    .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            (
                root,
                string_path(runtime_location::get_grok_config_path_async(&db_state(context)).await),
                string_path(runtime_location::get_grok_prompt_path_async(&db_state(context)).await),
                source,
            )
        }
        "kimi" => {
            let info = serde_json::to_value(
                crate::coding::kimi::commands::get_kimi_root_path_info(db_state(context))
                    .await
                    .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            (
                root,
                string_path(runtime_location::get_kimi_config_path_async(&db_state(context)).await),
                string_path(runtime_location::get_kimi_prompt_path_async(&db_state(context)).await),
                source,
            )
        }
        "geminicli" => {
            let info = serde_json::to_value(
                crate::coding::gemini_cli::commands::get_gemini_cli_root_path_info(db_state(
                    context,
                ))
                .await
                .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            (
                root,
                crate::coding::gemini_cli::commands::get_gemini_cli_config_path(db_state(context))
                    .await
                    .ok(),
                string_path(
                    runtime_location::get_gemini_cli_prompt_path_async(&db_state(context)).await,
                ),
                source,
            )
        }
        "antigravity" => {
            let info = serde_json::to_value(
                crate::coding::antigravity::commands::get_antigravity_root_path_info(db_state(
                    context,
                ))
                .await
                .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            (
                root,
                crate::coding::antigravity::commands::get_antigravity_config_path(db_state(
                    context,
                ))
                .await
                .ok(),
                string_path(
                    runtime_location::get_antigravity_prompt_path_async(&db_state(context)).await,
                ),
                source,
            )
        }
        "zcode" => {
            let info = serde_json::to_value(
                crate::coding::zcode::commands::get_zcode_root_path_info(db_state(context))
                    .await
                    .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            (
                root,
                string_path(
                    runtime_location::get_zcode_config_path_async(&db_state(context)).await,
                ),
                string_path(
                    runtime_location::get_zcode_prompt_path_async(&db_state(context)).await,
                ),
                source,
            )
        }
        "opencode" => {
            let info = serde_json::to_value(
                crate::coding::open_code::commands::get_opencode_config_path_info(db_state(
                    context,
                ))
                .await
                .map_err(internal)?,
            )?;
            let (config, source) = root_info(&info);
            (
                config.clone(),
                config,
                string_path(
                    runtime_location::get_opencode_prompt_path_async(&db_state(context)).await,
                ),
                source,
            )
        }
        "openclaw" => {
            let info = serde_json::to_value(
                crate::coding::open_claw::commands::get_openclaw_config_path_info(db_state(
                    context,
                ))
                .await
                .map_err(internal)?,
            )?;
            let (config, source) = root_info(&info);
            (config.clone(), config, None, source)
        }
        "pi" => {
            let runtime = crate::coding::pi::commands::read_pi_runtime_config(db_state(context))
                .await
                .map_err(internal)?;
            (
                Some(runtime.root_path_info.path),
                Some(runtime.settings_path),
                Some(runtime.prompt_path),
                Some(runtime.root_path_info.source),
            )
        }
        "oh_my_pi" => {
            let runtime =
                crate::coding::oh_my_pi::commands::read_omp_runtime_config(db_state(context))
                    .await
                    .map_err(internal)?;
            (
                Some(runtime.root_path_info.path),
                Some(runtime.config_path),
                Some(runtime.prompt_path),
                Some(runtime.root_path_info.source),
            )
        }
        "omo_native" => {
            let runtime = crate::coding::omo_native::commands::read_omo_native_runtime_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            (
                None,
                Some(runtime.config_path),
                string_path(
                    crate::coding::omo_native::prompt::get_omo_native_prompt_path_async(&db_state(
                        context,
                    ))
                    .await,
                ),
                Some("runtime".to_string()),
            )
        }
        "hermes" => {
            let runtime =
                crate::coding::hermes::commands::read_hermes_runtime_config(db_state(context))
                    .await
                    .map_err(internal)?;
            (
                Some(runtime.root_path_info.path),
                Some(runtime.config_path),
                Some(runtime.prompt_path),
                Some(runtime.root_path_info.source),
            )
        }
        "dsh" => {
            let runtime = crate::coding::dsh::commands::read_dsh_runtime_config(db_state(context))
                .await
                .map_err(internal)?;
            (
                Some(runtime.root_path_info.path),
                Some(runtime.config_path),
                Some(runtime.prompt_path),
                Some(runtime.root_path_info.source),
            )
        }
        "workbuddy" => {
            let info = serde_json::to_value(
                crate::coding::workbuddy::commands::get_workbuddy_path_info(db_state(context))
                    .await
                    .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            let runtime = crate::coding::workbuddy::commands::read_workbuddy_runtime_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            (root, Some(runtime.path), None, source)
        }
        "codebuddy" => {
            let info = serde_json::to_value(
                crate::coding::codebuddy::commands::get_codebuddy_path_info(db_state(context))
                    .await
                    .map_err(internal)?,
            )?;
            let (root, source) = root_info(&info);
            let runtime = crate::coding::codebuddy::commands::read_codebuddy_runtime_config(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            (root, Some(runtime.path), None, source)
        }
        other => return Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    };

    let module = match cli {
        "claudecode" => Some("claude"),
        "geminicli" => Some("gemini"),
        other => Some(other),
    };
    let wsl = match module {
        Some(module) => {
            runtime_location::get_wsl_direct_status_for_module_async(&db_state(context), module)
                .await
                .ok()
        }
        None => None,
    };
    Ok(json!({
        "cli": cli,
        "rootDir": root_dir,
        "configPath": config_path,
        "promptPath": prompt_path,
        "source": source,
        "isWslDirect": wsl.as_ref().map(|status| status.is_wsl_direct).unwrap_or(false),
        "wsl": wsl.map(|status| json!({
            "distro": status.distro,
            "linuxPath": status.linux_path,
            "linuxUserRoot": status.linux_user_root,
        })),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_patch_preserves_masked_secrets_and_applies_plain_fields() {
        let existing = r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-real","FOO":"old"},"model":"m1"}"#;
        let incoming = json!({
            "env": {"ANTHROPIC_AUTH_TOKEN": "***", "FOO": "new"},
            "model": "m2"
        });
        let merged = apply_config_patch(existing, &incoming, "json", "patch").unwrap();
        let parsed: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-real");
        assert_eq!(parsed["env"]["FOO"], "new");
        assert_eq!(parsed["model"], "m2");
    }

    #[test]
    fn json_replace_restores_masked_values_from_the_existing_tree() {
        let existing = r#"{"env":{"KEY":"sk-real","KEEP":"yes"},"model":"m1"}"#;
        let incoming = json!({
            "env": {"KEY": "***", "NEW": "value"},
            "model": "m2"
        });
        let replaced = apply_config_patch(existing, &incoming, "json", "replace").unwrap();
        let parsed: Value = serde_json::from_str(&replaced).unwrap();
        assert_eq!(parsed["env"]["KEY"], "sk-real");
        assert_eq!(parsed["env"]["NEW"], "value");
        // Replace mode drops keys the caller did not send.
        assert!(parsed["env"].get("KEEP").is_none());
        assert_eq!(parsed["model"], "m2");
    }

    #[test]
    fn config_formats_match_each_cli_file() {
        assert_eq!(config_format("codex"), "toml");
        assert_eq!(config_format("grok"), "toml");
        assert_eq!(config_format("kimi"), "toml");
        assert_eq!(config_format("zcode"), "toml");
        assert_eq!(config_format("oh_my_pi"), "yaml");
        assert_eq!(config_format("omo_native"), "jsonc");
        assert_eq!(config_format("claudecode"), "json");
        assert_eq!(config_kind("codebuddy"), "other");
        assert_eq!(config_kind("claudecode"), "common");
        // ZCode's CLI-scope file is product-owned JSON, not the TOML common config.
        assert_eq!(config_format_for_scope("zcode", "cli"), "json");
        assert_eq!(config_format_for_scope("zcode", "common"), "toml");
        assert_eq!(config_format_for_scope("codex", "cli"), "toml");
    }

    #[test]
    fn legacy_empty_toml_config_is_treated_as_an_empty_document() {
        assert_eq!(parse_config_text("{}", "toml"), json!(""));
        let patched = apply_config_patch("{}", &json!({"marker": "on"}), "toml", "patch").unwrap();
        assert!(patched.contains("marker = \"on\""));
    }

    #[test]
    fn dsh_profile_patch_refusal_is_reported_as_unsupported() {
        let error = dsh_other_settings_error(
            "dsh 0.1.7+ 已把配置迁到 profiles/<profile>/cordis.patch.yml".to_string(),
        );
        assert_eq!(error.status, 400);
        assert_eq!(error.code, "unsupported");
        let other = dsh_other_settings_error("disk full".to_string());
        assert_eq!(other.code, "internal_error");
    }
}
