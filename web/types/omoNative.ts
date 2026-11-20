/**
 * OmO Native 配置类型。
 *
 * OmO Native 是上游 oh-my-openagent 5.0 起的**独立 edition**（`omo` 二进制 +
 * senpi 引擎，npm 包 `omo-ai`），与 OpenCode 插件版共用 `~/.omo/omo.jsonc`
 * 但读不同的 harness 块：插件版读 `[opencode]`，Native 读 `[native]`。
 *
 * 两套 edition 的 agent / category 名单**不相交**，不要与本项目
 * `@/types/ohMyOpenAgent`（插件版名单）互相套用。
 */

/** 运行时根目录信息。 */
/**
 * 运行时根目录决议结果。`source` 与后端 `runtime_location` 的四种来源一一对应，
 * 也与共享的 `RootPathInfoLike` 同形——共享根目录弹窗直接消费这个类型。
 */
export interface OmoNativePathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

export interface OmoNativeSettingsConfig {
  rootDir?: string | null;
  updatedAt?: string;
}

export interface OmoNativeSettingsConfigInput {
  rootDir?: string | null;
  clearRootDir?: boolean;
}

/** `omo` 二进制检测结果。 */
export interface OmoNativeCliInfo {
  found: boolean;
  path?: string;
  version?: string;
  versionNumber?: string;
  engineVersion?: string;
  error?: string;
}

/** 一个 agent/category 配置方案。 */
export interface OmoNativeAgentsConfig {
  id: string;
  name: string;
  isApplied: boolean;
  isDisabled: boolean;
  agents?: Record<string, unknown> | null;
  categories?: Record<string, unknown> | null;
  modelProfiles?: Record<string, unknown> | null;
  modelProfile?: string | null;
  task?: Record<string, unknown> | null;
  otherFields?: Record<string, unknown> | null;
  sortIndex?: number;
  createdAt?: string;
  updatedAt?: string;
}

export interface OmoNativeAgentsConfigInput {
  id?: string;
  name: string;
  isApplied?: boolean;
  isDisabled?: boolean;
  agents?: Record<string, unknown> | null;
  categories?: Record<string, unknown> | null;
  modelProfiles?: Record<string, unknown> | null;
  modelProfile?: string | null;
  task?: Record<string, unknown> | null;
  otherFields?: Record<string, unknown> | null;
}

/**
 * `settings.json` 里的默认模型选择。
 *
 * 引擎文档 `docs/settings.md` 的 "Model & Thinking" 一节：`/model` 里 Ctrl+S
 * 保存的就是这三个键。与 Pi 同名同语义（两边都是 senpi 引擎）。
 */
export interface OmoNativeDefaultSelection {
  providerKey?: string;
  modelId?: string;
  thinkingLevel?: string;
}

/** `save_omo_native_model_settings` 的入参：`''` 删除该键，`undefined` 不动。 */
export interface OmoNativeModelSettingsInput {
  defaultProvider?: string;
  defaultModel?: string;
  defaultThinkingLevel?: string;
}

/**
 * `save_omo_native_other_settings` 的入参：只提交用户实际改动的键。
 *
 * 页面打开期间引擎仍会写 `settings.json`（`tipsHistory` 等）；整份覆盖
 * 「其他设置」切片会把这些新写入回退，所以后端按 `set` / `remove` 打补丁。
 */
export interface OmoNativeOtherSettingsInput {
  /** 新增或修改的键。 */
  set: Record<string, unknown>;
  /** 要删除的键。 */
  remove: string[];
}

/** `read_omo_native_runtime_config` 的返回。 */
export interface OmoNativeRuntimeConfig {
  /** 折叠后的 Native 生效视图（共享 base 键被 `[native]` 块覆盖）。 */
  effective: Record<string, unknown>;
  /** `[native]` 块原文（未折叠）。 */
  nativeBlock: Record<string, unknown>;
  /** 被 Native 视图采纳的共享 base 键。 */
  sharedBase: Record<string, unknown>;
  /** `settings.json` 的默认模型选择。 */
  modelSettings: OmoNativeDefaultSelection;
  /** `settings.json` 中不由本页其他区块管理的键（「其他配置」→ settings.json 页签）。 */
  otherSettings: Record<string, unknown>;
  configPath: string;
  /** `omo.jsonc` 整份原文（含 `[opencode]` / `[native]` 块、控制键与注释）。 */
  configContent?: string;
  parsed: boolean;
  parseError?: string;
  settingsContent?: string;
  modelsContent?: string;
  mcpContent?: string;
  /** `<agentDir>/auth.json` 原文（**含明文密钥**）。 */
  authContent?: string;
}

export interface OmoNativeModelProfile {
  name: string;
  displayName?: string;
  family?: string;
  tier?: string;
  models?: unknown;
  builtin: boolean;
}

export interface OmoNativeProvider {
  key: string;
  builtin: boolean;
  oauth: boolean;
  config: Record<string, unknown>;
  hasKey: boolean;
  /**
   * 当前生效的密钥明文（已还原 config value 转义）。
   *
   * 每个 provider 的「写入落点」就是它的读取优先处：
   * - **自定义 provider**：`models.json` 的 `apiKey` 优先，`auth.json` 兜底
   *   （早先版本把自定义 provider 写进过 `auth.json`，那些记录还在）；
   * - **内建 provider**：`auth.json` 优先——那是 `/login` 的落点，引擎真正读的地方。
   *
   * 本应用是配置管理器，密钥要能在界面上看到并编辑——弹窗直接回填这个值，
   * 不要用 `••••••••` 之类的占位（那会让「存了密钥」和「没存」长得一样）。
   */
  apiKey?: string;
  custom: boolean;
}

export interface OmoNativeProviderInput {
  key: string;
  config: Record<string, unknown>;
  apiKey?: string;
}

export interface OmoNativeMcpServer {
  name: string;
  config: Record<string, unknown>;
}

// ============================================================================
// 扩展（`settings.json` 的 `packages` + `<agentDir>/extensions`）
// ============================================================================
//
// 与 Pi 的 `PiExtension*` 同形：两个 CLI 共用 senpi 引擎的包体系
// （`omo list` 与 `pi list` 的输出格式逐字相同）。后端逻辑在
// `coding/cli_extensions.rs`，两侧只是类型名不同。

export type OmoNativeExtensionScope = 'user' | 'project' | 'unknown';

export type OmoNativeExtensionKind = 'package' | 'local_file' | 'local_directory';

export interface OmoNativeExtensionSummary {
  id: string;
  source: string;
  scope: OmoNativeExtensionScope;
  kind: OmoNativeExtensionKind;
  path?: string;
  builtIn: boolean;
  currentVersion?: string;
  /** npm registry 的 `dist-tags.latest`（仅未钉版本且能连上 registry 时）。 */
  latestVersion?: string;
  updateAvailable: boolean;
  /** 由 `settings.json` 的过滤器推导；`false` 表示这个包什么都不加载。 */
  enabled: boolean;
  /** `false` 表示界面不该给开关——能写的过滤器改不动它实际加载什么。 */
  switchSupported: boolean;
}

export interface OmoNativeExtensionListResult {
  extensionsPath: string;
  packagesPath: string;
  extensions: OmoNativeExtensionSummary[];
  raw: string;
  cliPath?: string;
  cliVersion?: string;
}

export interface OmoNativeExtensionInstallInput {
  source: string;
}

export interface OmoNativeExtensionUpdateInput {
  /** 有值只更新这一个（`omo update <source>`）；省略则更新全部。 */
  source?: string;
}

export interface OmoNativeExtensionActionInput {
  source: string;
  scope?: OmoNativeExtensionScope;
  kind?: OmoNativeExtensionKind;
  path?: string;
}

export interface OmoNativeExtensionCommandResult {
  command: string;
  output: string;
}

export interface OmoNativeExtensionEnabledInput {
  source: string;
  kind: OmoNativeExtensionKind;
  enabled: boolean;
}

/**
 * Native 内建 curated agents（核实于上游 v5.1.19
 * `packages/senpi-task/src/agents/builtin/*.ts`）。
 *
 * 前四个是 curated read-only agents；后三个是 ulw-loop reviewer。
 * `metis`/`momus` 已在上游 5.0.0-beta.51 后不再作为别名解析。
 */
export const OMO_NATIVE_AGENTS: string[] = [
  'explore',
  'librarian',
  'plan-consultant',
  'plan-reviewer',
  'omo-native-code-reviewer',
  'omo-native-gate-reviewer',
  'omo-native-qa-executor',
];

/**
 * Native 内建 categories（核实于上游 v5.1.19
 * `packages/senpi-task/src/category/*-categories.ts`）。
 *
 * 与插件版的关键差异：Native 独有 `architect`；`deep` 已拆成
 * `deep-low` / `deep-high`。
 */
export const OMO_NATIVE_CATEGORIES: string[] = [
  'architect',
  'visual-engineering',
  'artistry',
  'writing',
  'ultrabrain',
  'deep-low',
  'deep-high',
  'quick',
  'unspecified-low',
  'unspecified-high',
];

/**
 * `[native]` 块的合法键（上游 `OmoTypedHarnessConfigSchema`，`.strict()`）。
 * 写入时只能出现这些键，否则触发上游 unknown-key diagnostic。
 */
export const OMO_NATIVE_BLOCK_KEYS: string[] = [
  'formatOnMutation',
  'gateway',
  'categories',
  'agents',
  'git_master',
  'task',
  'teams',
  'models',
  'model_profiles',
  'model_profile',
  'memory',
  'telemetry',
  'computer',
  'disabled_skills',
];

/** 本模块接管的键（方案编辑器直接编辑的部分）。 */
export const OMO_NATIVE_MANAGED_KEYS: string[] = [
  'agents',
  'categories',
  'model_profiles',
  'model_profile',
  'task',
];

/** 全局提示词文件（写在引擎状态目录里，默认 `~/.omo/agent/AGENTS.md`）。 */
export const OMO_NATIVE_PROMPT_FILE = 'AGENTS.md';

/** 引擎内建 provider 下的一条内建模型。 */
export interface OmoNativeBuiltinModel {
  id: string;
  context?: number;
  output?: number;
  thinking: boolean;
  images: boolean;
}

/** 引擎内建的 provider（官方认证渠道），只读展示。 */
/**
 * 一个**已配置凭据**的引擎内建 provider。
 *
 * 列表接口只返回就绪的那些（与 OpenCode 的「官方 Auth 认证渠道」同语义），
 * 所以这里的 `ready` 恒为 `true`——留着它是为了让「为什么它出现在列表里」
 * 在类型上也读得出来，而不是靠调用方记住这条约定。
 */
export interface OmoNativeBuiltinProvider {
  id: string;
  name: string;
  /** 需要 OAuth 登录（`omo auth check` 的判定输入之一）。 */
  oauth: boolean;
  ready: boolean;
  authType?: string;
  reason?: string;
  models: OmoNativeBuiltinModel[];
}
