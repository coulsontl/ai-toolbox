mod args;
mod discovery;
mod http_client;

use std::io::Read;

use args::{
    Cli, CliCommand, Command, ConfigCommand, ConfigMode, GatewayCommand, ModelCommand,
    PluginCommand, PluginSettingsFields, PromptCommand, ProviderCommand, ProviderFields,
    TakeoverMode,
};
use http_client::{encode_query, encode_segment, Client, Method, RemoteError, TransportError};
use serde_json::{json, Map, Value};

fn main() {
    let cli = match <Cli as clap::Parser>::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // Clap already distinguishes --help/--version; usage errors exit 2.
            error.exit();
        }
    };
    match run(&cli) {
        Ok(value) => {
            print_value(cli.json, &value);
        }
        Err(failure) => {
            let code = failure.exit_code();
            if cli.json {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "error": {
                            "code": failure.code(),
                            "message": failure.message(),
                        }
                    })
                );
            } else {
                eprintln!("aitoolbox: {}", failure.message());
            }
            std::process::exit(code);
        }
    }
}

#[derive(Debug)]
enum Failure {
    Usage(String),
    Unavailable(String),
    Denied(RemoteError),
    Failed(RemoteError),
}

impl Failure {
    fn exit_code(&self) -> i32 {
        match self {
            Failure::Usage(_) => 2,
            Failure::Unavailable(_) => 3,
            Failure::Denied(_) => 4,
            Failure::Failed(_) => 5,
        }
    }

    /// The stable machine code written to `--json`. Server codes pass through
    /// unchanged (`approval_denied`, `approval_timeout`, `read_only`,
    /// `unsupported`, ...); local failures get a CLI-local code.
    fn code(&self) -> &str {
        match self {
            Failure::Usage(_) => "usage_error",
            Failure::Unavailable(_) => "app_not_running",
            Failure::Denied(error) | Failure::Failed(error) => &error.code,
        }
    }

    fn message(&self) -> &str {
        match self {
            Failure::Usage(message) | Failure::Unavailable(message) => message,
            Failure::Denied(error) | Failure::Failed(error) => &error.message,
        }
    }
}

impl From<TransportError> for Failure {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Unavailable(message) => Failure::Unavailable(message),
            TransportError::Denied(error) => Failure::Denied(error),
            TransportError::Failed(error) => Failure::Failed(error),
        }
    }
}

fn run(cli: &Cli) -> Result<Value, Failure> {
    match &cli.command {
        Command::Status => {
            let client = connect(cli)?;
            Ok(client.request(Method::Get, "/v1/status", None)?.body)
        }
        Command::Plugin { command } => run_plugin(cli, command),
        Command::Cli { command } => run_cli_command(cli, command),
        Command::Provider { command } => run_provider(cli, command),
        Command::Model { command } => run_model(cli, command),
        Command::Config { command } => run_config(cli, command),
        Command::Prompt { command } => run_prompt(cli, command),
        Command::Gateway { command } => run_gateway(cli, command),
    }
}

fn connect(cli: &Cli) -> Result<Client, Failure> {
    connect_with_timeout(cli, cli.timeout)
}

fn connect_with_timeout(cli: &Cli, timeout_secs: u64) -> Result<Client, Failure> {
    let path = cli
        .control_file
        .clone()
        .unwrap_or_else(discovery::default_path);
    let discovery = discovery::read(&path).map_err(Failure::Unavailable)?;
    let (host, port) = discovery.endpoint().map_err(Failure::Unavailable)?;
    Ok(Client::new(host, port, discovery.token, timeout_secs))
}

/// `bun add` can legitimately run for up to five minutes. Keep the local
/// socket alive long enough for one update, or for an update-all pass.
fn plugin_command_timeout(command: &PluginCommand, base: u64) -> u64 {
    match command {
        PluginCommand::Update {
            check_only: false,
            id: Some(_),
        } => base.max(600),
        PluginCommand::Update {
            check_only: false,
            id: None,
        } => base.max(1800),
        _ => base,
    }
}

fn run_plugin(cli: &Cli, command: &PluginCommand) -> Result<Value, Failure> {
    let client = connect_with_timeout(cli, plugin_command_timeout(command, cli.timeout))?;
    match command {
        PluginCommand::List => Ok(client.request(Method::Get, "/v1/plugins", None)?.body),
        PluginCommand::Get { id } => Ok(client
            .request(
                Method::Get,
                &format!("/v1/plugins/{}", encode_segment(id)),
                None,
            )?
            .body),
        PluginCommand::Install { spec, market_id } => {
            let body = json!({
                "spec": spec,
                "marketId": market_id,
            });
            Ok(client
                .request(Method::Post, "/v1/plugins/install", Some(&body))?
                .body)
        }
        PluginCommand::Reload { id } => {
            let body = json!({ "id": id });
            Ok(client
                .request(Method::Post, "/v1/plugins/reload", Some(&body))?
                .body)
        }
        PluginCommand::Update { id, check_only } => {
            let body = json!({ "id": id, "checkOnly": check_only });
            Ok(client
                .request(Method::Post, "/v1/plugins/update", Some(&body))?
                .body)
        }
        PluginCommand::Enable { id } => Ok(client
            .request(
                Method::Post,
                &format!("/v1/plugins/{}/enable", encode_segment(id)),
                Some(&json!({})),
            )?
            .body),
        PluginCommand::Disable { id } => Ok(client
            .request(
                Method::Post,
                &format!("/v1/plugins/{}/disable", encode_segment(id)),
                Some(&json!({})),
            )?
            .body),
        PluginCommand::Remove { id, purge_data } => Ok(client
            .request(
                Method::Delete,
                &format!(
                    "/v1/plugins/{}?purgeData={}",
                    encode_segment(id),
                    purge_data
                ),
                None,
            )?
            .body),
        PluginCommand::Options { id, set } => match set {
            Some(raw) => {
                let options = parse_json_object(raw, "--set")?;
                let body = json!({ "options": options });
                Ok(client
                    .request(
                        Method::Put,
                        &format!("/v1/plugins/{}/options", encode_segment(id)),
                        Some(&body),
                    )?
                    .body)
            }
            None => Ok(client
                .request(
                    Method::Get,
                    &format!("/v1/plugins/{}/options", encode_segment(id)),
                    None,
                )?
                .body),
        },
        PluginCommand::Logs { limit } => Ok(client
            .request(
                Method::Get,
                &format!("/v1/plugins/logs?limit={limit}"),
                None,
            )?
            .body),
        PluginCommand::ClearLogs => Ok(client
            .request(Method::Delete, "/v1/plugins/logs", None)?
            .body),
        PluginCommand::Middleware => Ok(client
            .request(Method::Get, "/v1/plugins/middleware", None)?
            .body),
        PluginCommand::Providers => Ok(client
            .request(Method::Get, "/v1/plugins/providers", None)?
            .body),
        PluginCommand::ProviderOptions { provider, set } => match set {
            Some(raw) => {
                let options = parse_json_object(raw, "--set")?;
                let body = json!({ "options": options });
                Ok(client
                    .request(
                        Method::Put,
                        &format!("/v1/plugins/providers/{}/options", encode_segment(provider)),
                        Some(&body),
                    )?
                    .body)
            }
            None => Ok(client
                .request(
                    Method::Get,
                    &format!("/v1/plugins/providers/{}/options", encode_segment(provider)),
                    None,
                )?
                .body),
        },
        PluginCommand::ProviderAutoCheckin { provider, enabled } => Ok(client
            .request(
                Method::Put,
                &format!(
                    "/v1/plugins/providers/{}/auto-checkin",
                    encode_segment(provider)
                ),
                Some(&json!({ "enabled": enabled })),
            )?
            .body),
        PluginCommand::Settings { fields } => {
            let patch = settings_patch(fields)?;
            if patch.as_object().is_some_and(|object| object.is_empty()) {
                Ok(client.request(Method::Get, "/v1/plugins/settings", None)?.body)
            } else {
                Ok(client
                    .request(Method::Put, "/v1/plugins/settings", Some(&patch))?
                    .body)
            }
        }
        PluginCommand::BunStatus => Ok(client.request(Method::Get, "/v1/plugins/bun", None)?.body),
        PluginCommand::BunEnsure => Ok(client
            .request(Method::Post, "/v1/plugins/bun/ensure", Some(&json!({})))?
            .body),
        PluginCommand::BunProbe { path } => Ok(client
            .request(
                Method::Post,
                "/v1/plugins/bun/probe",
                Some(&json!({ "path": path })),
            )?
            .body),
        PluginCommand::HostStatus => {
            Ok(client.request(Method::Get, "/v1/plugins/host", None)?.body)
        }
        PluginCommand::HostRestart => Ok(client
            .request(Method::Post, "/v1/plugins/host/restart", Some(&json!({})))?
            .body),
        PluginCommand::Pages => Ok(client
            .request(Method::Get, "/v1/plugins/pages", None)?
            .body),
        PluginCommand::RefreshModels => Ok(client
            .request(
                Method::Post,
                "/v1/plugins/providers/refresh-models",
                Some(&json!({})),
            )?
            .body),
        PluginCommand::LinkCli {
            provider,
            json_body,
        } => {
            let text = read_source(json_body)?;
            let input: Value = serde_json::from_str(&text).map_err(|error| {
                Failure::Usage(format!("--json-body is not valid JSON: {error}"))
            })?;
            if !input.is_object() {
                return Err(Failure::Usage(
                    "--json-body must contain a JSON object".to_string(),
                ));
            }
            Ok(client
                .request(
                    Method::Post,
                    &format!(
                        "/v1/plugins/providers/{}/link-cli",
                        encode_segment(provider)
                    ),
                    Some(&input),
                )?
                .body)
        }
        PluginCommand::Accounts => Ok(client
            .request(Method::Get, "/v1/plugins/accounts", None)?
            .body),
        PluginCommand::AccountCheck {
            provider,
            account,
            proxy,
        } => Ok(client
            .request(
                Method::Post,
                "/v1/plugins/accounts/check",
                Some(&json!({
                    "provider": provider,
                    "account": account,
                    "proxy": proxy,
                })),
            )?
            .body),
        PluginCommand::AccountUsage {
            provider,
            account,
            proxy,
        } => Ok(client
            .request(
                Method::Post,
                "/v1/plugins/accounts/usage",
                Some(&json!({
                    "provider": provider,
                    "account": account,
                    "proxy": proxy,
                })),
            )?
            .body),
        PluginCommand::AccountCheckin {
            provider,
            account,
            proxy,
        } => Ok(client
            .request(
                Method::Post,
                "/v1/plugins/accounts/checkin",
                Some(&json!({
                    "provider": provider,
                    "account": account,
                    "proxy": proxy,
                })),
            )?
            .body),
    }
}

fn run_cli_command(cli: &Cli, command: &CliCommand) -> Result<Value, Failure> {
    let client = connect(cli)?;
    match command {
        CliCommand::List => Ok(client.request(Method::Get, "/v1/clis", None)?.body),
        CliCommand::Paths { cli: key } => {
            check_cli_key(key)?;
            Ok(client
                .request(
                    Method::Get,
                    &format!("/v1/clis/{}/paths", encode_segment(key)),
                    None,
                )?
                .body)
        }
    }
}

fn run_provider(cli: &Cli, command: &ProviderCommand) -> Result<Value, Failure> {
    let client = connect(cli)?;
    match command {
        ProviderCommand::List { cli: key } => {
            check_cli_key(key)?;
            Ok(client
                .request(
                    Method::Get,
                    &format!("/v1/clis/{}/providers", encode_segment(key)),
                    None,
                )?
                .body)
        }
        ProviderCommand::Get { cli: key, id } => {
            check_cli_key(key)?;
            Ok(client
                .request(
                    Method::Get,
                    &format!(
                        "/v1/clis/{}/providers/{}",
                        encode_segment(key),
                        encode_segment(id)
                    ),
                    None,
                )?
                .body)
        }
        ProviderCommand::Create { cli: key, fields } => {
            check_cli_key(key)?;
            let body = provider_body(fields)?;
            Ok(client
                .request(
                    Method::Post,
                    &format!("/v1/clis/{}/providers", encode_segment(key)),
                    Some(&body),
                )?
                .body)
        }
        ProviderCommand::Update {
            cli: key,
            id,
            fields,
        } => {
            check_cli_key(key)?;
            let body = provider_body(fields)?;
            Ok(client
                .request(
                    Method::Patch,
                    &format!(
                        "/v1/clis/{}/providers/{}",
                        encode_segment(key),
                        encode_segment(id)
                    ),
                    Some(&body),
                )?
                .body)
        }
        ProviderCommand::Delete { cli: key, id } => {
            check_cli_key(key)?;
            Ok(client
                .request(
                    Method::Delete,
                    &format!(
                        "/v1/clis/{}/providers/{}",
                        encode_segment(key),
                        encode_segment(id)
                    ),
                    None,
                )?
                .body)
        }
        ProviderCommand::Apply {
            cli: key,
            id,
            model,
        } => {
            check_cli_key(key)?;
            let body = json!({ "modelId": model });
            Ok(client
                .request(
                    Method::Post,
                    &format!(
                        "/v1/clis/{}/providers/{}/apply",
                        encode_segment(key),
                        encode_segment(id)
                    ),
                    Some(&body),
                )?
                .body)
        }
        ProviderCommand::Test {
            cli: key,
            id,
            model,
            prompt,
        } => {
            check_cli_key(key)?;
            let body = json!({
                "modelIds": model.iter().cloned().collect::<Vec<_>>(),
                "prompt": prompt,
            });
            Ok(client
                .request(
                    Method::Post,
                    &format!(
                        "/v1/clis/{}/providers/{}/test",
                        encode_segment(key),
                        encode_segment(id)
                    ),
                    Some(&body),
                )?
                .body)
        }
        ProviderCommand::FetchModels { cli: key, id } => {
            check_cli_key(key)?;
            Ok(client
                .request(
                    Method::Post,
                    &format!(
                        "/v1/clis/{}/providers/{}/fetch-models",
                        encode_segment(key),
                        encode_segment(id)
                    ),
                    Some(&json!({})),
                )?
                .body)
        }
    }
}

fn run_model(cli: &Cli, command: &ModelCommand) -> Result<Value, Failure> {
    let client = connect(cli)?;
    match command {
        ModelCommand::List { cli: key, provider } => Ok(client
            .request(
                Method::Get,
                &format!(
                    "/v1/clis/{}/providers/{}/models",
                    encode_segment(key),
                    encode_segment(provider)
                ),
                None,
            )?
            .body),
        ModelCommand::Add {
            cli: key,
            provider,
            id,
            name,
        } => {
            let body = json!({ "modelId": id, "name": name });
            Ok(client
                .request(
                    Method::Post,
                    &format!(
                        "/v1/clis/{}/providers/{}/models",
                        encode_segment(key),
                        encode_segment(provider)
                    ),
                    Some(&body),
                )?
                .body)
        }
        ModelCommand::Delete {
            cli: key,
            provider,
            id,
        } => Ok(client
            .request(
                Method::Delete,
                &format!(
                    "/v1/clis/{}/providers/{}/models/{}",
                    encode_segment(key),
                    encode_segment(provider),
                    encode_segment(id)
                ),
                None,
            )?
            .body),
        ModelCommand::Default {
            cli: key,
            provider,
            id,
        } => {
            let body = json!({ "modelId": id });
            Ok(client
                .request(
                    Method::Put,
                    &format!(
                        "/v1/clis/{}/providers/{}/default-model",
                        encode_segment(key),
                        encode_segment(provider)
                    ),
                    Some(&body),
                )?
                .body)
        }
    }
}

fn run_config(cli: &Cli, command: &ConfigCommand) -> Result<Value, Failure> {
    let client = connect(cli)?;
    match command {
        ConfigCommand::Get { cli: key, scope } => {
            check_cli_key(key)?;
            let path = with_scope(
                format!("/v1/clis/{}/config", encode_segment(key)),
                scope.as_deref(),
            );
            Ok(client.request(Method::Get, &path, None)?.body)
        }
        ConfigCommand::Set {
            cli: key,
            file,
            mode,
            root_dir,
            clear_root_dir,
            scope,
        } => {
            check_cli_key(key)?;
            let text = read_source(file)?;
            let config = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
            let body = json!({
                "config": config,
                "mode": match mode {
                    ConfigMode::Patch => "patch",
                    ConfigMode::Replace => "replace",
                },
                "rootDir": root_dir,
                "clearRootDir": clear_root_dir,
            });
            let path = with_scope(
                format!("/v1/clis/{}/config", encode_segment(key)),
                scope.as_deref(),
            );
            Ok(client.request(Method::Put, &path, Some(&body))?.body)
        }
        ConfigCommand::Extract { cli: key, scope } => {
            check_cli_key(key)?;
            let path = with_scope(
                format!("/v1/clis/{}/config/extract", encode_segment(key)),
                scope.as_deref(),
            );
            Ok(client.request(Method::Post, &path, Some(&json!({})))?.body)
        }
    }
}

fn run_prompt(cli: &Cli, command: &PromptCommand) -> Result<Value, Failure> {
    let client = connect(cli)?;
    match command {
        PromptCommand::List { cli: key } => Ok(client
            .request(
                Method::Get,
                &format!("/v1/clis/{}/prompts", encode_segment(key)),
                None,
            )?
            .body),
        PromptCommand::Create {
            cli: key,
            name,
            content_file,
        } => {
            check_cli_key(key)?;
            let content = read_source(content_file)?;
            let body = json!({ "name": name, "content": content });
            Ok(client
                .request(
                    Method::Post,
                    &format!("/v1/clis/{}/prompts", encode_segment(key)),
                    Some(&body),
                )?
                .body)
        }
        PromptCommand::Update {
            cli: key,
            id,
            name,
            content_file,
        } => {
            check_cli_key(key)?;
            // Either field may be updated alone. The backend fills the missing
            // half from the stored preset, so sending only what changed keeps
            // a rename from rewriting the content and vice versa.
            if name.is_none() && content_file.is_none() {
                return Err(Failure::Usage(
                    "prompt update needs --name and/or --content-file".to_string(),
                ));
            }
            let mut body = json!({ "id": id });
            if let Some(name) = name {
                body["name"] = json!(name);
            }
            if let Some(path) = content_file {
                body["content"] = json!(read_source(path)?);
            }
            Ok(client
                .request(
                    Method::Put,
                    &format!(
                        "/v1/clis/{}/prompts/{}",
                        encode_segment(key),
                        encode_segment(id)
                    ),
                    Some(&body),
                )?
                .body)
        }
        PromptCommand::Delete { cli: key, id } => Ok(client
            .request(
                Method::Delete,
                &format!(
                    "/v1/clis/{}/prompts/{}",
                    encode_segment(key),
                    encode_segment(id)
                ),
                None,
            )?
            .body),
        PromptCommand::Apply { cli: key, id } => Ok(client
            .request(
                Method::Post,
                &format!(
                    "/v1/clis/{}/prompts/{}/apply",
                    encode_segment(key),
                    encode_segment(id)
                ),
                Some(&json!({})),
            )?
            .body),
        PromptCommand::Disable { cli: key, id } => Ok(client
            .request(
                Method::Post,
                &format!("/v1/clis/{}/prompts/disable", encode_segment(key)),
                Some(&json!({ "id": id })),
            )?
            .body),
    }
}

fn run_gateway(cli: &Cli, command: &GatewayCommand) -> Result<Value, Failure> {
    let client = connect(cli)?;
    match command {
        GatewayCommand::Status => Ok(client
            .request(Method::Get, "/v1/gateway/status", None)?
            .body),
        GatewayCommand::Start => Ok(client
            .request(Method::Post, "/v1/gateway/start", Some(&json!({})))?
            .body),
        GatewayCommand::Stop => Ok(client
            .request(Method::Post, "/v1/gateway/stop", Some(&json!({})))?
            .body),
        GatewayCommand::Restart => Ok(client
            .request(Method::Post, "/v1/gateway/restart", Some(&json!({})))?
            .body),
        GatewayCommand::Logs {
            page,
            limit,
            only_failed,
            cli: key,
        } => {
            let mut path = format!("/v1/gateway/logs?page={page}&pageSize={limit}");
            if *only_failed {
                path.push_str("&onlyFailed=true");
            }
            if let Some(key) = key {
                path.push_str("&cliKey=");
                path.push_str(&encode_query(key));
            }
            Ok(client.request(Method::Get, &path, None)?.body)
        }
        GatewayCommand::Health => Ok(client
            .request(Method::Get, "/v1/gateway/health", None)?
            .body),
        GatewayCommand::ResetHealth => Ok(client
            .request(Method::Post, "/v1/gateway/health/reset", Some(&json!({})))?
            .body),
        GatewayCommand::Clis => Ok(client
            .request(Method::Get, "/v1/gateway/clis", None)?
            .body),
        GatewayCommand::Switch { cli: key, provider } => {
            let body = json!({ "cliKey": key, "providerId": provider });
            Ok(client
                .request(Method::Post, "/v1/gateway/takeover/switch", Some(&body))?
                .body)
        }
        GatewayCommand::Engage {
            cli: key,
            mode,
            provider_ids,
            separator,
        } => {
            match mode {
                TakeoverMode::Single if provider_ids.is_empty() => {
                    return Err(Failure::Usage(
                        "--mode single requires --provider <id>".to_string(),
                    ))
                }
                TakeoverMode::Aggregate if provider_ids.is_empty() => {
                    return Err(Failure::Usage(
                        "--mode aggregate requires at least one --provider".to_string(),
                    ))
                }
                _ => {}
            }
            let mode = match mode {
                TakeoverMode::Single => "single",
                TakeoverMode::Failover => "failover",
                TakeoverMode::Aggregate => "aggregate",
            };
            let body = json!({
                "cliKey": key,
                "mode": mode,
                "providerId": provider_ids.first(),
                "providerIds": provider_ids,
                "separator": separator,
            });
            Ok(client
                .request(Method::Post, "/v1/gateway/takeover/engage", Some(&body))?
                .body)
        }
        GatewayCommand::Restore { cli: key } => {
            let body = json!({ "cliKey": key });
            Ok(client
                .request(Method::Post, "/v1/gateway/takeover/restore", Some(&body))?
                .body)
        }
    }
}

fn with_scope(path: String, scope: Option<&str>) -> String {
    match scope {
        Some(scope) if !scope.is_empty() => format!("{path}?scope={}", encode_query(scope)),
        _ => path,
    }
}

/// Registry values must match the desktop settings page
/// (`web/features/coding/plugins/utils/pluginFormatters.ts`).
const NPM_REGISTRY_OFFICIAL: &str = "https://registry.npmjs.org";
const NPM_REGISTRY_CHINA: &str = "https://registry.npmmirror.com";

/// Builds the partial settings body for `plugin settings`. An empty object
/// means the caller passed no edit flags; the command then reads instead of
/// writing, because the server rejects a PUT with no known field.
fn settings_patch(fields: &PluginSettingsFields) -> Result<Value, Failure> {
    let mut patch = match &fields.json_body {
        Some(source) => {
            let text = read_source(source)?;
            let parsed: Value = serde_json::from_str(&text).map_err(|error| {
                Failure::Usage(format!("--json-body is not valid JSON: {error}"))
            })?;
            if !parsed.is_object() {
                return Err(Failure::Usage(
                    "--json-body must contain a JSON object".to_string(),
                ));
            }
            parsed
        }
        None => Value::Object(Map::new()),
    };

    if let Some(path) = &fields.bun_path {
        patch["bunManualPath"] = json!(path);
    }
    if fields.clear_bun_path {
        patch["bunManualPath"] = json!("");
    }
    if let Some(registry) = &fields.npm_registry {
        patch["npmRegistry"] = json!(registry);
    }
    if let Some(china_mirror) = fields.china_mirror {
        patch["npmRegistry"] = json!(if china_mirror {
            NPM_REGISTRY_CHINA
        } else {
            NPM_REGISTRY_OFFICIAL
        });
    }
    if let Some(follow_app_proxy) = fields.follow_app_proxy {
        patch["followAppProxy"] = json!(follow_app_proxy);
    }
    if let Some(auto_update) = fields.auto_update {
        patch["autoUpdateEnabled"] = json!(auto_update);
    }
    if let Some(auto_update_interval) = fields.auto_update_interval {
        patch["autoUpdateIntervalHours"] = json!(auto_update_interval);
    }
    Ok(patch)
}

fn provider_body(fields: &ProviderFields) -> Result<Value, Failure> {
    let mut body = match &fields.json_body {
        Some(source) => {
            let text = read_source(source)?;
            let parsed: Value = serde_json::from_str(&text).map_err(|error| {
                Failure::Usage(format!("--json-body is not valid JSON: {error}"))
            })?;
            if !parsed.is_object() {
                return Err(Failure::Usage(
                    "--json-body must contain a JSON object".to_string(),
                ));
            }
            parsed
        }
        None => Value::Object(Map::new()),
    };

    if let Some(name) = &fields.name {
        body["name"] = json!(name);
    }
    if let Some(base_url) = &fields.base_url {
        body["baseUrl"] = json!(base_url);
    }
    if let Some(model) = &fields.model {
        body["model"] = json!(model);
    }
    if let Some(api_key) = read_api_key(fields)? {
        body["apiKey"] = json!(api_key);
    }
    Ok(body)
}

fn read_api_key(fields: &ProviderFields) -> Result<Option<String>, Failure> {
    let value = if let Some(variable) = &fields.api_key_env {
        let value = std::env::var(variable)
            .map_err(|_| Failure::Usage(format!("Environment variable '{variable}' is not set")))?;
        Some(value)
    } else if fields.api_key_stdin {
        Some(read_stdin()?)
    } else {
        None
    };
    Ok(value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn read_source(source: &str) -> Result<String, Failure> {
    if source == "-" {
        read_stdin()
    } else {
        std::fs::read_to_string(source)
            .map_err(|error| Failure::Usage(format!("Cannot read {source}: {error}")))
    }
}

fn read_stdin() -> Result<String, Failure> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| Failure::Usage(format!("Cannot read stdin: {error}")))?;
    Ok(text)
}

fn parse_json_object(raw: &str, flag: &str) -> Result<Value, Failure> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| Failure::Usage(format!("{flag} is not valid JSON: {error}")))?;
    if !value.is_object() {
        return Err(Failure::Usage(format!("{flag} must be a JSON object")));
    }
    Ok(value)
}

fn check_cli_key(raw: &str) -> Result<(), Failure> {
    let normalized = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    let known = matches!(
        normalized.as_str(),
        "claudecode"
            | "claude"
            | "claude_code"
            | "claudedesktop"
            | "claude_desktop"
            | "codex"
            | "grok"
            | "kimi"
            | "geminicli"
            | "gemini_cli"
            | "gemini"
            | "antigravity"
            | "opencode"
            | "open_code"
            | "openclaw"
            | "open_claw"
            | "pi"
            | "oh_my_pi"
            | "omp"
            | "omo_native"
            | "omo"
            | "hermes"
            | "dsh"
            | "zcode"
            | "workbuddy"
            | "codebuddy"
    );
    if known {
        Ok(())
    } else {
        Err(Failure::Usage(format!(
            "Unknown CLI key '{raw}'. Run `aitoolbox cli list` to see the supported keys."
        )))
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn print_value(json_mode: bool, value: &Value) {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string())
        );
        return;
    }
    print_human(value);
}

fn print_human(value: &Value) {
    if value.is_null() {
        println!("ok");
        return;
    }
    if let Some(object) = value.as_object() {
        if object.contains_key("running") && object.contains_key("discoveryFile") {
            println!("enabled:        {}", bool_text(object.get("enabled")));
            println!("running:        {}", bool_text(object.get("running")));
            println!("url:            {}", text(object.get("url")));
            println!("approval mode:  {}", text(object.get("approvalMode")));
            println!("discovery file: {}", text(object.get("discoveryFile")));
            return;
        }
        for key in ["clis", "providers", "plugins", "prompts", "models", "items"] {
            if let Some(items) = object.get(key).and_then(Value::as_array) {
                if items.is_empty() {
                    println!("(none)");
                    return;
                }
                for item in items {
                    println!("{}", summarize_item(item));
                }
                return;
            }
        }
        if let Some(logs) = object.get("logs").and_then(Value::as_array) {
            for log in logs {
                println!("{}", serde_json::to_string(log).unwrap_or_default());
            }
            return;
        }
        if object.contains_key("ok") {
            println!("ok");
            return;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

fn summarize_item(item: &Value) -> String {
    let object = match item.as_object() {
        Some(object) => object,
        None => return item.to_string(),
    };
    let id = [
        "id",
        "providerKey",
        "providerId",
        "key",
        "cliKey",
        "pluginId",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(Value::as_str))
    .unwrap_or("?");
    let mut line = id.to_string();
    let name = ["displayName", "name", "packageName"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str));
    if let Some(name) = name {
        if name != id {
            line.push_str(&format!("  {name}"));
        }
    }
    if object.get("isApplied").and_then(Value::as_bool) == Some(true)
        || object.get("is_applied").and_then(Value::as_bool) == Some(true)
    {
        line.push_str("  [applied]");
    }
    if object.get("hasApiKey").and_then(Value::as_bool) == Some(true) {
        line.push_str("  [key set]");
    }
    if let Some(kind) = object.get("kind").and_then(Value::as_str) {
        line.push_str(&format!("  ({kind})"));
    }
    let current = object
        .get("currentVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let latest = object
        .get("latestVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !current.is_empty() && !latest.is_empty() {
        line.push_str(&format!("  v{current} -> v{latest}"));
    }
    if let Some(status) = object.get("status").and_then(Value::as_str) {
        line.push_str(&format!("  [{status}]"));
    }
    if let Some(message) = object.get("message").and_then(Value::as_str) {
        if !message.trim().is_empty() {
            line.push_str(&format!("  {message}"));
        }
    }
    line
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => "-".to_string(),
        Some(other) => other.to_string(),
    }
}

fn bool_text(value: Option<&Value>) -> &'static str {
    match value.and_then(Value::as_bool) {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn settings_fields() -> PluginSettingsFields {
        PluginSettingsFields {
            bun_path: None,
            clear_bun_path: false,
            npm_registry: None,
            china_mirror: None,
            follow_app_proxy: None,
            auto_update: None,
            auto_update_interval: None,
            json_body: None,
        }
    }

    #[test]
    fn settings_patch_is_empty_without_edit_flags() {
        let patch = settings_patch(&settings_fields()).unwrap();
        assert!(patch.as_object().unwrap().is_empty());
    }

    #[test]
    fn settings_patch_translates_the_china_mirror_switch() {
        let fields = PluginSettingsFields {
            china_mirror: Some(true),
            ..settings_fields()
        };
        let patch = settings_patch(&fields).unwrap();
        assert_eq!(patch["npmRegistry"], NPM_REGISTRY_CHINA);

        let fields = PluginSettingsFields {
            china_mirror: Some(false),
            ..settings_fields()
        };
        let patch = settings_patch(&fields).unwrap();
        assert_eq!(patch["npmRegistry"], NPM_REGISTRY_OFFICIAL);
    }

    #[test]
    fn settings_patch_clears_the_bun_path() {
        let fields = PluginSettingsFields {
            clear_bun_path: true,
            ..settings_fields()
        };
        let patch = settings_patch(&fields).unwrap();
        assert_eq!(patch["bunManualPath"], "");
    }

    #[test]
    fn settings_patch_maps_auto_update_fields() {
        let fields = PluginSettingsFields {
            auto_update: Some(true),
            auto_update_interval: Some(6),
            ..settings_fields()
        };
        let patch = settings_patch(&fields).unwrap();
        assert_eq!(patch["autoUpdateEnabled"], true);
        assert_eq!(patch["autoUpdateIntervalHours"], 6);
    }

    #[test]
    fn plugin_update_gets_a_longer_socket_timeout() {
        let base = 120;
        assert_eq!(
            plugin_command_timeout(
                &PluginCommand::Update {
                    id: Some("demo".to_string()),
                    check_only: false,
                },
                base,
            ),
            600
        );
        assert_eq!(
            plugin_command_timeout(
                &PluginCommand::Update {
                    id: None,
                    check_only: false,
                },
                base,
            ),
            1800
        );
        assert_eq!(
            plugin_command_timeout(
                &PluginCommand::Update {
                    id: Some("demo".to_string()),
                    check_only: true,
                },
                base,
            ),
            120
        );
        assert_eq!(plugin_command_timeout(&PluginCommand::List, base), 120);
    }

    #[test]
    fn summarize_item_shows_update_versions_and_status() {
        let item = json!({
            "pluginId": "demo",
            "displayName": "Demo",
            "currentVersion": "1.0.0",
            "latestVersion": "1.1.0",
            "status": "available",
            "message": "",
        });
        assert_eq!(
            summarize_item(&item),
            "demo  Demo  v1.0.0 -> v1.1.0  [available]"
        );
    }

    #[test]
    fn provider_body_reads_api_key_from_the_environment() {
        std::env::set_var("AITOOLBOX_TEST_KEY", "sk-test");
        let fields = ProviderFields {
            name: Some("Demo".to_string()),
            base_url: Some("https://example.test".to_string()),
            model: Some("demo-model".to_string()),
            api_key_env: Some("AITOOLBOX_TEST_KEY".to_string()),
            api_key_stdin: false,
            json_body: None,
        };
        let body = provider_body(&fields).unwrap();
        assert_eq!(body["name"], "Demo");
        assert_eq!(body["baseUrl"], "https://example.test");
        assert_eq!(body["apiKey"], "sk-test");
    }

    #[test]
    fn provider_body_merges_over_native_json() {
        let fields = ProviderFields {
            name: Some("Renamed".to_string()),
            base_url: None,
            model: None,
            api_key_env: None,
            api_key_stdin: false,
            json_body: None,
        };
        let body = provider_body(&fields).unwrap();
        assert_eq!(body["name"], "Renamed");
    }

    #[test]
    fn denial_families_share_exit_four_but_keep_the_server_code() {
        for code in [
            "approval_denied",
            "approval_timeout",
            "read_only",
            "unsupported",
        ] {
            let failure = Failure::from(TransportError::Denied(RemoteError {
                code: code.to_string(),
                message: "no".to_string(),
            }));
            assert_eq!(failure.exit_code(), 4, "{code} must exit 4");
            assert_eq!(failure.code(), code);
        }
        let execution = Failure::from(TransportError::Failed(RemoteError {
            code: "bad_request".to_string(),
            message: "fix it".to_string(),
        }));
        assert_eq!(execution.exit_code(), 5);
        assert_eq!(execution.code(), "bad_request");
    }

    #[test]
    fn unknown_cli_keys_are_usage_errors() {
        assert!(check_cli_key("claude").is_ok());
        assert!(check_cli_key("oh-my-pi").is_ok());
        assert!(check_cli_key("nope").is_err());
    }

    #[test]
    fn command_parse_accepts_quoted_paths() {
        let parsed = Cli::try_parse_from([
            "aitoolbox",
            "config",
            "set",
            "--cli",
            "codex",
            "--file",
            "-",
            "--mode",
            "replace",
        ])
        .unwrap();
        assert!(matches!(
            parsed.command,
            Command::Config {
                command: ConfigCommand::Set { .. }
            }
        ));
    }
}
