import { parse as parseToml } from 'smol-toml';
import { extractProviderConnectionFields } from './providerShareUrl';

export const PROVIDER_SHARE_TOOLS = [
  { app: 'claude', label: 'Claude Code', path: '/coding/claudecode' },
  { app: 'claudedesktop', label: 'Claude Desktop', path: '/coding/claudedesktop' },
  { app: 'codex', label: 'Codex', path: '/coding/codex' },
  { app: 'grok', label: 'Grok CLI', path: '/coding/grok' },
  { app: 'kimi', label: 'Kimi Code', path: '/coding/kimi' },
  { app: 'gemini', label: 'Gemini CLI', path: '/coding/geminicli' },
  { app: 'opencode', label: 'OpenCode', path: '/coding/opencode' },
  { app: 'openclaw', label: 'OpenClaw', path: '/coding/openclaw' },
  { app: 'pi', label: 'Pi', path: '/coding/pi' },
  { app: 'omp', label: 'Oh My Pi', path: '/coding/oh-my-pi' },
  { app: 'hermes', label: 'Hermes', path: '/coding/hermes' },
  { app: 'dsh', label: 'DeepSeek Harness', path: '/coding/dsh' },
] as const;

export type ProviderShareApp = typeof PROVIDER_SHARE_TOOLS[number]['app'];
export type SharedApiFormat = 'anthropic_messages' | 'openai_responses' | 'openai_chat' | 'gemini_native' | 'ollama/chat';

export interface SharedModel {
  id: string;
  name?: string;
  contextWindow?: number;
  maxTokens?: number;
  reasoning?: boolean;
  input?: string[];
}

export interface SharedHeader {
  op: 'set' | 'delete' | 'rename' | 'copy';
  name?: string;
  value?: string;
  from?: string;
  to?: string;
}

export interface SharedConnectionFields {
  sourceApp?: ProviderShareApp;
  baseUrlStyle?: 'root' | 'versioned';
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  apiFormat?: SharedApiFormat;
  apiVersion?: string;
  models?: SharedModel[];
  modelRoles?: Record<string, string>;
  headers?: SharedHeader[];
  gatewayProfile?: { tool?: string; profileId: string; endpointId: string };
  providerType?: string;
  apiKeyField?: string;
}

export interface ShareableProvider {
  id?: string;
  name: string;
  category: string;
  settingsConfig: string;
  meta?: unknown;
  credential?: unknown;
  defaultModel?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  connectionDefaults?: SharedConnectionFields;
  credentialUnavailable?: boolean;
}

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const readString = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim() ? value.trim() : undefined;
const firstString = (record: Record<string, unknown>, ...keys: string[]) =>
  keys.map((key) => readString(record[key])).find(Boolean);
const stripMarker = (model: string) => model.trim().replace(/\[1m\]$/i, '').trim();

export function normalizeSharedApiFormat(value: unknown): SharedApiFormat | undefined {
  const key = readString(value)?.toLowerCase().replace(/[/-]/g, '_');
  if (['anthropic', 'anthropic_messages', 'claude', 'messages'].includes(key ?? '')) return 'anthropic_messages';
  if (['openai', 'openai_legacy', 'kimi', 'openai_chat', 'openai_completions', 'chat', 'chat_completions'].includes(key ?? '')) return 'openai_chat';
  if (['responses', 'openai_responses'].includes(key ?? '')) return 'openai_responses';
  if (['google', 'google_genai', 'google_generative_ai', 'gemini', 'gemini_native'].includes(key ?? '')) return 'gemini_native';
  if (['ollama', 'ollama_chat'].includes(key ?? '')) return 'ollama/chat';
  return undefined;
}

function modelEntry(id: string, raw: unknown): SharedModel {
  const record = asRecord(raw);
  const limit = asRecord(record.limit);
  const modalities = asRecord(record.modalities);
  const positive = (...values: unknown[]) => values.find((value) => typeof value === 'number' && Number.isInteger(value) && value > 0 && value <= 0xffff_ffff) as number | undefined;
  const capability = [record.supportsImage, record.vision, record.attachment].find((value) => typeof value === 'boolean');
  const rawInput = record.input ?? modalities.input;
  const capabilities = Array.isArray(record.capabilities) ? record.capabilities : [];
  return {
    id: id.trim(),
    name: firstString(record, 'displayName', 'name', 'labelOverride'),
    contextWindow: positive(record.contextWindow, record.maxContextSize, record.context_length, record.context_window, limit.context),
    maxTokens: positive(record.maxTokens, record.max_tokens, limit.output),
    reasoning: typeof record.reasoning === 'boolean' ? record.reasoning : capabilities.includes('thinking') || capabilities.includes('always_thinking') ? true : undefined,
    input: Array.isArray(rawInput) ? rawInput.filter((item): item is string => typeof item === 'string')
      : typeof capability === 'boolean' ? capability ? ['text', 'image'] : ['text']
        : capabilities.length ? ['text', ...(capabilities.includes('image_in') ? ['image'] : []), ...(capabilities.includes('video_in') ? ['video'] : [])] : undefined,
  };
}

function parseSettings(raw: string): Record<string, unknown> {
  try { return asRecord(JSON.parse(raw)); } catch { return {}; }
}

function isCredentialReference(value: string): boolean {
  return /^(?:\{(?:env|file):|\$\{|\$[A-Z_]|!|env:)/.test(value);
}

export function hasUnresolvedShareCredential(provider: ShareableProvider): boolean {
  const settings = parseSettings(provider.settingsConfig);
  const env = asRecord(settings.env);
  const auth = asRecord(settings.auth);
  const options = asRecord(settings.options);
  const providerConfigs = Object.values(asRecord(settings.providerConfigs)).map(asRecord);
  const selectedAuth = asRecord(asRecord(asRecord(settings.config).security).auth).selectedType;
  if (auth.tokens || auth.auth_mode === 'chatgpt' || auth.type === 'oauth'
    || providerConfigs.some((config) => config.oauth)
    || selectedAuth === 'oauth-personal' || env.GOOGLE_APPLICATION_CREDENTIALS) return true;
  return [provider.credential, settings.apiKey, settings.api_key, options.apiKey, options.authToken,
    env.ANTHROPIC_AUTH_TOKEN, env.ANTHROPIC_API_KEY, env.GEMINI_API_KEY, auth.API_KEY, auth.OPENAI_API_KEY,
    ...providerConfigs.map((config) => config.api_key)]
    .some((value) => typeof value === 'string' && isCredentialReference(value));
}

export function extractProviderShareFields(sourceApp: ProviderShareApp, provider: ShareableProvider): SharedConnectionFields {
  const settings = parseSettings(provider.settingsConfig);
  const meta = asRecord(provider.meta);
  const fields: SharedConnectionFields = { ...provider.connectionDefaults, sourceApp };
  let models: SharedModel[] = [];
  let headers: unknown = meta.customHeaders ?? meta.custom_headers;
  const rawProfile = asRecord(meta.gatewayProfile ?? meta.gateway_profile);
  const profileId = firstString(rawProfile, 'profileId', 'profile_id');
  const endpointId = firstString(rawProfile, 'endpointId', 'endpoint_id');
  if (profileId && endpointId) {
    fields.gatewayProfile = { tool: readString(rawProfile.tool), profileId, endpointId };
  } else {
    fields.providerType = firstString(meta, 'providerType', 'provider_type') ?? fields.providerType;
    fields.apiKeyField = firstString(meta, 'apiKeyField', 'api_key_field') ?? fields.apiKeyField;
  }
  const explicitFormat = normalizeSharedApiFormat(meta.apiFormat ?? meta.api_format ?? settings.apiFormat ?? settings.api_format);
  fields.apiFormat = fields.gatewayProfile ? fields.apiFormat ?? explicitFormat : explicitFormat ?? fields.apiFormat;

  if (sourceApp === 'claude' || sourceApp === 'claudedesktop') {
    Object.assign(fields, extractProviderConnectionFields('claude', provider.settingsConfig));
    fields.apiFormat ??= settings.openrouter_compat_mode === true ? 'openai_chat' : 'anthropic_messages';
    fields.baseUrlStyle = fields.apiFormat === 'anthropic_messages' ? 'root' : 'versioned';
    const env = asRecord(settings.env);
    fields.apiKeyField ??= fields.apiFormat === 'anthropic_messages'
      ? readString(env.ANTHROPIC_AUTH_TOKEN) ? 'bearer' : readString(env.ANTHROPIC_API_KEY) ? 'x-api-key' : undefined
      : fields.apiFormat === 'gemini_native' ? 'x-goog-api-key' : 'bearer';
    fields.baseUrl ??= 'https://api.anthropic.com';
    const roles: Record<string, string> = {};
    const desktopRoutes = asRecord(meta.claudeDesktopModelRoutes ?? meta.claude_desktop_model_routes);
    for (const route of Object.values(desktopRoutes)) {
      const record = asRecord(route);
      const model = readString(record.model);
      if (model) models.push(modelEntry(stripMarker(model), record));
    }
    for (const role of ['sonnet', 'opus', 'fable', 'haiku']) {
      const route = Object.entries(desktopRoutes).find(([, value]) => asRecord(value).tierAlias === role)
        ?? Object.entries(desktopRoutes).find(([key]) => key.includes(`-${role}-`));
      const routeValue = asRecord(route?.[1]);
      const id = readString(routeValue.model) ?? readString(env[`ANTHROPIC_DEFAULT_${role.toUpperCase()}_MODEL`]) ?? readString(settings[`${role}Model`]);
      if (id) {
        roles[role] = stripMarker(id);
        models.push(modelEntry(stripMarker(id), { name: routeValue.labelOverride ?? env[`ANTHROPIC_DEFAULT_${role.toUpperCase()}_MODEL_NAME`] }));
      }
    }
    fields.modelRoles = roles;
    fields.model ??= models[0]?.id;
  } else if (sourceApp === 'codex') {
    Object.assign(fields, extractProviderConnectionFields('codex', provider.settingsConfig));
    let toml: Record<string, unknown> = {};
    try { toml = asRecord(parseToml(readString(settings.config) ?? '')); } catch { /* Fields remain editable. */ }
    const providerTables = asRecord(toml.model_providers);
    const selected = asRecord(providerTables[readString(toml.model_provider) ?? ''] ?? Object.values(providerTables)[0]);
    fields.apiFormat ??= normalizeSharedApiFormat(selected.wire_api ?? selected.api_format ?? toml.wire_api) ?? 'openai_responses';
    fields.baseUrlStyle = 'versioned';
    fields.apiKey ??= readString(selected.experimental_bearer_token);
    fields.apiKeyField ??= fields.apiFormat === 'gemini_native' ? 'x-goog-api-key' : 'bearer';
    fields.baseUrl ??= 'https://api.openai.com/v1';
    headers ??= selected.http_headers;
    fields.model = readString(asRecord(toml.chat).model) ?? fields.model;
    const catalog = asRecord(settings.modelCatalog).models;
    if (Array.isArray(catalog)) models = catalog.flatMap((value) => {
      const record = asRecord(value); const id = readString(record.model); return id ? [modelEntry(id, record)] : [];
    });
  } else if (sourceApp === 'gemini') {
    Object.assign(fields, extractProviderConnectionFields('gemini', provider.settingsConfig));
    fields.apiKey ??= readString(asRecord(settings.env).GOOGLE_API_KEY);
    fields.apiFormat ??= 'gemini_native';
    fields.baseUrlStyle = fields.apiFormat === 'gemini_native' ? 'root' : 'versioned';
    fields.baseUrl ??= 'https://generativelanguage.googleapis.com';
    fields.apiVersion = readString(asRecord(settings.env).GOOGLE_GENAI_API_VERSION);
    fields.apiKeyField ??= fields.apiFormat === 'gemini_native'
      ? asRecord(settings.env).GEMINI_API_KEY_AUTH_MECHANISM === 'bearer' ? 'bearer' : 'x-goog-api-key'
      : fields.apiFormat === 'anthropic_messages' ? 'x-api-key' : 'bearer';
  } else if (sourceApp === 'grok' || sourceApp === 'kimi') {
    fields.apiKey = readString(asRecord(settings.auth).API_KEY);
    const catalog = asRecord(settings.modelCatalog).models;
    const entries = Array.isArray(catalog) ? catalog.map(asRecord) : [];
    const selected = entries.find((entry) => entry.key === settings.defaultModelKey) ?? entries[0] ?? {};
    models = entries.flatMap((entry) => { const id = readString(entry.model); return id ? [modelEntry(id, entry)] : []; });
    fields.model = readString(selected.model);
    if (sourceApp === 'grok') {
      fields.baseUrl = firstString(selected, 'baseUrl', 'base_url') ?? 'https://api.x.ai/v1';
      fields.apiKey = firstString(selected, 'apiKey', 'api_key') ?? fields.apiKey;
      fields.apiFormat ??= normalizeSharedApiFormat(selected.apiBackend ?? selected.api_backend) ?? 'openai_chat';
      fields.baseUrlStyle = normalizeSharedApiFormat(selected.apiBackend ?? selected.api_backend) === 'anthropic_messages' ? 'root' : 'versioned';
      fields.apiKeyField ??= fields.apiFormat === 'gemini_native' ? 'x-goog-api-key' : 'bearer';
    } else {
      const configs = asRecord(settings.providerConfigs);
      const config = asRecord(configs[readString(selected.provider) ?? ''] ?? Object.values(configs)[0]);
      fields.baseUrl = readString(config.base_url) ?? 'https://api.kimi.com/coding/v1';
      fields.apiKey ??= readString(config.api_key);
      headers ??= config.custom_headers;
      fields.apiFormat ??= normalizeSharedApiFormat(config.type) ?? 'openai_chat';
      fields.baseUrlStyle = ['anthropic', 'gemini', 'google_genai'].includes(String(config.type)) ? 'root' : 'versioned';
      fields.apiKeyField ??= fields.apiFormat === 'anthropic_messages' && config.type === 'anthropic' ? 'x-api-key'
        : fields.apiFormat === 'gemini_native' ? 'x-goog-api-key' : 'bearer';
    }
  } else if (sourceApp === 'opencode') {
    fields.baseUrlStyle = 'versioned';
    const options = asRecord(settings.options);
    fields.apiKey = readString(options.authToken) ?? readString(options.apiKey) ?? fields.apiKey;
    fields.baseUrl = readString(options.baseURL) ?? fields.baseUrl;
    fields.apiFormat = ({ '@ai-sdk/anthropic': 'anthropic_messages', '@ai-sdk/google': 'gemini_native', '@ai-sdk/openai': 'openai_responses', '@ai-sdk/openai-compatible': 'openai_chat', '@openrouter/ai-sdk-provider': 'openai_chat', '@ai-sdk/xai': 'openai_chat', '@ai-sdk/mistral': 'openai_chat' } as Record<string, SharedApiFormat>)[readString(settings.npm) ?? ''] ?? fields.apiFormat;
    if (fields.apiFormat === 'anthropic_messages') fields.apiKeyField = readString(options.authToken) ? 'bearer' : 'x-api-key';
    models = Object.entries(asRecord(settings.models)).map(([id, value]) => modelEntry(readString(asRecord(value).id) ?? id, value));
    if (provider.defaultModel) fields.model = readString(asRecord(asRecord(settings.models)[provider.defaultModel]).id) ?? provider.defaultModel;
    headers ??= options.headers;
  } else {
    fields.baseUrlStyle = 'root';
    const credential = asRecord(provider.credential);
    const isHermes = sourceApp === 'hermes';
    fields.apiKey = firstString(settings, isHermes ? 'api_key' : 'apiKey')
      ?? (typeof provider.credential === 'string' ? provider.credential : credential.type === 'api_key' || credential.type === 'api' ? readString(credential.key) : undefined) ?? fields.apiKey;
    fields.baseUrl = firstString(settings, 'baseUrl', 'baseURL', 'base_url') ?? fields.baseUrl;
    fields.apiFormat = normalizeSharedApiFormat(settings.api_mode ?? settings.api) ?? fields.apiFormat;
    if (settings.authHeader === true && fields.apiFormat === 'anthropic_messages') fields.apiKeyField = 'bearer';
    else if (fields.apiFormat === 'anthropic_messages') fields.apiKeyField ??= 'x-api-key';
    const rawModels = settings.models;
    if (Array.isArray(rawModels)) {
      models = rawModels.flatMap((value) => {
        if (typeof value === 'string') return [modelEntry(value, {})];
        const id = firstString(asRecord(value), 'id', 'model'); return id ? [modelEntry(id, value)] : [];
      });
    } else {
      models = Object.entries(asRecord(rawModels)).map(([id, value]) => modelEntry(id, value));
    }
    fields.model = readString(settings.model);
    headers ??= settings.headers;
  }
  if (sourceApp !== 'opencode') fields.model = provider.defaultModel ?? fields.model;
  if (!models.length) models = fields.models ?? [];
  if (fields.model) fields.model = sourceApp === 'claude' || sourceApp === 'claudedesktop' ? stripMarker(fields.model) : fields.model.trim();
  if (fields.model && !models.some((model) => model.id === fields.model)) models.unshift(modelEntry(fields.model, {}));
  fields.models = models.filter((model, index) => model.id && models.findIndex((candidate) => candidate.id === model.id) === index);
  if (sourceApp !== 'codex') fields.model ??= fields.models[0]?.id;
  if (Array.isArray(headers)) fields.headers = headers.map((header) => asRecord(header) as unknown as SharedHeader);
  else if (headers && typeof headers === 'object') fields.headers = Object.entries(asRecord(headers)).flatMap(([name, value]) => typeof value === 'string' ? [{ op: 'set' as const, name, value }] : []);
  if (fields.apiKey && isCredentialReference(fields.apiKey)) delete fields.apiKey;
  return fields;
}

/** A catalog may contain multiple upstream connections. Share one coherent
 * connection at a time instead of sending every model to the first URL/key. */
export function extractProviderShareVariants(sourceApp: ProviderShareApp, provider: ShareableProvider) {
  const settings = parseSettings(provider.settingsConfig);
  const nativeVariants = extractNativeShareVariants(sourceApp, provider, settings);
  if (nativeVariants) return nativeVariants;
  const entries = asRecord(settings.modelCatalog).models;
  if (!['grok', 'kimi'].includes(sourceApp) || !Array.isArray(entries)) {
    return [{ id: '', label: provider.name, fields: extractProviderShareFields(sourceApp, provider) }];
  }
  const groups = new Map<string, Record<string, unknown>[]>();
  for (const rawEntry of entries) {
    const entry = asRecord(rawEntry);
    const providerConfig = asRecord(asRecord(settings.providerConfigs)[readString(entry.provider) ?? '']);
    const identity = sourceApp === 'kimi' ? JSON.stringify(providerConfig)
      : JSON.stringify([entry.baseUrl ?? entry.base_url, entry.apiBackend ?? entry.api_backend, entry.apiKey ?? entry.api_key]);
    groups.set(identity, [...(groups.get(identity) ?? []), entry]);
  }
  if (groups.size <= 1) return [{ id: '', label: provider.name, fields: extractProviderShareFields(sourceApp, provider) }];
  return [...groups.values()].sort((left, right) => Number(right.some((entry) => entry.key === settings.defaultModelKey)) - Number(left.some((entry) => entry.key === settings.defaultModelKey))).map((models, index) => {
    const selected = models.find((entry) => entry.key === settings.defaultModelKey) ?? models[0];
    const fields = extractProviderShareFields(sourceApp, {
      ...provider,
      defaultModel: undefined,
      settingsConfig: JSON.stringify({ ...settings, defaultModelKey: selected.key, modelCatalog: { ...asRecord(settings.modelCatalog), models } }),
    });
    return { id: readString(selected.key) ?? String(index + 1), label: `${fields.apiFormat ?? ''} · ${fields.baseUrl ?? ''} · ${fields.model ?? ''}`, fields };
  });
}

function extractNativeShareVariants(sourceApp: ProviderShareApp, provider: ShareableProvider, settings: Record<string, unknown>) {
  const entries: Array<[string, Record<string, unknown>]> = sourceApp === 'opencode'
    ? Object.entries(asRecord(settings.models)).map(([id, model]) => [id, asRecord(model)])
    : ['pi', 'omp', 'openclaw'].includes(sourceApp) && Array.isArray(settings.models)
      ? settings.models.flatMap((value) => {
        const model = asRecord(value);
        const id = readString(model.id);
        return id ? [[id, model] as [string, Record<string, unknown>]] : [];
      }) : [];
  if (!entries.length) return undefined;
  const groups = new Map<string, { settings: Record<string, unknown>; models: typeof entries }>();
  for (const entry of entries) {
    const [, model] = entry;
    let connection: Record<string, unknown>;
    if (sourceApp === 'opencode') {
      const modelProvider = asRecord(model.provider);
      const modelOptions = asRecord(model.options);
      const options = { ...asRecord(settings.options) };
      if (readString(modelProvider.api)) options.baseURL = modelProvider.api;
      for (const key of ['baseURL', 'apiKey', 'authToken', 'headers']) {
        if (modelOptions[key] !== undefined) options[key] = modelOptions[key];
      }
      connection = { npm: modelProvider.npm ?? settings.npm, options };
    } else {
      connection = Object.fromEntries(['api', 'baseUrl', 'apiKey', 'headers', 'authHeader'].map((key) => [key, model[key] ?? settings[key]]));
    }
    const identity = JSON.stringify(connection);
    const group = groups.get(identity) ?? { settings: { ...settings, ...connection }, models: [] };
    group.models.push(entry);
    groups.set(identity, group);
  }
  if (groups.size <= 1 && !entries.some(([, model]) => model.provider || model.baseUrl || model.api || model.options)) return undefined;
  return [...groups.values()].sort((left, right) => Number(right.models.some(([id]) => id === provider.defaultModel)) - Number(left.models.some(([id]) => id === provider.defaultModel))).map((group) => {
    const selected = group.models.find(([id]) => id === provider.defaultModel) ?? group.models[0];
    const fields = extractProviderShareFields(sourceApp, {
      ...provider,
      defaultModel: selected[0],
      settingsConfig: JSON.stringify({ ...group.settings, models: sourceApp === 'opencode' ? Object.fromEntries(group.models) : group.models.map(([, model]) => model) }),
    });
    return { id: groups.size > 1 ? selected[0] : '', label: `${fields.apiFormat ?? ''} · ${fields.baseUrl ?? ''} · ${fields.model ?? ''}`, fields };
  });
}
