import { useState, useCallback, useEffect, useRef } from 'react';
import {
  extractCodexBaseUrl,
  ensureCodexCustomProviderConfig,
  setCodexBaseUrl,
  extractCodexModel,
  setCodexModel,
  normalizeQuotes,
  normalizeCodexConfigForOfficialMode,
  removeCodexBaseUrl,
  removeCodexModel,
} from '@/utils/codexConfigUtils';
import type {
  CodexCatalogModel,
  CodexProviderCategory,
  CodexRequiresOpenaiAuthModeSelection,
  CodexSettingsConfig,
} from '@/types/codex';
import {
  normalizeCodexCatalogModalities,
  normalizeCodexCatalogModels,
  normalizeCodexCatalogReasoningLevels,
  normalizeCodexCatalogServiceTiers,
} from '../utils/codexCatalogModels';
import {
  buildCodexSettingsConfig,
  normalizeCodexRequiresOpenaiAuthMode,
  resolveCodexAutoReviewModelOverride,
} from '../utils/codexSettingsConfig';

interface UseCodexConfigStateProps {
  initialData?: {
    settingsConfig?: string;
  };
}

export interface CodexSettingsConfigSnapshot {
  category?: CodexProviderCategory;
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  config?: string;
  catalogModels?: CodexCatalogModel[];
  autoReviewModelOverride?: string;
  requiresOpenaiAuthMode?: CodexRequiresOpenaiAuthModeSelection;
}

// 新建配置的默认 config.toml 模板
// `requires_openai_auth` is deliberately absent (issue #394): the backend
// projection owns that line, so the editor must not advertise a value the next
// apply would override. Intent is expressed through the form's explicit override.
const DEFAULT_CONFIG_TOML = `model_provider = "custom"
model_reasoning_effort = "high"

[model_providers.custom]
name = "OpenAI"
wire_api = "responses"`;

/** Absent/`auto` selection: let the backend derive the flag from the auth mechanism. */
const DEFAULT_REQUIRES_OPENAI_AUTH_MODE: CodexRequiresOpenaiAuthModeSelection = 'auto';

function parseCodexCatalogModels(config: CodexSettingsConfig): CodexCatalogModel[] {
  const rawModels = Array.isArray(config.modelCatalog?.models)
    ? config.modelCatalog.models
    : [];

  return normalizeCodexCatalogModels(
    rawModels.map((item) => {
      const compatibleItem = item as CodexCatalogModel & {
        display_name?: unknown;
        context_window?: unknown;
        reasoning_levels?: unknown;
        default_reasoning_level?: unknown;
      };

      return {
        model: typeof compatibleItem.model === 'string' ? compatibleItem.model : '',
        displayName:
          typeof compatibleItem.displayName === 'string'
            ? compatibleItem.displayName
            : typeof compatibleItem.display_name === 'string'
              ? compatibleItem.display_name
              : '',
        contextWindow:
          typeof compatibleItem.contextWindow === 'string' || typeof compatibleItem.contextWindow === 'number'
            ? compatibleItem.contextWindow
            : typeof compatibleItem.context_window === 'string' || typeof compatibleItem.context_window === 'number'
              ? compatibleItem.context_window
              : '',
        supportsImage:
          typeof compatibleItem.supportsImage === 'boolean' ? compatibleItem.supportsImage : undefined,
        vision: typeof compatibleItem.vision === 'boolean' ? compatibleItem.vision : undefined,
        attachment: typeof compatibleItem.attachment === 'boolean' ? compatibleItem.attachment : undefined,
        modalities: normalizeCodexCatalogModalities(compatibleItem.modalities),
        reasoningLevels: normalizeCodexCatalogReasoningLevels(
          Array.isArray(compatibleItem.reasoningLevels)
            ? compatibleItem.reasoningLevels
            : compatibleItem.reasoning_levels,
        ),
        serviceTiers: normalizeCodexCatalogServiceTiers(compatibleItem.serviceTiers),
        defaultReasoningLevel:
          typeof compatibleItem.defaultReasoningLevel === 'string' && compatibleItem.defaultReasoningLevel.trim()
            ? compatibleItem.defaultReasoningLevel.trim()
            : typeof compatibleItem.default_reasoning_level === 'string' && compatibleItem.default_reasoning_level.trim()
              ? compatibleItem.default_reasoning_level.trim()
              : undefined,
      };
    }),
  );
}

function parseInitialCodexState(initialData?: { settingsConfig?: string }) {
  if (!initialData?.settingsConfig) {
    const defaultBaseUrl = extractCodexBaseUrl(DEFAULT_CONFIG_TOML) || '';
    const defaultModel = extractCodexModel(DEFAULT_CONFIG_TOML) || '';

    return {
      category: 'custom' as CodexProviderCategory,
      apiKey: '',
      auth: {} as Record<string, unknown>,
      baseUrl: defaultBaseUrl,
      model: defaultModel,
      config: DEFAULT_CONFIG_TOML,
      catalogModels: [] as CodexCatalogModel[],
      autoReviewModelOverride: '',
      requiresOpenaiAuthMode: DEFAULT_REQUIRES_OPENAI_AUTH_MODE,
    };
  }

  try {
    const config: CodexSettingsConfig = JSON.parse(initialData.settingsConfig);
    const authObj = config.auth || {};
    const apiKey = typeof authObj.OPENAI_API_KEY === 'string' ? authObj.OPENAI_API_KEY : '';
    const configStr = config.config || '';
    const baseUrl = extractCodexBaseUrl(configStr) || '';
    const model = extractCodexModel(configStr) || '';
    const category: CodexProviderCategory = apiKey.trim() || baseUrl.trim() ? 'custom' : 'official';
    // Only custom providers honour the override; official ones stay automatic.
    const requiresOpenaiAuthMode =
      category === 'custom'
        ? (normalizeCodexRequiresOpenaiAuthMode(config.requiresOpenaiAuthMode) ??
          DEFAULT_REQUIRES_OPENAI_AUTH_MODE)
        : DEFAULT_REQUIRES_OPENAI_AUTH_MODE;

    return {
      category,
      apiKey,
      auth: authObj as Record<string, unknown>,
      baseUrl,
      model,
      config: configStr,
      catalogModels: category === 'custom' ? parseCodexCatalogModels(config) : [],
      autoReviewModelOverride:
        category === 'custom' ? (resolveCodexAutoReviewModelOverride(config) || '') : '',
      requiresOpenaiAuthMode,
    };
  } catch {
    return {
      category: 'custom' as CodexProviderCategory,
      apiKey: '',
      auth: {} as Record<string, unknown>,
      baseUrl: '',
      model: '',
      config: '',
      catalogModels: [] as CodexCatalogModel[],
      autoReviewModelOverride: '',
      requiresOpenaiAuthMode: DEFAULT_REQUIRES_OPENAI_AUTH_MODE,
    };
  }
}

/**
 * Codex 配置状态管理 Hook
 */
export function useCodexConfigState({ initialData }: UseCodexConfigStateProps = {}) {
  const parsedInitial = parseInitialCodexState(initialData);

  // 基础状态（使用解析后的初始值）
  const [codexApiKey, setCodexApiKey] = useState(parsedInitial.apiKey);
  const [codexBaseUrl, setCodexBaseUrlState] = useState(parsedInitial.baseUrl);
  const [codexModel, setCodexModelState] = useState(parsedInitial.model);
  const [codexConfig, setCodexConfigState] = useState(parsedInitial.config);
  const [codexAuth, setCodexAuthState] = useState<Record<string, unknown>>(parsedInitial.auth);
  const [codexCatalogModels, setCodexCatalogModels] = useState<CodexCatalogModel[]>(parsedInitial.catalogModels);
  const [codexAutoReviewModelOverride, setCodexAutoReviewModelOverride] = useState(
    parsedInitial.autoReviewModelOverride,
  );
  const [codexRequiresOpenaiAuthMode, setCodexRequiresOpenaiAuthMode] =
    useState<CodexRequiresOpenaiAuthModeSelection>(parsedInitial.requiresOpenaiAuthMode);
  const [providerCategory, setProviderCategoryState] = useState<CodexProviderCategory>(parsedInitial.category);

  // 防止循环更新的标志位
  const isUpdatingBaseUrlRef = useRef(false);
  const isUpdatingModelRef = useRef(false);

  // 用户是否在输入框中手动设置了值（输入框优先于 TOML 编辑器）
  const userSetBaseUrlRef = useRef(false);
  const userSetModelRef = useRef(false);

  // 标记 API Key 输入框是否正在更新（用于同步到 auth.json）
  const isUpdatingApiKeyRef = useRef(false);

  // 与 TOML 配置保持 Base URL 同步（configToml 变化 → 提取 baseUrl）
  // 只有当 config 中存在 base_url 字段时才更新，且用户未在输入框中手动设置
  useEffect(() => {
    if (isUpdatingBaseUrlRef.current || userSetBaseUrlRef.current) {
      return;
    }
    const extracted = extractCodexBaseUrl(codexConfig);
    // 只有当 config 中存在 base_url 时才更新
    if (extracted && extracted !== codexBaseUrl) {
      setCodexBaseUrlState(extracted);
    }
  }, [codexConfig, codexBaseUrl]);

  // 与 TOML 配置保持 Model 同步（configToml 变化 → 提取 model）
  // 只有当 config 中存在 model 字段时才更新，且用户未在输入框中手动设置
  useEffect(() => {
    if (isUpdatingModelRef.current || userSetModelRef.current) {
      return;
    }
    const extracted = extractCodexModel(codexConfig);
    // 只有当 config 中存在 model 时才更新
    if (extracted && extracted !== codexModel) {
      setCodexModelState(extracted);
    }
  }, [codexConfig, codexModel]);

  // 处理 API Key 变化（同步更新 auth.json 中的 OPENAI_API_KEY）
  const handleApiKeyChange = useCallback((key: string) => {
    const trimmedKey = key.trim();
    setCodexApiKey(trimmedKey);
    if (trimmedKey) {
      setProviderCategoryState('custom');
    }
    // 标记正在从 API Key 输入框更新，需要同步到 auth.json 编辑器
    isUpdatingApiKeyRef.current = true;
    // 同步更新 auth.json，保留其他字段
    setCodexAuthState((prev) => {
      const nextAuth = { ...prev };
      if (trimmedKey) {
        nextAuth.OPENAI_API_KEY = trimmedKey;
      } else {
        delete nextAuth.OPENAI_API_KEY;
      }
      return nextAuth;
    });
    // 使用 requestAnimationFrame 确保在下一帧重置
    requestAnimationFrame(() => {
      setTimeout(() => {
        isUpdatingApiKeyRef.current = false;
      }, 50);
    });
  }, []);

  // 处理 auth.json 变化（从 JSON 编辑器同步 OPENAI_API_KEY 到输入框）
  const handleAuthChange = useCallback((authObj: Record<string, unknown>) => {
    setCodexAuthState(authObj);
    // 从 auth.json 中提取 OPENAI_API_KEY 并同步到输入框
    const apiKey = typeof authObj.OPENAI_API_KEY === 'string' ? authObj.OPENAI_API_KEY : '';
    if (apiKey !== codexApiKey) {
      setCodexApiKey(apiKey);
    }
  }, [codexApiKey]);

  // 处理 Base URL 变化（baseUrl 变化 → 写入 configToml）
  const handleBaseUrlChange = useCallback((url: string) => {
    const sanitized = normalizeQuotes(url).replace(/['"`]/g, '').trim();
    setCodexBaseUrlState(sanitized);
    if (sanitized) {
      setProviderCategoryState('custom');
    }

    // 标记用户已在输入框中设置值，后续不再从 TOML 编辑器覆盖
    userSetBaseUrlRef.current = true;

    if (!sanitized) {
      // 如果清空，从 config 中移除
      userSetBaseUrlRef.current = false;
      setCodexConfigState((prev) => removeCodexBaseUrl(prev));
      return;
    }

    // 标记正在更新，防止循环
    isUpdatingBaseUrlRef.current = true;
    setCodexConfigState((prev) => {
      const newConfig = setCodexBaseUrl(prev, sanitized);
      return newConfig;
    });
    // 使用 requestAnimationFrame 确保在下一帧重置
    requestAnimationFrame(() => {
      setTimeout(() => {
        isUpdatingBaseUrlRef.current = false;
      }, 50);
    });
  }, []);

  // 处理 Model 变化（model 变化 → 写入 configToml）
  const handleModelChange = useCallback((model: string) => {
    const trimmed = normalizeQuotes(model).replace(/['"`]/g, '').trim();
    setCodexModelState(trimmed);

    // 标记用户已在输入框中设置值，后续不再从 TOML 编辑器覆盖
    userSetModelRef.current = true;

    if (!trimmed) {
      // 如果清空，从 config 中移除
      userSetModelRef.current = false;
      setCodexConfigState((prev) => removeCodexModel(prev));
      return;
    }

    // 标记正在更新，防止循环
    isUpdatingModelRef.current = true;
    setCodexConfigState((prev) => {
      const newConfig = setCodexModel(prev, trimmed);
      return newConfig;
    });
    // 使用 requestAnimationFrame 确保在下一帧重置
    requestAnimationFrame(() => {
      setTimeout(() => {
        isUpdatingModelRef.current = false;
      }, 50);
    });
  }, []);

  // 处理 Config 变化（手动编辑 configToml → 提取字段）
  const handleConfigChange = useCallback((value: string) => {
    // 归一化中文/全角/弯引号，避免 TOML 解析报错
    const normalized = normalizeQuotes(value);
    setCodexConfigState(normalized);

    // 自动提取 Base URL 和 Model（如果不是正在更新中，且用户未在输入框中手动设置）
    // 输入框的值优先于 TOML 编辑器的值
    if (!isUpdatingBaseUrlRef.current && !userSetBaseUrlRef.current) {
      const extracted = extractCodexBaseUrl(normalized);
      // 只有当 config 中存在 base_url 字段时才更新状态
      if (extracted && extracted !== codexBaseUrl) {
        setCodexBaseUrlState(extracted);
      }
    }

    if (!isUpdatingModelRef.current && !userSetModelRef.current) {
      const extractedModel = extractCodexModel(normalized);
      // 只有当 config 中存在 model 字段时才更新状态
      if (extractedModel && extractedModel !== codexModel) {
        setCodexModelState(extractedModel);
      }
    }
  }, [codexBaseUrl, codexModel]);

  // 设置 Config（支持函数更新）
  const setCodexConfig = useCallback((value: string | ((prev: string) => string)) => {
    if (typeof value === 'function') {
      setCodexConfigState((prev) => {
        const newValue = value(prev);
        return normalizeQuotes(newValue);
      });
    } else {
      setCodexConfigState(normalizeQuotes(value));
    }
  }, []);

  // 重置配置（用于预设切换或导入）
  const resetCodexConfig = useCallback((auth: Record<string, unknown>, config: string) => {
    // 设置 API Key
    const apiKey = typeof auth.OPENAI_API_KEY === 'string' ? auth.OPENAI_API_KEY : '';
    setCodexApiKey(apiKey);

    // 提取并设置字段
    const baseUrl = extractCodexBaseUrl(config) || '';
    const model = extractCodexModel(config) || '';

    setCodexBaseUrlState(baseUrl);
    setCodexModelState(model);
    setCodexCatalogModels([]);
    setCodexAutoReviewModelOverride('');
    setCodexRequiresOpenaiAuthMode(DEFAULT_REQUIRES_OPENAI_AUTH_MODE);
    setProviderCategoryState(
      apiKey.trim() || baseUrl.trim() ? 'custom' : 'official',
    );

    // 从 config 中移除已提取的字段
    let cleanedConfig = config;
    if (baseUrl) {
      cleanedConfig = removeCodexBaseUrl(cleanedConfig);
    }
    if (model) {
      cleanedConfig = removeCodexModel(cleanedConfig);
    }

    setCodexConfigState(cleanedConfig);
  }, []);

  const resetFromSettingsConfig = useCallback((settingsConfig?: string) => {
    const nextState = parseInitialCodexState(
      settingsConfig ? { settingsConfig } : undefined,
    );

    userSetBaseUrlRef.current = false;
    userSetModelRef.current = false;
    isUpdatingBaseUrlRef.current = false;
    isUpdatingModelRef.current = false;
    isUpdatingApiKeyRef.current = false;

    setCodexApiKey(nextState.apiKey);
    setCodexAuthState(nextState.auth);
    setCodexBaseUrlState(nextState.baseUrl);
    setCodexModelState(nextState.model);
    setCodexConfigState(nextState.config);
    setCodexCatalogModels(nextState.catalogModels);
    setCodexAutoReviewModelOverride(nextState.autoReviewModelOverride);
    setCodexRequiresOpenaiAuthMode(nextState.requiresOpenaiAuthMode);
    setProviderCategoryState(nextState.category);
  }, []);

  const handleProviderCategoryChange = useCallback((nextCategory: CodexProviderCategory) => {
    setProviderCategoryState(nextCategory);

    if (nextCategory === 'official') {
      setCodexApiKey('');
      setCodexBaseUrlState('');
      setCodexAuthState((prev) => {
        const nextAuth = { ...prev };
        delete nextAuth.OPENAI_API_KEY;
        return nextAuth;
      });
      userSetBaseUrlRef.current = false;
      setCodexCatalogModels([]);
      setCodexAutoReviewModelOverride('');
      setCodexRequiresOpenaiAuthMode(DEFAULT_REQUIRES_OPENAI_AUTH_MODE);
      setCodexConfigState((prev) => normalizeCodexConfigForOfficialMode(prev));
    } else {
      setCodexConfigState((prev) => ensureCodexCustomProviderConfig(prev));
    }
  }, []);

  const handleAutoReviewModelOverrideChange = useCallback((value: string) => {
    setCodexAutoReviewModelOverride(value);
  }, []);

  // 获取最终的 settingsConfig（用于保存）
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const getFinalSettingsConfig = useCallback((snapshot: CodexSettingsConfigSnapshot = {}): string => {
    const finalCategory = snapshot.category ?? providerCategory;
    const finalApiKey = snapshot.apiKey ?? codexApiKey;
    const finalBaseUrl = snapshot.baseUrl ?? codexBaseUrl;
    const finalModel = snapshot.model ?? codexModel;
    return buildCodexSettingsConfig({
      category: finalCategory,
      apiKey: finalApiKey,
      baseUrl: finalBaseUrl,
      model: finalModel,
      config: snapshot.config ?? codexConfig,
      catalogModels: snapshot.catalogModels ?? codexCatalogModels,
      autoReviewModelOverride:
        snapshot.autoReviewModelOverride ?? codexAutoReviewModelOverride,
      requiresOpenaiAuthMode:
        snapshot.requiresOpenaiAuthMode ?? codexRequiresOpenaiAuthMode,
      auth: codexAuth,
    });
  }, [
    codexApiKey,
    codexAuth,
    codexAutoReviewModelOverride,
    codexRequiresOpenaiAuthMode,
    codexBaseUrl,
    codexCatalogModels,
    codexModel,
    codexConfig,
    providerCategory,
  ]);

  return {
    // 状态
    codexApiKey,
    codexAuth,
    codexBaseUrl,
    codexModel,
    codexConfig,
    codexCatalogModels,
    codexAutoReviewModelOverride,
    codexRequiresOpenaiAuthMode,
    providerCategory,

    // 标志位（用于同步控制）
    isUpdatingApiKeyRef,

    // 变更处理器
    handleApiKeyChange,
    handleAuthChange,
    handleBaseUrlChange,
    handleModelChange,
    handleConfigChange,
    handleAutoReviewModelOverrideChange,
    handleProviderCategoryChange,

    // 工具方法
    setCodexConfig,
    setCodexCatalogModels,
    setCodexAutoReviewModelOverride,
    setCodexRequiresOpenaiAuthMode,
    resetCodexConfig,
    resetFromSettingsConfig,
    getFinalSettingsConfig,
  };
}
