//! Command-line surface for `aitoolbox`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "aitoolbox",
    version,
    about = "Local Agent Control API client for AI Toolbox",
    long_about = "Talk to the running AI Toolbox desktop app through its loopback control API. The bearer token is read from ~/.ai-toolbox/control.json (or AI_TOOLBOX_CONTROL_FILE) and is never printed."
)]
pub struct Cli {
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    pub json: bool,
    /// Request timeout in seconds. Keep it above the desktop approval timeout
    /// (`approvalTimeoutSecs` in the control API settings, default 60) or a
    /// write that is waiting for the user's click will be cut off locally.
    #[arg(long, global = true, default_value_t = 120)]
    pub timeout: u64,
    /// Override the discovery file path
    #[arg(long, global = true, value_name = "PATH")]
    pub control_file: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the control-plane status
    Status,
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// List CLI capabilities
    Cli {
        #[command(subcommand)]
        command: CliCommand,
    },
    /// Manage providers (channels)
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    /// Manage provider models
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Read or write a CLI's global config file
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage global prompt presets
    Prompt {
        #[command(subcommand)]
        command: PromptCommand,
    },
    /// Control the local proxy gateway
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// List installed plugins
    List,
    /// Show one plugin
    Get {
        id: String,
    },
    /// Install a plugin from a local path, npm package or github: spec
    Install {
        spec: String,
        #[arg(long)]
        market_id: Option<String>,
    },
    /// Reload all plugins, or one plugin when an id is given
    Reload {
        id: Option<String>,
    },
    /// Check for plugin updates, or update them (latest npm / GitHub reinstall)
    Update {
        id: Option<String>,
        /// Only report available updates; do not install anything
        #[arg(long)]
        check_only: bool,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Uninstall a plugin
    Remove {
        id: String,
        #[arg(long)]
        purge_data: bool,
    },
    /// Read plugin options, or replace them with --set
    Options {
        id: String,
        /// JSON object with the new options
        #[arg(long, value_name = "JSON")]
        set: Option<String>,
    },
    /// Read the runtime log buffer
    Logs {
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Clear the runtime log buffer
    ClearLogs,
    /// Show middleware states
    Middleware,
    /// List plugin providers
    Providers,
    /// Read provider options, or replace them with --set
    ProviderOptions {
        provider: String,
        /// JSON object with the new options
        #[arg(long, value_name = "JSON")]
        set: Option<String>,
    },
    /// Enable or disable a provider's daily auto check-in
    ProviderAutoCheckin {
        provider: String,
        /// Whether the background pass should check this provider in daily
        #[arg(long, value_name = "true|false", action = clap::ArgAction::Set)]
        enabled: bool,
    },
    /// Read the global plugin settings, or update them with flags / --json-body
    Settings {
        #[command(flatten)]
        fields: PluginSettingsFields,
    },
    /// Show the Bun runtime status
    BunStatus,
    /// Download (or verify) the managed Bun runtime
    BunEnsure,
    /// Probe one Bun executable path
    BunProbe {
        path: String,
    },
    /// Show the Bun plugin host status
    HostStatus,
    /// Restart the Bun plugin host
    HostRestart,
    /// List custom pages of enabled plugins
    Pages,
    /// Refresh the model list of every plugin provider
    RefreshModels,
    /// Add a plugin provider to a coding CLI (native input JSON)
    LinkCli {
        provider: String,
        /// JSON object with the link-cli input (file path or - for stdin)
        #[arg(long, value_name = "PATH|-")]
        json_body: String,
    },
    /// List plugin accounts (credentials are masked)
    Accounts,
    /// Check one plugin account
    AccountCheck {
        provider: String,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        proxy: Option<String>,
    },
    /// Fetch one plugin account's usage
    AccountUsage {
        provider: String,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        proxy: Option<String>,
    },
    /// Run one plugin account's check-in
    AccountCheckin {
        provider: String,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        proxy: Option<String>,
    },
}

/// Partial update flags for `plugin settings`. No flag (and no `--json-body`)
/// reads the current settings instead of writing.
#[derive(Debug, Args, Clone)]
pub struct PluginSettingsFields {
    /// Explicit Bun executable path
    #[arg(long, value_name = "PATH", conflicts_with = "clear_bun_path")]
    pub bun_path: Option<String>,
    /// Clear the explicit Bun executable path and fall back to discovery
    #[arg(long)]
    pub clear_bun_path: bool,
    /// npm registry URL
    #[arg(long, value_name = "URL", conflicts_with = "china_mirror")]
    pub npm_registry: Option<String>,
    /// Use the China npmmirror registry (true) or the official npm registry (false)
    #[arg(long, value_name = "true|false")]
    pub china_mirror: Option<bool>,
    /// Route market and npm requests through the app proxy
    #[arg(long, value_name = "true|false")]
    pub follow_app_proxy: Option<bool>,
    /// Enable background auto-updates
    #[arg(long, value_name = "true|false")]
    pub auto_update: Option<bool>,
    /// Auto-update interval in hours (clamped to 1..=720 by the backend)
    #[arg(long, value_name = "HOURS")]
    pub auto_update_interval: Option<u32>,
    /// Full settings patch JSON (file path or - for stdin)
    #[arg(long, value_name = "PATH|-")]
    pub json_body: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// List all 17 CLIs and their capabilities
    List,
    /// Show the resolved root/config/prompt paths of one CLI
    Paths {
        #[arg(long)]
        cli: String,
    },
}

#[derive(Debug, Args, Clone)]
pub struct ProviderFields {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub base_url: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    /// Read the API key from this environment variable (never from argv)
    #[arg(long, value_name = "ENV", conflicts_with = "api_key_stdin")]
    pub api_key_env: Option<String>,
    /// Read the API key from stdin
    #[arg(long, conflicts_with = "api_key_env")]
    pub api_key_stdin: bool,
    /// Full native provider JSON (file path or - for stdin)
    #[arg(long, value_name = "PATH|-")]
    pub json_body: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    List {
        #[arg(long)]
        cli: String,
    },
    Get {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
    },
    Create {
        #[arg(long)]
        cli: String,
        #[command(flatten)]
        fields: ProviderFields,
    },
    Update {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
        #[command(flatten)]
        fields: ProviderFields,
    },
    Delete {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
    },
    Apply {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        model: Option<String>,
    },
    Test {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        prompt: Option<String>,
    },
    FetchModels {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    List {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        provider: String,
    },
    Add {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: Option<String>,
    },
    Delete {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        id: String,
    },
    /// Change the default model of a provider
    Default {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        id: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigMode {
    Patch,
    Replace,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Get {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        scope: Option<String>,
    },
    Set {
        #[arg(long)]
        cli: String,
        /// JSON/TOML/YAML file, or - for stdin
        #[arg(long, value_name = "PATH|-")]
        file: String,
        #[arg(long, value_enum, default_value_t = ConfigMode::Patch)]
        mode: ConfigMode,
        #[arg(long)]
        root_dir: Option<String>,
        #[arg(long)]
        clear_root_dir: bool,
        #[arg(long)]
        scope: Option<String>,
    },
    /// Re-import the config from the CLI's current file
    Extract {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        scope: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PromptCommand {
    List {
        #[arg(long)]
        cli: String,
    },
    Create {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        name: String,
        /// Prompt content file, or - for stdin
        #[arg(long, value_name = "PATH|-")]
        content_file: String,
    },
    Update {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_name = "PATH|-")]
        content_file: Option<String>,
    },
    Delete {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
    },
    Apply {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: String,
    },
    /// Remove the currently applied prompt (or the one passed with --id)
    Disable {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TakeoverMode {
    Single,
    Failover,
    Aggregate,
}

#[derive(Debug, Subcommand)]
pub enum GatewayCommand {
    Status,
    Start,
    Stop,
    Restart,
    Logs {
        #[arg(long, default_value_t = 0)]
        page: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        only_failed: bool,
        #[arg(long)]
        cli: Option<String>,
    },
    Health,
    ResetHealth,
    /// Show every gateway-managed CLI and its takeover state
    Clis,
    /// Switch the primary provider of a CLI already managed by the gateway
    Switch {
        #[arg(long)]
        cli: String,
        /// Provider id to make primary
        #[arg(long)]
        provider: String,
    },
    /// Route a CLI through the gateway
    Engage {
        #[arg(long)]
        cli: String,
        #[arg(long, value_enum, default_value_t = TakeoverMode::Single)]
        mode: TakeoverMode,
        #[arg(long = "provider")]
        provider_ids: Vec<String>,
        #[arg(long)]
        separator: Option<String>,
    },
    Restore {
        #[arg(long)]
        cli: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_global_flags_after_the_subcommand() {
        let parsed = Cli::try_parse_from([
            "aitoolbox",
            "provider",
            "list",
            "--cli",
            "claude",
            "--json",
            "--timeout",
            "10",
        ])
        .unwrap();
        assert!(parsed.json);
        assert_eq!(parsed.timeout, 10);
        assert!(matches!(parsed.command, Command::Provider { .. }));
    }

    #[test]
    fn default_timeout_covers_the_desktop_approval_window() {
        let parsed = Cli::try_parse_from(["aitoolbox", "status"]).unwrap();
        assert!(
            parsed.timeout >= 120,
            "the default timeout must outlast a 60s approval wait plus overhead"
        );
    }

    #[test]
    fn parses_gateway_clis_and_switch() {
        let parsed = Cli::try_parse_from(["aitoolbox", "gateway", "clis"]).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Gateway {
                command: GatewayCommand::Clis
            }
        ));
        let parsed = Cli::try_parse_from([
            "aitoolbox",
            "gateway",
            "switch",
            "--cli",
            "codex",
            "--provider",
            "p1",
        ])
        .unwrap();
        assert!(matches!(
            parsed.command,
            Command::Gateway {
                command: GatewayCommand::Switch { .. }
            }
        ));
    }

    #[test]
    fn parses_plugin_settings_edit_flags() {
        let parsed = Cli::try_parse_from([
            "aitoolbox",
            "plugin",
            "settings",
            "--bun-path",
            "/opt/bun",
            "--china-mirror",
            "true",
            "--follow-app-proxy",
            "false",
            "--auto-update",
            "true",
            "--auto-update-interval",
            "6",
        ])
        .unwrap();
        let Command::Plugin {
            command:
                PluginCommand::Settings {
                    fields:
                        PluginSettingsFields {
                            bun_path,
                            clear_bun_path,
                            china_mirror,
                            follow_app_proxy,
                            auto_update,
                            auto_update_interval,
                            ..
                        },
                },
        } = parsed.command
        else {
            panic!("expected `plugin settings`");
        };
        assert_eq!(bun_path.as_deref(), Some("/opt/bun"));
        assert!(!clear_bun_path);
        assert_eq!(china_mirror, Some(true));
        assert_eq!(follow_app_proxy, Some(false));
        assert_eq!(auto_update, Some(true));
        assert_eq!(auto_update_interval, Some(6));

        let parsed = Cli::try_parse_from(["aitoolbox", "plugin", "settings"]).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Plugin {
                command: PluginCommand::Settings { .. }
            }
        ));
    }

    #[test]
    fn parses_plugin_update_commands() {
        let parsed =
            Cli::try_parse_from(["aitoolbox", "plugin", "update", "demo-id"]).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Plugin {
                command: PluginCommand::Update {
                    id: Some(ref id),
                    check_only: false,
                }
            } if id == "demo-id"
        ));

        let parsed =
            Cli::try_parse_from(["aitoolbox", "plugin", "update", "--check-only"]).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Plugin {
                command: PluginCommand::Update {
                    id: None,
                    check_only: true,
                }
            }
        ));
    }

    #[test]
    fn bun_path_flags_conflict() {
        assert!(Cli::try_parse_from([
            "aitoolbox",
            "plugin",
            "settings",
            "--bun-path",
            "/opt/bun",
            "--clear-bun-path",
        ])
        .is_err());
    }

    #[test]
    fn parses_plugin_provider_commands() {
        let parsed = Cli::try_parse_from([
            "aitoolbox",
            "plugin",
            "provider-options",
            "demo",
            "--set",
            "{\"fast\":true}",
        ])
        .unwrap();
        assert!(matches!(
            parsed.command,
            Command::Plugin {
                command: PluginCommand::ProviderOptions { set: Some(_), .. }
            }
        ));

        // `--enabled` takes a value; a bare flag must not be accepted.
        let parsed = Cli::try_parse_from([
            "aitoolbox",
            "plugin",
            "provider-auto-checkin",
            "demo",
            "--enabled",
            "false",
        ])
        .unwrap();
        let Command::Plugin {
            command: PluginCommand::ProviderAutoCheckin { provider, enabled },
        } = parsed.command
        else {
            panic!("expected `plugin provider-auto-checkin`");
        };
        assert_eq!(provider, "demo");
        assert!(!enabled);
        assert!(Cli::try_parse_from([
            "aitoolbox",
            "plugin",
            "provider-auto-checkin",
            "demo",
            "--enabled"
        ])
        .is_err());
    }

    #[test]
    fn parses_plugin_runtime_commands() {
        for (argv, expected) in [
            (vec!["plugin", "bun-status"], "bun-status"),
            (vec!["plugin", "bun-ensure"], "bun-ensure"),
            (vec!["plugin", "host-status"], "host-status"),
            (vec!["plugin", "host-restart"], "host-restart"),
            (vec!["plugin", "pages"], "pages"),
            (vec!["plugin", "bun-probe", "/opt/bun"], "bun-probe"),
        ] {
            let parsed = Cli::try_parse_from(
                std::iter::once("aitoolbox").chain(argv.iter().copied()),
            )
            .unwrap();
            assert!(
                matches!(parsed.command, Command::Plugin { .. }),
                "{expected} must parse under `plugin`"
            );
        }
    }

    #[test]
    fn api_key_sources_are_mutually_exclusive() {
        assert!(Cli::try_parse_from([
            "aitoolbox",
            "provider",
            "create",
            "--cli",
            "codex",
            "--api-key-env",
            "KEY",
            "--api-key-stdin",
        ])
        .is_err());
    }
}
