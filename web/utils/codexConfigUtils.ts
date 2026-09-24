import { parse as parseToml, stringify as stringifyToml } from 'smol-toml';

const DEFAULT_CODEX_PROVIDER_KEY = 'custom';
const CODEX_RESERVED_MODEL_PROVIDER_KEYS = new Set([
  'amazon-bedrock',
  'openai',
  'ollama',
  'lmstudio',
  'oss',
  'ollama-chat',
]);
const TOML_SECTION_HEADER_PATTERN = /^\s*\[([^\]\r\n]+)\]\s*(?:#.*)?$/;
const TOML_MODEL_PROVIDER_LINE_PATTERN =
  /^\s*model_provider\s*=\s*(['"])([^"'\r\n]+)\1\s*(?:#.*)?$/;
const TOML_PROVIDER_NAME_REPLACE_PATTERN =
  /^(\s*name\s*=\s*)(?:"(?:\\.|[^"\\\r\n])*"|'[^'\r\n]*')(\s*(?:#.*)?)$/;

interface TomlSectionRange {
  headerLineIndex: number;
  bodyStartIndex: number;
  bodyEndIndex: number;
}

function isCodexProviderConfigSection(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function getCodexProviderSectionKeys(modelProviders: unknown): string[] {
  if (!isCodexProviderConfigSection(modelProviders)) {
    return [];
  }

  return Object.entries(modelProviders)
    .filter(([providerKey, providerConfig]) => providerKey.trim() && isCodexProviderConfigSection(providerConfig))
    .map(([providerKey]) => providerKey);
}

function resolveCodexCustomProviderKey(parsedConfig: Record<string, unknown>): string {
  const configuredProviderKey = typeof parsedConfig.model_provider === 'string'
    ? parsedConfig.model_provider.trim()
    : '';
  if (configuredProviderKey) {
    return configuredProviderKey;
  }

  const providerSectionKeys = getCodexProviderSectionKeys(parsedConfig.model_providers);
  if (providerSectionKeys.length === 0) {
    return DEFAULT_CODEX_PROVIDER_KEY;
  }

  if (providerSectionKeys.includes(DEFAULT_CODEX_PROVIDER_KEY)) {
    return DEFAULT_CODEX_PROVIDER_KEY;
  }

  return providerSectionKeys[0];
}

function resolveCodexCustomProviderKeyFromText(configText: string): string {
  const configuredProviderKey = configText.match(/^model_provider\s*=\s*(['"])([^'"]+)\1/m)?.[2]?.trim();
  if (configuredProviderKey) {
    return configuredProviderKey;
  }

  const providerSectionKeys = Array.from(configText.matchAll(/^\[model_providers\.([^\]]+)\]\s*$/gm))
    .map((match) => match[1]?.trim() || '')
    .filter(Boolean);

  if (providerSectionKeys.length === 0) {
    return DEFAULT_CODEX_PROVIDER_KEY;
  }

  if (providerSectionKeys.includes(DEFAULT_CODEX_PROVIDER_KEY)) {
    return DEFAULT_CODEX_PROVIDER_KEY;
  }

  return providerSectionKeys[0];
}

function getTomlSectionRange(lines: string[], sectionName: string): TomlSectionRange | undefined {
  let headerLineIndex = -1;

  for (let index = 0; index < lines.length; index += 1) {
    const match = lines[index].match(TOML_SECTION_HEADER_PATTERN);
    if (!match) {
      continue;
    }

    if (headerLineIndex === -1) {
      if (match[1] === sectionName) {
        headerLineIndex = index;
      }
      continue;
    }

    return {
      headerLineIndex,
      bodyStartIndex: headerLineIndex + 1,
      bodyEndIndex: index,
    };
  }

  if (headerLineIndex === -1) {
    return undefined;
  }

  return {
    headerLineIndex,
    bodyStartIndex: headerLineIndex + 1,
    bodyEndIndex: lines.length,
  };
}

function getTopLevelEndIndex(lines: string[]): number {
  const firstSectionIndex = lines.findIndex((line) => TOML_SECTION_HEADER_PATTERN.test(line));
  return firstSectionIndex === -1 ? lines.length : firstSectionIndex;
}

function getTomlSectionInsertIndex(lines: string[], sectionRange: TomlSectionRange): number {
  let insertIndex = sectionRange.bodyEndIndex;
  while (insertIndex > sectionRange.bodyStartIndex && lines[insertIndex - 1].trim() === '') {
    insertIndex -= 1;
  }
  return insertIndex;
}

function findTomlLineInRange(
  lines: string[],
  pattern: RegExp,
  startIndex: number,
  endIndex: number,
): number {
  for (let index = startIndex; index < endIndex; index += 1) {
    if (pattern.test(lines[index])) {
      return index;
    }
  }
  return -1;
}

function finalizeTomlText(lines: string[]): string {
  return lines
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/^\n+/, '');
}

function getTopLevelModelProviderKey(configText: string): string | undefined {
  const normalizedText = normalizeQuotes(configText);
  try {
    const parsedConfig = parseToml(normalizedText) as Record<string, unknown>;
    const providerKey = typeof parsedConfig.model_provider === 'string'
      ? parsedConfig.model_provider.trim()
      : '';
    if (providerKey) {
      return providerKey;
    }
  } catch {
    // Fall back to a line scan while the editor is in an intermediate state.
  }

  const lines = normalizedText.split('\n');
  const topLevelEndIndex = getTopLevelEndIndex(lines);
  for (let index = 0; index < topLevelEndIndex; index += 1) {
    const match = lines[index].match(TOML_MODEL_PROVIDER_LINE_PATTERN);
    const providerKey = match?.[2]?.trim();
    if (providerKey) {
      return providerKey;
    }
  }
  return undefined;
}

function getCodexCustomProviderSectionName(configText: string): string | undefined {
  const providerKey = getTopLevelModelProviderKey(configText);
  if (!providerKey || !isCustomCodexProviderKey(providerKey)) {
    return undefined;
  }
  return `model_providers.${providerKey}`;
}

function isCustomCodexProviderKey(providerKey: string): boolean {
  const normalizedKey = providerKey.trim().toLowerCase();
  return Boolean(normalizedKey) && !CODEX_RESERVED_MODEL_PROVIDER_KEYS.has(normalizedKey);
}

const TOML_BASIC_STRING_ESCAPES: Record<string, string> = {
  '"': '\\"',
  '\\': '\\\\',
  '\b': '\\b',
  '\t': '\\t',
  '\n': '\\n',
  '\f': '\\f',
  '\r': '\\r',
};

function tomlBasicString(value: string): string {
  const escaped = value.replace(/["\\\u0000-\u001f]/g, (character) => {
    const knownEscape = TOML_BASIC_STRING_ESCAPES[character];
    if (knownEscape) {
      return knownEscape;
    }
    return `\\u${character.charCodeAt(0).toString(16).padStart(4, '0')}`;
  });
  return `"${escaped}"`;
}

/**
 * Codex TOML 配置工具函数
 * 提供 TOML 配置的提取、写入、归一化等功能
 */

/**
 * 引号归一化：将中文引号、全角引号转换为英文引号
 * @param text - 原始文本
 * @returns 归一化后的文本
 */
export function normalizeQuotes(text: string): string {
  if (!text) return text;
  
  return text
    // 中文双引号 → 英文双引号
    .replace(/"/g, '"')
    .replace(/"/g, '"')
    // 中文单引号 → 英文单引号
    .replace(/'/g, "'")
    .replace(/'/g, "'")
    // 全角单引号 → 英文单引号
    .replace(/＇/g, "'")
    // 全角双引号 → 英文双引号
    .replace(/＂/g, '"');
}

/**
 * 从 TOML 配置文本中提取 base_url
 * @param configText - TOML 配置文本
 * @returns base_url 值，不存在则返回 undefined
 */
export function extractCodexBaseUrl(configText: string | undefined | null): string | undefined {
  try {
    const raw = typeof configText === 'string' ? configText : '';
    // 归一化中文/全角引号，避免正则提取失败
    const text = normalizeQuotes(raw);
    if (!text) return undefined;
    
    const parsedConfig = parseToml(text) as Record<string, unknown>;
    const providerKey = resolveCodexCustomProviderKey(parsedConfig);
    const modelProviders = isCodexProviderConfigSection(parsedConfig.model_providers)
      ? parsedConfig.model_providers
      : undefined;
    const providerConfig = modelProviders && isCodexProviderConfigSection(modelProviders[providerKey])
      ? modelProviders[providerKey]
      : undefined;
    if (typeof providerConfig?.base_url === 'string' && providerConfig.base_url.trim()) {
      return providerConfig.base_url.trim();
    }
    if (typeof parsedConfig.base_url === 'string' && parsedConfig.base_url.trim()) {
      return parsedConfig.base_url.trim();
    }
    const fallbackProvider = getCodexProviderSectionKeys(modelProviders)
      .map((key) => modelProviders?.[key])
      .find((value) => isCodexProviderConfigSection(value) && typeof value.base_url === 'string');
    return isCodexProviderConfigSection(fallbackProvider)
      && typeof fallbackProvider.base_url === 'string'
      ? fallbackProvider.base_url.trim() || undefined
      : undefined;
  } catch {
    const match = normalizeQuotes(typeof configText === 'string' ? configText : '')
      .match(/base_url\s*=\s*(['"])([^'"]+)\1/);
    return match?.[2];
  }
}

/**
 * 在 TOML 配置文本中写入或更新 base_url
 * 如果已存在则替换，不存在则追加到末尾
 * @param configText - 原始 TOML 配置文本
 * @param baseUrl - base_url 值
 * @returns 更新后的 TOML 配置文本
 */
export function setCodexBaseUrl(configText: string, baseUrl: string): string {
  const trimmed = baseUrl.trim();
  if (!trimmed) {
    return configText;
  }
  
  // 归一化原文本中的引号（既能匹配，也能输出稳定格式）
  const normalizedText = normalizeQuotes(configText);
  
  // 移除 URL 中的空格
  const normalizedUrl = trimmed.replace(/\s+/g, '');
  const replacementLine = `base_url = "${normalizedUrl}"`;
  const pattern = /base_url\s*=\s*(['"])([^'"]+)\1/;
  
  // 如果已存在 base_url，则替换
  if (pattern.test(normalizedText)) {
    return normalizedText.replace(pattern, replacementLine);
  }
  
  // 如果不存在，追加到末尾
  const prefix = normalizedText && !normalizedText.endsWith('\n')
    ? `${normalizedText}\n`
    : normalizedText;
  return `${prefix}${replacementLine}\n`;
}

/**
 * 从 TOML 配置文本中移除 base_url 行
 * @param configText - 原始 TOML 配置文本
 * @returns 移除后的 TOML 配置文本
 */
export function removeCodexBaseUrl(configText: string): string {
  const normalized = normalizeQuotes(configText);
  // 移除 base_url 行（包括行尾换行符）
  return normalized.replace(/base_url\s*=\s*(['"])[^'"]+\1\n?/g, '').trim();
}

/**
 * 从 TOML 配置文本中提取 model（在 [chat] section 或顶层）
 * @param configText - TOML 配置文本
 * @returns model 值，不存在则返回 undefined
 */
export function extractCodexModel(configText: string | undefined | null): string | undefined {
  try {
    const raw = typeof configText === 'string' ? configText : '';
    const text = normalizeQuotes(raw);
    if (!text) return undefined;
    
    // 优先匹配 [chat] section 中的 model
    const chatSectionMatch = text.match(/\[chat\]\s*\n\s*model\s*=\s*(['"])([^'"]+)\1/);
    if (chatSectionMatch?.[2]) {
      return chatSectionMatch[2];
    }
    
    // 其次匹配顶层的 model（行首或前面只有空白）
    const topLevelMatch = text.match(/^model\s*=\s*(['"])([^'"]+)\1/m);
    return topLevelMatch?.[2];
  } catch {
    return undefined;
  }
}

/**
 * 从 TOML 配置文本中提取 model_reasoning_effort（顶层）
 * @param configText - TOML 配置文本
 * @returns reasoning effort 值，不存在则返回 undefined
 */
export function extractCodexReasoningEffort(
  configText: string | undefined | null
): string | undefined {
  try {
    const raw = typeof configText === 'string' ? configText : '';
    const text = normalizeQuotes(raw);
    if (!text) return undefined;

    const match = text.match(/^model_reasoning_effort\s*=\s*(['"])([^'"]+)\1/m);
    return match?.[2];
  } catch {
    return undefined;
  }
}

/**
 * 在 TOML 配置文本中写入或更新顶层 model_reasoning_effort
 * 如果已存在则替换，不存在则插到 model_provider 之前（与 model 的插入策略一致）
 * @param configText - 原始 TOML 配置文本
 * @param effort - 默认思考档位值
 * @returns 更新后的 TOML 配置文本
 */
export function setCodexReasoningEffort(configText: string, effort: string): string {
  const trimmed = effort.trim();
  if (!trimmed) {
    return configText;
  }

  const normalizedText = normalizeQuotes(configText);
  const replacementLine = `model_reasoning_effort = "${trimmed}"`;
  const topLevelPattern = /^model_reasoning_effort\s*=\s*(['"])([^'"]+)\1/m;

  if (topLevelPattern.test(normalizedText)) {
    return normalizedText.replace(topLevelPattern, replacementLine);
  }

  const modelProviderPattern = /(model_provider\s*=\s*(['"])[^'"]+\2)/;
  if (modelProviderPattern.test(normalizedText)) {
    return normalizedText.replace(modelProviderPattern, `${replacementLine}\n$1`);
  }

  const prefix = normalizedText && !normalizedText.endsWith('\n')
    ? `${normalizedText}\n`
    : normalizedText;
  return `${prefix}${replacementLine}\n`;
}

/**
 * 在 TOML 配置文本中写入或更新 model
 * 优先更新已存在的 model（无论在 [chat] section 还是顶层）
 * 如果都不存在，则在顶层添加（不创建 [chat] section）
 * @param configText - 原始 TOML 配置文本
 * @param model - model 值
 * @returns 更新后的 TOML 配置文本
 */
export function setCodexModel(configText: string, model: string): string {
  const trimmed = model.trim();
  if (!trimmed) {
    return configText;
  }
  
  const normalizedText = normalizeQuotes(configText);
  const replacementLine = `model = "${trimmed}"`;
  
  // 检查是否存在 [chat] section
  const hasChatSection = /\[chat\]/i.test(normalizedText);
  
  if (hasChatSection) {
    // 在 [chat] section 中查找 model
    const chatModelPattern = /(\[chat\]\s*\n)(\s*model\s*=\s*(['"])[^'"]+\3)/;
    if (chatModelPattern.test(normalizedText)) {
      // [chat] section 中已有 model，替换
      return normalizedText.replace(
        chatModelPattern,
        `$1${replacementLine}`
      );
    } else {
      // [chat] section 存在但没有 model，在 [chat] 后插入
      return normalizedText.replace(
        /\[chat\]\s*\n/,
        `[chat]\n${replacementLine}\n`
      );
    }
  }
  
  // 检查顶层是否有 model（注意：不是 model_provider）
  const topLevelPattern = /^model\s*=\s*(['"])[^'"]+\1/m;
  if (topLevelPattern.test(normalizedText)) {
    // 顶层已有 model，替换
    return normalizedText.replace(topLevelPattern, replacementLine);
  }
  
  // 都不存在，在顶层添加 model（不创建 [chat] section）
  // 尝试在 model_provider 附近添加，保持配置整洁
  const modelProviderPattern = /(model_provider\s*=\s*(['"])[^'"]+\2)/;
  if (modelProviderPattern.test(normalizedText)) {
    // 在 model_provider 之前插入 model
    return normalizedText.replace(modelProviderPattern, `${replacementLine}\n$1`);
  }
  
  // 没有 model_provider，直接追加到末尾
  const prefix = normalizedText && !normalizedText.endsWith('\n')
    ? `${normalizedText}\n`
    : normalizedText;
  return `${prefix}${replacementLine}\n`;
}

/**
 * 从 TOML 配置文本中移除 model 行
 * @param configText - 原始 TOML 配置文本
 * @returns 移除后的 TOML 配置文本
 */
export function removeCodexModel(configText: string): string {
  const normalized = normalizeQuotes(configText);
  
  // 移除 [chat] section 中的 model
  let result = normalized.replace(/(\[chat\]\s*\n)\s*model\s*=\s*(['"])[^'"]+\2\n?/, '$1');
  
  // 移除顶层的 model
  result = result.replace(/^model\s*=\s*(['"])[^'"]+\1\n?/m, '');
  
  return result.trim();
}

/**
 * 从 TOML 配置文本中移除指定字段
 * @param configText - 原始 TOML 配置文本
 * @param fieldName - 要移除的字段名
 * @returns 移除后的 TOML 配置文本
 */
export function removeCodexField(configText: string, fieldName: string): string {
  const normalized = normalizeQuotes(configText);
  // 创建匹配指定字段的正则（支持单引号和双引号）
  const pattern = new RegExp(`^${fieldName}\\s*=\\s*(['"])[^'"]+\\1\\n?`, 'gm');
  return normalized.replace(pattern, '').trim();
}

/**
 * 将 provider TOML 清理为官方订阅模式：
 * - 移除 AI Toolbox 管理的第三方 provider/base_url 指向
 * - 保留与运行时无关的其它配置
 * - 保留用户显式选择的 model / model_reasoning_effort 等通用字段
 */
export function normalizeCodexConfigForOfficialMode(configText: string): string {
  const normalized = normalizeQuotes(configText);
  const trimmedConfig = normalized.trim();
  if (!trimmedConfig) {
    return '';
  }

  try {
    const parsedConfig = parseToml(trimmedConfig) as Record<string, unknown>;
    const nextConfig: Record<string, unknown> = { ...parsedConfig };

    const providerKey = typeof nextConfig.model_provider === 'string'
      ? nextConfig.model_provider.trim()
      : '';
    const modelProviders = nextConfig.model_providers;

    if (typeof nextConfig.base_url === 'string') {
      delete nextConfig.base_url;
    }

    if (providerKey && modelProviders && typeof modelProviders === 'object' && !Array.isArray(modelProviders)) {
      const nextModelProviders = { ...(modelProviders as Record<string, unknown>) };
      if (providerKey in nextModelProviders) {
        delete nextModelProviders[providerKey];
        nextConfig.model_providers = nextModelProviders;
        delete nextConfig.model_provider;

        if (Object.keys(nextModelProviders).length === 0) {
          delete nextConfig.model_providers;
        }
      }
    }

    return stringifyToml(nextConfig).trim();
  } catch {
    let cleanedConfig = removeCodexBaseUrl(trimmedConfig);
    cleanedConfig = cleanedConfig.replace(/^model_provider\s*=\s*(['"]).*?\1\s*$/gm, '').trim();
    cleanedConfig = cleanedConfig.replace(
      /\[model_providers\.[^\]]+\][\s\S]*?(?=\n\[[^\]]+\]|\s*$)/g,
      '',
    ).trim();
    return cleanedConfig;
  }
}

/**
 * 确保当前 TOML 至少具备一份可用的 custom provider 骨架。
 * 如果原配置已经有 model_provider 与对应 provider 段，则保持不动。
 */
export function ensureCodexCustomProviderConfig(configText: string): string {
  const normalized = normalizeQuotes(configText);
  const trimmedConfig = normalized.trim();

  try {
    const parsedConfig = (trimmedConfig ? parseToml(trimmedConfig) : {}) as Record<string, unknown>;
    const nextConfig: Record<string, unknown> = { ...parsedConfig };
    const providerKey = resolveCodexCustomProviderKey(nextConfig);

    if (typeof nextConfig.model_provider !== 'string' || !nextConfig.model_provider.trim()) {
      nextConfig.model_provider = providerKey;
    }

    const modelProviders =
      isCodexProviderConfigSection(nextConfig.model_providers)
        ? { ...(nextConfig.model_providers as Record<string, unknown>) }
        : {};

    const customProvider =
      isCodexProviderConfigSection(modelProviders[providerKey])
        ? { ...(modelProviders[providerKey] as Record<string, unknown>) }
        : {};

    if (typeof customProvider.name !== 'string' || !customProvider.name.trim()) {
      customProvider.name = 'OpenAI';
    }
    if (typeof customProvider.wire_api !== 'string' || !customProvider.wire_api.trim()) {
      customProvider.wire_api = 'responses';
    }
    // `requires_openai_auth` is intentionally NOT defaulted here. The backend
    // projection owns that line (see issue #394): it always writes or removes it
    // from the runtime config.toml based on the provider's auth mechanism and the
    // provider's explicit `settingsConfig.requiresOpenaiAuthMode`. Injecting a
    // default here only made the TOML editor advertise a value that the next
    // apply silently overrode.

    modelProviders[providerKey] = customProvider;
    nextConfig.model_providers = modelProviders;

    return stringifyToml(nextConfig).trim();
  } catch {
    const providerKey = resolveCodexCustomProviderKeyFromText(trimmedConfig);
    const hasModelProvider = /^model_provider\s*=/m.test(trimmedConfig);
    const hasProviderSection = new RegExp(`\\[model_providers\\.${providerKey.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\]`).test(trimmedConfig);
    const nextChunks = trimmedConfig ? [trimmedConfig] : [];

    if (!hasModelProvider) {
      nextChunks.push(`model_provider = "${providerKey}"`);
    }

    if (!hasProviderSection) {
      nextChunks.push(
        `[model_providers.${providerKey}]\nname = "OpenAI"\nwire_api = "responses"`,
      );
    }

    return nextChunks.join('\n\n').trim();
  }
}

export function isCodexGoalModeEnabled(configText: string | undefined | null): boolean {
  try {
    const raw = typeof configText === 'string' ? configText : '';
    const normalizedText = normalizeQuotes(raw);
    if (!normalizedText.trim()) {
      return false;
    }

    const parsedConfig = parseToml(normalizedText) as Record<string, unknown>;
    const features = isCodexProviderConfigSection(parsedConfig.features)
      ? parsedConfig.features
      : {};
    return features.goals === true;
  } catch {
    return false;
  }
}

export function setCodexGoalMode(configText: string, enabled: boolean): string {
  const normalizedText = normalizeQuotes(configText);
  const nextConfig = (normalizedText.trim()
    ? parseToml(normalizedText)
    : {}) as Record<string, unknown>;

  const features = isCodexProviderConfigSection(nextConfig.features)
    ? { ...nextConfig.features }
    : {};

  if (enabled) {
    features.goals = true;
    nextConfig.features = features;
  } else {
    delete features.goals;
    if (Object.keys(features).length > 0) {
      nextConfig.features = features;
    } else {
      delete nextConfig.features;
    }
  }

  return stringifyToml(nextConfig).trim();
}

export function getCodexIgnoredCommonConfigKeys(configText: string | undefined | null): string[] {
  try {
    const raw = typeof configText === 'string' ? configText : '';
    const normalizedText = normalizeQuotes(raw);
    if (!normalizedText.trim()) {
      return [];
    }

    const parsedConfig = parseToml(normalizedText) as Record<string, unknown>;
    const ignoredKeys: string[] = [];

    if (isCodexProviderConfigSection(parsedConfig.mcp_servers)) {
      ignoredKeys.push('[mcp_servers]');
    }
    if (isCodexProviderConfigSection(parsedConfig.plugins)) {
      ignoredKeys.push('[plugins]');
    }

    const features = isCodexProviderConfigSection(parsedConfig.features)
      ? parsedConfig.features
      : {};
    if (Object.prototype.hasOwnProperty.call(features, 'plugins')) {
      ignoredKeys.push('[features].plugins');
    }

    return ignoredKeys;
  } catch {
    return [];
  }
}

export function isCodexRemoteCompactionEnabled(configText: string | undefined | null): boolean {
  try {
    const raw = typeof configText === 'string' ? configText : '';
    const normalizedText = normalizeQuotes(raw);
    if (!normalizedText.trim()) {
      return false;
    }

    const parsedConfig = parseToml(normalizedText) as Record<string, unknown>;
    const providerKey = typeof parsedConfig.model_provider === 'string'
      ? parsedConfig.model_provider.trim()
      : '';
    if (!providerKey || !isCustomCodexProviderKey(providerKey)) {
      return false;
    }

    const modelProviders = isCodexProviderConfigSection(parsedConfig.model_providers)
      ? parsedConfig.model_providers
      : {};
    const providerConfig = isCodexProviderConfigSection(modelProviders[providerKey])
      ? modelProviders[providerKey]
      : {};
    return providerConfig.name === 'OpenAI';
  } catch {
    return false;
  }
}

export function canToggleCodexRemoteCompaction(configText: string | undefined | null): boolean {
  const raw = typeof configText === 'string' ? configText : '';
  return Boolean(getCodexCustomProviderSectionName(normalizeQuotes(raw)));
}

export function setCodexRemoteCompaction(
  configText: string,
  enabled: boolean,
  fallbackProviderName?: string,
): string {
  const normalizedText = normalizeQuotes(configText);
  const lines = normalizedText ? normalizedText.split('\n') : [];
  const targetSectionName = getCodexCustomProviderSectionName(normalizedText);
  if (!targetSectionName) {
    return normalizedText;
  }

  const replacementName = enabled
    ? 'OpenAI'
    : fallbackProviderName?.trim() ||
      getTopLevelModelProviderKey(normalizedText) ||
      DEFAULT_CODEX_PROVIDER_KEY;
  const replacementLine = `name = ${tomlBasicString(replacementName)}`;
  const targetSectionRange = getTomlSectionRange(lines, targetSectionName);

  if (targetSectionRange) {
    const nameLineIndex = findTomlLineInRange(
      lines,
      TOML_PROVIDER_NAME_REPLACE_PATTERN,
      targetSectionRange.bodyStartIndex,
      targetSectionRange.bodyEndIndex,
    );

    if (nameLineIndex !== -1) {
      lines[nameLineIndex] = lines[nameLineIndex].replace(
        TOML_PROVIDER_NAME_REPLACE_PATTERN,
        `$1${tomlBasicString(replacementName)}$2`,
      );
      return finalizeTomlText(lines);
    }

    lines.splice(getTomlSectionInsertIndex(lines, targetSectionRange), 0, replacementLine);
    return finalizeTomlText(lines);
  }

  if (!enabled) {
    return normalizedText;
  }

  if (lines.length > 0 && lines[lines.length - 1].trim() !== '') {
    lines.push('');
  }
  lines.push(`[${targetSectionName}]`, replacementLine);
  return finalizeTomlText(lines);
}
