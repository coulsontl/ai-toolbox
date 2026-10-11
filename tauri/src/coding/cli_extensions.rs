//! **Pi 系 CLI**（Pi / OmO Native）的扩展管理共用实现。
//!
//! 两个 CLI 共用 senpi 引擎的包体系：同一个 `settings.json` 形状（`packages` 数组 +
//! 顶层 `extensions` 过滤器）、同一套 CLI 子命令（`list` / `install` / `remove` /
//! `update` / `config`）、逐字相同的 `list` 输出格式。差异只有四处，全部收在
//! [`CliExtensionSpec`] 里。
//!
//! **为什么抽共享而不是复制**：`pi/extensions.rs` 原有 2054 行，其中约 1800 行是
//! 与 CLI 无关的纯逻辑（列表解析、过滤器语义、glob 匹配、npm 版本比对、目录扫描）。
//! 复制一份意味着以后每修一处语义都要改两处，而语义是**照着上游 JS 反推**的
//! （见各函数的注释），最容易漂移。
//!
//! 调用方（`pi` / `omo_native`）只提供 spec 与 runtime location，其余全在这里。
//!
//! **共享的代价**：这里的每条语义都同时作用于 Pi 与 OmO，改之前要确认两边都成立。
//! 已验证的等价性写在 `omo_native/extensions.rs` 的 spec 注释里。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::Emitter;
use tokio::process::Command;

use crate::coding::cli_resolver::{apply_create_no_window_tokio, build_local_tokio_command};
use crate::coding::runtime_location::{RuntimeLocationInfo, RuntimeLocationMode};
use crate::coding::url_utils::encode_url_path_segment;
use crate::db::SqliteDbState;
use crate::http_client;

const NPM_REGISTRY_BASE_URL: &str = "https://registry.npmjs.org";
const NPM_LATEST_LOOKUP_TIMEOUT_SECS: u64 = 8;
const NPM_LATEST_LOOKUP_CONCURRENCY: usize = 6;

/// `list` 给带过滤器的包打的后缀。
const CLI_LIST_FILTERED_SUFFIX: &str = " (filtered)";

/// 行开关「禁用整包」时，暂存用户自己那些逐文件过滤器的旁路键。
///
/// **禁用必须把四类资源数组都写成 `[]`**：上游 `applyPackageFilter` 对空数组走
/// 无条件全禁分支（官方文档也写明 `[]` = "load none of that type"）。不能用
/// `!**` 这类排除模式代替——`applyPatterns` 的强制包含（`+path`）在排除**之后**
/// 执行，用户只要有一条 `+path`，`!**` 就挡不住它，而界面还会显示「已禁用」。
///
/// 但覆盖成 `[]` 会抹掉用户自己的过滤器，所以覆盖前先把非空的几类挪进这个键，
/// 启用时原样搬回。pi / omo 读设置时不校验未知键、`addSourceToSettings` 也只改
/// 它要改的字段，所以这个键能安全留存（都跑真实引擎实测过）。
const DISABLED_FILTERS_KEY: &str = "aiToolboxDisabledFilters";

/// 单条 CLI 扩展命令的上界（install 可能真在跑 npm，给宽一点）。
///
/// 没有这个上界时，WSL 发行版停摆会让 `output()` 永久挂起、面板一直转圈。
const CLI_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

const NPM_LEGACY_PEER_DEPS_ENV_KEY: &str = "NPM_CONFIG_LEGACY_PEER_DEPS";
const NPM_LEGACY_PEER_DEPS_ENV_VALUE: &str = "true";

/// WSL 下执行 CLI 的包装脚本。参数：路径前缀、agent 目录、环境变量**键名**、真正的命令。
///
/// 位置参数而不是插值，避免 root 路径里的空格 / 分号被 shell 解释
/// （`build_cli_command` 的测试覆盖了注入场景）。
///
/// 键名必须走位置参数：`command.env()` 设的是 **Windows 侧**环境变量，
/// 不经 `WSLENV` 转发根本不会出现在 WSL 进程里（实测 `AGENT_DIR_ENV_KEY` 为空，
/// 脚本会执行 `export ""="…"` 并以 `bad variable name` 退出）。
const WSL_CLI_COMMAND_SCRIPT: &str = r#"path_prefix=$1
agent_dir=$2
env_key=$3
shift 3
if [ -n "$path_prefix" ]; then
    PATH="$path_prefix${PATH:+:$PATH}"
    export PATH
fi
export "$env_key"="$agent_dir"
exec "$@""#;

// ============================================================================
// Spec
// ============================================================================

/// 两个 CLI 之间**全部**的差异点。
///
/// 加第三个同族 CLI 时只需要加一个 `const` 与一个 runtime location 取法，
/// 不需要碰本文件里 1800 行的逻辑。
#[derive(Debug, Clone, Copy)]
pub struct CliExtensionSpec {
    /// CLI 命令名，用于错误提示、命令回显与手动路径配置提示（`pi` / `omo`）。
    pub cli_name: &'static str,
    /// 指定 agent 目录的环境变量（`PI_CODING_AGENT_DIR` / `OMO_CODING_AGENT_DIR`）。
    pub env_key: &'static str,
    /// 离线开关的环境变量（两处都是 `PI_OFFLINE`，实测 `omo` 二进制里也只有这个拼写）。
    pub offline_env_key: &'static str,
    /// 本地扩展目录名（相对 agent 根，两处都是 `extensions`）。
    pub extensions_dir: &'static str,
    /// 不可删除的内置本地扩展前缀。
    pub protected_prefixes: &'static [&'static str],
    /// 包过滤器的资源类型键（`applyPackageFilter` 认的四种）。
    pub resource_type_keys: &'static [&'static str],
    /// 扫描本地扩展目录的超时（WSL UNC 根可能很慢）。
    pub scan_timeout: std::time::Duration,
    /// 请求 WSL/SSH 同步该 CLI 配置的事件名。
    ///
    /// settings.json 是两个 CLI 的 WSL 文件映射（`pi-settings` / `omo-native-config`），
    /// 只发 `config-changed` 的话 Windows 侧改了、WSL 侧的副本还是旧的：页面显示已停用，
    /// 而 WSL 里的 CLI 照旧加载。其它设置写入路径都会发这个事件。
    pub wsl_sync_event: &'static str,
}

// ============================================================================
// 类型（前端契约与 `PiExtension*` 同形，两个页面共用一套）
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliExtensionScope {
    User,
    Project,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliExtensionKind {
    Package,
    LocalFile,
    LocalDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliExtensionSummary {
    pub id: String,
    pub source: String,
    pub scope: CliExtensionScope,
    pub kind: CliExtensionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub built_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    /// npm registry 的 `dist-tags.latest`（仅未钉版本且能连上 registry 时）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub update_available: bool,
    /// 由 `settings.json` 的过滤器推导：`false` 表示这个包什么都不加载
    /// （四种资源类型全空）或本地扩展被 `-path` 排除。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `false` 表示界面不该给开关——本应用能写的过滤器改不动它实际加载什么。
    #[serde(default = "default_true")]
    pub switch_supported: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliExtensionListResult {
    pub extensions_path: String,
    pub packages_path: String,
    pub extensions: Vec<CliExtensionSummary>,
    pub raw: String,
    /// 解析出的宿主侧 CLI 路径（WSL 下是调用标签）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    /// 尽力取到的 `<cli> --version` 首行；探测失败则省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliExtensionInstallInput {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliExtensionUpdateInput {
    /// 有值只更新这一个（`<cli> update <source>`）；省略则更新全部。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliExtensionActionInput {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<CliExtensionScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<CliExtensionKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliExtensionCommandResult {
    pub command: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliExtensionEnabledInput {
    pub source: String,
    pub kind: CliExtensionKind,
    /// `true` 写回启用形态；`false` 写禁用过滤器。
    pub enabled: bool,
}

// ============================================================================
// 路径
// ============================================================================

pub fn extensions_path_from_root(root_dir: &Path, spec: &CliExtensionSpec) -> PathBuf {
    root_dir.join(spec.extensions_dir)
}

pub fn packages_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join("npm").join("node_modules")
}

// ============================================================================
// 命令构造与执行
// ============================================================================

/// 一个已构造好的 CLI 调用：命令 + 用于错误提示的程序标签。
pub struct CliCommandInvocation {
    pub command: Command,
    pub local_program_label: Option<String>,
}

/// 由调用方提供的宿主侧二进制解析结果（各 CLI 的安装位置不同，见 `cli_resolver`）。
pub struct ResolvedCliProgram {
    pub path: PathBuf,
}

fn cli_extension_npm_compat_env() -> [(&'static str, &'static str); 1] {
    [(NPM_LEGACY_PEER_DEPS_ENV_KEY, NPM_LEGACY_PEER_DEPS_ENV_VALUE)]
}

fn apply_cli_extension_npm_compat_env(command: &mut Command) {
    for (key, value) in cli_extension_npm_compat_env() {
        command.env(key, value);
    }
}

/// 用户级 bin 目录前缀（WSL 下 GUI 进程不继承登录 shell 的 PATH）。
fn wsl_path_prefix(linux_user_root: Option<&str>) -> String {
    let Some(linux_user_root) = linux_user_root.filter(|root| !root.trim().is_empty()) else {
        return String::new();
    };
    let linux_user_root = linux_user_root.trim_end_matches('/');
    [
        format!("{linux_user_root}/.local/share/mise/shims"),
        format!("{linux_user_root}/.asdf/shims"),
        format!("{linux_user_root}/.local/bin"),
        format!("{linux_user_root}/.bun/bin"),
        format!("{linux_user_root}/.volta/bin"),
        format!("{linux_user_root}/.local/share/fnm/aliases/default/bin"),
        format!("{linux_user_root}/.fnm/aliases/default/bin"),
        format!("{linux_user_root}/.fnm/current/bin"),
        format!("{linux_user_root}/.npm-global/bin"),
    ]
    .join(":")
}

pub fn build_cli_command(
    spec: &CliExtensionSpec,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
    offline: bool,
) -> Result<CliCommandInvocation, String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            let local_program_label = program.path.display().to_string();
            let mut command = build_local_tokio_command(&program.path);
            command.args(args);
            command.env(spec.env_key, &runtime_location.host_path);
            apply_cli_extension_npm_compat_env(&mut command);
            if offline {
                command.env(spec.offline_env_key, "1");
            }
            Ok(CliCommandInvocation {
                command,
                local_program_label: Some(local_program_label),
            })
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = runtime_location.wsl.as_ref().ok_or_else(|| {
                format!(
                    "Missing WSL runtime metadata for {} extension command",
                    spec.cli_name
                )
            })?;
            let local_program_label = format!("wsl -d {} -- {}", wsl.distro, spec.cli_name);
            let mut command = Command::new("wsl");
            apply_create_no_window_tokio(&mut command);
            command.args([
                "-d",
                &wsl.distro,
                "--exec",
                "/bin/sh",
                "-c",
                WSL_CLI_COMMAND_SCRIPT,
                "ai-toolbox-ext",
                &wsl_path_prefix(wsl.linux_user_root.as_deref()),
                &wsl.linux_path,
                // 两个 CLI 的键名不同，所以走位置参数传给脚本里的
                // `export "$env_key"`；用 `command.env()` 设 Windows 侧变量
                // 是无效的（不转发进 WSL）。
                spec.env_key,
                "env",
            ]);
            for (key, value) in cli_extension_npm_compat_env() {
                command.arg(format!("{key}={value}"));
            }
            if offline {
                command.arg(format!("{}={}", spec.offline_env_key, 1));
            }
            command.arg(spec.cli_name);
            command.args(args);
            Ok(CliCommandInvocation {
                command,
                local_program_label: Some(local_program_label),
            })
        }
    }
}

/// 供界面显示的 CLI 位置。
pub fn cli_display_path(
    spec: &CliExtensionSpec,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
) -> Option<String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => Some(program.path.display().to_string()),
        RuntimeLocationMode::WslDirect => runtime_location
            .wsl
            .as_ref()
            .map(|wsl| format!("wsl -d {} -- {}", wsl.distro, spec.cli_name)),
    }
}

/// npm 安装包目录（`<agentDir>/npm`）的展示路径。
///
/// `.npmrc` 就写在这里：npm 把 `<prefix>/.npmrc` 当**项目级**配置读，
/// 而 CLI 装包用的 prefix 正是这个目录（实测：放这里 `allow-remote=all` 生效）。
///
/// 路径风格跟着**当前运行模式**走，不把两种风格都写进提示——用户只需要知道
/// 他这台机器上该改哪个文件。
fn packages_dir_display(runtime_location: &RuntimeLocationInfo) -> String {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            runtime_location.host_path.join("npm").display().to_string()
        }
        RuntimeLocationMode::WslDirect => runtime_location
            .wsl
            .as_ref()
            .map(|wsl| format!("{}/npm", wsl.linux_path.trim_end_matches('/')))
            .unwrap_or_else(|| runtime_location.host_path.join("npm").display().to_string()),
    }
}

/// 失败输出里若含 npm 策略错误，追加提示；否则原样返回。
fn annotate_npm_policy_error(message: String, runtime_location: &RuntimeLocationInfo) -> String {
    match npm_policy_hint(&message, runtime_location) {
        Some(hint) => format!("{message}\n\n{hint}"),
        None => message,
    }
}

/// 从 npm 失败输出里认出 npm 12 的「允许清单」策略错误，并给出可操作提示。
///
/// npm 12 起默认拒绝两类依赖，装包命令的原始输出只有错误码和一句「已禁用」，
/// 用户看不出该改哪个文件的哪个键：
///
/// - `EALLOWREMOTE`：依赖指向 tarball URL（如 `https://pkg.pr.new/...`）时拒绝，
///   默认 `allow-remote=none`；
/// - `EALLOWSCRIPTS`：项目级安装里不允许用 `--allow-scripts` 命令行参数，
///   必须写进 `package.json` 的 `allowScripts` 或 `.npmrc`。
///
/// 这里只**追加**提示，不改写原始输出——原文里的包名和 URL 是排查的关键信息。
fn npm_policy_hint(message: &str, runtime_location: &RuntimeLocationInfo) -> Option<String> {
    let packages_dir = packages_dir_display(runtime_location);

    if message.contains("EALLOWREMOTE") {
        let blocked = extract_refused_package(message);
        let mut hint = String::from(
            "npm 12 默认禁止从 URL 安装依赖（`allow-remote=none`）。上面被拒的包在它的依赖里写死了 tarball 地址。\
             两种处理方式：\n\
             1. 先单独把那个包升到不再引用远程地址的新版本（通常是首选，能一次性去掉问题）；\n",
        );
        hint.push_str(&format!(
            "2. 在 `{packages_dir}/.npmrc` 里加一行 `allow-remote=all`，然后重新执行更新。\
             这会放开该目录下所有 URL 依赖，请只在你信任这些包时使用。",
        ));
        if let Some(package) = blocked {
            hint.push_str(&format!("\n被拒绝的依赖：`{package}`"));
        }
        return Some(hint);
    }

    if message.contains("EALLOWSCRIPTS") {
        return Some(format!(
            "npm 12 不允许在项目级安装里用 `--allow-scripts` 命令行参数，\
             必须把允许的包写进配置：在 `{packages_dir}/.npmrc` 里加\
             `allow-scripts=<包名，逗号分隔>`，或写进该目录 `package.json` 的 `allowScripts` 字段，然后重试。",
        ));
    }

    None
}

/// 从 npm 的 `Refusing to fetch "<spec>"` 行里取出被拒的包名。
fn extract_refused_package(message: &str) -> Option<String> {
    for line in message.lines() {
        let Some(rest) = line.split_once("Refusing to fetch ").map(|(_, rest)| rest) else {
            continue;
        };
        let trimmed = rest.trim().trim_matches('"').trim();
        if trimmed.is_empty() {
            continue;
        }
        // `@scope/name@https://...`：包名到第二个 `@` 之前为止（作用域包有一个前导 `@`）。
        let without_scope = trimmed.strip_prefix('@').unwrap_or(trimmed);
        let name = match without_scope.split_once('@') {
            Some((name, _)) => name,
            None => without_scope,
        };
        let full = if trimmed.starts_with('@') {
            format!("@{name}")
        } else {
            name.to_string()
        };
        if !full.is_empty() && full != "@" {
            return Some(full);
        }
    }
    None
}

/// 把解析出的 CLI 路径附到错误上，让多份安装的 PATH 问题可诊断。
fn annotate_cli_command_error(
    spec: &CliExtensionSpec,
    message: String,
    local_program_label: Option<&str>,
) -> String {
    let Some(label) = local_program_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return message;
    };
    if message.contains(label) {
        return message;
    }
    format!("{message}\n{}_cli={label}", spec.cli_name)
}

fn build_cli_spawn_error(
    spec: &CliExtensionSpec,
    error: &std::io::Error,
    local_program_label: Option<&str>,
) -> String {
    let base_message = format!("Failed to run {} extension command: {error}", spec.cli_name);
    let manual_hint = crate::coding::cli_resolver::manual_cli_config_hint(spec.cli_name);
    let with_hint = if error.kind() == std::io::ErrorKind::NotFound {
        let message = if let Some(label) = local_program_label {
            format!(
                "{base_message}. attempted_program={label}. {}",
                crate::coding::cli_resolver::local_cli_missing_hint(spec.cli_name)
            )
        } else {
            format!(
                "{base_message}. {}",
                crate::coding::cli_resolver::local_cli_missing_hint(spec.cli_name)
            )
        };
        if manual_hint.is_empty() {
            message
        } else {
            format!("{message} {manual_hint}")
        }
    } else if !manual_hint.is_empty() {
        format!("{base_message} {manual_hint}")
    } else {
        base_message
    };
    annotate_cli_command_error(spec, with_hint, local_program_label)
}

pub async fn run_cli_command(
    spec: &CliExtensionSpec,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
    offline: bool,
) -> Result<String, String> {
    let CliCommandInvocation {
        mut command,
        local_program_label,
    } = build_cli_command(spec, program, runtime_location, args, offline)?;

    // 必须有界：WSL 发行版停摆 / npm 卡在网络上时 `output()` 会一直挂着，面板就
    // 永远转圈。`kill_on_drop` 保证超时后子进程被回收——只丢 future 会留下孤儿
    // 进程继续烧 CPU（`cli_resolver` 的版本探测用同一套组合）。
    command.kill_on_drop(true);
    let output = match tokio::time::timeout(CLI_COMMAND_TIMEOUT, command.output()).await {
        Ok(result) => result
            .map_err(|error| build_cli_spawn_error(spec, &error, local_program_label.as_deref()))?,
        Err(_) => {
            return Err(format!(
                "{} extension command timed out after {}s: {}. For WSL or network paths, check that the distro or network is reachable.",
                spec.cli_name,
                CLI_COMMAND_TIMEOUT.as_secs(),
                args.join(" ")
            ));
        }
    };

    let stdout_output = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        return Ok(stdout_output);
    }

    let stderr_output = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout_trimmed = stdout_output.trim().to_string();
    let failure_message = if !stderr_output.is_empty() {
        stderr_output
    } else if !stdout_trimmed.is_empty() {
        stdout_trimmed
    } else {
        format!("Unknown {} extension command failure", spec.cli_name)
    };
    // npm 12 的策略错误原文只有错误码，这里补一段「改哪个文件的哪个键」再抛。
    let failure_message = annotate_npm_policy_error(failure_message, runtime_location);
    Err(annotate_cli_command_error(
        spec,
        failure_message,
        local_program_label.as_deref(),
    ))
}

pub async fn probe_cli_version(
    spec: &CliExtensionSpec,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
) -> Option<String> {
    let version = run_cli_command(spec, program, runtime_location, &["--version"], true)
        .await
        .ok()?;
    let trimmed = version.lines().next().unwrap_or(version.as_str()).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_unknown_no_approve_option_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let mentions_unknown_option = lower.contains("unknown option")
        || lower.contains("unknown argument")
        || lower.contains("unrecognized option");
    let mentions_no_approve = lower.contains("--no-approve")
        || lower.contains("-no-approve")
        || lower.contains("'-na'")
        || lower.contains("\"-na\"")
        || lower.split_whitespace().any(|token| {
            token.trim_matches(|c| c == '\'' || c == '"' || c == ',' || c == '.') == "-na"
        });
    mentions_unknown_option && mentions_no_approve
}

fn args_without_no_approve<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter()
        .copied()
        .filter(|arg| *arg != "--no-approve" && *arg != "-na")
        .collect()
}

/// 优先带 `--no-approve`，让非交互式操作跳过项目信任提示。
///
/// 老版本 / 非官方构建的 CLI 在部分子命令上不认这个 flag（例如 `list`），
/// 此时不带它重试一次，保证界面还能用。
pub async fn run_cli_command_preferring_no_approve(
    spec: &CliExtensionSpec,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
    offline: bool,
) -> Result<(String, Vec<String>), String> {
    let used_args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    match run_cli_command(spec, program, runtime_location, args, offline).await {
        Ok(output) => Ok((output, used_args)),
        Err(error)
            if args
                .iter()
                .any(|arg| *arg == "--no-approve" || *arg == "-na")
                && is_unknown_no_approve_option_error(&error) =>
        {
            let fallback_args = args_without_no_approve(args);
            let output =
                run_cli_command(spec, program, runtime_location, &fallback_args, offline).await?;
            Ok((
                output,
                fallback_args.iter().map(|arg| (*arg).to_string()).collect(),
            ))
        }
        Err(error) => Err(error),
    }
}

pub fn format_cli_command_owned(spec: &CliExtensionSpec, args: &[String]) -> String {
    format!("{} {}", spec.cli_name, args.join(" "))
}

// ============================================================================
// 列表解析
// ============================================================================

fn is_cli_package_source(source: &str) -> bool {
    let lower_source = source.trim().to_ascii_lowercase();
    ["npm:", "file:", "github:", "git:", "http:", "https:"]
        .iter()
        .any(|prefix| lower_source.starts_with(prefix))
}

fn is_protected_local_extension_source(spec: &CliExtensionSpec, source: &str) -> bool {
    let source = source.trim();
    spec.protected_prefixes
        .iter()
        .any(|prefix| source.starts_with(prefix))
}

/// 解析 `<cli> list` 的输出。
///
/// 格式（两个 CLI 逐字相同）：
/// ```text
/// User packages:
///   npm:foo
///     /path/to/foo
/// Project packages:
///   github:owner/repo
///     /path/to/repo
/// ```
pub fn parse_list_output(raw: &str) -> Vec<CliExtensionSummary> {
    let mut result = Vec::new();
    let mut scope = CliExtensionScope::Unknown;
    let mut pending_index: Option<usize> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("User packages:") {
            scope = CliExtensionScope::User;
            pending_index = None;
            continue;
        }
        if trimmed.eq_ignore_ascii_case("Project packages:") {
            scope = CliExtensionScope::Project;
            pending_index = None;
            continue;
        }
        if is_cli_package_source(trimmed) {
            let source = strip_list_filtered_suffix(trimmed);
            result.push(CliExtensionSummary {
                id: format!("{}:{}", scope_id(scope), source),
                source: source.to_string(),
                scope,
                kind: CliExtensionKind::Package,
                path: None,
                built_in: false,
                current_version: None,
                latest_version: None,
                update_available: false,
                enabled: true,
                switch_supported: true,
            });
            pending_index = Some(result.len() - 1);
            continue;
        }
        if let Some(index) = pending_index {
            if result[index].path.is_none() {
                result[index].path = Some(trimmed.to_string());
            }
        }
    }

    result
}

/// `list` 对「对象形式」的 settings 条目打印 `npm:foo (filtered)`；
/// 后缀不能漏进 remove / update 用的 source。
fn strip_list_filtered_suffix(source: &str) -> &str {
    source
        .strip_suffix(CLI_LIST_FILTERED_SUFFIX)
        .map(str::trim_end)
        .unwrap_or(source)
}

fn scope_id(scope: CliExtensionScope) -> &'static str {
    match scope {
        CliExtensionScope::User => "user",
        CliExtensionScope::Project => "project",
        CliExtensionScope::Unknown => "unknown",
    }
}

// ============================================================================
// 过滤器语义
// ============================================================================

/// 拆开 `+path` / `-path` / `!pattern` 的前缀。
fn split_pattern_prefix(value: &str) -> (Option<char>, &str) {
    match value.chars().next() {
        Some(prefix @ ('+' | '-' | '!')) => (Some(prefix), &value[prefix.len_utf8()..]),
        _ => (None, value),
    }
}

/// 上游 `normalizeExactPattern` 会去掉开头的 `./` 再跑 `toPosixPath`（把
/// `path.sep` 换成 `/`）。Windows 上 CLI 自己写出的模式是反斜杠
/// （`path.relative` 返回 `extensions\\foo.ts`），两边必须归一化后再比，
/// 否则读回来的模式匹配不上、显示成启用。
fn normalize_exact_pattern(pattern: &str) -> String {
    // 上游 `normalizeExactPattern` 剥的是 `./` **或** `.\`（`startsWith("./") ||
    // startsWith(".\\")`），再 `toPosixPath`。只剥 `./` 会让 `.\\extensions\\foo.ts`
    // 归一成 `./extensions/foo.ts`，永远比较不相等。
    let without_dot = pattern
        .strip_prefix("./")
        .or_else(|| pattern.strip_prefix(".\\"))
        .unwrap_or(pattern);
    without_dot.replace('\\', "/")
}

/// 本地扩展的强制包含 / 排除模式（相对运行时根）。
fn local_extension_pattern(
    spec: &CliExtensionSpec,
    source: &str,
    kind: CliExtensionKind,
) -> String {
    match kind {
        CliExtensionKind::LocalDirectory => {
            format!("{}/{source}/index.ts", spec.extensions_dir)
        }
        _ => format!("{}/{source}", spec.extensions_dir),
    }
}

/// 上游 `resolveExtensionEntries` 优先用目录 `package.json` 里的 `pi.extensions`
/// 列表而不是 `index.ts`，所以针对 `index.ts` 写的排除模式根本匹配不到实际加载的文件。
fn directory_has_manifest_extensions(directory: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(directory.join("package.json")) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    parsed
        .get("pi")
        .and_then(|pi| pi.get("extensions"))
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.as_str().is_some_and(|entry| !entry.trim().is_empty()))
        })
}

/// 一个包条目在两种情况下什么都不加载：
///
/// 1. **每种**资源类型都存在且为空——`applyPackageFilter` 把 `[]` 当「这类一个都不加载」，
///    而条目没列出的类型仍走 `collectDefaultResources` 正常加载。
/// 2. `autoload: false` 且每类资源都只由 `-`/`!` 模式构成——上游
///    `applyAutoloadDisabledPatterns` 对 `+` 和普通模式设 `enabled = true`，
///    对 `-`/`!` 设 `false`。**只要混进一个 `+`，那个文件就会被加载**，
///    哪怕其余模式全是排除（实测：`["+index.ts", "!nope"]` 时包内 `index.ts`
///    仍为 `[x]`）。
///
/// 对照上游 `collectPackageResources` / `applyPackageFilter` / `applyPackageDeltaFilter`
/// 核实，并跑过真实的包管理器验证。
fn is_disabled_package_entry(spec: &CliExtensionSpec, entry: &Value) -> bool {
    let Some(object) = entry.as_object() else {
        return false;
    };
    if object.get("autoload").and_then(Value::as_bool) == Some(false) {
        // 每一类都必须「全由排除模式构成」才算挡住；**缺键也算挡住**——上游把缺失
        // 读成 `patterns ?? []`，`applyPackageDeltaFilter` 见空列表直接返回，该类型
        // 一个资源都不贡献。四类全缺 = 什么都不加载 = 确实禁用（实测确认）。
        // `+` 或普通模式则相反：`applyAutoloadDisabledPatterns` 会给它 `enabled = true`，
        // 所以混进一个 `+index.ts` 就有一个文件被加载，不能判成禁用。
        return spec.resource_type_keys.iter().all(|key| {
            object
                .get(*key)
                .and_then(Value::as_array)
                .is_none_or(|patterns| {
                    patterns.iter().all(|pattern| {
                        pattern
                            .as_str()
                            .is_some_and(|pattern| pattern.starts_with(['-', '!']))
                    })
                })
        });
    }
    // 每一类资源都必须是空数组，整包才真的不加载：上游 `applyPackageFilter` 对
    // `[]` 走无条件全禁分支。**不能用「含排除模式」代替**——`+path` 在排除之后
    // 执行，一条 `+path` 就能把 `!**` 之类全部推翻。
    // 键缺失不算挡住——上游会走 `collectDefaultResources` 照常加载。
    spec.resource_type_keys
        .iter()
        .all(|key| match object.get(*key).and_then(Value::as_array) {
            Some(array) => array.is_empty(),
            None => false,
        })
}

/// 行开关能不能把这个条目改回「普通、全加载」形态？
///
/// `autoload: false` 是项目级增量覆盖：它的基准在项目 settings 里，这里写 `[]`
/// 什么也改不动，却会让这一行声称一个它并不掌控的状态。
fn package_entry_supports_switch(entry: &Value) -> bool {
    match entry.as_object() {
        Some(object) => object.get("autoload").and_then(Value::as_bool) != Some(false),
        None => true,
    }
}

fn package_entry_source(entry: &Value) -> Option<&str> {
    match entry {
        Value::String(source) => Some(source.as_str()),
        Value::Object(object) => object.get("source").and_then(Value::as_str),
        _ => None,
    }
}

/// CLI 实际认的那个条目。
///
/// `dedupePackages` 对每个 identity 只保留**第一个**用户级条目、丢掉后面的重复项，
/// 所以第二个条目（例如手工禁用的那个）对加载没有影响。读最后一个会报「已禁用」，
/// 而 CLI 其实还在加载全部。
fn effective_package_entry<'a>(
    packages: Option<&'a Vec<Value>>,
    source: &str,
) -> Option<&'a Value> {
    packages?
        .iter()
        .find(|entry| package_entry_source(entry) == Some(source))
}

/// 镜像上游 `isEnabledByOverrides`：先 `!` 排除，再 `+` 强制包含，最后 `-` 强制排除。
fn is_local_extension_enabled(top_level_entries: Option<&Value>, pattern: &str) -> bool {
    let Some(entries) = top_level_entries.and_then(Value::as_array) else {
        return true;
    };
    let matches_exact = |wanted: char| {
        entries.iter().any(|entry| {
            entry.as_str().is_some_and(|value| {
                let (prefix, payload) = split_pattern_prefix(value);
                prefix == Some(wanted) && normalize_exact_pattern(payload) == pattern
            })
        })
    };
    let matches_any = |wanted: char| {
        entries.iter().any(|entry| {
            entry.as_str().is_some_and(|value| {
                let (prefix, payload) = split_pattern_prefix(value);
                prefix == Some(wanted) && glob_matches(payload, pattern)
            })
        })
    };
    // 顺序执行、不提前返回：上游按顺序跑三趟，所以后面的 `+` 能重新启用 `!` 排除的。
    // `!` 走 glob 匹配（`matchesAnyPattern`），`+`/`-` 比精确路径（`matchesAnyExactPattern`）。
    let mut enabled = true;
    if matches_any('!') {
        enabled = false;
    }
    if matches_exact('+') {
        enabled = true;
    }
    if matches_exact('-') {
        enabled = false;
    }
    enabled
}

/// 上游用户在 settings 里会写的那个 glob 子集（`*`、`?`、字面段）。
///
/// `!` 过滤器走上游的 `matchesAnyPattern`（minimatch），所以精确比较会把
/// `!extensions/*.ts` 报成启用。与 minimatch 一致：`*` / `?` 不跨 `/`。
fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = normalize_exact_pattern(pattern);
    let value = normalize_exact_pattern(value);
    if glob_match_path(&pattern, &value) {
        return true;
    }
    // 上游也测 basename，所以 `!foo.ts` 能排除 `extensions/foo.ts`。
    let basename = value.rsplit('/').next().unwrap_or(value.as_str());
    !pattern.contains('/') && glob_match_path(&pattern, basename)
}

fn glob_match_path(pattern: &str, value: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    let value_segments: Vec<&str> = value.split('/').collect();
    if pattern_segments.len() != value_segments.len() {
        return false;
    }
    pattern_segments
        .iter()
        .zip(value_segments.iter())
        .all(|(pattern_segment, value_segment)| glob_match_segment(pattern_segment, value_segment))
}

/// 单个路径段内的 `*` / `?`（不跨 `/`）。
fn glob_match_segment(pattern: &str, value: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let value_chars: Vec<char> = value.chars().collect();
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let mut star_pattern_index: Option<usize> = None;
    let mut star_value_index = 0usize;
    while value_index < value_chars.len() {
        if pattern_index < pattern_chars.len()
            && (pattern_chars[pattern_index] == '?'
                || pattern_chars[pattern_index] == value_chars[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern_chars.len() && pattern_chars[pattern_index] == '*' {
            star_pattern_index = Some(pattern_index);
            star_value_index = value_index;
            pattern_index += 1;
        } else if let Some(star_index) = star_pattern_index {
            pattern_index = star_index + 1;
            star_value_index += 1;
            value_index = star_value_index;
        } else {
            return false;
        }
    }
    while pattern_index < pattern_chars.len() && pattern_chars[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern_chars.len()
}

/// 由 `settings.json` 推导每个条目的 `enabled`：`list` 只透露条目带过滤器，
/// 不透露还有什么会被加载。
///
/// 项目级条目保持默认 `enabled: true`：它们的过滤器在项目 settings 里，本模块不读，
/// 页面也不给它们开关——所以这一行的意思是「列在这个项目里」，不是「已验证加载」。
fn apply_extension_enabled_state(
    spec: &CliExtensionSpec,
    settings: &Value,
    extensions: &mut [CliExtensionSummary],
) {
    let packages = settings.get("packages").and_then(Value::as_array);
    let extensions_entries = settings.get(spec.extensions_dir);
    for extension in extensions.iter_mut() {
        // 项目条目在项目 settings 里，本模块不读：保持原状，别去匹配同名的用户级条目。
        if extension.scope == CliExtensionScope::Project {
            continue;
        }
        extension.enabled = match extension.kind {
            CliExtensionKind::Package => effective_package_entry(packages, &extension.source)
                .is_none_or(|entry| !is_disabled_package_entry(spec, entry)),
            CliExtensionKind::LocalFile | CliExtensionKind::LocalDirectory => {
                is_local_extension_enabled(
                    extensions_entries,
                    &local_extension_pattern(spec, &extension.source, extension.kind),
                )
            }
        };
        // 开关掌控不了的包条目（`autoload: false` 增量）没有可写错的状态。
        if extension.kind == CliExtensionKind::Package {
            extension.switch_supported = effective_package_entry(packages, &extension.source)
                .is_none_or(package_entry_supports_switch);
        }
    }
}

fn set_package_extension_enabled(
    spec: &CliExtensionSpec,
    settings_object: &mut Map<String, Value>,
    source: &str,
    enabled: bool,
) -> Result<(), String> {
    let packages = settings_object
        .get_mut("packages")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("Package '{source}' is not listed in settings.json"))?;
    // 与读路径同一条「取第一个」规则：CLI 会丢掉后面的重复项，
    // 编辑那些等于写一个它永远不读的过滤器。
    let entry_index = packages
        .iter()
        .position(|entry| package_entry_source(entry) == Some(source))
        .ok_or_else(|| format!("Package '{source}' is not listed in settings.json"))?;
    let entry = &mut packages[entry_index];

    if entry.is_string() {
        // 纯字符串形态已经启用全部资源，没什么可写。
        if enabled {
            return Ok(());
        }
        *entry = Value::Object(Map::from_iter([(
            "source".to_string(),
            Value::String(source.to_string()),
        )]));
    }

    let object = entry
        .as_object_mut()
        .ok_or_else(|| format!("Package '{source}' has an unsupported settings entry"))?;

    // 行开关掌控整个包：禁用让该包什么都不加载，启用则撤掉这个开关写下的东西。
    //
    // 禁用写 `[]`（每类一个都不加载，能压过用户自己的 `+path`）；用户原有的非空
    // 过滤器先挪进旁路键，启用时原样搬回——不搬就等于永久抹掉用户在 `config` TUI
    // 里逐文件的选择，且无法还原。
    if !enabled {
        let mut stashed = Map::new();
        for key in spec.resource_type_keys {
            if let Some(array) = object.get(*key).and_then(Value::as_array) {
                if !array.is_empty() {
                    stashed.insert((*key).to_string(), Value::Array(array.clone()));
                }
            }
            object.insert((*key).to_string(), Value::Array(Vec::new()));
        }
        if !stashed.is_empty() {
            object.insert(DISABLED_FILTERS_KEY.to_string(), Value::Object(stashed));
        }
        return Ok(());
    }

    // 启用：把旁路键里存的过滤器搬回各自的位置，并删掉本开关写的空数组。
    if let Some(stashed) = object.remove(DISABLED_FILTERS_KEY) {
        if let Some(stashed) = stashed.as_object() {
            for (key, value) in stashed {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    // 只删空数组。非空过滤器是用户在 `config` TUI 里逐文件的选择，不属于这个开关
    // ——删掉会静默复活用户排除的文件（上游自己的 TUI 也只删它切换的那一个模式）。
    // 这种条目保持部分过滤，读作启用。
    for key in spec.resource_type_keys {
        if object
            .get(*key)
            .is_some_and(|value| value.as_array().is_some_and(Vec::is_empty))
        {
            object.remove(*key);
        }
    }
    if entry.as_object().is_some_and(|object| object.len() == 1) {
        // 只剩 `source`：上游会把这种条目收回字符串形态
        // （`autoload` / 未知键会让它保持对象形态）。
        *entry = Value::String(source.to_string());
    }
    Ok(())
}

fn set_local_extension_enabled(
    spec: &CliExtensionSpec,
    settings_object: &mut Map<String, Value>,
    source: &str,
    kind: CliExtensionKind,
    enabled: bool,
) -> Result<(), String> {
    let pattern = local_extension_pattern(spec, source, kind);
    // 非数组值（手改出来的）不是这个命令该重写的：直接拒绝，
    // 而不是静默替换或删掉用户放的东西。
    let mut entries = match settings_object.get(spec.extensions_dir) {
        None => Vec::new(),
        Some(value) => value.as_array().cloned().ok_or_else(|| {
            format!(
                "{} in settings.json is not an array, so the extension switch cannot edit it",
                spec.extensions_dir
            )
        })?,
    };
    // 只有 `-pattern` 属于这个开关。用户自己的 `!pattern`（手工排除）或
    // `+pattern`（强制包含）保持不动：删掉会在用户背后改变加载内容。
    entries.retain(|entry| {
        let Some(value) = entry.as_str() else {
            return true;
        };
        let (prefix, payload) = split_pattern_prefix(value);
        prefix != Some('-') || normalize_exact_pattern(payload) != pattern
    });
    if !enabled {
        entries.push(Value::String(format!("-{pattern}")));
    }
    if entries.is_empty() {
        settings_object.remove(spec.extensions_dir);
    } else {
        settings_object.insert(spec.extensions_dir.to_string(), Value::Array(entries));
    }
    Ok(())
}

// ============================================================================
// 本地扩展扫描
// ============================================================================

fn scan_local_extensions(
    spec: &CliExtensionSpec,
    extensions_path: &Path,
) -> Result<Vec<CliExtensionSummary>, String> {
    let entries = match fs::read_dir(extensions_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Failed to read extensions directory {}: {error}",
                extensions_path.display()
            ));
        }
    };

    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read extensions directory entry in {}: {error}",
                extensions_path.display()
            )
        })?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with('.') || file_name == "node_modules" || file_name.ends_with(".d.ts")
        {
            continue;
        }

        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect extension {}: {error}", path.display()))?;
        let (source, kind) = if file_type.is_file() && file_name.ends_with(".ts") {
            (file_name.to_string(), CliExtensionKind::LocalFile)
        } else if file_type.is_dir() && path.join("index.ts").is_file() {
            (file_name.to_string(), CliExtensionKind::LocalDirectory)
        } else {
            continue;
        };

        // 清单驱动的目录会加载 `index.ts` 以外的文件，`-extensions/<dir>/index.ts`
        // 排除不掉它。这种行不给开关，而不是给一个静默无效的开关。
        let switch_supported =
            kind != CliExtensionKind::LocalDirectory || !directory_has_manifest_extensions(&path);

        result.push(CliExtensionSummary {
            id: format!("local:{source}"),
            built_in: is_protected_local_extension_source(spec, &source),
            source,
            scope: CliExtensionScope::User,
            kind,
            switch_supported,
            path: Some(path.to_string_lossy().to_string()),
            current_version: None,
            latest_version: None,
            update_available: false,
            enabled: true,
        });
    }

    result.sort_by(|left, right| left.source.cmp(&right.source));
    Ok(result)
}

// ============================================================================
// 版本与更新
// ============================================================================

fn read_package_current_version(extension: &CliExtensionSummary) -> Option<String> {
    if extension.kind != CliExtensionKind::Package {
        return None;
    }
    let path = extension.path.as_deref()?;
    let package_json_path = Path::new(path).join("package.json");
    let raw = fs::read_to_string(package_json_path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn enrich_current_versions(extensions: Vec<CliExtensionSummary>) -> Vec<CliExtensionSummary> {
    extensions
        .into_iter()
        .map(|extension| CliExtensionSummary {
            current_version: read_package_current_version(&extension),
            ..extension
        })
        .collect()
}

/// 解析 `npm:name` / `npm:@scope/name` 与可选的尾部 `@version` 钉版。
/// 是 npm 包时返回 `(package_name, pinned_version)`。
fn parse_npm_package_source(source: &str) -> Option<(String, Option<String>)> {
    let trimmed = source.trim();
    let without_prefix = trimmed.strip_prefix("npm:")?;
    if without_prefix.is_empty() {
        return None;
    }

    if let Some(rest) = without_prefix.strip_prefix('@') {
        // 带 scope：@scope/name 或 @scope/name@version
        let (name_part, version_part) = match rest.rsplit_once('@') {
            Some((name, version)) if name.contains('/') => (name, Some(version)),
            _ => (rest, None),
        };
        if name_part.is_empty() || !name_part.contains('/') {
            return None;
        }
        let package_name = format!("@{name_part}");
        let pinned = version_part
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_string);
        return Some((package_name, pinned));
    }

    // 不带 scope：name 或 name@version
    let (name_part, version_part) = match without_prefix.rsplit_once('@') {
        Some((name, version)) if !name.is_empty() => (name, Some(version)),
        _ => (without_prefix, None),
    };
    if name_part.is_empty() || name_part.contains('/') {
        return None;
    }
    let pinned = version_part
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string);
    Some((name_part.to_string(), pinned))
}

fn is_version_newer(latest: &str, current: &str) -> bool {
    let parse = |value: &str| -> Vec<u64> {
        value
            .trim()
            .trim_start_matches('v')
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u64>().ok())
            .collect()
    };
    let latest_parts = parse(latest);
    let current_parts = parse(current);
    if latest_parts.is_empty() || current_parts.is_empty() {
        return latest.trim() != current.trim() && !latest.trim().is_empty();
    }

    let max_len = latest_parts.len().max(current_parts.len());
    for index in 0..max_len {
        let left = latest_parts.get(index).copied().unwrap_or(0);
        let right = current_parts.get(index).copied().unwrap_or(0);
        if left > right {
            return true;
        }
        if left < right {
            return false;
        }
    }
    false
}

async fn fetch_npm_latest_version(client: &reqwest::Client, package_name: &str) -> Option<String> {
    let package_url = format!(
        "{}/{}",
        NPM_REGISTRY_BASE_URL,
        encode_url_path_segment(package_name)
    );
    let response = client
        .get(&package_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "AI-Toolbox")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let metadata = response.json::<Value>().await.ok()?;
    metadata
        .get("dist-tags")
        .and_then(|dist_tags| dist_tags.get("latest"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn enrich_npm_update_availability(
    db: &SqliteDbState,
    extensions: Vec<CliExtensionSummary>,
) -> Vec<CliExtensionSummary> {
    let client = match http_client::client_with_timeout(db, NPM_LATEST_LOOKUP_TIMEOUT_SECS).await {
        Ok(client) => client,
        Err(_) => return extensions,
    };

    let mut package_names = Vec::new();
    let mut seen_names = HashSet::new();
    for extension in &extensions {
        if extension.kind != CliExtensionKind::Package {
            continue;
        }
        let Some((package_name, pinned_version)) = parse_npm_package_source(&extension.source)
        else {
            continue;
        };
        if pinned_version.is_some() {
            continue;
        }
        if extension.current_version.is_none() {
            continue;
        }
        let key = package_name.to_ascii_lowercase();
        if seen_names.insert(key) {
            package_names.push(package_name);
        }
    }

    if package_names.is_empty() {
        return extensions;
    }

    let mut latest_by_package = HashMap::new();
    for chunk in package_names.chunks(NPM_LATEST_LOOKUP_CONCURRENCY) {
        let lookups = chunk.iter().map(|package_name| {
            let client = client.clone();
            let package_name = package_name.clone();
            async move {
                let latest = fetch_npm_latest_version(&client, &package_name).await;
                (package_name, latest)
            }
        });
        for (package_name, latest) in join_all(lookups).await {
            if let Some(version) = latest {
                latest_by_package.insert(package_name.to_ascii_lowercase(), version);
            }
        }
    }

    extensions
        .into_iter()
        .map(|mut extension| {
            if extension.kind != CliExtensionKind::Package {
                return extension;
            }
            let Some((package_name, pinned_version)) = parse_npm_package_source(&extension.source)
            else {
                return extension;
            };
            if pinned_version.is_some() {
                return extension;
            }
            let Some(latest_version) = latest_by_package
                .get(&package_name.to_ascii_lowercase())
                .cloned()
            else {
                return extension;
            };
            let update_available = extension
                .current_version
                .as_deref()
                .map(|current| is_version_newer(&latest_version, current))
                .unwrap_or(false);
            extension.latest_version = Some(latest_version);
            extension.update_available = update_available;
            extension
        })
        .collect()
}

fn merge_extensions(
    cli_extensions: Vec<CliExtensionSummary>,
    local_extensions: Vec<CliExtensionSummary>,
) -> Vec<CliExtensionSummary> {
    let mut seen = HashSet::new();
    let mut merged: Vec<CliExtensionSummary> = Vec::new();

    for extension in cli_extensions {
        seen.insert(extension_identity(&extension));
        merged.push(extension);
    }
    for extension in local_extensions {
        let identity = extension_identity(&extension);
        if seen.insert(identity) {
            merged.push(extension);
        }
    }

    // `list` 每个 settings 条目打一行，所以同一个 source 可能出现两次
    // （一个普通条目 + 一个过滤条目）。id 喂给 React key 与逐行补丁，
    // 重复必须消歧。
    let mut id_counts: HashMap<String, usize> = HashMap::new();
    for extension in merged.iter_mut() {
        let count = id_counts.entry(extension.id.clone()).or_insert(0);
        if *count > 0 {
            extension.id = format!("{}#{count}", extension.id);
        }
        *count += 1;
    }

    merged
}

fn extension_identity(extension: &CliExtensionSummary) -> String {
    extension
        .path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .unwrap_or(&extension.source)
        .to_string()
}

fn delete_local_extension(
    spec: &CliExtensionSpec,
    extensions_path: &Path,
    input: &CliExtensionActionInput,
) -> Result<(), String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("Extension source cannot be empty".to_string());
    }
    if is_protected_local_extension_source(spec, source) {
        return Err("Built-in extension cannot be deleted".to_string());
    }

    let target_path = input
        .path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| extensions_path.join(source));
    let canonical_extensions_path = fs::canonicalize(extensions_path).map_err(|error| {
        format!(
            "Failed to resolve extensions directory {}: {error}",
            extensions_path.display()
        )
    })?;
    let canonical_target_path = fs::canonicalize(&target_path).map_err(|error| {
        format!(
            "Failed to resolve extension path {}: {error}",
            target_path.display()
        )
    })?;
    if !canonical_target_path.starts_with(&canonical_extensions_path) {
        return Err(format!(
            "Extension path is outside extensions directory: {}",
            canonical_target_path.display()
        ));
    }

    if canonical_target_path.is_dir() {
        fs::remove_dir_all(&canonical_target_path).map_err(|error| {
            format!(
                "Failed to delete extension directory {}: {error}",
                canonical_target_path.display()
            )
        })
    } else {
        fs::remove_file(&canonical_target_path).map_err(|error| {
            format!(
                "Failed to delete extension file {}: {error}",
                canonical_target_path.display()
            )
        })
    }
}

// ============================================================================
// 命令入口（两个 CLI 共用）
// ============================================================================

/// `list` 的完整实现：跑 CLI、扫本地目录、合并、查更新、推导启用状态。
///
/// `settings_path` 由调用方给（各 CLI 的 settings 位置不同）。
pub async fn list_cli_extensions(
    spec: &CliExtensionSpec,
    db: &SqliteDbState,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
    settings_path: &Path,
) -> Result<CliExtensionListResult, String> {
    let extensions_path = extensions_path_from_root(&runtime_location.host_path, spec);
    let packages_path = packages_path_from_root(&runtime_location.host_path);
    let (raw, _) = run_cli_command_preferring_no_approve(
        spec,
        program,
        runtime_location,
        &["list", "--no-approve"],
        true,
    )
    .await?;
    let cli_extensions = enrich_current_versions(parse_list_output(&raw));
    // 扫描会碰 `<root>/extensions` 与每个条目的 `package.json`；WSL Direct 下
    // 那是 UNC 路径，所以在阻塞池上跑并加超时（见 `coding/AGENTS.md` 关于 UNC 读取）。
    let extensions_path_for_scan = extensions_path.clone();
    let spec_for_scan = *spec;
    let local_extensions = crate::coding::file_io::run_blocking_fs_operation(
        spec.scan_timeout,
        &format!("scan the {} extensions directory", spec.cli_name),
        &extensions_path.to_string_lossy(),
        move || scan_local_extensions(&spec_for_scan, &extensions_path_for_scan),
    )
    .await?;
    let merged = merge_extensions(cli_extensions, local_extensions);
    let mut extensions = enrich_npm_update_availability(db, merged).await;
    // 尽力而为：坏掉 / 读不了的 settings.json 不能让整个列表失败
    // （安装 / 卸载 / 更新是唯一能恢复的手段）。
    if let Ok(settings) = read_settings_object(settings_path) {
        apply_extension_enabled_state(spec, &settings, &mut extensions);
    }
    let cli_path = cli_display_path(spec, program, runtime_location);
    let cli_version = probe_cli_version(spec, program, runtime_location).await;

    Ok(CliExtensionListResult {
        extensions_path: extensions_path.to_string_lossy().to_string(),
        packages_path: packages_path.to_string_lossy().to_string(),
        extensions,
        raw,
        cli_path,
        cli_version,
    })
}

pub async fn install_cli_extension(
    spec: &CliExtensionSpec,
    app: &tauri::AppHandle,
    payload: &str,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
    input: &CliExtensionInstallInput,
) -> Result<CliExtensionCommandResult, String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("Extension source cannot be empty".to_string());
    }

    let args = ["install", source, "--no-approve"];
    let (output, used_args) =
        run_cli_command_preferring_no_approve(spec, program, runtime_location, &args, true).await?;
    emit_extensions_changed(spec, app, payload);

    Ok(CliExtensionCommandResult {
        command: format_cli_command_owned(spec, &used_args),
        output: output.trim().to_string(),
    })
}

pub async fn uninstall_cli_extension(
    spec: &CliExtensionSpec,
    app: &tauri::AppHandle,
    payload: &str,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
    input: &CliExtensionActionInput,
) -> Result<CliExtensionCommandResult, String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("Extension source cannot be empty".to_string());
    }

    let extensions_path = extensions_path_from_root(&runtime_location.host_path, spec);
    let kind = input.kind.unwrap_or(CliExtensionKind::Package);

    if kind != CliExtensionKind::Package {
        delete_local_extension(spec, &extensions_path, input)?;
        emit_extensions_changed(spec, app, payload);
        return Ok(CliExtensionCommandResult {
            command: format!("delete {}", source),
            output: String::new(),
        });
    }

    let mut args = vec!["remove", source];
    if input.scope == Some(CliExtensionScope::Project) {
        args.push("-l");
    }
    args.push("--no-approve");

    let (output, used_args) =
        run_cli_command_preferring_no_approve(spec, program, runtime_location, &args, true).await?;
    emit_extensions_changed(spec, app, payload);

    Ok(CliExtensionCommandResult {
        command: format_cli_command_owned(spec, &used_args),
        output: output.trim().to_string(),
    })
}

pub async fn update_cli_extensions(
    spec: &CliExtensionSpec,
    app: &tauri::AppHandle,
    payload: &str,
    program: &ResolvedCliProgram,
    runtime_location: &RuntimeLocationInfo,
    input: Option<&CliExtensionUpdateInput>,
) -> Result<CliExtensionCommandResult, String> {
    let single_source = input
        .as_ref()
        .and_then(|value| value.source.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    // 能带 `--no-approve` 就带；被拒则回退。
    // 单个包：`<cli> update <source>`；全部：`<cli> update --extensions`。
    let (output, used_args) = if let Some(source) = single_source.as_deref() {
        run_cli_command_preferring_no_approve(
            spec,
            program,
            runtime_location,
            &["update", source, "--no-approve"],
            false,
        )
        .await?
    } else {
        run_cli_command_preferring_no_approve(
            spec,
            program,
            runtime_location,
            &["update", "--extensions", "--no-approve"],
            false,
        )
        .await?
    };
    emit_extensions_changed(spec, app, payload);

    Ok(CliExtensionCommandResult {
        command: format_cli_command_owned(spec, &used_args),
        output: output.trim().to_string(),
    })
}

/// 通过写 `settings.json` 过滤器启用 / 停用一个已装扩展。
///
/// 只支持全局作用域：包条目在 `packages` 里，本地 `.ts` / 目录扩展通过顶层
/// `extensions` 数组强制排除。项目级条目直接拒绝——它们不在这个命令编辑的
/// 运行时根 settings 里。
pub async fn set_cli_extension_enabled(
    spec: &CliExtensionSpec,
    app: &tauri::AppHandle,
    payload: &str,
    settings_path: &Path,
    input: &CliExtensionEnabledInput,
) -> Result<(), String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("Extension source cannot be empty".to_string());
    }

    let mut settings = read_settings_object(settings_path)?;
    let settings_object = settings
        .as_object_mut()
        .ok_or_else(|| "settings.json is not a JSON object".to_string())?;

    match input.kind {
        CliExtensionKind::Package => {
            set_package_extension_enabled(spec, settings_object, source, input.enabled)?
        }
        CliExtensionKind::LocalFile | CliExtensionKind::LocalDirectory => {
            set_local_extension_enabled(spec, settings_object, source, input.kind, input.enabled)?
        }
    }

    write_settings_object(settings_path, &settings)?;
    emit_extensions_changed(spec, app, payload);
    Ok(())
}

// ============================================================================
// settings.json 读写
// ============================================================================

/// 读 settings.json；不存在或不可解析时返回空对象（列表页要能容忍坏文件）。
fn read_settings_object(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str::<Value>(&raw)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))
}

fn write_settings_object(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("Failed to serialize {}: {error}", path.display()))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn emit_extensions_changed(spec: &CliExtensionSpec, app: &tauri::AppHandle, payload: &str) {
    let _ = app.emit("config-changed", payload);
    // settings.json 是 WSL/SSH 的文件映射，不请求同步就会让远端留着旧配置。
    let _ = app.emit(spec.wsl_sync_event, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SPEC: CliExtensionSpec = CliExtensionSpec {
        cli_name: "pi",
        env_key: "PI_CODING_AGENT_DIR",
        offline_env_key: "PI_OFFLINE",
        extensions_dir: "extensions",
        protected_prefixes: &["pi-deck-", "ai-toolbox-"],
        resource_type_keys: &["extensions", "skills", "prompts", "themes"],
        scan_timeout: std::time::Duration::from_secs(20),
        wsl_sync_event: "wsl-sync-request-pi",
    };

    #[test]
    fn wsl_path_prefix_includes_common_user_install_locations() {
        let prefix = wsl_path_prefix(Some("/home/tester"));
        assert_eq!(
            prefix,
            "/home/tester/.local/share/mise/shims:\
/home/tester/.asdf/shims:\
/home/tester/.local/bin:\
/home/tester/.bun/bin:\
/home/tester/.volta/bin:\
/home/tester/.local/share/fnm/aliases/default/bin:\
/home/tester/.fnm/aliases/default/bin:\
/home/tester/.fnm/current/bin:\
/home/tester/.npm-global/bin"
        );
        assert_eq!(wsl_path_prefix(None), "");
    }

    #[test]
    fn build_cli_command_keeps_wsl_paths_and_cli_args_out_of_shell_script() {
        let runtime_location = crate::coding::runtime_location::RuntimeLocationInfo {
            mode: crate::coding::runtime_location::RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from(
                r"\\wsl.localhost\Ubuntu\home\test user\.pi;echo injected\agent",
            ),
            wsl: Some(crate::coding::runtime_location::WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/test user/.pi;echo injected/agent".to_string(),
                linux_user_root: Some("/home/test user".to_string()),
            }),
        };
        let package_source = "file:/tmp/extension dir;$(touch /tmp/injected)";
        let invocation = build_cli_command(
            &TEST_SPEC,
            &ResolvedCliProgram {
                path: PathBuf::from("pi"),
            },
            &runtime_location,
            &["install", package_source],
            true,
        )
        .expect("build WSL command");
        let command = invocation.command.as_std();
        let command_args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program(), "wsl");
        assert_eq!(
            command_args[0..7],
            [
                "-d",
                "Ubuntu",
                "--exec",
                "/bin/sh",
                "-c",
                WSL_CLI_COMMAND_SCRIPT,
                "ai-toolbox-ext"
            ]
        );
        assert_eq!(
            command_args[7],
            "/home/test user/.local/share/mise/shims:\
/home/test user/.asdf/shims:\
/home/test user/.local/bin:\
/home/test user/.bun/bin:\
/home/test user/.volta/bin:\
/home/test user/.local/share/fnm/aliases/default/bin:\
/home/test user/.fnm/aliases/default/bin:\
/home/test user/.fnm/current/bin:\
/home/test user/.npm-global/bin"
        );
        assert_eq!(command_args[8], "/home/test user/.pi;echo injected/agent");
        // 环境变量**键名**必须作为位置参数传给脚本：`command.env()` 设的是
        // Windows 侧变量，不转发进 WSL，脚本会 `export ""=…` 直接失败。
        assert_eq!(command_args[9], TEST_SPEC.env_key);
        assert_eq!(
            &command_args[10..],
            [
                "env",
                "NPM_CONFIG_LEGACY_PEER_DEPS=true",
                "PI_OFFLINE=1",
                TEST_SPEC.cli_name,
                "install",
                package_source,
            ]
        );
        // 动态键名不能出现在脚本正文里（脚本里只有 `$env_key`）。
        assert!(!WSL_CLI_COMMAND_SCRIPT.contains(TEST_SPEC.env_key));
        assert!(WSL_CLI_COMMAND_SCRIPT.contains(r#"export "$env_key"="$agent_dir""#));
        assert!(!WSL_CLI_COMMAND_SCRIPT.contains("test user"));
        assert!(!WSL_CLI_COMMAND_SCRIPT.contains("injected"));
        assert!(WSL_CLI_COMMAND_SCRIPT.contains("${PATH:+:$PATH}"));
    }

    #[test]
    fn parses_list_output_with_user_and_project_scopes() {
        let raw = r#"
User packages:
  npm:context-mode
    /home/tester/.pi/agent/npm/node_modules/context-mode
Project packages:
  github:owner/repo
    /project/.pi/extensions/repo
"#;

        let extensions = parse_list_output(raw);

        assert_eq!(extensions.len(), 2);
        assert_eq!(extensions[0].source, "npm:context-mode");
        assert_eq!(extensions[0].scope, CliExtensionScope::User);
        assert_eq!(extensions[0].kind, CliExtensionKind::Package);
        assert_eq!(
            extensions[0].path.as_deref(),
            Some("/home/tester/.pi/agent/npm/node_modules/context-mode")
        );
        assert_eq!(extensions[1].source, "github:owner/repo");
        assert_eq!(extensions[1].scope, CliExtensionScope::Project);
    }

    #[test]
    fn strips_filtered_suffix_from_listed_package_sources() {
        let raw = r#"
User packages:
  npm:pi-web-access
    /home/tester/.pi/agent/npm/node_modules/pi-web-access
  npm:pi-mcp-adapter (filtered)
    /home/tester/.pi/agent/npm/node_modules/pi-mcp-adapter
"#;

        let extensions = parse_list_output(raw);

        assert_eq!(extensions.len(), 2);
        assert_eq!(extensions[1].source, "npm:pi-mcp-adapter");
        assert_eq!(extensions[1].id, "user:npm:pi-mcp-adapter");
        assert_eq!(
            strip_list_filtered_suffix("npm:pi-web-access"),
            "npm:pi-web-access"
        );
    }

    fn local_extension_summary(source: &str, kind: CliExtensionKind) -> CliExtensionSummary {
        CliExtensionSummary {
            id: format!("local:{source}"),
            source: source.to_string(),
            scope: CliExtensionScope::User,
            kind,
            path: None,
            built_in: false,
            current_version: None,
            latest_version: None,
            update_available: false,
            enabled: true,
            switch_supported: true,
        }
    }

    fn package_summary(source: &str) -> CliExtensionSummary {
        local_extension_summary(source, CliExtensionKind::Package)
    }

    #[test]
    fn marks_packages_with_empty_resource_arrays_as_disabled() {
        let settings: Value = serde_json::from_str(
            r#"{
  "packages": [
    "npm:pi-web-access",
    { "source": "npm:pi-mcp-adapter", "extensions": [], "skills": [], "prompts": [], "themes": [] },
    { "source": "npm:pi-slopchop", "extensions": ["-src/index.ts"] },
    { "source": "npm:@samfp/pi-memory", "extensions": [] }
  ]
}"#,
        )
        .expect("parse settings");

        let mut extensions = vec![
            package_summary("npm:pi-web-access"),
            package_summary("npm:pi-mcp-adapter"),
            package_summary("npm:pi-slopchop"),
            package_summary("npm:@samfp/pi-memory"),
            package_summary("npm:@narumitw/pi-lsp"),
        ];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        let enabled: Vec<bool> = extensions.iter().map(|item| item.enabled).collect();
        // 普通字符串；每种类型都空；手写的逐文件过滤器（其它资源仍在加载，所以这行是启用）；
        // 单一空类型（只过滤列出的类型，skills 等照常加载）；未列出的 source。
        assert_eq!(enabled, vec![true, false, true, true, true]);
    }

    #[test]
    fn only_a_package_entry_with_every_resource_type_empty_counts_as_disabled() {
        // 只写 `extensions: []` 时 skills/prompts/themes 仍走 `collectDefaultResources`，
        // 所以这一行不能声称「已禁用」。
        let settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-mcp-adapter", "extensions": [] }] }"#,
        )
        .expect("parse settings");

        let mut extensions = vec![package_summary("npm:pi-mcp-adapter")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        assert!(extensions[0].enabled);
    }

    #[test]
    fn autoload_delta_with_a_force_include_is_not_disabled() {
        // 实测：`applyAutoloadDisabledPatterns` 对 `+` 前缀设 `enabled = true`，
        // 所以 `["+index.ts", "!nope"]` 会把 `index.ts` 加载起来——哪怕其余模式
        // 全是排除。这种条目不能读作「已禁用」。
        let settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-mcp-adapter", "autoload": false, "extensions": ["+index.ts", "!nope"], "skills": ["!nope"], "prompts": ["!nope"], "themes": ["!nope"] }] }"#,
        )
        .expect("parse settings");

        let mut extensions = vec![package_summary("npm:pi-mcp-adapter")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        assert!(extensions[0].enabled, "有 `+` 强制包含时不算禁用");
    }

    #[test]
    fn autoload_delta_with_only_excludes_is_disabled() {
        // 全由 `-`/`!` 构成的 delta 一个资源都不加载（缺键同样不贡献资源）。
        let settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-mcp-adapter", "autoload": false, "extensions": ["!nope"], "skills": ["!nope"], "prompts": ["!nope"], "themes": ["!nope"] }] }"#,
        )
        .expect("parse settings");

        let mut extensions = vec![package_summary("npm:pi-mcp-adapter")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        assert!(!extensions[0].enabled);
    }

    #[test]
    fn marks_force_excluded_local_extensions_as_disabled() {
        let settings: Value = serde_json::from_str(
            r#"{ "extensions": ["-extensions/foo.ts", "-extensions/bar/index.ts"] }"#,
        )
        .expect("parse settings");

        let mut extensions = vec![
            local_extension_summary("foo.ts", CliExtensionKind::LocalFile),
            local_extension_summary("bar", CliExtensionKind::LocalDirectory),
            local_extension_summary("baz.ts", CliExtensionKind::LocalFile),
        ];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        let enabled: Vec<bool> = extensions.iter().map(|item| item.enabled).collect();
        assert_eq!(enabled, vec![false, false, true]);
    }

    #[test]
    fn force_include_and_force_exclude_follow_override_order() {
        // `!` 先排除，`+` 再包含，`-` 最后排除——顺序不能变。
        let settings: Value = serde_json::from_str(
            r#"{ "extensions": ["!extensions/*.ts", "+extensions/keep.ts", "-extensions/drop.ts"] }"#,
        )
        .expect("parse settings");

        let mut extensions = vec![
            local_extension_summary("keep.ts", CliExtensionKind::LocalFile),
            local_extension_summary("drop.ts", CliExtensionKind::LocalFile),
            local_extension_summary("other.ts", CliExtensionKind::LocalFile),
        ];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        let enabled: Vec<bool> = extensions.iter().map(|item| item.enabled).collect();
        assert_eq!(enabled, vec![true, false, false]);
    }

    #[test]
    fn disabling_a_package_writes_empty_resource_arrays() {
        let mut settings: Value =
            serde_json::from_str(r#"{ "packages": ["npm:pi-web-access"] }"#).expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_package_extension_enabled(&TEST_SPEC, object, "npm:pi-web-access", false)
            .expect("disable");

        let entry = &object["packages"][0];
        for key in TEST_SPEC.resource_type_keys {
            assert_eq!(
                entry[key],
                Value::Array(Vec::new()),
                "{key} should be empty"
            );
        }
    }

    #[test]
    fn disabling_a_package_beats_a_force_include() {
        // 实测（真实 pi 引擎的 `DefaultPackageManager.resolve`）：用户写 `+path`
        // 强制包含时，`!**` 挡不住它——`applyPatterns` 的强制包含在排除**之后**
        // 执行，包照常加载。只有 `[]` 走 `applyPackageFilter` 的无条件全禁分支。
        // 所以禁用必须写 `[]`，这个测试守住这条。
        let mut settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-slopchop", "extensions": ["+src/index.ts"], "skills": [], "prompts": [], "themes": [] }] }"#,
        )
        .expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_package_extension_enabled(&TEST_SPEC, object, "npm:pi-slopchop", false)
            .expect("disable");

        let entry = &object["packages"][0];
        for key in TEST_SPEC.resource_type_keys {
            assert_eq!(
                entry[key],
                Value::Array(Vec::new()),
                "{key} 必须是空数组，`+path` 才不会把包重新拉起来"
            );
        }
        assert_eq!(
            entry[DISABLED_FILTERS_KEY]["extensions"],
            serde_json::json!(["+src/index.ts"]),
            "被压过的 `+path` 也要原样存着，启用时还回去"
        );
    }

    #[test]
    fn disabling_twice_keeps_the_stashed_filters() {
        // 第二次禁用时四类已经全空，`stashed` 为空——这时**不能**把已有的旁路键
        // 清掉，否则用户点两下开关（或前端重试）就把自己的过滤器弄丢了。
        let mut settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-slopchop", "extensions": ["-src/index.ts"] }] }"#,
        )
        .expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_package_extension_enabled(&TEST_SPEC, object, "npm:pi-slopchop", false)
            .expect("first disable");
        set_package_extension_enabled(&TEST_SPEC, object, "npm:pi-slopchop", false)
            .expect("second disable");

        assert_eq!(
            object["packages"][0][DISABLED_FILTERS_KEY]["extensions"],
            serde_json::json!(["-src/index.ts"]),
            "重复禁用不能丢掉已暂存的过滤器"
        );
    }

    #[test]
    fn disabling_a_package_stashes_the_users_own_filters() {
        // 禁用必须写 `[]`（唯一压得过用户 `+path` 的形态），但用户自己的
        // `-src/index.ts` 不能因此消失，先挪进旁路键，启用时搬回来。
        let mut settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-slopchop", "extensions": ["-src/index.ts"] }] }"#,
        )
        .expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_package_extension_enabled(&TEST_SPEC, object, "npm:pi-slopchop", false)
            .expect("disable");

        let entry = &object["packages"][0];
        assert_eq!(
            entry["extensions"],
            Value::Array(Vec::new()),
            "四类资源都必须清空"
        );
        assert_eq!(
            entry[DISABLED_FILTERS_KEY]["extensions"],
            serde_json::json!(["-src/index.ts"]),
            "用户自己的过滤器挪进旁路键"
        );
    }

    #[test]
    fn disable_enable_round_trip_restores_the_users_own_filters() {
        let mut settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-slopchop", "extensions": ["-src/index.ts"] }] }"#,
        )
        .expect("parse");

        set_package_extension_enabled(
            &TEST_SPEC,
            settings.as_object_mut().expect("object"),
            "npm:pi-slopchop",
            false,
        )
        .expect("disable");
        let mut extensions = vec![package_summary("npm:pi-slopchop")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);
        assert!(!extensions[0].enabled, "四类清空后必须被读作已禁用");

        set_package_extension_enabled(
            &TEST_SPEC,
            settings.as_object_mut().expect("object"),
            "npm:pi-slopchop",
            true,
        )
        .expect("enable");
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);
        assert!(extensions[0].enabled);

        // 往返之后用户的过滤器逐字还原，旁路键不残留。
        assert_eq!(
            settings["packages"][0]["extensions"],
            serde_json::json!(["-src/index.ts"])
        );
        assert!(
            settings["packages"][0].get(DISABLED_FILTERS_KEY).is_none(),
            "启用后旁路键必须消失"
        );
    }

    #[test]
    fn enabling_a_package_drops_empty_filters_and_collapses_to_string() {
        let mut settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-web-access", "extensions": [], "skills": [] }] }"#,
        )
        .expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_package_extension_enabled(&TEST_SPEC, object, "npm:pi-web-access", true)
            .expect("enable");

        // 空过滤器全删、只剩 source：收回字符串形态。
        assert_eq!(
            object["packages"][0],
            Value::String("npm:pi-web-access".to_string())
        );
    }

    #[test]
    fn enabling_a_package_keeps_hand_written_filters() {
        let mut settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-slopchop", "extensions": ["-src/index.ts"] }] }"#,
        )
        .expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_package_extension_enabled(&TEST_SPEC, object, "npm:pi-slopchop", true).expect("enable");

        // 非空过滤器不是这个开关写的，删掉会静默复活用户排除的文件。
        assert_eq!(
            object["packages"][0]["extensions"][0],
            Value::String("-src/index.ts".to_string())
        );
    }

    #[test]
    fn switch_state_stays_consistent_after_a_disable_enable_round_trip() {
        let mut settings: Value =
            serde_json::from_str(r#"{ "packages": ["npm:pi-web-access"] }"#).expect("parse");

        set_package_extension_enabled(
            &TEST_SPEC,
            settings.as_object_mut().expect("object"),
            "npm:pi-web-access",
            false,
        )
        .expect("disable");
        let mut extensions = vec![package_summary("npm:pi-web-access")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);
        assert!(!extensions[0].enabled);

        set_package_extension_enabled(
            &TEST_SPEC,
            settings.as_object_mut().expect("object"),
            "npm:pi-web-access",
            true,
        )
        .expect("enable");
        let mut extensions = vec![package_summary("npm:pi-web-access")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);
        assert!(extensions[0].enabled);
    }

    #[test]
    fn autoload_false_entries_offer_no_switch() {
        let settings: Value = serde_json::from_str(
            r#"{ "packages": [{ "source": "npm:pi-web-access", "autoload": false }] }"#,
        )
        .expect("parse settings");

        let mut extensions = vec![package_summary("npm:pi-web-access")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        assert!(!extensions[0].switch_supported);
    }

    #[test]
    fn detects_manifest_driven_directory_extensions() {
        let temp_dir =
            std::env::temp_dir().join(format!("ai-toolbox-ext-manifest-{}", std::process::id()));
        let with_manifest = temp_dir.join("with-manifest");
        let without_manifest = temp_dir.join("without-manifest");
        fs::create_dir_all(&with_manifest).expect("create with-manifest");
        fs::create_dir_all(&without_manifest).expect("create without-manifest");
        fs::write(
            with_manifest.join("package.json"),
            r#"{ "pi": { "extensions": ["src/index.ts"] } }"#,
        )
        .expect("write package.json");
        fs::write(without_manifest.join("package.json"), r#"{ "name": "x" }"#)
            .expect("write package.json");

        assert!(directory_has_manifest_extensions(&with_manifest));
        assert!(!directory_has_manifest_extensions(&without_manifest));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn disambiguates_duplicate_extension_ids() {
        let merged = merge_extensions(
            vec![package_summary("npm:dup"), package_summary("npm:dup")],
            Vec::new(),
        );

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "local:npm:dup");
        assert_eq!(merged[1].id, "local:npm:dup#1");
    }

    #[test]
    fn reads_the_duplicate_package_entry_pi_actually_honours() {
        // `dedupePackages` 保留第一个条目、丢掉后面的，所以读第二个会报错状态。
        let settings: Value = serde_json::from_str(
            r#"{ "packages": [
                "npm:dup",
                { "source": "npm:dup", "extensions": [], "skills": [], "prompts": [], "themes": [] }
            ] }"#,
        )
        .expect("parse settings");

        let mut extensions = vec![package_summary("npm:dup")];
        apply_extension_enabled_state(&TEST_SPEC, &settings, &mut extensions);

        assert!(extensions[0].enabled, "第一个条目是纯字符串，应读作启用");
    }

    #[test]
    fn writes_to_the_duplicate_package_entry_pi_actually_honours() {
        let mut settings: Value = serde_json::from_str(
            r#"{ "packages": [
                "npm:dup",
                { "source": "npm:dup", "extensions": [] }
            ] }"#,
        )
        .expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_package_extension_enabled(&TEST_SPEC, object, "npm:dup", false).expect("disable");

        assert_eq!(
            object["packages"][0]["extensions"],
            Value::Array(Vec::new())
        );
        // 第二个条目原样不动。
        assert_eq!(
            object["packages"][1]["extensions"],
            Value::Array(Vec::new())
        );
    }

    #[test]
    fn rejects_non_array_top_level_extensions() {
        let mut settings: Value =
            serde_json::from_str(r#"{ "extensions": "nope" }"#).expect("parse");
        let object = settings.as_object_mut().expect("object");

        let result = set_local_extension_enabled(
            &TEST_SPEC,
            object,
            "foo.ts",
            CliExtensionKind::LocalFile,
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn keeps_user_written_bang_and_plus_entries_untouched() {
        let mut settings: Value = serde_json::from_str(
            r#"{ "extensions": ["!extensions/foo.ts", "+extensions/foo.ts"] }"#,
        )
        .expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_local_extension_enabled(
            &TEST_SPEC,
            object,
            "foo.ts",
            CliExtensionKind::LocalFile,
            false,
        )
        .expect("disable");

        let entries = object["extensions"].as_array().expect("array");
        assert!(entries.contains(&Value::String("!extensions/foo.ts".to_string())));
        assert!(entries.contains(&Value::String("+extensions/foo.ts".to_string())));
        assert!(entries.contains(&Value::String("-extensions/foo.ts".to_string())));
    }

    #[test]
    fn glob_excludes_match_like_pi_for_bang_entries() {
        assert!(glob_matches("extensions/*.ts", "extensions/foo.ts"));
        assert!(!glob_matches("extensions/*.ts", "extensions/nested/foo.ts"));
        // 不带 `/` 的模式也测 basename。
        assert!(glob_matches("foo.ts", "extensions/foo.ts"));
        assert!(glob_matches("extensions/?.ts", "extensions/a.ts"));
    }

    #[test]
    fn normalizes_windows_backslash_patterns_from_pi() {
        assert_eq!(
            normalize_exact_pattern(r"extensions\foo.ts"),
            "extensions/foo.ts"
        );
        assert_eq!(
            normalize_exact_pattern("./extensions/foo.ts"),
            "extensions/foo.ts"
        );
        // 上游剥的是 `./` 或 `.\`，只剥前者会让这条永远比较不相等。
        assert_eq!(
            normalize_exact_pattern(r".\extensions\foo.ts"),
            "extensions/foo.ts"
        );
    }

    #[test]
    fn rejects_package_sources_missing_from_settings() {
        let mut settings: Value = serde_json::from_str(r#"{ "packages": [] }"#).expect("parse");
        let object = settings.as_object_mut().expect("object");

        let result = set_package_extension_enabled(&TEST_SPEC, object, "npm:ghost", false);

        assert!(result.is_err());
    }

    #[test]
    fn toggling_a_local_extension_writes_and_removes_force_exclude() {
        let mut settings: Value = serde_json::from_str(r#"{}"#).expect("parse");
        let object = settings.as_object_mut().expect("object");

        set_local_extension_enabled(
            &TEST_SPEC,
            object,
            "foo.ts",
            CliExtensionKind::LocalFile,
            false,
        )
        .expect("disable");
        assert_eq!(
            object["extensions"],
            serde_json::json!(["-extensions/foo.ts"])
        );

        set_local_extension_enabled(
            &TEST_SPEC,
            object,
            "foo.ts",
            CliExtensionKind::LocalFile,
            true,
        )
        .expect("enable");
        // 空了就把整个键删掉，不留空数组垃圾。
        assert!(!object.contains_key("extensions"));
    }

    #[test]
    fn detects_unknown_no_approve_option_errors() {
        assert!(is_unknown_no_approve_option_error(
            "error: unknown option '--no-approve'"
        ));
        assert!(is_unknown_no_approve_option_error("Unknown argument: -na"));
        assert!(!is_unknown_no_approve_option_error("network unreachable"));
    }

    #[test]
    fn strips_no_approve_flags_from_args() {
        assert_eq!(
            args_without_no_approve(&["list", "--no-approve", "-na", "extra"]),
            vec!["list", "extra"]
        );
    }

    #[test]
    fn parses_npm_package_source_with_optional_pin() {
        assert_eq!(
            parse_npm_package_source("npm:pi-web-access"),
            Some(("pi-web-access".to_string(), None))
        );
        assert_eq!(
            parse_npm_package_source("npm:pi-web-access@1.2.3"),
            Some(("pi-web-access".to_string(), Some("1.2.3".to_string())))
        );
        assert_eq!(
            parse_npm_package_source("npm:@scope/pkg@1.0.0"),
            Some(("@scope/pkg".to_string(), Some("1.0.0".to_string())))
        );
        assert_eq!(
            parse_npm_package_source("npm:@scope/pkg"),
            Some(("@scope/pkg".to_string(), None))
        );
        assert_eq!(parse_npm_package_source("github:owner/repo"), None);
    }

    #[test]
    fn compares_semverish_versions_for_update_detection() {
        assert!(is_version_newer("1.2.4", "1.2.3"));
        assert!(is_version_newer("2.0.0", "1.9.9"));
        assert!(!is_version_newer("1.2.3", "1.2.3"));
        assert!(!is_version_newer("1.2.3", "1.2.4"));
        assert!(is_version_newer("v1.3.0", "1.2.9"));
    }

    #[test]
    fn scans_local_file_and_directory_extensions() {
        let temp_dir =
            std::env::temp_dir().join(format!("ai-toolbox-ext-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(temp_dir.join("dir-ext")).expect("create dir-ext");
        fs::write(
            temp_dir.join("dir-ext").join("index.ts"),
            "export default {};",
        )
        .expect("write index.ts");
        fs::write(temp_dir.join("file-ext.ts"), "export default {};").expect("write file-ext");
        fs::write(temp_dir.join("ignored.d.ts"), "declare const x: number;")
            .expect("write ignored");
        fs::create_dir_all(temp_dir.join("not-an-extension")).expect("create plain dir");

        let extensions = scan_local_extensions(&TEST_SPEC, &temp_dir).expect("scan");

        let sources: Vec<&str> = extensions.iter().map(|item| item.source.as_str()).collect();
        assert_eq!(sources, vec!["dir-ext", "file-ext.ts"]);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn npm_env_uses_legacy_peer_deps() {
        let env = cli_extension_npm_compat_env();
        assert_eq!(env[0].0, "NPM_CONFIG_LEGACY_PEER_DEPS");
        assert_eq!(env[0].1, "true");
    }

    #[test]
    fn protects_builtin_local_extension_prefixes() {
        assert!(is_protected_local_extension_source(
            &TEST_SPEC,
            "pi-deck-thing"
        ));
        assert!(is_protected_local_extension_source(
            &TEST_SPEC,
            "ai-toolbox-x"
        ));
        assert!(!is_protected_local_extension_source(&TEST_SPEC, "my-ext"));
    }

    fn local_runtime_location(host_path: &str) -> RuntimeLocationInfo {
        RuntimeLocationInfo {
            mode: RuntimeLocationMode::LocalWindows,
            source: "test".to_string(),
            host_path: PathBuf::from(host_path),
            wsl: None,
        }
    }

    #[test]
    fn npm_remote_policy_error_gets_actionable_hint() {
        // 原文来自 2026-10-07 实测：`pi-mcp-adapter@2.33.0` 的传递依赖指向
        // pkg.pr.new 的 tarball，npm 12 默认 `allow-remote=none` 直接拒绝。
        let raw = "npm error code EALLOWREMOTE\n\
npm error Fetching packages of type \"remote\" have been disabled\n\
npm error Refusing to fetch \"@modelcontextprotocol/core@https://pkg.pr.new/modelcontextprotocol/typescript-sdk/@modelcontextprotocol/core@3b205e7\"";

        let location = local_runtime_location("C:\\Users\\tester\\.omo\\agent");
        let annotated = annotate_npm_policy_error(raw.to_string(), &location);

        // 原始输出必须保留——包名和 URL 是排查的关键信息。
        assert!(annotated.contains("EALLOWREMOTE"));
        assert!(annotated.contains("Refusing to fetch"));
        assert!(annotated.contains("allow-remote=all"));
        assert!(annotated.contains("@modelcontextprotocol/core"));
        // 提示里给出的 `.npmrc` 路径必须与实际要写的文件一致。这里按实现
        // 取期望值，而不是写死分隔符：Windows 运行时给 `\\npm`，而 CI 的
        // Linux runner 上 `PathBuf::join` 产出 `/npm`（报告 2026-10-08 的
        // `Run Rust tests` 失败正是断言写死了 Windows 分隔符）。
        assert!(annotated.contains(&format!("{}/.npmrc", packages_dir_display(&location))));
    }

    #[test]
    fn npm_scripts_policy_error_gets_actionable_hint() {
        let raw = "npm error code EALLOWSCRIPTS\n\
npm error --allow-scripts is not allowed in project-scoped installs. Add the entries to the \"allowScripts\" field in package.json, or to .npmrc, instead.";

        let location = local_runtime_location("C:\\Users\\tester\\.pi\\agent");
        let annotated = annotate_npm_policy_error(raw.to_string(), &location);

        assert!(annotated.contains("EALLOWSCRIPTS"));
        assert!(annotated.contains("allow-scripts=<包名，逗号分隔>"));
        assert!(annotated.contains("allowScripts"));
        assert!(annotated.contains(&format!("{}/.npmrc", packages_dir_display(&location))));
    }

    #[test]
    fn wsl_npm_hint_uses_linux_path() {
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "test".to_string(),
            host_path: PathBuf::from("\\\\wsl$\\Debian\\home\\tester\\.pi\\agent"),
            wsl: Some(crate::coding::runtime_location::WslLocationInfo {
                distro: "Debian".to_string(),
                linux_path: "/home/tester/.pi/agent".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };

        let annotated =
            annotate_npm_policy_error("npm error code EALLOWREMOTE".to_string(), &location);

        assert!(annotated.contains("`/home/tester/.pi/agent/npm/.npmrc`"));
        // 宿主 UNC 路径不应出现在提示里——用户要在 WSL 里改那个文件。
        assert!(!annotated.contains("wsl$"));
    }

    #[test]
    fn unrelated_failure_gets_no_npm_hint() {
        let raw = "npm error code ENOTFOUND\nnpm error network request failed";
        assert_eq!(
            annotate_npm_policy_error(raw.to_string(), &local_runtime_location("C:\\agent")),
            raw
        );
    }

    #[test]
    fn extracts_scoped_and_unscoped_refused_packages() {
        assert_eq!(
            extract_refused_package("npm error Refusing to fetch \"@scope/pkg@https://x/y\""),
            Some("@scope/pkg".to_string())
        );
        assert_eq!(
            extract_refused_package("npm error Refusing to fetch \"plain@https://x/y\""),
            Some("plain".to_string())
        );
        assert_eq!(extract_refused_package("no marker here"), None);
    }
}
