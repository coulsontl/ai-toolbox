//! Command-line surface for `aitb`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "aitb",
    version,
    about = "Local Agent Control API client for AI Toolbox",
    long_about = "Talk to the running AI Toolbox desktop app through its loopback control API. The bearer token is read from ~/.ai-toolbox/control.json (or AI_TOOLBOX_CONTROL_FILE) and is never printed."
)]
pub struct Cli {
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    pub json: bool,
    /// Request timeout in seconds
    #[arg(long, global = true, default_value_t = 75)]
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
    /// Read plugin provider options
    ProviderOptions {
        provider: String,
    },
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
            "aitb",
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
    fn api_key_sources_are_mutually_exclusive() {
        assert!(Cli::try_parse_from([
            "aitb",
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
