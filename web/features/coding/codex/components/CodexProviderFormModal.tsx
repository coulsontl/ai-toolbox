import React from 'react';
import { Modal, Form, Input, Select, Space, Button, Alert, message, Typography, AutoComplete } from 'antd';
import {
  CloudDownloadOutlined,
  DeleteOutlined,
  DownOutlined,
  EyeInvisibleOutlined,
  EyeOutlined,
  PlusOutlined,
  RightOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '@/stores';
import type { CodexApiFormat, CodexCatalogModel, CodexProvider, CodexProviderFormValues, CodexSettingsConfig, GatewayProviderMeta } from '@/types/codex';
import { fetchCodexOfficialModels } from '@/services/codexApi';
import { readCurrentOpenCodeProviders } from '@/services/opencodeApi';
import type { FetchedModel, FetchModelsResponse } from '@/components/common/FetchModelsModal/types';
import BillingConfigCollapse from '@/features/coding/shared/providerBilling/BillingConfigCollapse';
import ProviderNotesCollapse from '@/features/coding/shared/providerConfig/ProviderNotesCollapse';
import {
  getBillingConfigFromMeta,
  mergeBillingConfigIntoMeta,
} from '@/features/coding/shared/providerBilling/billingConfigUtils';
import {
  CUSTOM_PROVIDER_ENDPOINT_KEY,
  CUSTOM_PROVIDER_PROFILE_ID,
  findGatewayProviderEndpoint,
  findGatewayProviderProfile,
  getGatewayProviderProfilesForTool,
  getGatewayProviderProfilesVersion,
  inferGatewayProviderEndpointSelection,
  parseGatewayProviderEndpointKey,
  subscribeGatewayProviderProfiles,
  toGatewayProviderEndpointKey,
  type GatewayProviderEndpointProfile,
} from '@/features/coding/shared/gateway/providerProfiles';
import {
  extractCodexBaseUrl,
  extractCodexModel,
  setCodexModel,
} from '@/utils/codexConfigUtils';
import TomlEditor from '@/components/common/TomlEditor';
import { parse as parseToml } from 'smol-toml';
import { useCodexConfigState } from '../hooks/useCodexConfigState';
import styles from './CodexProviderFormModal.module.less';

const { Text } = Typography;

const CODEX_OFFICIAL_FALLBACK_MODELS: FetchedModel[] = [
  { id: 'gpt-5.2', name: 'GPT 5.2' },
  { id: 'gpt-5.3-codex', name: 'GPT 5.3 Codex' },
  { id: 'gpt-5.3-codex-spark', name: 'GPT 5.3 Codex Spark' },
  { id: 'gpt-5.4', name: 'GPT 5.4' },
  { id: 'gpt-5.4-mini', name: 'GPT 5.4 Mini' },
  { id: 'gpt-5.5', name: 'GPT 5.5' },
  { id: 'codex-auto-review', name: 'Codex Auto Review' },
  { id: 'gpt-image-2', name: 'GPT Image 2' },
].map((model) => ({
  ...model,
  ownedBy: 'openai',
  created: undefined,
}));

const DEFAULT_CODEX_API_FORMAT: CodexApiFormat = 'openai_responses';
const OFFICIAL_PROVIDER_ENDPOINT_KEY = '__official__:';

function normalizeCodexApiFormat(value?: string): CodexApiFormat {
  if (value === 'openai_chat' || value === 'anthropic_messages' || value === 'gemini_native') {
    return value;
  }
  return DEFAULT_CODEX_API_FORMAT;
}

function mergeGatewayMetaIntoProviderMeta(
  meta: GatewayProviderMeta | undefined,
  apiFormat: CodexApiFormat | undefined,
  providerType?: string,
): GatewayProviderMeta | undefined {
  const nextMeta: GatewayProviderMeta = { ...(meta || {}) };
  delete nextMeta.apiFormat;
  delete nextMeta.providerType;
  if (apiFormat) {
    nextMeta.apiFormat = apiFormat;
  }
  if (providerType?.trim()) {
    nextMeta.providerType = providerType.trim();
  }
  return Object.values(nextMeta).some((value) => value !== undefined && value !== null && value !== '')
    ? nextMeta
    : undefined;
}

function getEndpointCatalogModels(endpoint?: GatewayProviderEndpointProfile): CodexCatalogModel[] {
  if (!Array.isArray(endpoint?.modelCatalog?.models)) {
    return [];
  }

  return endpoint.modelCatalog.models
    .map((item) => ({
      model: item.model?.trim() || '',
      displayName: item.displayName?.trim() || undefined,
      contextWindow: item.contextWindow,
    }))
    .filter((item) => item.model);
}

function getUniqueClaudeRoleModels(endpoint?: GatewayProviderEndpointProfile): string[] {
  const seenModels = new Set<string>();
  const roleModels = [
    endpoint?.models?.primary,
    endpoint?.models?.sonnet,
    endpoint?.models?.opus,
    endpoint?.models?.haiku,
    endpoint?.model,
  ];

  return roleModels
    .map((model) => model?.trim() || '')
    .filter((model) => {
      if (!model || seenModels.has(model)) {
        return false;
      }
      seenModels.add(model);
      return true;
    });
}

function getSiblingCatalogModel(
  profileId: string | null | undefined,
  model: string,
): CodexCatalogModel | undefined {
  const profile = findGatewayProviderProfile(profileId);
  const codexEndpoints = profile?.tools.codex?.endpoints || [];

  return codexEndpoints
    .flatMap((endpoint) => getEndpointCatalogModels(endpoint))
    .find((catalogModel) => catalogModel.model === model);
}

function getDerivedAnthropicCatalogModels(
  profileId: string | null | undefined,
  endpoint?: GatewayProviderEndpointProfile,
): CodexCatalogModel[] {
  if (!endpoint || normalizeCodexApiFormat(endpoint.apiFormat) !== 'anthropic_messages') {
    return [];
  }

  const claudeEndpoint = findGatewayProviderEndpoint(profileId, 'claude', endpoint.id)
    ?? findGatewayProviderEndpoint(profileId, 'claude');

  return getUniqueClaudeRoleModels(claudeEndpoint).map((model) => {
    const siblingCatalogModel = getSiblingCatalogModel(profileId, model);
    return {
      model,
      displayName: siblingCatalogModel?.displayName || model,
      contextWindow: siblingCatalogModel?.contextWindow,
    };
  });
}

function getCodexEndpointCatalogModels(
  profileId: string | null | undefined,
  endpoint?: GatewayProviderEndpointProfile,
): CodexCatalogModel[] {
  const endpointCatalogModels = getEndpointCatalogModels(endpoint);
  if (endpointCatalogModels.length > 0) {
    return endpointCatalogModels;
  }
  return getDerivedAnthropicCatalogModels(profileId, endpoint);
}

function applyEndpointToCodexSettingsConfig(
  settingsConfig: string,
  profileId: string | null | undefined,
  endpoint: GatewayProviderEndpointProfile | undefined,
  selectedModel?: string,
): string {
  if (!endpoint) {
    return settingsConfig;
  }

  try {
    const parsed = JSON.parse(settingsConfig || '{}') as CodexSettingsConfig;
    const catalogModels = getCodexEndpointCatalogModels(profileId, endpoint);
    let configText = parsed.config || '';
    const modelFromEndpoint = catalogModels[0]?.model || selectedModel?.trim() || endpoint.model?.trim();
    if (modelFromEndpoint) {
      configText = setCodexModel(configText, modelFromEndpoint);
    }

    const nextSettingsConfig: CodexSettingsConfig = {
      ...parsed,
      config: configText.trim(),
    };
    if (catalogModels.length > 0) {
      nextSettingsConfig.modelCatalog = { models: catalogModels };
    } else {
      delete nextSettingsConfig.modelCatalog;
    }
    return JSON.stringify(nextSettingsConfig);
  } catch {
    return settingsConfig;
  }
}

// TomlEditor 与 antd Form.Item 集成的包装组件
interface TomlEditorFormItemProps {
  value?: string;
  onChange?: (value: string) => void;
  placeholder?: string;
}

// 用于追踪 TOML 是否有效（在提交时验证）
const tomlValidityRef = { current: true };

const TomlEditorFormItem: React.FC<TomlEditorFormItemProps> = ({
  value = '',
  onChange,
  placeholder,
}) => {
  return (
    <TomlEditor
      value={value}
      onChange={(newValue) => {
        // 验证 TOML 有效性
        try {
          if (newValue.trim()) {
            parseToml(newValue);
          }
          tomlValidityRef.current = true;
        } catch {
          tomlValidityRef.current = false;
        }
        
        // 始终调用 onChange，保持编辑器内容
        if (onChange) {
          onChange(newValue);
        }
      }}
      height={220}
      placeholder={placeholder}
    />
  );
};

// 验证 TOML 有效性的规则（仅在提交时验证）
const validateTomlRule = (message: string) => ({
  validator: () => {
    if (!tomlValidityRef.current) {
      return Promise.reject(new Error(message));
    }
    return Promise.resolve();
  },
});

// OpenCode provider display type
interface OpenCodeProviderDisplay {
  id: string;
  name: string;
  baseUrl: string | undefined;
  apiKey?: string;
  models: { id: string; name: string }[];
}

interface CodexProviderFormModalProps {
  open: boolean;
  provider?: CodexProvider | null;
  isCopy?: boolean;
  mode?: 'manual' | 'import';
  onCancel: () => void;
  onSubmit: (values: CodexProviderFormValues) => Promise<void>;
}

const CodexProviderFormModal: React.FC<CodexProviderFormModalProps> = ({
  open,
  provider,
  isCopy = false,
  mode = 'manual',
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const language = useAppStore((state) => state.language);
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);

  const labelCol = { span: language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };
  const sectionWrapperCol = { span: 24 };
  const notesCollapseResetKey = `${open ? 'open' : 'closed'}:${mode}:${provider?.id ?? 'new'}:${isCopy ? 'copy' : 'normal'}`;

  // OpenCode import related state
  const [openCodeProviders, setOpenCodeProviders] = React.useState<OpenCodeProviderDisplay[]>([]);
  const [selectedProvider, setSelectedProvider] = React.useState<OpenCodeProviderDisplay | null>(null);
  const [availableModels, setAvailableModels] = React.useState<{ id: string; name: string }[]>([]);
  const [loadingProviders, setLoadingProviders] = React.useState(false);
  const [processedBaseUrl, setProcessedBaseUrl] = React.useState<string>('');
  const [fetchedModels, setFetchedModels] = React.useState<FetchedModel[]>([]);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [modelMappingExpanded, setModelMappingExpanded] = React.useState(false);
  // 当前表单的 baseUrl（用于匹配供应商）
  const [currentBaseUrl, setCurrentBaseUrl] = React.useState<string>('');
  const [billingConfig, setBillingConfig] = React.useState(() => getBillingConfigFromMeta(provider?.meta));
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const apiFormatOptions = React.useMemo(() => [
    {
      value: 'openai_responses',
      label: t('codex.provider.apiFormatOpenAIResponses'),
    },
    {
      value: 'openai_chat',
      label: t('codex.provider.apiFormatOpenAIChat'),
    },
    {
      value: 'anthropic_messages',
      label: t('codex.provider.apiFormatAnthropicMessages'),
    },
    {
      value: 'gemini_native',
      label: t('codex.provider.apiFormatGeminiNative'),
    },
  ], [t]);

  const isEdit = !!provider && !isCopy;
  const canSelectProviderCategory = !provider && mode === 'manual';

  // 使用新的配置状态管理 Hook
  const {
    codexApiKey,
    codexBaseUrl,
    codexModel,
    codexConfig,
    codexCatalogModels,
    providerCategory,
    handleApiKeyChange,
    handleBaseUrlChange,
    handleModelChange,
    handleConfigChange,
    handleProviderCategoryChange,
    setCodexCatalogModels,
    resetFromSettingsConfig,
    getFinalSettingsConfig,
  } = useCodexConfigState({
    initialData: provider ? { settingsConfig: provider.settingsConfig } : undefined,
  });
  const lockedProviderCategory = provider?.category === 'official' ? 'official' : 'custom';
  const activeProviderCategory = canSelectProviderCategory ? providerCategory : lockedProviderCategory;
  const isOfficialMode = activeProviderCategory === 'official';
  const watchOptions = React.useMemo(() => ({ form, preserve: true }), [form]);
  const selectedApiFormat = Form.useWatch('apiFormat', watchOptions) as CodexApiFormat | undefined;
  const selectedProviderProfileId = Form.useWatch('providerProfileId', watchOptions) as string | undefined;
  const selectedIsCustomProviderProfile = (selectedProviderProfileId || CUSTOM_PROVIDER_PROFILE_ID) === CUSTOM_PROVIDER_PROFILE_ID;

  const providerEndpointOptions = React.useMemo(() => {
    if (isOfficialMode && !canSelectProviderCategory) {
      return [{
        value: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        label: t('codex.provider.providerProfileOfficial'),
      }];
    }

    return [
      {
        value: CUSTOM_PROVIDER_ENDPOINT_KEY,
        label: t('codex.provider.providerProfileCustom'),
      },
      ...(canSelectProviderCategory ? [{
        value: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        label: t('codex.provider.providerProfileOfficial'),
      }] : []),
      ...getGatewayProviderProfilesForTool('codex').flatMap((profile) => {
        const endpoints = profile.tools.codex?.endpoints || [];
        return endpoints.map((endpoint) => ({
          value: toGatewayProviderEndpointKey(profile.id, endpoint.id),
          label: `${profile.label} / ${endpoint.label}`,
        }));
      }),
    ];
  }, [canSelectProviderCategory, gatewayProviderProfilesVersion, isOfficialMode, t]);

  const providerHasModelMapping = React.useCallback((settingsConfig?: string) => {
    if (!settingsConfig) {
      return false;
    }
    try {
      const parsed = JSON.parse(settingsConfig);
      const models = parsed?.modelCatalog?.models;
      return Array.isArray(models) && models.some((item) => typeof item?.model === 'string' && item.model.trim());
    } catch {
      return false;
    }
  }, []);

  // Load OpenCode providers list when import tab is active or in edit mode
  React.useEffect(() => {
    if (mode === 'import' || isEdit) {
      loadOpenCodeProviders();
    }
  }, [mode, isEdit]);

  // 设置 currentBaseUrl
  React.useEffect(() => {
    if (isEdit && selectedIsCustomProviderProfile && codexBaseUrl) {
      setCurrentBaseUrl(codexBaseUrl);
    }
  }, [isEdit, selectedIsCustomProviderProfile, codexBaseUrl]);

  const formInitializedRef = React.useRef(false);
  React.useEffect(() => {
    if (!open) {
      formInitializedRef.current = false;
      return;
    }

    resetFromSettingsConfig(provider?.settingsConfig);
    if (provider) {
      handleProviderCategoryChange(lockedProviderCategory);
    }
    setBillingConfig(getBillingConfigFromMeta(provider?.meta));

    if (provider) {
      let settingsConfig: CodexSettingsConfig = {};
      try {
        settingsConfig = JSON.parse(provider.settingsConfig || '{}') as CodexSettingsConfig;
      } catch {
        settingsConfig = {};
      }
      const baseUrl = extractCodexBaseUrl(settingsConfig.config) || '';
      const providerEndpointSelection = lockedProviderCategory === 'official'
        ? {
            providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
            providerEndpointId: undefined,
          }
        : inferGatewayProviderEndpointSelection({
            tool: 'codex',
            providerType: provider.meta?.providerType,
            apiFormat: provider.meta?.apiFormat,
            baseUrl,
          });
      const providerEndpoint = providerEndpointSelection.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            providerEndpointSelection.providerProfileId,
            'codex',
            providerEndpointSelection.providerEndpointId,
          );
      form.setFieldsValue({
        category: provider.category,
        name: provider.name,
        providerEndpointKey: lockedProviderCategory === 'official'
          ? OFFICIAL_PROVIDER_ENDPOINT_KEY
          : toGatewayProviderEndpointKey(
              providerEndpointSelection.providerProfileId,
              providerEndpointSelection.providerEndpointId,
            ),
        providerProfileId: providerEndpointSelection.providerProfileId,
        providerEndpointId: providerEndpointSelection.providerEndpointId,
        baseUrl: baseUrl || providerEndpoint?.baseUrl || '',
        model: extractCodexModel(settingsConfig.config) || providerEndpoint?.model || '',
        apiFormat: providerEndpoint
          ? normalizeCodexApiFormat(providerEndpoint.apiFormat)
          : normalizeCodexApiFormat(provider.meta?.apiFormat),
        notes: provider.notes || '',
      });
      setCurrentBaseUrl(baseUrl || providerEndpoint?.baseUrl || '');
    } else {
      form.resetFields();
      form.setFieldsValue({
        category: 'custom',
        name: undefined,
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiKey: '',
        baseUrl: '',
        model: '',
        apiFormat: DEFAULT_CODEX_API_FORMAT,
        configToml: '',
        notes: '',
        sourceProvider: undefined,
      });
    }

    setSelectedProvider(null);
    setAvailableModels([]);
    setFetchedModels([]);
    setModelMappingExpanded(providerHasModelMapping(provider?.settingsConfig));
    setProcessedBaseUrl('');
    if (!provider) {
      setCurrentBaseUrl('');
    }
    formInitializedRef.current = true;
  }, [form, handleProviderCategoryChange, lockedProviderCategory, open, provider, providerHasModelMapping, resetFromSettingsConfig]);

  React.useEffect(() => {
    if (!open || !formInitializedRef.current) {
      return;
    }

    const currentEndpoint = findGatewayProviderEndpoint(
      form.getFieldValue('providerProfileId'),
      'codex',
      form.getFieldValue('providerEndpointId'),
    );
    const currentEndpointModel = currentEndpoint
      ? getCodexEndpointCatalogModels(form.getFieldValue('providerProfileId'), currentEndpoint)[0]?.model
        || currentEndpoint.model
      : undefined;
    const nextFieldValues = provider
      ? {
          name: provider.name,
          apiKey: codexApiKey,
          baseUrl: codexBaseUrl || currentEndpoint?.baseUrl || '',
          model: codexModel || currentEndpointModel || '',
          apiFormat: currentEndpoint
            ? normalizeCodexApiFormat(currentEndpoint.apiFormat)
            : normalizeCodexApiFormat(provider.meta?.apiFormat),
          configToml: codexConfig,
          notes: provider.notes || '',
        }
      : {
          apiKey: codexApiKey,
          baseUrl: codexBaseUrl,
          model: codexModel,
          apiFormat: form.getFieldValue('apiFormat') || DEFAULT_CODEX_API_FORMAT,
          configToml: codexConfig,
        };

    form.setFieldsValue(nextFieldValues);
  }, [
    codexApiKey,
    codexBaseUrl,
    codexConfig,
    codexModel,
    form,
    gatewayProviderProfilesVersion,
    open,
    provider,
  ]);

  React.useEffect(() => {
    if (!open || mode !== 'manual' || isOfficialMode) {
      return;
    }
    if (normalizeCodexApiFormat(selectedApiFormat) !== DEFAULT_CODEX_API_FORMAT) {
      setModelMappingExpanded(true);
    }
  }, [isOfficialMode, mode, open, selectedApiFormat]);

  // 同步 Hook 的 codexConfig 到 Form 的 configToml 字段
  // 当用户在 baseUrl 或 model 输入框输入时，需要实时更新 TOML 编辑器
  const prevCodexConfigRef = React.useRef(codexConfig);
  React.useEffect(() => {
    // 只在表单已初始化且 codexConfig 变化时同步
    if (!formInitializedRef.current) return;
    if (prevCodexConfigRef.current === codexConfig) return;
    
    prevCodexConfigRef.current = codexConfig;
    
    // 获取当前表单的 configToml 值
    const currentFormConfig = form.getFieldValue('configToml') || '';
    
    // 只有当 Hook 的值与 Form 的值不同时才更新，避免不必要的更新
    if (currentFormConfig !== codexConfig) {
      form.setFieldsValue({ configToml: codexConfig });
    }
  }, [codexConfig, form]);

  const loadOpenCodeProviders = async () => {
    setLoadingProviders(true);
    try {
      const providers = await readCurrentOpenCodeProviders();

      // 直接读取 OpenCode 当前配置，避免把“我使用过的供应商”历史库当作当前导入源。
      const openaiProviders: OpenCodeProviderDisplay[] = Object.entries(providers)
        .filter(([, providerConfig]) => providerConfig.npm === '@ai-sdk/openai')
        .map(([providerId, providerConfig]) => {
          const models = Object.entries(providerConfig.models || {}).map(([modelId, model]) => ({
            id: modelId,
            name: model.name || modelId,
          }));

          return {
            id: providerId,
            name: providerConfig.name || providerId,
            baseUrl: providerConfig.options?.baseURL,
            apiKey: providerConfig.options?.apiKey,
            models,
          };
        });

      setOpenCodeProviders(openaiProviders);
    } catch (error) {
      console.error('Failed to load OpenCode providers:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setLoadingProviders(false);
    }
  };

  const handleProviderSelect = (providerId: string) => {
    const providerData = openCodeProviders.find((p) => p.id === providerId);
    if (!providerData) return;

    setSelectedProvider(providerData);
    setAvailableModels(providerData.models);

    // Process baseUrl: only remove trailing /
    let processedUrl = providerData.baseUrl || '';
    if (processedUrl.endsWith('/')) {
      processedUrl = processedUrl.slice(0, -1);
    }
    setProcessedBaseUrl(processedUrl);
    setCurrentBaseUrl(processedUrl);

    // Update Hook state
    handleApiKeyChange(providerData.apiKey || '');
    handleBaseUrlChange(processedUrl);

    // Auto-fill form
    form.setFieldsValue({
      name: providerData.name,
      providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
      providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
      providerEndpointId: undefined,
      baseUrl: processedUrl,
      apiKey: providerData.apiKey || '',
      apiFormat: 'openai_chat',
    });
  };

  const handleProviderEndpointChange = (selectionKey: string) => {
    if (selectionKey === OFFICIAL_PROVIDER_ENDPOINT_KEY) {
      if (!canSelectProviderCategory) {
        return;
      }
      handleProviderCategoryChange('official');
      setFetchedModels([]);
      setCurrentBaseUrl('');
      form.setFieldsValue({
        category: 'official',
        apiKey: undefined,
        baseUrl: undefined,
        providerEndpointKey: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiFormat: DEFAULT_CODEX_API_FORMAT,
      });
      return;
    }

    const { providerProfileId, providerEndpointId } = parseGatewayProviderEndpointKey(selectionKey);
    if (canSelectProviderCategory && activeProviderCategory !== 'custom') {
      handleProviderCategoryChange('custom');
      form.setFieldsValue({ category: 'custom' });
    }

    if (providerProfileId === CUSTOM_PROVIDER_PROFILE_ID) {
      form.setFieldsValue({
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId,
        providerEndpointId: undefined,
        apiFormat: form.getFieldValue('apiFormat') || DEFAULT_CODEX_API_FORMAT,
      });
      return;
    }

    const endpoint = findGatewayProviderEndpoint(providerProfileId, 'codex', providerEndpointId);
    if (!endpoint) {
      return;
    }

    const catalogModels = getCodexEndpointCatalogModels(providerProfileId, endpoint);
    const nextModel = catalogModels[0]?.model || endpoint.model || form.getFieldValue('model');

    form.setFieldsValue({
      providerEndpointKey: toGatewayProviderEndpointKey(providerProfileId, endpoint.id),
      providerProfileId,
      providerEndpointId: endpoint.id,
      apiFormat: normalizeCodexApiFormat(endpoint.apiFormat),
      baseUrl: endpoint.baseUrl,
      model: nextModel,
    });
    handleBaseUrlChange(endpoint.baseUrl);
    setCurrentBaseUrl(endpoint.baseUrl);

    if (nextModel) {
      handleModelChange(nextModel);
    }

    if (catalogModels.length > 0) {
      setCodexCatalogModels(catalogModels);
      setModelMappingExpanded(true);
    } else {
      setCodexCatalogModels([]);
      setModelMappingExpanded(false);
    }
  };

  const handleSubmit = async () => {
    try {
      const fieldsToValidate = mode === 'import'
        ? ['sourceProvider', 'name', 'apiKey', 'apiFormat', 'configToml', 'notes']
        : [...(canSelectProviderCategory ? ['category'] : []), 'name', ...(!isOfficialMode ? ['providerEndpointKey', 'apiKey', 'baseUrl', 'apiFormat'] : []), 'configToml', 'notes'];

      // 强制触发一次同步，确保所有字段都已同步到最终 settingsConfig
      const currentValues = form.getFieldsValue();
      if (currentValues.apiKey !== undefined) {
        handleApiKeyChange(currentValues.apiKey || '');
      }
      if (currentValues.baseUrl !== undefined) {
        handleBaseUrlChange(currentValues.baseUrl || '');
      }
      if (currentValues.model !== undefined) {
        handleModelChange(currentValues.model || '');
      }

      const values = await form.validateFields(fieldsToValidate);
      const submittedValues = {
        ...(form.getFieldsValue(true) as CodexProviderFormValues),
        ...values,
      };

      setLoading(true);

      // 从表单获取最新的 config.toml 值（同步后表单中的值是最新的）
      const latestConfigToml = (form.getFieldValue('configToml') as string) || '';
      // 使用 Hook 提供的最终配置（已合并字段），但 config 使用表单最新值
      const settingsConfig = getFinalSettingsConfig(latestConfigToml);
      const selectedCategory = mode === 'import'
        ? 'custom'
        : (activeProviderCategory === 'official' ? 'official' : 'custom');
      const selectedEndpoint = selectedCategory === 'official' || submittedValues.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            submittedValues.providerProfileId,
            'codex',
            submittedValues.providerEndpointId,
          );
      const selectedProfile = selectedEndpoint
        ? findGatewayProviderProfile(submittedValues.providerProfileId)
        : undefined;
      const selectedApiFormat = selectedCategory === 'official'
        ? undefined
        : selectedEndpoint
          ? normalizeCodexApiFormat(selectedEndpoint.apiFormat)
          : normalizeCodexApiFormat(submittedValues.apiFormat);
      const finalSettingsConfig = selectedCategory === 'official'
        ? settingsConfig
        : applyEndpointToCodexSettingsConfig(
            settingsConfig,
            submittedValues.providerProfileId,
            selectedEndpoint,
            submittedValues.model,
          );

      const formValues: CodexProviderFormValues = {
        name: submittedValues.name,
        category: selectedCategory,
        providerEndpointKey: selectedEndpoint
          ? toGatewayProviderEndpointKey(selectedProfile?.id || submittedValues.providerProfileId || '', selectedEndpoint.id)
          : CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: selectedEndpoint
          ? selectedProfile?.id
          : CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: selectedEndpoint?.id,
        settingsConfig: finalSettingsConfig,
        apiFormat: selectedApiFormat,
        meta: mergeBillingConfigIntoMeta(
          mergeGatewayMetaIntoProviderMeta(
            provider?.meta,
            selectedApiFormat,
            selectedCategory === 'official' ? undefined : selectedProfile?.providerType,
          ),
          selectedCategory === 'official'
            ? { enabled: false, pricingModelSource: 'inherit' }
            : billingConfig,
        ),
        notes: submittedValues.notes,
        sourceProviderId: mode === 'import' ? selectedProvider?.id : undefined,
      };

      await onSubmit(formValues);
      form.resetFields();
      setSelectedProvider(null);
      setAvailableModels([]);
      onCancel();
    } catch (error) {
      console.error('Form validation failed:', error);
    } finally {
      setLoading(false);
    }
  };

  const modelSelectOptions = availableModels.map((model) => ({
    label: `${model.name} (${model.id})`,
    value: model.id,
  }));

  const handleFetchModels = async () => {
    if (isOfficialMode) {
      setLoadingModels(true);
      try {
        const response = await fetchCodexOfficialModels();
        const models = response.models.length > 0 ? response.models : CODEX_OFFICIAL_FALLBACK_MODELS;
        setFetchedModels(models);

        if (response.source === 'bundled') {
          message.info(t('codex.fetchModels.officialBundled', { count: models.length }));
        } else {
          message.success(t('codex.fetchModels.officialUpdated', { count: models.length }));
        }
      } catch (error) {
        console.error('Failed to fetch Codex official models:', error);
        setFetchedModels(CODEX_OFFICIAL_FALLBACK_MODELS);
        message.info(t('codex.fetchModels.officialBundled', { count: CODEX_OFFICIAL_FALLBACK_MODELS.length }));
      } finally {
        setLoadingModels(false);
      }
      return;
    }

    const baseUrl = (form.getFieldValue('baseUrl') as string | undefined)?.trim();
    const apiKey = (form.getFieldValue('apiKey') as string | undefined)?.trim();

    if (!baseUrl) {
      message.warning(t('codex.fetchModels.baseUrlRequired'));
      return;
    }

    setLoadingModels(true);
    try {
      const response = await invoke<FetchModelsResponse>('fetch_provider_models', {
        request: {
          baseUrl,
          apiKey: apiKey || undefined,
          apiType: 'openai_compat',
          sdkType: '@ai-sdk/openai',
        },
      });

      setFetchedModels(response.models);
      if (response.models.length > 0) {
        message.success(t('codex.fetchModels.success', { count: response.models.length }));
      } else {
        message.info(t('codex.fetchModels.noModels'));
      }
    } catch (error) {
      console.error('Failed to fetch Codex models:', error);
      message.error(t('codex.fetchModels.failed'));
    } finally {
      setLoadingModels(false);
    }
  };

  const handleAddModelMapping = React.useCallback(() => {
    setModelMappingExpanded(true);
    setCodexCatalogModels((prev) => [
      ...prev,
      {
        model: '',
        displayName: '',
        contextWindow: '',
      },
    ]);
  }, [setCodexCatalogModels]);

  const handleUpdateModelMapping = React.useCallback((
    index: number,
    patch: Partial<CodexCatalogModel>,
  ) => {
    setCodexCatalogModels((prev) => prev.map((item, itemIndex) => (
      itemIndex === index ? { ...item, ...patch } : item
    )));
  }, [setCodexCatalogModels]);

  const handleRemoveModelMapping = React.useCallback((index: number) => {
    setCodexCatalogModels((prev) => prev.filter((_, itemIndex) => itemIndex !== index));
  }, [setCodexCatalogModels]);

  // 根据 baseUrl 匹配供应商的模型列表
  // OpenCode 的 URL 可能包含 /v1，所以用包含匹配
  const matchedProviderModels = React.useMemo(() => {
    if (!currentBaseUrl || openCodeProviders.length === 0) {
      return [];
    }

    // 标准化 URL：去掉末尾的 /
    const normalizeUrl = (url: string) => {
      return url.replace(/\/$/, '').toLowerCase();
    };

    const normalizedCurrentUrl = normalizeUrl(currentBaseUrl);

    // 查找匹配的供应商
    const matchedProvider = openCodeProviders.find((p) => {
      if (!p.baseUrl) return false;
      const normalizedProviderUrl = normalizeUrl(p.baseUrl);
      // OpenCode 的 URL 包含 Codex 的 URL，或者反过来
      return normalizedProviderUrl.includes(normalizedCurrentUrl) ||
             normalizedCurrentUrl.includes(normalizedProviderUrl);
    });

    return matchedProvider?.models || [];
  }, [currentBaseUrl, openCodeProviders]);

  // 计算 AutoComplete 选项
  // 优先保留 OpenCode 当前配置里的友好显示名，再补充主动拉取到的额外模型。
  const modelOptions = React.useMemo(() => {
    const options: { label: string; value: string }[] = [];
    const seenIds = new Set<string>();

    matchedProviderModels.forEach((model) => {
      if (!seenIds.has(model.id)) {
        seenIds.add(model.id);
        options.push({
          label: model.name && model.name !== model.id ? `${model.name} (${model.id})` : model.id,
          value: model.id,
        });
      }
    });

    fetchedModels.forEach((model) => {
      if (!seenIds.has(model.id)) {
        seenIds.add(model.id);
        const displayName = model.name || model.id;
        options.push({
          label: displayName && displayName !== model.id ? `${displayName} (${model.id})` : model.id,
          value: model.id,
        });
      }
    });

    return options;
  }, [fetchedModels, matchedProviderModels]);

  const renderManualTab = () => (
    <Form
      form={form}
      layout="horizontal"
      labelCol={labelCol}
      wrapperCol={wrapperCol}
      onValuesChange={(changedValues) => {
        // 当表单值变化时，同步到 Hook 状态
        if ('apiKey' in changedValues) {
          handleApiKeyChange(changedValues.apiKey || '');
        }
        if ('baseUrl' in changedValues) {
          handleBaseUrlChange(changedValues.baseUrl || '');
          setCurrentBaseUrl(changedValues.baseUrl || '');
        }
        if ('model' in changedValues) {
          handleModelChange(changedValues.model || '');
        }
        if ('configToml' in changedValues) {
          handleConfigChange(changedValues.configToml || '');
        }
      }}
    >
      <Form.Item
        label={t('codex.provider.providerProfile')}
        required
        help={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.provider.providerProfileHelp')}</Text>}
      >
        <div className={isOfficialMode ? undefined : styles.providerProfileRow}>
          <Form.Item
            name="providerEndpointKey"
            noStyle
            initialValue={CUSTOM_PROVIDER_ENDPOINT_KEY}
            rules={[{ required: true, message: t('common.error') }]}
          >
            <Select
              options={providerEndpointOptions}
              disabled={isOfficialMode && !canSelectProviderCategory}
              onChange={handleProviderEndpointChange}
            />
          </Form.Item>
          {!isOfficialMode && (
            <Form.Item
              name="apiFormat"
              noStyle
              initialValue={DEFAULT_CODEX_API_FORMAT}
            >
              <Select
                options={apiFormatOptions}
                disabled={!selectedIsCustomProviderProfile}
              />
            </Form.Item>
          )}
        </div>
      </Form.Item>
      <Form.Item name="providerProfileId" hidden initialValue={CUSTOM_PROVIDER_PROFILE_ID}>
        <Input />
      </Form.Item>
      <Form.Item name="providerEndpointId" hidden>
        <Input />
      </Form.Item>

      <Form.Item
        name="name"
        label={t('codex.provider.formName')}
        rules={[{ required: true, message: t('common.error') }]}
      >
        <Input placeholder={t('codex.provider.namePlaceholder')} />
      </Form.Item>

      {!isOfficialMode && (
        <>
          <Form.Item
            name="apiKey"
            label={t('codex.provider.apiKey')}
            rules={[{ required: true, message: t('common.error') }]}
          >
            <Input
              type={showApiKey ? 'text' : 'password'}
              placeholder={t('codex.provider.apiKeyPlaceholder')}
              addonAfter={
                <Button
                  type="text"
                  size="small"
                  icon={showApiKey ? <EyeInvisibleOutlined /> : <EyeOutlined />}
                  onClick={() => setShowApiKey(!showApiKey)}
                >
                  {showApiKey ? t('codex.provider.hideApiKey') : t('codex.provider.showApiKey')}
                </Button>
              }
            />
          </Form.Item>

          <Form.Item
            name="baseUrl"
            label={t('codex.provider.baseUrl')}
            rules={[{ required: true, message: t('common.error') }]}
            help={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.provider.baseUrlHelp')}</Text>}
          >
            <Input
              placeholder="https://your-api-endpoint.com/v1"
            />
          </Form.Item>
        </>
      )}

      <Form.Item
        label={t('codex.provider.modelName')}
        help={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.provider.modelNameHelp')}</Text>}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%' }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <Form.Item name="model" noStyle>
              <AutoComplete
                options={modelOptions}
                placeholder={t('codex.provider.modelNamePlaceholder')}
                style={{ width: '100%' }}
                filterOption={(inputValue, option) =>
                  (option?.label?.toString().toLowerCase().includes(inputValue.toLowerCase()) ||
                  option?.value?.toString().toLowerCase().includes(inputValue.toLowerCase())) ?? false
                }
              />
            </Form.Item>
          </div>
          <Button
            icon={<CloudDownloadOutlined />}
            loading={loadingModels}
            onClick={handleFetchModels}
          >
            {t('codex.fetchModels.button')}
          </Button>
          {!isOfficialMode && (
            <Button
              icon={modelMappingExpanded ? <DownOutlined /> : <RightOutlined />}
              onClick={() => setModelMappingExpanded((prev) => !prev)}
            >
              {t('codex.provider.modelMapping')}
            </Button>
          )}
          {fetchedModels.length > 0 && (
            <Text type="secondary" style={{ whiteSpace: 'nowrap' }}>
              {t('codex.fetchModels.loaded', { count: fetchedModels.length })}
            </Text>
          )}
        </div>
        {!isOfficialMode && modelMappingExpanded && (
          <div
            style={{
              marginTop: 12,
              border: '1px solid var(--color-border)',
              borderRadius: 8,
              background: 'var(--color-bg-elevated)',
              padding: 12,
            }}
          >
            <Space direction="vertical" size={10} style={{ width: '100%' }}>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t('codex.provider.modelMappingHint')}
              </Text>
              {codexCatalogModels.map((item, index) => (
                <div
                  key={index}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: 'minmax(120px, 1fr) minmax(160px, 1.2fr) 120px 32px',
                    gap: 8,
                    alignItems: 'center',
                  }}
                >
                  <Input
                    value={item.displayName ?? ''}
                    placeholder={t('codex.provider.modelMappingDisplayNamePlaceholder')}
                    aria-label={t('codex.provider.modelMappingDisplayName')}
                    onChange={(event) => handleUpdateModelMapping(index, { displayName: event.target.value })}
                  />
                  <AutoComplete
                    value={item.model}
                    options={modelOptions}
                    placeholder={t('codex.provider.modelMappingModelPlaceholder')}
                    aria-label={t('codex.provider.modelMappingModel')}
                    filterOption={(inputValue, option) =>
                      (option?.label?.toString().toLowerCase().includes(inputValue.toLowerCase()) ||
                      option?.value?.toString().toLowerCase().includes(inputValue.toLowerCase())) ?? false
                    }
                    onChange={(value) => handleUpdateModelMapping(index, { model: value })}
                  />
                  <Input
                    value={item.contextWindow ?? ''}
                    inputMode="numeric"
                    placeholder={t('codex.provider.modelMappingContextWindowPlaceholder')}
                    aria-label={t('codex.provider.modelMappingContextWindow')}
                    onChange={(event) => handleUpdateModelMapping(index, {
                      contextWindow: event.target.value.replace(/[^\d]/g, ''),
                    })}
                  />
                  <Button
                    type="text"
                    danger
                    icon={<DeleteOutlined />}
                    aria-label={t('codex.provider.modelMappingRemove')}
                    onClick={() => handleRemoveModelMapping(index)}
                  />
                </div>
              ))}
              <Button
                type="dashed"
                icon={<PlusOutlined />}
                onClick={handleAddModelMapping}
              >
                {t('codex.provider.modelMappingAdd')}
              </Button>
            </Space>
          </div>
        )}
      </Form.Item>

      <Form.Item 
        name="configToml" 
        label="config.toml"
        extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.provider.configTomlHelp')}</Text>}
        rules={[validateTomlRule(t('codex.provider.configTomlInvalid'))]}
      >
        <TomlEditorFormItem 
          placeholder={t('codex.provider.configTomlPlaceholder')}
        />
      </Form.Item>

      {!isOfficialMode && (
        <Form.Item wrapperCol={sectionWrapperCol}>
          <BillingConfigCollapse
            value={billingConfig}
            onChange={setBillingConfig}
          />
        </Form.Item>
      )}

      <Form.Item name="notes" wrapperCol={sectionWrapperCol}>
        <ProviderNotesCollapse
          title={t('codex.provider.notes')}
          placeholder={t('codex.provider.notesPlaceholder')}
          rows={2}
          resetKey={notesCollapseResetKey}
        />
      </Form.Item>
    </Form>
  );

  const renderImportTab = () => (
    <div>
      <Form
        form={form}
        layout="horizontal"
        labelCol={labelCol}
        wrapperCol={wrapperCol}
        onValuesChange={(changedValues) => {
          // 当表单值变化时，同步到 Hook 状态
          if ('apiKey' in changedValues) {
            handleApiKeyChange(changedValues.apiKey || '');
          }
          if ('baseUrl' in changedValues) {
            handleBaseUrlChange(changedValues.baseUrl || '');
          }
          if ('model' in changedValues) {
            handleModelChange(changedValues.model || '');
          }
          if ('configToml' in changedValues) {
            handleConfigChange(changedValues.configToml || '');
          }
        }}
      >
        <Form.Item
          name="sourceProvider"
          label={t('codex.import.selectProvider')}
          rules={[{ required: true, message: t('common.error') }]}
        >
          <Select
            placeholder={t('codex.import.selectProviderPlaceholder')}
            loading={loadingProviders}
            onChange={handleProviderSelect}
            options={openCodeProviders.map((p) => ({
              label: `${p.name} (${p.baseUrl || ''})`,
              value: p.id,
            }))}
          />
        </Form.Item>

        {selectedProvider && (
          <Alert
            message={t('codex.import.importInfo')}
            description={
              <Space direction="vertical" size={4}>
                <div>{t('codex.import.providerName')}: {selectedProvider.name}</div>
                <div>{t('codex.import.baseUrl')}: {processedBaseUrl}</div>
                <div>{t('codex.import.availableModels')}: {availableModels.length > 0 ? t('codex.import.modelsCount', { count: availableModels.length }) : '-'}</div>
              </Space>
            }
            type="success"
            showIcon
            style={{ marginBottom: 16 }}
          />
        )}

        <Form.Item name="name" label={t('codex.provider.formName')}>
          <Input placeholder={t('codex.provider.namePlaceholder')} disabled />
        </Form.Item>

        <Form.Item name="apiKey" label={t('codex.provider.apiKey')}>
          <Input type="password" disabled />
        </Form.Item>

        <Form.Item name="apiFormat" label={t('codex.provider.apiFormat')} initialValue="openai_chat">
          <Select options={apiFormatOptions} disabled />
        </Form.Item>

        {availableModels.length > 0 && (
          <>
            <Alert
              message={t('codex.model.selectFromProvider')}
              type="info"
              showIcon
              style={{ marginBottom: 16 }}
            />

            <Form.Item name="model" label={t('codex.import.selectDefaultModel')}>
              <Select
                placeholder={t('codex.model.defaultModelPlaceholder')}
                options={modelSelectOptions}
                allowClear
                showSearch
              />
            </Form.Item>
          </>
        )}

        <Form.Item 
          name="configToml" 
          label="config.toml"
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.provider.configTomlHelp')}</Text>}
          rules={[validateTomlRule(t('codex.provider.configTomlInvalid'))]}
        >
          <TomlEditorFormItem 
            placeholder={t('codex.provider.configTomlPlaceholder')}
          />
        </Form.Item>

        <Form.Item name="notes" wrapperCol={sectionWrapperCol}>
          <ProviderNotesCollapse
            title={t('codex.provider.notes')}
            placeholder={t('codex.provider.notesPlaceholder')}
            rows={2}
            resetKey={notesCollapseResetKey}
          />
        </Form.Item>
      </Form>
    </div>
  );

  return (
    <Modal
      title={
        isEdit
          ? t('codex.provider.editProvider')
          : mode === 'import'
            ? t('codex.import.title')
            : t('codex.provider.addProvider')
      }
      open={open}
      onCancel={onCancel}
      onOk={handleSubmit}
      confirmLoading={loading}
      width={800}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      {isEdit || mode === 'manual' ? renderManualTab() : renderImportTab()}
    </Modal>
  );
};

export default CodexProviderFormModal;
