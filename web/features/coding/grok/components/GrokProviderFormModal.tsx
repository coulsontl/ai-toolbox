import React from 'react';
import { Modal, Form, Input, Select, Space, Button, Alert, message, Typography } from 'antd';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';
import ImeSafeInput from '@/components/common/ImeSafeInput';
import {
  CloudDownloadOutlined,
  EyeInvisibleOutlined,
  EyeOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '@/stores';
import type { GrokApiFormat, GrokCatalogModel, GrokProvider, GrokProviderFormValues, GrokSettingsConfig, GatewayProviderMeta } from '@/types/grok';
import { fetchGrokOfficialModels } from '@/services/grokApi';
import { readCurrentOpenCodeProviders } from '@/services/opencodeApi';
import type { FetchedModel, FetchModelsResponse } from '@/components/common/FetchModelsModal/types';
import BillingConfigCollapse from '@/features/coding/shared/providerBilling/BillingConfigCollapse';
import CustomHeadersCollapse from '@/features/coding/shared/providerHeaders/CustomHeadersCollapse';
import ProviderConfigCollapse from '@/features/coding/shared/providerConfig/ProviderConfigCollapse';
import ProviderNotesCollapse from '@/features/coding/shared/providerConfig/ProviderNotesCollapse';
import { FileCode2 } from 'lucide-react';
import {
  getBillingConfigFromMeta,
  mergeBillingConfigIntoMeta,
} from '@/features/coding/shared/providerBilling/billingConfigUtils';
import {
  getCustomHeadersFromMeta,
  getEffectiveCustomHeaders,
  mergeCustomHeadersIntoMeta,
} from '@/features/coding/shared/providerHeaders/customHeadersUtils';
import {
  getModelRewritesFromMeta,
  mergeModelRewritesIntoMeta,
  type ModelRewritesState,
} from '@/features/coding/shared/providerModelRewrites/modelRewritesUtils';
import ModelRewritesCollapse from '@/features/coding/shared/providerModelRewrites/ModelRewritesCollapse';
import { useGatewaySupportedCliKeys } from '@/features/coding/shared/gateway/useGatewaySupportedCliKeys';
import {
  CUSTOM_PROVIDER_ENDPOINT_KEY,
  CUSTOM_PROVIDER_PROFILE_ID,
  findGatewayProviderEndpoint,
  findGatewayProviderProfile,
  getGatewayProviderProfilesForTool,
  getGatewayProviderProfilesVersion,
  inferGatewayProviderEndpointSelection,
  mergeGatewayProfileReferenceIntoMeta,
  parseGatewayProviderEndpointKey,
  subscribeGatewayProviderProfiles,
  toGatewayProviderEndpointKey,
  toGatewayProviderProfileReference,
  type GatewayProviderEndpointProfile,
  type GatewayProviderProfileReference,
} from '@/features/coding/shared/gateway/providerProfiles';
import {
  extractGrokSettingsApiBackend,
  extractGrokSettingsBaseUrl,
  extractGrokSettingsModel,
} from '@/utils/grokConfigUtils';
import TomlEditor from '@/components/common/TomlEditor';
import { parse as parseToml } from 'smol-toml';
import { useGrokConfigState } from '../hooks/useGrokConfigState';
import {
  applyGrokEndpointSettingsConfig,
  DEFAULT_GROK_MODEL,
  GROK_REASONING_EFFORT_OPTIONS,
} from '../utils/grokSettingsConfig';
import styles from './GrokProviderFormModal.module.less';

const { Text } = Typography;

const GROK_OFFICIAL_FALLBACK_MODELS: FetchedModel[] = [
  { id: DEFAULT_GROK_MODEL, name: 'Grok 4.5' },
  { id: 'grok-build', name: 'Grok Build' },
].map((model) => ({
  ...model,
  ownedBy: 'xai',
  created: undefined,
}));

const DEFAULT_GROK_API_FORMAT: GrokApiFormat = 'openai_chat';
const OFFICIAL_PROVIDER_ENDPOINT_KEY = '__official__:';

function normalizeGrokApiFormat(value?: string): GrokApiFormat {
  if (
    value === 'openai_chat'
    || value === 'openai_responses'
    || value === 'anthropic_messages'
  ) {
    return value;
  }
  return DEFAULT_GROK_API_FORMAT;
}

function mapGrokApiBackendToApiFormat(apiBackend?: string): GrokApiFormat | undefined {
  const normalized = apiBackend?.trim().toLowerCase();
  if (!normalized) {
    return undefined;
  }
  if (normalized === 'responses' || normalized === 'openai_responses') {
    return 'openai_responses';
  }
  if (
    normalized === 'anthropic'
    || normalized === 'anthropic_messages'
    || normalized === 'messages'
  ) {
    return 'anthropic_messages';
  }
  if (
    normalized === 'chat'
    || normalized === 'chat_completions'
    || normalized === 'openai_chat'
  ) {
    return 'openai_chat';
  }
  return undefined;
}

function resolveGrokProviderApiFormat(
  settingsConfig: GrokSettingsConfig,
  provider?: GrokProvider | null,
  providerEndpoint?: GatewayProviderEndpointProfile,
): GrokApiFormat {
  if (providerEndpoint?.apiFormat) {
    return normalizeGrokApiFormat(providerEndpoint.apiFormat);
  }
  if (provider?.meta?.apiFormat) {
    return normalizeGrokApiFormat(provider.meta.apiFormat);
  }
  const fromBackend = mapGrokApiBackendToApiFormat(
    extractGrokSettingsApiBackend(settingsConfig),
  );
  if (fromBackend) {
    return fromBackend;
  }
  return DEFAULT_GROK_API_FORMAT;
}

function mergeGatewayMetaIntoProviderMeta(
  meta: GatewayProviderMeta | undefined,
  gatewayProfile: GatewayProviderProfileReference | undefined,
  apiFormat: GrokApiFormat | undefined,
): GatewayProviderMeta | undefined {
  return mergeGatewayProfileReferenceIntoMeta(meta, gatewayProfile, apiFormat);
}

function getEndpointCatalogModels(endpoint?: GatewayProviderEndpointProfile): GrokCatalogModel[] {
  if (!Array.isArray(endpoint?.modelCatalog?.models)) {
    return [];
  }

  return endpoint.modelCatalog.models
    .map((item) => ({
      ...item,
      key: item.model?.trim() || undefined,
      model: item.model?.trim() || '',
      displayName: item.displayName?.trim() || undefined,
      contextWindow: item.contextWindow,
      supportsImage: item.supportsImage,
      vision: item.vision,
      attachment: item.attachment,
      modalities: item.modalities,
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
): GrokCatalogModel | undefined {
  const profile = findGatewayProviderProfile(profileId);
  const grokEndpoints = profile?.tools.grok?.endpoints || [];

  return grokEndpoints
    .flatMap((endpoint) => getEndpointCatalogModels(endpoint))
    .find((catalogModel) => catalogModel.model === model);
}

function getDerivedAnthropicCatalogModels(
  profileId: string | null | undefined,
  endpoint?: GatewayProviderEndpointProfile,
): GrokCatalogModel[] {
  if (!endpoint || normalizeGrokApiFormat(endpoint.apiFormat) !== 'anthropic_messages') {
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

function getGrokEndpointCatalogModels(
  profileId: string | null | undefined,
  endpoint?: GatewayProviderEndpointProfile,
): GrokCatalogModel[] {
  const endpointCatalogModels = getEndpointCatalogModels(endpoint);
  if (endpointCatalogModels.length > 0) {
    return endpointCatalogModels;
  }
  return getDerivedAnthropicCatalogModels(profileId, endpoint);
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

interface GrokProviderFormModalProps {
  open: boolean;
  provider?: GrokProvider | null;
  isCopy?: boolean;
  mode?: 'manual' | 'import';
  onCancel: () => void;
  onSubmit: (values: GrokProviderFormValues) => Promise<void>;
}

const GrokProviderFormModal: React.FC<GrokProviderFormModalProps> = ({
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
  const [advancedConfigExpanded, setAdvancedConfigExpanded] = React.useState(false);

  React.useEffect(() => {
    setAdvancedConfigExpanded(false);
  }, [notesCollapseResetKey]);

  // OpenCode import related state
  const [openCodeProviders, setOpenCodeProviders] = React.useState<OpenCodeProviderDisplay[]>([]);
  const [selectedProvider, setSelectedProvider] = React.useState<OpenCodeProviderDisplay | null>(null);
  const [availableModels, setAvailableModels] = React.useState<{ id: string; name: string }[]>([]);
  const [loadingProviders, setLoadingProviders] = React.useState(false);
  const [processedBaseUrl, setProcessedBaseUrl] = React.useState<string>('');
  const [fetchedModels, setFetchedModels] = React.useState<FetchedModel[]>([]);
  const [loadingModels, setLoadingModels] = React.useState(false);
  // 当前表单的 baseUrl（仅用于辅助匹配 OpenCode 导入候选）
  const [currentBaseUrl, setCurrentBaseUrl] = React.useState<string>('');
  const [billingConfig, setBillingConfig] = React.useState(() => getBillingConfigFromMeta(provider?.meta));
  const [customHeaders, setCustomHeaders] = React.useState(() => getCustomHeadersFromMeta(provider?.meta));
  const [modelRewrites, setModelRewrites] = React.useState<ModelRewritesState>(() => getModelRewritesFromMeta(provider?.meta));
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const apiFormatOptions = React.useMemo(() => [
    {
      value: 'openai_responses',
      label: t('grok.provider.apiFormatOpenAIResponses'),
    },
    {
      value: 'openai_chat',
      label: t('grok.provider.apiFormatOpenAIChat'),
    },
    {
      value: 'anthropic_messages',
      label: t('grok.provider.apiFormatAnthropicMessages'),
    },
  ], [t]);

  const isEdit = !!provider && !isCopy;
  const canSelectProviderCategory = !provider && mode === 'manual';

  // 使用新的配置状态管理 Hook
  const {
    grokApiKey,
    grokBaseUrl,
    grokModel,
    grokConfig,
    grokCatalogModels,
    grokReasoningEffort,
    providerCategory,
    handleApiKeyChange,
    handleBaseUrlChange,
    handleModelChange,
    handleConfigChange,
    handleProviderCategoryChange,
    handleReasoningEffortChange,
    setGrokCatalogModels,
    resetFromSettingsConfig,
    getFinalSettingsConfig,
  } = useGrokConfigState({
    initialData: provider ? { settingsConfig: provider.settingsConfig } : undefined,
  });
  const lockedProviderCategory = provider?.category === 'official' ? 'official' : 'custom';
  const activeProviderCategory = canSelectProviderCategory ? providerCategory : lockedProviderCategory;
  const isOfficialMode = activeProviderCategory === 'official';
  const watchOptions = React.useMemo(() => ({ form, preserve: true }), [form]);
  const selectedProviderProfileId = Form.useWatch('providerProfileId', watchOptions) as string | undefined;
  const selectedIsCustomProviderProfile = (selectedProviderProfileId || CUSTOM_PROVIDER_PROFILE_ID) === CUSTOM_PROVIDER_PROFILE_ID;
  // A protocol selector only means something when the gateway can take this
  // CLI over; otherwise the upstream format is fixed by the channel and the
  // dropdown is a no-op. `undefined` while the support list loads.
  const { isGatewaySupported } = useGatewaySupportedCliKeys();
  const gatewaySupportsThisCli = isGatewaySupported('grok') !== false;


  const providerEndpointOptions = React.useMemo(() => {
    if (isOfficialMode && !canSelectProviderCategory) {
      return [{
        value: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        label: t('grok.provider.providerProfileOfficial'),
      }];
    }

    return [
      {
        value: CUSTOM_PROVIDER_ENDPOINT_KEY,
        label: t('grok.provider.providerProfileCustom'),
      },
      ...(canSelectProviderCategory ? [{
        value: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        label: t('grok.provider.providerProfileOfficial'),
      }] : []),
      ...getGatewayProviderProfilesForTool('grok').flatMap((profile) => {
        const endpoints = profile.tools.grok?.endpoints || [];
        return endpoints.map((endpoint) => ({
          value: toGatewayProviderEndpointKey(profile.id, endpoint.id),
          label: `${profile.label} / ${endpoint.label}`,
        }));
      }),
    ];
  }, [canSelectProviderCategory, gatewayProviderProfilesVersion, isOfficialMode, t]);

  const parseProviderCatalogModels = React.useCallback((settingsConfig?: string): GrokCatalogModel[] => {
    if (!settingsConfig) {
      return [];
    }
    try {
      const parsed = JSON.parse(settingsConfig) as GrokSettingsConfig;
      return Array.isArray(parsed.modelCatalog?.models) ? parsed.modelCatalog.models : [];
    } catch {
      return [];
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
    if (isEdit && selectedIsCustomProviderProfile && grokBaseUrl) {
      setCurrentBaseUrl(grokBaseUrl);
    }
  }, [isEdit, selectedIsCustomProviderProfile, grokBaseUrl]);

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
    setCustomHeaders(getCustomHeadersFromMeta(provider?.meta));
    setModelRewrites(getModelRewritesFromMeta(provider?.meta));

    if (provider) {
      let settingsConfig: GrokSettingsConfig = {};
      try {
        settingsConfig = JSON.parse(provider.settingsConfig || '{}') as GrokSettingsConfig;
      } catch {
        settingsConfig = {};
      }
      const baseUrl = extractGrokSettingsBaseUrl(settingsConfig) || '';
      const providerEndpointSelection = lockedProviderCategory === 'official'
        ? {
            providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
            providerEndpointId: undefined,
          }
        : inferGatewayProviderEndpointSelection({
            tool: 'grok',
            meta: provider.meta,
            providerType: provider.meta?.providerType,
            apiFormat: provider.meta?.apiFormat,
          });
      const providerEndpoint = providerEndpointSelection.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            providerEndpointSelection.providerProfileId,
            'grok',
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
        model: extractGrokSettingsModel(settingsConfig) || providerEndpoint?.model || '',
        apiFormat: resolveGrokProviderApiFormat(settingsConfig, provider, providerEndpoint),
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
        apiFormat: DEFAULT_GROK_API_FORMAT,
        configToml: '',
        notes: '',
        sourceProvider: undefined,
      });
    }

    setSelectedProvider(null);
    setAvailableModels([]);
    setFetchedModels([]);
    // Create flow: prefill grok-4.5 + high, but keep both editable.
    if (!provider) {
      handleModelChange(DEFAULT_GROK_MODEL);
      handleReasoningEffortChange('high');
      form.setFieldValue('model', DEFAULT_GROK_MODEL);
      setCurrentBaseUrl('');
    } else {
      // Edit flow: keep existing catalog; effort is the default model's effort.
      const catalog = parseProviderCatalogModels(provider.settingsConfig);
      const settingsConfig = (() => {
        try {
          return JSON.parse(provider.settingsConfig || '{}') as GrokSettingsConfig;
        } catch {
          return {} as GrokSettingsConfig;
        }
      })();
      const defaultKey = settingsConfig.defaultModelKey?.trim();
      const defaultModel = catalog.find((item) => (item.key?.trim() || item.model) === defaultKey)
        || catalog[0];
      if (defaultModel?.reasoningEffort) {
        handleReasoningEffortChange(defaultModel.reasoningEffort);
      }
    }
    setProcessedBaseUrl('');
    formInitializedRef.current = true;
  }, [
    form,
    handleModelChange,
    handleProviderCategoryChange,
    handleReasoningEffortChange,
    lockedProviderCategory,
    open,
    parseProviderCatalogModels,
    provider,
    resetFromSettingsConfig,
  ]);

  React.useEffect(() => {
    if (!open || !formInitializedRef.current) {
      return;
    }

    const currentEndpoint = findGatewayProviderEndpoint(
      form.getFieldValue('providerProfileId'),
      'grok',
      form.getFieldValue('providerEndpointId'),
    );
    const currentEndpointModel = currentEndpoint?.model;
    let settingsConfig: GrokSettingsConfig = {};
    if (provider?.settingsConfig) {
      try {
        settingsConfig = JSON.parse(provider.settingsConfig) as GrokSettingsConfig;
      } catch {
        settingsConfig = {};
      }
    }
    const nextFieldValues = provider
      ? {
          name: provider.name,
          apiKey: grokApiKey,
          baseUrl: grokBaseUrl || currentEndpoint?.baseUrl || '',
          model: grokModel || currentEndpointModel || '',
          apiFormat: resolveGrokProviderApiFormat(settingsConfig, provider, currentEndpoint),
          configToml: grokConfig,
          notes: provider.notes || '',
        }
      : {
          apiKey: grokApiKey,
          baseUrl: grokBaseUrl,
          model: grokModel,
          apiFormat: form.getFieldValue('apiFormat') || DEFAULT_GROK_API_FORMAT,
          configToml: grokConfig,
        };

    form.setFieldsValue(nextFieldValues);
  }, [
    grokApiKey,
    grokBaseUrl,
    grokConfig,
    grokModel,
    form,
    gatewayProviderProfilesVersion,
    open,
    provider,
  ]);

  // 同步 Hook 的 grokConfig 到 Form 的 configToml 字段
  // 当用户在 baseUrl 或 model 输入框输入时，需要实时更新 TOML 编辑器
  const prevGrokConfigRef = React.useRef(grokConfig);
  React.useEffect(() => {
    // 只在表单已初始化且 grokConfig 变化时同步
    if (!formInitializedRef.current) return;
    if (prevGrokConfigRef.current === grokConfig) return;
    
    prevGrokConfigRef.current = grokConfig;
    
    // 获取当前表单的 configToml 值
    const currentFormConfig = form.getFieldValue('configToml') || '';
    
    // 只有当 Hook 的值与 Form 的值不同时才更新，避免不必要的更新
    if (currentFormConfig !== grokConfig) {
      form.setFieldsValue({ configToml: grokConfig });
    }
  }, [grokConfig, form]);

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
        apiFormat: DEFAULT_GROK_API_FORMAT,
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
        apiFormat: form.getFieldValue('apiFormat') || DEFAULT_GROK_API_FORMAT,
      });
      return;
    }

    const endpoint = findGatewayProviderEndpoint(providerProfileId, 'grok', providerEndpointId);
    if (!endpoint) {
      return;
    }

    const catalogModels = getGrokEndpointCatalogModels(providerProfileId, endpoint);
    const nextModel = endpoint.model || form.getFieldValue('model');

    form.setFieldsValue({
      providerEndpointKey: toGatewayProviderEndpointKey(providerProfileId, endpoint.id),
      providerProfileId,
      providerEndpointId: endpoint.id,
      apiFormat: normalizeGrokApiFormat(endpoint.apiFormat),
      baseUrl: endpoint.baseUrl,
      model: nextModel,
    });
    handleBaseUrlChange(endpoint.baseUrl);
    setCurrentBaseUrl(endpoint.baseUrl);

    if (nextModel) {
      handleModelChange(nextModel);
    }

    // Preserve endpoint/catalog model fields as-is; backend-search is per-model only.
    setGrokCatalogModels(catalogModels);
  };


  const shouldConfirmOpenAiBaseUrlV1 = (
    baseUrl: string | undefined,
    apiFormat: GrokApiFormat | undefined,
    hasSelectedGatewayEndpoint: boolean,
  ): boolean => {
    // 内置 gateway profile endpoint 的 baseUrl 由 profile 提供且已验证可用，
    // 不套用"自定义渠道需以 /v1 结尾"的通用校验。
    if (hasSelectedGatewayEndpoint) {
      return false;
    }
    if (!baseUrl?.trim()) {
      return false;
    }
    const format = normalizeGrokApiFormat(apiFormat);
    if (format !== 'openai_chat' && format !== 'openai_responses') {
      return false;
    }

    // Full-URL providers (AxonHub-style ## suffix) must not be rewritten or warned as base paths.
    let normalizedBaseUrl = baseUrl.trim();
    if (normalizedBaseUrl.endsWith('/')) {
      normalizedBaseUrl = normalizedBaseUrl.slice(0, -1);
    }
    if (normalizedBaseUrl.endsWith('##')) {
      return false;
    }
    return !normalizedBaseUrl.endsWith('/v1');
  };

  const submitProviderForm = async (submittedValues: GrokProviderFormValues & {
    apiKey?: string;
    baseUrl?: string;
    model?: string;
    configToml?: string;
  }) => {
    setLoading(true);
    try {
      const selectedCategory = mode === 'import'
        ? 'custom'
        : (activeProviderCategory === 'official' ? 'official' : 'custom');
      // Create/edit both use form values. Create prefills grok-4.5 + high but stays editable.
      const selectedModelValue = (submittedValues.model || '').trim() || DEFAULT_GROK_MODEL;
      const selectedEffort = grokReasoningEffort || (!isEdit ? 'high' : undefined);
      let catalogForSave = grokCatalogModels;
      let preferredDefaultKey: string | undefined;
      if (selectedCategory === 'custom' && isEdit) {
        const matched = grokCatalogModels.find((item) => (
          (item.key?.trim() || item.model) === selectedModelValue
          || item.model === selectedModelValue
        ));
        preferredDefaultKey = matched?.key?.trim() || matched?.model || selectedModelValue || undefined;
        if (matched && selectedEffort) {
          catalogForSave = grokCatalogModels.map((item) => {
            const key = item.key?.trim() || item.model;
            if (key !== preferredDefaultKey) {
              return item;
            }
            const efforts = Array.isArray(item.reasoningEfforts) && item.reasoningEfforts.length > 0
              ? item.reasoningEfforts
              : ['low', 'medium', 'high', 'xhigh'];
            return {
              ...item,
              supportsReasoningEffort: true,
              reasoningEfforts: efforts,
              reasoningEffort: efforts.includes(selectedEffort)
                ? selectedEffort
                : efforts[0],
            };
          });
        }
      } else if (selectedCategory === 'custom' && !isEdit) {
        // Bootstrap one default model: key and upstream model id are the same string
        // (form "模型名称"), matching the multi-model editor rule.
        const bootstrapKey = selectedModelValue;
        const bootstrapEffort = selectedEffort || 'high';
        catalogForSave = [{
          key: bootstrapKey,
          model: bootstrapKey,
          displayName: bootstrapKey,
          supportsReasoningEffort: true,
          reasoningEfforts: ['low', 'medium', 'high', 'xhigh'],
          reasoningEffort: bootstrapEffort,
        }];
        preferredDefaultKey = bootstrapKey;
      }

      const settingsConfig = getFinalSettingsConfig({
        category: selectedCategory,
        apiKey: submittedValues.apiKey || '',
        baseUrl: submittedValues.baseUrl || '',
        model: selectedModelValue,
        apiFormat: normalizeGrokApiFormat(submittedValues.apiFormat),
        // Channel form no longer owns backend-search toggle; preserve per-model flags already on catalog.
        supportsBackendSearch: undefined,
        reasoningEffort: selectedEffort,
        defaultModelKey: preferredDefaultKey,
        config: submittedValues.configToml || '',
        catalogModels: catalogForSave,
      });
      const selectedEndpoint = selectedCategory === 'official' || submittedValues.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            submittedValues.providerProfileId,
            'grok',
            submittedValues.providerEndpointId,
          );
      const selectedApiFormat = selectedCategory === 'official'
        ? undefined
        : selectedEndpoint
          ? normalizeGrokApiFormat(selectedEndpoint.apiFormat)
          : normalizeGrokApiFormat(submittedValues.apiFormat);
      const gatewayProfile = selectedEndpoint
        ? toGatewayProviderProfileReference('grok', submittedValues.providerProfileId || '', selectedEndpoint.id)
        : undefined;
      const finalSettingsConfig = selectedCategory === 'official'
        ? settingsConfig
        : selectedEndpoint
          ? applyGrokEndpointSettingsConfig({
              settingsConfig,
              apiFormat: normalizeGrokApiFormat(selectedEndpoint.apiFormat),
              endpointBaseUrl: selectedEndpoint.baseUrl,
              endpointModel: selectedEndpoint.model,
              endpointCatalogModels: getGrokEndpointCatalogModels(
                submittedValues.providerProfileId,
                selectedEndpoint,
              ),
            })
          : settingsConfig;

      const formValues: GrokProviderFormValues = {
        name: submittedValues.name,
        category: selectedCategory,
        providerEndpointKey: selectedEndpoint
          ? toGatewayProviderEndpointKey(submittedValues.providerProfileId || '', selectedEndpoint.id)
          : CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: selectedEndpoint
          ? submittedValues.providerProfileId
          : CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: selectedEndpoint?.id,
        settingsConfig: finalSettingsConfig,
        apiFormat: selectedApiFormat,
        meta: mergeModelRewritesIntoMeta(
          mergeCustomHeadersIntoMeta(
            mergeBillingConfigIntoMeta(
              mergeGatewayMetaIntoProviderMeta(
                provider?.meta,
                gatewayProfile,
                gatewayProfile ? undefined : selectedApiFormat,
              ),
              selectedCategory === 'official'
                ? { enabled: false, pricingModelSource: 'inherit' }
                : billingConfig,
            ),
            selectedCategory === 'official'
              ? { enabled: false, headers: [] }
              : customHeaders,
          ),
          selectedCategory === 'official'
            ? { enabled: false, rewrites: [] }
            : modelRewrites,
        ),
        notes: submittedValues.notes,
        sourceProviderId: mode === 'import' ? selectedProvider?.id : undefined,
      };

      await onSubmit(formValues);
      form.resetFields();
      setSelectedProvider(null);
      setAvailableModels([]);
      onCancel();
    } finally {
      setLoading(false);
    }
  };

  const handleSubmit = async () => {
    try {
      const fieldsToValidate = mode === 'import'
        ? ['sourceProvider', 'name', 'apiKey', 'apiFormat', 'configToml', 'notes']
        : [...(canSelectProviderCategory ? ['category'] : []), 'name', ...(!isOfficialMode ? ['providerEndpointKey', 'apiKey', 'baseUrl', 'apiFormat'] : []), 'configToml', 'notes'];

      const values = await form.validateFields(fieldsToValidate);
      const submittedValues = {
        ...(form.getFieldsValue(true) as GrokProviderFormValues),
        ...values,
      };

      const selectedCategory = mode === 'import'
        ? 'custom'
        : (activeProviderCategory === 'official' ? 'official' : 'custom');
      const selectedEndpoint = selectedCategory === 'official' || submittedValues.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            submittedValues.providerProfileId,
            'grok',
            submittedValues.providerEndpointId,
          );
      const effectiveApiFormat = selectedCategory === 'official'
        ? undefined
        : selectedEndpoint
          ? normalizeGrokApiFormat(selectedEndpoint.apiFormat)
          : normalizeGrokApiFormat(submittedValues.apiFormat);

      if (shouldConfirmOpenAiBaseUrlV1(submittedValues.baseUrl, effectiveApiFormat, !!selectedEndpoint)) {
        Modal.confirm({
          title: t('common.confirm'),
          content: t('grok.provider.baseUrlConfirmV1'),
          okText: t('common.confirm'),
          cancelText: t('common.cancel'),
          onOk: () => submitProviderForm(submittedValues),
        });
        return;
      }

      await submitProviderForm(submittedValues);
    } catch (error) {
      console.error('Form validation failed:', error);
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
        const response = await fetchGrokOfficialModels();
        const models = response.models.length > 0 ? response.models : GROK_OFFICIAL_FALLBACK_MODELS;
        setFetchedModels(models);

        if (response.source === 'bundled') {
          message.info(t('grok.fetchModels.officialBundled', { count: models.length }));
        } else {
          message.success(t('grok.fetchModels.officialUpdated', { count: models.length }));
        }
      } catch (error) {
        console.error('Failed to fetch Grok official models:', error);
        setFetchedModels(GROK_OFFICIAL_FALLBACK_MODELS);
        message.info(t('grok.fetchModels.officialBundled', { count: GROK_OFFICIAL_FALLBACK_MODELS.length }));
      } finally {
        setLoadingModels(false);
      }
      return;
    }

    const baseUrl = (form.getFieldValue('baseUrl') as string | undefined)?.trim();
    const apiKey = (form.getFieldValue('apiKey') as string | undefined)?.trim();

    if (!baseUrl) {
      message.warning(t('grok.fetchModels.baseUrlRequired'));
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
          headers: getEffectiveCustomHeaders(customHeaders),
        },
      });

      setFetchedModels(response.models);
      if (response.models.length > 0) {
        message.success(t('grok.fetchModels.success', { count: response.models.length }));
      } else {
        message.info(t('grok.fetchModels.noModels'));
      }
    } catch (error) {
      console.error('Failed to fetch Grok models:', error);
      message.error(t('grok.fetchModels.failed'));
    } finally {
      setLoadingModels(false);
    }
  };

  // 根据 baseUrl 辅助匹配 OpenCode 导入候选的模型列表
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
      // OpenCode 的 URL 包含 Grok 的 URL，或者反过来
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
        label={t('grok.provider.providerProfile')}
        required
        help={<Text type="secondary" style={{ fontSize: 12 }}>{t('grok.provider.providerProfileHelp')}</Text>}
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
              initialValue={DEFAULT_GROK_API_FORMAT}
            >
              <Select
                options={apiFormatOptions}
                disabled={!gatewaySupportsThisCli || !selectedIsCustomProviderProfile}
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
        label={t('grok.provider.formName')}
        rules={[{ required: true, message: t('common.error') }]}
      >
        <Input placeholder={t('grok.provider.namePlaceholder')} />
      </Form.Item>

      {!isOfficialMode && (
        <>
          <Form.Item
            name="apiKey"
            label={t('grok.provider.apiKey')}
            rules={[{ required: true, message: t('common.error') }]}
          >
            {/* ImeSafeInput: the value is written back from hook state on
                every keystroke (the sanitizer in useGrokConfigState), which on
                WebKit ends an IME composition mid-word — issue #409. */}
            <ImeSafeInput
              type={showApiKey ? 'text' : 'password'}
              placeholder={t('grok.provider.apiKeyPlaceholder')}
              addonAfter={
                <Button
                  type="text"
                  size="small"
                  icon={showApiKey ? <EyeInvisibleOutlined /> : <EyeOutlined />}
                  onClick={() => setShowApiKey(!showApiKey)}
                >
                  {showApiKey ? t('common.provider.hideApiKey') : t('common.provider.showApiKey')}
                </Button>
              }
            />
          </Form.Item>

          <Form.Item
            name="baseUrl"
            label={t('grok.provider.baseUrl')}
            rules={[{ required: true, message: t('common.error') }]}
            help={<Text type="secondary" style={{ fontSize: 12 }}>{t('grok.provider.baseUrlHelp')}</Text>}
          >
            <ImeSafeInput
              placeholder="https://api.x.ai/v1"
            />
          </Form.Item>

        </>
      )}

      {/* Model + effort: create prefills grok-4.5/high (editable); edit custom picks from card catalog. */}
      <Form.Item
        label={t('grok.provider.modelName')}
        help={(
          <Text type="secondary" style={{ fontSize: 12 }}>
            {isOfficialMode
              ? t('grok.provider.modelNameOfficialHint')
              : isEdit
                ? t('grok.provider.modelNameEditHint')
                : t('grok.provider.modelNameCreateHint')}
          </Text>
        )}
      >
        <div style={{ display: 'flex', gap: 8, width: '100%' }}>
          {isOfficialMode || !isEdit ? (
            <Form.Item name="model" noStyle initialValue={DEFAULT_GROK_MODEL}>
              <ImeSafeAutoComplete
                options={modelOptions}
                placeholder={t('grok.provider.modelNamePlaceholder')}
                style={{ flex: 1, minWidth: 0 }}
                filterOption={(inputValue, option) =>
                  (option?.label?.toString().toLowerCase().includes(inputValue.toLowerCase()) ||
                  option?.value?.toString().toLowerCase().includes(inputValue.toLowerCase())) ?? false
                }
                onChange={(value) => handleModelChange(String(value || ''))}
              />
            </Form.Item>
          ) : (
            <Form.Item name="model" noStyle>
              <Select
                style={{ flex: 1, minWidth: 0 }}
                placeholder={t('grok.provider.modelNamePlaceholder')}
                options={grokCatalogModels.map((item) => {
                  const key = item.key?.trim() || item.model;
                  const label = item.displayName || item.model || key;
                  return {
                    value: key,
                    label: item.model && item.model !== key
                      ? `${label} (${item.model})`
                      : label,
                  };
                })}
                onChange={(value) => {
                  handleModelChange(String(value || ''));
                  const selected = grokCatalogModels.find((item) => (
                    (item.key?.trim() || item.model) === value
                  ));
                  if (selected?.reasoningEffort) {
                    handleReasoningEffortChange(selected.reasoningEffort);
                  }
                }}
              />
            </Form.Item>
          )}
          <Select
            style={{ width: 168, flexShrink: 0 }}
            allowClear
            placeholder={t('grok.provider.reasoningEffortPlaceholder')}
            value={grokReasoningEffort || undefined}
            onChange={(value) => handleReasoningEffortChange(
              typeof value === 'string' ? value : undefined,
            )}
            options={GROK_REASONING_EFFORT_OPTIONS.map((value) => ({
              value,
              label: value,
            }))}
          />
          {isOfficialMode && (
            <Button
              icon={<CloudDownloadOutlined />}
              loading={loadingModels}
              onClick={handleFetchModels}
            >
              {t('grok.fetchModels.button')}
            </Button>
          )}
        </div>
      </Form.Item>

      <Form.Item wrapperCol={sectionWrapperCol}>
        <ProviderConfigCollapse
          title={t('grok.provider.advancedConfig')}
          expanded={advancedConfigExpanded}
          onExpandedChange={setAdvancedConfigExpanded}
          icon={<FileCode2 />}
        >
          <Form.Item
            name="configToml"
            label="config.toml"
            extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('grok.provider.configTomlHelp')}</Text>}
            rules={[validateTomlRule(t('grok.provider.configTomlInvalid'))]}
            style={{ marginBottom: 0 }}
          >
            <TomlEditorFormItem
              placeholder={t('grok.provider.configTomlPlaceholder')}
            />
          </Form.Item>
        </ProviderConfigCollapse>
      </Form.Item>

      {!isOfficialMode && (
        <Form.Item wrapperCol={sectionWrapperCol}>
          <BillingConfigCollapse
            value={billingConfig}
            onChange={setBillingConfig}
          />
        </Form.Item>
      )}

      {!isOfficialMode && (
        <Form.Item wrapperCol={sectionWrapperCol}>
          <CustomHeadersCollapse
            value={customHeaders}
            onChange={setCustomHeaders}
          />
        </Form.Item>
      )}

      {!isOfficialMode && (
        <Form.Item wrapperCol={sectionWrapperCol}>
          <ModelRewritesCollapse
            value={modelRewrites}
            onChange={setModelRewrites}
          />
        </Form.Item>
      )}

      <Form.Item name="notes" wrapperCol={sectionWrapperCol}>
        <ProviderNotesCollapse
          title={t('grok.provider.notes')}
          placeholder={t('grok.provider.notesPlaceholder')}
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
          label={t('grok.import.selectProvider')}
          rules={[{ required: true, message: t('common.error') }]}
        >
          <Select
            placeholder={t('grok.import.selectProviderPlaceholder')}
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
            message={t('grok.import.importInfo')}
            description={
              <Space direction="vertical" size={4}>
                <div>{t('grok.import.providerName')}: {selectedProvider.name}</div>
                <div>{t('grok.import.baseUrl')}: {processedBaseUrl}</div>
                <div>{t('grok.import.availableModels')}: {availableModels.length > 0 ? t('grok.import.modelsCount', { count: availableModels.length }) : '-'}</div>
              </Space>
            }
            type="success"
            showIcon
            style={{ marginBottom: 16 }}
          />
        )}

        <Form.Item name="name" label={t('grok.provider.formName')}>
          <Input placeholder={t('grok.provider.namePlaceholder')} disabled />
        </Form.Item>

        <Form.Item name="apiKey" label={t('grok.provider.apiKey')}>
          <Input type="password" disabled />
        </Form.Item>

        <Form.Item name="apiFormat" label={t('grok.provider.apiFormat')} initialValue="openai_chat">
          <Select options={apiFormatOptions} disabled />
        </Form.Item>

        {availableModels.length > 0 && (
          <>
            <Alert
              message={t('grok.model.selectFromProvider')}
              type="info"
              showIcon
              style={{ marginBottom: 16 }}
            />

            <Form.Item name="model" label={t('grok.import.selectDefaultModel')}>
              <Select
                placeholder={t('grok.model.defaultModelPlaceholder')}
                options={modelSelectOptions}
                allowClear
                showSearch
              />
            </Form.Item>
          </>
        )}

        <Form.Item wrapperCol={sectionWrapperCol}>
          <ProviderConfigCollapse
            title={t('grok.provider.advancedConfig')}
            expanded={advancedConfigExpanded}
            onExpandedChange={setAdvancedConfigExpanded}
            icon={<FileCode2 />}
          >
            <Form.Item
              name="configToml"
              label="config.toml"
              extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('grok.provider.configTomlHelp')}</Text>}
              rules={[validateTomlRule(t('grok.provider.configTomlInvalid'))]}
              style={{ marginBottom: 0 }}
            >
              <TomlEditorFormItem
                placeholder={t('grok.provider.configTomlPlaceholder')}
              />
            </Form.Item>
          </ProviderConfigCollapse>
        </Form.Item>

        {!isOfficialMode && (
          <Form.Item wrapperCol={sectionWrapperCol}>
            <CustomHeadersCollapse
              value={customHeaders}
              onChange={setCustomHeaders}
            />
          </Form.Item>
        )}

        {!isOfficialMode && (
          <Form.Item wrapperCol={sectionWrapperCol}>
            <ModelRewritesCollapse
              value={modelRewrites}
              onChange={setModelRewrites}
            />
          </Form.Item>
        )}

        <Form.Item name="notes" wrapperCol={sectionWrapperCol}>
          <ProviderNotesCollapse
            title={t('grok.provider.notes')}
            placeholder={t('grok.provider.notesPlaceholder')}
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
          ? t('grok.provider.editProvider')
          : mode === 'import'
            ? t('grok.import.title')
            : t('grok.provider.addProvider')
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

export default GrokProviderFormModal;
