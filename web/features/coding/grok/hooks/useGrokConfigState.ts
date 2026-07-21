import { useState, useCallback, useEffect, useRef } from 'react';
import {
  extractGrokBaseUrl,
  extractGrokModel,
  extractGrokReasoningEffort,
  normalizeQuotes,
} from '@/utils/grokConfigUtils';
import type {
  GrokApiFormat,
  GrokCatalogModel,
  GrokProviderCategory,
  GrokSettingsConfig,
} from '@/types/grok';
import {
  normalizeGrokCatalogModalities,
  normalizeGrokCatalogModels,
} from '../utils/grokCatalogModels';
import {
  buildGrokSettingsConfig,
  normalizeGrokReasoningEffort,
  resolveGrokCatalogReasoningEffort,
} from '../utils/grokSettingsConfig';

interface UseGrokConfigStateProps {
  initialData?: {
    settingsConfig?: string;
  };
}

export interface GrokSettingsConfigSnapshot {
  category?: GrokProviderCategory;
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  config?: string;
  catalogModels?: GrokCatalogModel[];
  apiFormat?: GrokApiFormat;
  supportsBackendSearch?: boolean;
  reasoningEffort?: string;
}

const DEFAULT_CONFIG_TOML = '';

function parseGrokCatalogModels(config: GrokSettingsConfig): GrokCatalogModel[] {
  const rawModels = Array.isArray(config.modelCatalog?.models)
    ? config.modelCatalog.models
    : [];

  return normalizeGrokCatalogModels(
    rawModels.map((item) => {
      const compatibleItem = item as GrokCatalogModel & {
        display_name?: unknown;
        context_window?: unknown;
      };
      return {
        ...compatibleItem,
        key: typeof compatibleItem.key === 'string' ? compatibleItem.key : undefined,
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
        baseUrl: typeof compatibleItem.baseUrl === 'string' ? compatibleItem.baseUrl : undefined,
        apiBackend: typeof compatibleItem.apiBackend === 'string' ? compatibleItem.apiBackend : undefined,
        supportsBackendSearch:
          typeof compatibleItem.supportsBackendSearch === 'boolean'
            ? compatibleItem.supportsBackendSearch
            : undefined,
        supportsImage:
          typeof compatibleItem.supportsImage === 'boolean' ? compatibleItem.supportsImage : undefined,
        vision: typeof compatibleItem.vision === 'boolean' ? compatibleItem.vision : undefined,
        attachment: typeof compatibleItem.attachment === 'boolean' ? compatibleItem.attachment : undefined,
        modalities: normalizeGrokCatalogModalities(compatibleItem.modalities),
      };
    }),
  );
}

function parseInitialGrokState(initialData?: { settingsConfig?: string }) {
  if (!initialData?.settingsConfig) {
    const defaultBaseUrl = extractGrokBaseUrl(DEFAULT_CONFIG_TOML) || '';
    const defaultModel = extractGrokModel(DEFAULT_CONFIG_TOML) || '';

    return {
      category: 'custom' as GrokProviderCategory,
      apiKey: '',
      auth: {} as Record<string, unknown>,
      baseUrl: defaultBaseUrl,
      model: defaultModel,
      config: DEFAULT_CONFIG_TOML,
      catalogModels: [] as GrokCatalogModel[],
      reasoningEffort: undefined as string | undefined,
    };
  }

  try {
    const config: GrokSettingsConfig = JSON.parse(initialData.settingsConfig);
    const authObj = config.auth || {};
    const apiKey = typeof authObj.API_KEY === 'string' ? authObj.API_KEY : '';
    const configStr = config.config || '';
    const catalogModels = parseGrokCatalogModels(config);
    // Form "model name" is the upstream model ID, not the local catalog key.
    // Custom providers fix key="custom" in defaultModelKey; read .model from that slot.
    const defaultModelKey = config.defaultModelKey?.trim() || '';
    const selectedCatalogModel = catalogModels.find((item) => item.key === defaultModelKey)
      || catalogModels.find((item) => item.model === defaultModelKey)
      || catalogModels[0];
    const model = selectedCatalogModel?.model?.trim()
      || (defaultModelKey && defaultModelKey !== 'custom' ? defaultModelKey : '')
      || extractGrokModel(configStr)
      || '';
    const baseUrl = selectedCatalogModel?.baseUrl?.trim() || extractGrokBaseUrl(configStr) || '';
    const category: GrokProviderCategory = apiKey.trim() || baseUrl.trim() ? 'custom' : 'official';
    // Official: settings.defaultReasoningEffort; custom: shared catalog effort; fall back to live TOML.
    const reasoningEffort = normalizeGrokReasoningEffort(
      category === 'official'
        ? (config.defaultReasoningEffort || extractGrokReasoningEffort(configStr))
        : (
          resolveGrokCatalogReasoningEffort(catalogModels)
          || extractGrokReasoningEffort(configStr)
        ),
    );

    return {
      category,
      apiKey,
      auth: authObj as Record<string, unknown>,
      baseUrl,
      model,
      config: configStr,
      catalogModels: category === 'custom' ? catalogModels : [],
      reasoningEffort,
    };
  } catch {
    return {
      category: 'custom' as GrokProviderCategory,
      apiKey: '',
      auth: {} as Record<string, unknown>,
      baseUrl: '',
      model: '',
      config: '',
      catalogModels: [] as GrokCatalogModel[],
      reasoningEffort: undefined as string | undefined,
    };
  }
}

/**
 * Grok 配置状态管理 Hook
 */
export function useGrokConfigState({ initialData }: UseGrokConfigStateProps = {}) {
  const parsedInitial = parseInitialGrokState(initialData);

  // 基础状态（使用解析后的初始值）
  const [grokApiKey, setGrokApiKey] = useState(parsedInitial.apiKey);
  const [grokBaseUrl, setGrokBaseUrlState] = useState(parsedInitial.baseUrl);
  const [grokModel, setGrokModelState] = useState(parsedInitial.model);
  const [grokConfig, setGrokConfigState] = useState(parsedInitial.config);
  const [grokAuth, setGrokAuthState] = useState<Record<string, unknown>>(parsedInitial.auth);
  const [grokCatalogModels, setGrokCatalogModels] = useState<GrokCatalogModel[]>(parsedInitial.catalogModels);
  const [providerCategory, setProviderCategoryState] = useState<GrokProviderCategory>(parsedInitial.category);
  const [grokReasoningEffort, setGrokReasoningEffort] = useState<string | undefined>(
    parsedInitial.reasoningEffort,
  );

  // 防止循环更新的标志位
  const isUpdatingBaseUrlRef = useRef(false);
  const isUpdatingModelRef = useRef(false);

  // 用户是否在输入框中手动设置了值（输入框优先于 TOML 编辑器）
  const userSetBaseUrlRef = useRef(false);
  const userSetModelRef = useRef(false);

  // 标记 API Key 输入框是否正在更新（用于同步到 auth.json）
  const isUpdatingApiKeyRef = useRef(false);

  // 兼容读取早期版本误写入 config.toml 的模型字段；新保存只写结构化 modelCatalog。
  useEffect(() => {
    if (isUpdatingBaseUrlRef.current || userSetBaseUrlRef.current) {
      return;
    }
    const extracted = extractGrokBaseUrl(grokConfig);
    // 只有当 config 中存在 base_url 时才更新
    if (extracted && extracted !== grokBaseUrl) {
      setGrokBaseUrlState(extracted);
    }
  }, [grokConfig, grokBaseUrl]);

  // 与 TOML 配置保持 Model 同步（configToml 变化 → 提取 model）
  // 只有当 config 中存在 model 字段时才更新，且用户未在输入框中手动设置
  useEffect(() => {
    if (isUpdatingModelRef.current || userSetModelRef.current) {
      return;
    }
    const extracted = extractGrokModel(grokConfig);
    // 只有当 config 中存在 model 时才更新
    if (extracted && extracted !== grokModel) {
      setGrokModelState(extracted);
    }
  }, [grokConfig, grokModel]);

  // 处理 API Key 变化（同步更新 provider settings 中的 API_KEY）
  const handleApiKeyChange = useCallback((key: string) => {
    const trimmedKey = key.trim();
    setGrokApiKey(trimmedKey);
    if (trimmedKey) {
      setProviderCategoryState('custom');
    }
    // 标记正在从 API Key 输入框更新，需要同步到 auth.json 编辑器
    isUpdatingApiKeyRef.current = true;
    // 同步更新 auth.json，保留其他字段
    setGrokAuthState((prev) => {
      const nextAuth = { ...prev };
      if (trimmedKey) {
        nextAuth.API_KEY = trimmedKey;
      } else {
        delete nextAuth.API_KEY;
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

  // 处理 provider auth 配置变化（从高级 JSON 编辑器同步 API_KEY 到输入框）
  const handleAuthChange = useCallback((authObj: Record<string, unknown>) => {
    setGrokAuthState(authObj);
    // 从 provider auth 配置中提取 API_KEY 并同步到输入框
    const apiKey = typeof authObj.API_KEY === 'string' ? authObj.API_KEY : '';
    if (apiKey !== grokApiKey) {
      setGrokApiKey(apiKey);
    }
  }, [grokApiKey]);

  // Base URL 属于结构化模型目录，不再写入通用 TOML 编辑器。
  const handleBaseUrlChange = useCallback((url: string) => {
    const sanitized = normalizeQuotes(url).replace(/['"`]/g, '').trim();
    setGrokBaseUrlState(sanitized);
    if (sanitized) {
      setProviderCategoryState('custom');
    }

    // 标记用户已在输入框中设置值，后续不再从 TOML 编辑器覆盖
    userSetBaseUrlRef.current = true;

    if (!sanitized) userSetBaseUrlRef.current = false;
    isUpdatingBaseUrlRef.current = true;
    // 使用 requestAnimationFrame 确保在下一帧重置
    requestAnimationFrame(() => {
      setTimeout(() => {
        isUpdatingBaseUrlRef.current = false;
      }, 50);
    });
  }, []);

  // 默认模型属于 defaultModelKey，不再写入通用 TOML 编辑器。
  const handleModelChange = useCallback((model: string) => {
    const trimmed = normalizeQuotes(model).replace(/['"`]/g, '').trim();
    setGrokModelState(trimmed);

    // 标记用户已在输入框中设置值，后续不再从 TOML 编辑器覆盖
    userSetModelRef.current = true;

    if (!trimmed) userSetModelRef.current = false;
    isUpdatingModelRef.current = true;
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
    setGrokConfigState(normalized);

    // 自动提取 Base URL 和 Model（如果不是正在更新中，且用户未在输入框中手动设置）
    // 输入框的值优先于 TOML 编辑器的值
    if (!isUpdatingBaseUrlRef.current && !userSetBaseUrlRef.current) {
      const extracted = extractGrokBaseUrl(normalized);
      // 只有当 config 中存在 base_url 字段时才更新状态
      if (extracted && extracted !== grokBaseUrl) {
        setGrokBaseUrlState(extracted);
      }
    }

    if (!isUpdatingModelRef.current && !userSetModelRef.current) {
      const extractedModel = extractGrokModel(normalized);
      // 只有当 config 中存在 model 字段时才更新状态
      if (extractedModel && extractedModel !== grokModel) {
        setGrokModelState(extractedModel);
      }
    }
  }, [grokBaseUrl, grokModel]);

  // 设置 Config（支持函数更新）
  const setGrokConfig = useCallback((value: string | ((prev: string) => string)) => {
    if (typeof value === 'function') {
      setGrokConfigState((prev) => {
        const newValue = value(prev);
        return normalizeQuotes(newValue);
      });
    } else {
      setGrokConfigState(normalizeQuotes(value));
    }
  }, []);

  // 重置配置（用于预设切换或导入）
  const resetGrokConfig = useCallback((auth: Record<string, unknown>, config: string) => {
    // 设置 API Key
    const apiKey = typeof auth.API_KEY === 'string' ? auth.API_KEY : '';
    setGrokApiKey(apiKey);

    // 提取并设置字段
    const baseUrl = extractGrokBaseUrl(config) || '';
    const model = extractGrokModel(config) || '';

    setGrokBaseUrlState(baseUrl);
    setGrokModelState(model);
    setGrokCatalogModels([]);
    setGrokReasoningEffort(normalizeGrokReasoningEffort(extractGrokReasoningEffort(config)));
    setProviderCategoryState(
      apiKey.trim() || baseUrl.trim() ? 'custom' : 'official',
    );

    setGrokConfigState(config);
  }, []);

  const resetFromSettingsConfig = useCallback((settingsConfig?: string) => {
    const nextState = parseInitialGrokState(
      settingsConfig ? { settingsConfig } : undefined,
    );

    userSetBaseUrlRef.current = false;
    userSetModelRef.current = false;
    isUpdatingBaseUrlRef.current = false;
    isUpdatingModelRef.current = false;
    isUpdatingApiKeyRef.current = false;

    setGrokApiKey(nextState.apiKey);
    setGrokAuthState(nextState.auth);
    setGrokBaseUrlState(nextState.baseUrl);
    setGrokModelState(nextState.model);
    setGrokConfigState(nextState.config);
    setGrokCatalogModels(nextState.catalogModels);
    setProviderCategoryState(nextState.category);
    setGrokReasoningEffort(nextState.reasoningEffort);
  }, []);

  const handleProviderCategoryChange = useCallback((nextCategory: GrokProviderCategory) => {
    setProviderCategoryState(nextCategory);

    if (nextCategory === 'official') {
      setGrokApiKey('');
      setGrokBaseUrlState('');
      setGrokAuthState((prev) => {
        const nextAuth = { ...prev };
        delete nextAuth.API_KEY;
        return nextAuth;
      });
      userSetBaseUrlRef.current = false;
      setGrokCatalogModels([]);
    }
  }, []);

  const handleReasoningEffortChange = useCallback((value: string | undefined) => {
    setGrokReasoningEffort(normalizeGrokReasoningEffort(value));
  }, []);

  // 获取最终的 settingsConfig（用于保存）
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const getFinalSettingsConfig = useCallback((snapshot: GrokSettingsConfigSnapshot = {}): string => {
    const finalCategory = snapshot.category ?? providerCategory;
    const finalApiKey = snapshot.apiKey ?? grokApiKey;
    const finalBaseUrl = snapshot.baseUrl ?? grokBaseUrl;
    const finalModel = snapshot.model ?? grokModel;
    return buildGrokSettingsConfig({
      category: finalCategory,
      apiKey: finalApiKey,
      baseUrl: finalBaseUrl,
      model: finalModel,
      apiFormat: snapshot.apiFormat,
      supportsBackendSearch: snapshot.supportsBackendSearch,
      reasoningEffort: snapshot.reasoningEffort ?? grokReasoningEffort,
      config: snapshot.config ?? grokConfig,
      catalogModels: snapshot.catalogModels ?? grokCatalogModels,
      auth: grokAuth,
    });
  }, [
    grokApiKey,
    grokAuth,
    grokBaseUrl,
    grokCatalogModels,
    grokModel,
    grokConfig,
    grokReasoningEffort,
    providerCategory,
  ]);

  return {
    // 状态
    grokApiKey,
    grokAuth,
    grokBaseUrl,
    grokModel,
    grokConfig,
    grokCatalogModels,
    grokReasoningEffort,
    providerCategory,

    // 标志位（用于同步控制）
    isUpdatingApiKeyRef,

    // 变更处理器
    handleApiKeyChange,
    handleAuthChange,
    handleBaseUrlChange,
    handleModelChange,
    handleConfigChange,
    handleProviderCategoryChange,
    handleReasoningEffortChange,

    // 工具方法
    setGrokConfig,
    setGrokCatalogModels,
    resetGrokConfig,
    resetFromSettingsConfig,
    getFinalSettingsConfig,
  };
}
