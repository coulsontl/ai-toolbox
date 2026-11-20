use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 运行时根目录信息（`~/.omo/agent` 或自定义），source 为
/// `custom` / `env` / `default`，与 `oh_my_pi` 的 `OmpPathInfo` 语义一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativePathInfo {
    pub path: String,
    pub source: String,
}

/// Native 的运行时根目录设置（DB 表 `omo_native_settings_config` 的 `common` 记录）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeSettingsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeSettingsConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

/// 一个引擎内建 provider 的只读视图（官方认证渠道）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeBuiltinProvider {
    pub id: String,
    pub name: String,
    /// 是否需要 OAuth 登录（`omo auth check` 的判定输入之一）。
    pub oauth: bool,
    /// 凭据是否已就绪；`false` 时该 provider 在 `/model` 里不可用。
    pub ready: bool,
    /// `omo auth check` 报出的认证方式，例如 `api_key` / `oauth`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    /// 未就绪时的原因，例如 `credentials_not_configured`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 该 provider 的内建模型（`omo --list-models` 里 provider 列匹配的那些）。
    pub models: Vec<OmoNativeBuiltinModel>,
}

/// 内建 provider 下的一条内建模型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeBuiltinModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<i64>,
    pub thinking: bool,
    pub images: bool,
}

/// 二进制检测结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeCliInfo {
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `omo --version` 的原始输出，例如 `omo 5.1.19 (engine: senpi 2026.10.10; scheme nodef)`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 从 version 输出里解析出的产品版本号，例如 `5.1.19`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_number: Option<String>,
    /// 引擎版本，例如 `2026.10.10`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Agent/Category 配置方案（DB 表 `omo_native_agents_config`）。
///
/// 只承载 `[native]` 块内的 `agents` / `categories` / `model_profiles` /
/// `model_profile` / `task` 五个键——`other_fields` 用于保留用户在方案里
/// 手写的其他 Native 合法键，避免保存时被吞掉。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeAgentsConfig {
    pub id: String,
    pub name: String,
    pub is_applied: bool,
    pub is_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profiles: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// 写入用的方案内容（`id`/时间戳由后端生成）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeAgentsConfigContent {
    pub name: String,
    pub is_applied: bool,
    pub is_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profiles: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeAgentsConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub is_applied: bool,
    #[serde(default)]
    pub is_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profiles: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
}

/// `settings.json` 里的默认模型选择（`/model` 里 Ctrl+S 保存的那三个键）。
///
/// 引擎文档 `docs/settings.md` 的 "Model & Thinking" 一节：`defaultProvider` /
/// `defaultModel` / `defaultThinkingLevel`。与 Pi 同名同语义——两边都是 senpi 引擎。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeDefaultSelection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
}

/// 写入用的默认模型选择。字段为 `Some("")` 表示删除该键，`None` 表示不动。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeModelSettingsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<String>,
}

/// `save_omo_native_other_settings` 的入参：只提交用户实际改动的键。
///
/// 页面打开期间引擎仍会写 `settings.json`（`tipsHistory` 等）；如果整份覆盖
/// 「其他设置」切片，页面内存副本会把引擎的新写入回退。所以前端只发
/// `set`（新增/修改）与 `remove`（删除），后端在盘上最新内容上打补丁。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeOtherSettingsInput {
    /// 新增或修改的键。
    #[serde(default)]
    pub set: Map<String, Value>,
    /// 要删除的键。
    #[serde(default)]
    pub remove: Vec<String>,
}

/// `read_omo_native_runtime_config` 的返回：当前生效视图 + 各文件原文。
///
/// `effective` 是**折叠后的 Native 生效视图**（共享 base 键被 `[native]` 块覆盖后的结果），
/// 因为上游 `resolveOmoConfigView` 是 base → harness 块 → profile → profile 的 harness 块，
/// 后者胜。只展示块内会让界面与实际运行不一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeRuntimeConfig {
    /// Native harness 生效视图（折叠后）。
    pub effective: Value,
    /// `[native]` 块原文（未折叠）。
    pub native_block: Value,
    /// 共享 base 层里被 Native 视图采纳的键（顶层非控制键）。
    pub shared_base: Value,
    /// `settings.json` 的 `defaultProvider` / `defaultModel` / `defaultThinkingLevel`。
    pub model_settings: OmoNativeDefaultSelection,
    /// `settings.json` 里不由本页其他区块管理的键（「其他配置」→ settings.json 页签）。
    ///
    /// `defaultProvider` / `defaultModel` / `defaultThinkingLevel` 归「模型设置」，
    /// `packages` / `extensions` 归「扩展」区块；这些键不在这里出现，也不会被
    /// `save_omo_native_other_settings` 覆盖。
    pub other_settings: Value,
    /// 统一配置文件路径。
    pub config_path: String,
    /// `omo.jsonc` **整份原文**（含 `[opencode]` / `[native]` 块、控制键与注释）。
    ///
    /// 预览里显示它而不是只显示共享键：本页与 OpenCode 插件版共用这一份文件，
    /// 只显示本页拥有的那一层会让用户以为「文件里只有这些」，看不到另一半
    /// （2026-10-07 用户要求「直接展示完整的 json」）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_content: Option<String>,
    /// 配置是否可解析。false 时 `parse_error` 有值。
    pub parsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    /// `<agentDir>/settings.json` 原文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_content: Option<String>,
    /// `<agentDir>/models.json` 原文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_content: Option<String>,
    /// `<agentDir>/mcp.json` 原文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_content: Option<String>,
    /// `<agentDir>/auth.json` 原文（**含明文密钥**）。
    ///
    /// 本应用是配置管理器，密钥要能在界面上看到并编辑（2026-10-07 用户明确要求）。
    /// 不要为了「防肩窥」把它换成 `hasKey` 之类的布尔——那会让「存了密钥」和
    /// 「没存密钥」在界面上长得一模一样。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_content: Option<String>,
}

/// 模型档（`model_profile` / `model_profiles`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeModelProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// `daily` / `geeky`，纯元数据。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// `normal` / `heavy`，纯元数据。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Value>,
    /// 是否是上游内建档（内建档可被同名覆盖，覆盖是整体替换不是逐字段合并）。
    #[serde(default)]
    pub builtin: bool,
}

/// 一个 provider 在 `models.json` 里的条目 + 其密钥状态。
///
/// `models.json` 没有对外发布的 schema，字段形状反推自上游
/// `packages/omo-native/bin/lib/setup-opencode-providers.js` 的 `convertProvider`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeProvider {
    pub key: String,
    /// 是否引擎内建（内建 provider 不该在 `models.json` 里重复定义）。
    pub builtin: bool,
    /// 是否需要 OAuth 登录而非 API key。
    pub oauth: bool,
    /// `models.json` 里该 provider 的完整条目（含未知字段）。
    pub config: Value,
    /// `auth.json` 里是否已有该 provider 的密钥。
    pub has_key: bool,
    /// 当前生效的密钥明文（`auth.json` 优先，其次 `models.json` 的 `apiKey`）。
    ///
    /// 本应用是配置管理器，密钥要能在界面上看到并编辑（2026-10-07 用户明确要求）。
    /// 两处都没有时是 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// 是否在 omo 的自定义 provider 表里（`models.json` 的 `providers` 键下）。
    pub custom: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeProviderInput {
    pub key: String,
    pub config: Value,
    /// 可选：同时写入 `auth.json`。空字符串表示不动密钥。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

// ============================================================================
// 扩展（`settings.json` 的 `packages` + `<agentDir>/extensions`）
// ============================================================================
//
// 与 Pi 的 `PiExtension*` 同形：两个 CLI 共用 senpi 引擎的包体系，
// 逻辑在 `crate::coding::cli_extensions`，这里只是各自的类型名。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmoNativeExtensionScope {
    User,
    Project,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmoNativeExtensionKind {
    Package,
    LocalFile,
    LocalDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeExtensionSummary {
    pub id: String,
    pub source: String,
    pub scope: OmoNativeExtensionScope,
    pub kind: OmoNativeExtensionKind,
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
    /// 由 `settings.json` 的过滤器推导；`false` 表示这个包什么都不加载。
    #[serde(default = "crate::coding::omo_native::types::default_true")]
    pub enabled: bool,
    /// `false` 表示界面不该给开关——能写的过滤器改不动它实际加载什么。
    #[serde(default = "crate::coding::omo_native::types::default_true")]
    pub switch_supported: bool,
}

pub fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeExtensionListResult {
    pub extensions_path: String,
    pub packages_path: String,
    pub extensions: Vec<OmoNativeExtensionSummary>,
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeExtensionInstallInput {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeExtensionUpdateInput {
    /// 有值只更新这一个（`omo update <source>`）；省略则更新全部。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeExtensionActionInput {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<OmoNativeExtensionScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<OmoNativeExtensionKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeExtensionCommandResult {
    pub command: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeExtensionEnabledInput {
    pub source: String,
    pub kind: OmoNativeExtensionKind,
    /// `true` 写回启用形态；`false` 写禁用过滤器。
    pub enabled: bool,
}

/// MCP server 条目（`<agentDir>/mcp.json` 的 `mcpServers` 记录）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeMcpServer {
    pub name: String,
    pub config: Value,
}

/// 一个 skill 目录（`<agentDir>/skills/<name>`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativeSkill {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 全局提示词预设（DB 表 `omo_native_prompt_config`）。
///
/// 运行时产物是 `<agentDir>/AGENTS.md`（默认 `~/.omo/agent/AGENTS.md`）——
/// senpi 引擎的项目规则文件，与 OMP/Pi 的 `AGENTS.md` 同名同语义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativePromptConfig {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// `omo_native_prompt_config` 表的存储形状（`id` 在行主键里，不在 JSON 内）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmoNativePromptConfigContent {
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmoNativePromptConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}
