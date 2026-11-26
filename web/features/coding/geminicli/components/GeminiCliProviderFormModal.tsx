import React from 'react';
import { Alert, Button, Form, Input, message, Modal, Select, Typography } from 'antd';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';
import { CloudDownloadOutlined, EyeInvisibleOutlined, EyeOutlined } from '@ant-design/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import type { FetchedModel, FetchModelsResponse } from '@/components/common/FetchModelsModal/types';
import BillingConfigCollapse from '@/features/coding/shared/providerBilling/BillingConfigCollapse';
import CustomHeadersCollapse from '@/features/coding/shared/providerHeaders/CustomHeadersCollapse';
import ProviderNotesCollapse from '@/features/coding/shared/providerConfig/ProviderNotesCollapse';
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
  getGatewayProviderProfilesForTool,
  getGatewayProviderProfilesVersion,
  inferGatewayProviderEndpointSelection,
  mergeGatewayProfileReferenceIntoMeta,
  parseGatewayProviderEndpointKey,
  subscribeGatewayProviderProfiles,
  toGatewayProviderEndpointKey,
  toGatewayProviderProfileReference,
  type GatewayProviderProfileReference,
} from '@/features/coding/shared/gateway/providerProfiles';
import type {
  GatewayProviderMeta,
  GeminiCliApiFormat,
  GeminiCliProvider,
  GeminiCliProviderFormValues,
  GeminiCliSettingsConfig,
} from '@/types/geminicli';
import styles from './GeminiCliProviderFormModal.module.less';

const { Text } = Typography;

const DEFAULT_GEMINI_MODELS_BASE_URL = 'https://generativelanguage.googleapis.com/v1beta';
const DEFAULT_GEMINI_API_FORMAT: GeminiCliApiFormat = 'gemini_native';
const OFFICIAL_GEMINI_CHANNEL_KEY = '__official__';

const DEFAULT_GEMINI_MODEL_OPTIONS = [
  'auto',
  'auto-gemini-3',
  'auto-gemini-2.5',
  'pro',
  'flash',
  'flash-lite',
  'gemini-3.1-flash-lite-preview',
  'gemini-3.1-pro-preview',
  'gemini-3.1-pro-preview-customtools',
  'gemini-3-pro-preview',
  'gemini-3-flash-preview',
  'gemini-2.5-pro',
  'gemini-2.5-flash',
  'gemini-2.5-flash-lite',
  'gemma-4-31b-it',
  'gemma-4-26b-a4b-it',
].map((model) => ({ label: model, value: model }));

const GEMINI_CLI_OFFICIAL_MODELS: FetchedModel[] = DEFAULT_GEMINI_MODEL_OPTIONS.map((option) => ({
  id: option.value,
  name: option.label,
  ownedBy: 'google',
  created: undefined,
}));

function normalizeGeminiApiFormat(value?: string): GeminiCliApiFormat {
  if (value === 'openai_chat' || value === 'openai_responses') {
    return value;
  }
  if (value === 'anthropic' || value === 'anthropic_messages') {
    return 'anthropic';
  }
  return DEFAULT_GEMINI_API_FORMAT;
}

function mergeGatewayMetaIntoProviderMeta(
  meta: GatewayProviderMeta | undefined,
  gatewayProfile: GatewayProviderProfileReference | undefined,
  apiFormat: GeminiCliApiFormat | undefined,
): GatewayProviderMeta | undefined {
  return mergeGatewayProfileReferenceIntoMeta(meta, gatewayProfile, apiFormat);
}

interface GeminiCliProviderFormModalProps {
  open: boolean;
  provider?: GeminiCliProvider | null;
  isCopy?: boolean;
  onCancel: () => void;
  onSubmit: (values: GeminiCliProviderFormValues) => Promise<void>;
}

interface GeminiCliOfficialModelsResponse extends FetchModelsResponse {
  source?: 'remote' | 'bundled';
}

const defaultOfficialConfig: GeminiCliSettingsConfig = {
  env: {},
  config: {
    security: {
      auth: {
        selectedType: 'oauth-personal',
      },
    },
  },
};

const defaultCustomConfig: GeminiCliSettingsConfig = {
  env: {
    GOOGLE_GEMINI_BASE_URL: '',
    GEMINI_API_KEY: '',
    GEMINI_MODEL: '',
  },
  config: {
    security: {
      auth: {
        selectedType: 'gemini-api-key',
      },
    },
  },
};

const OFFICIAL_PROVIDER_REMOVED_ENV_KEYS = [
  'GEMINI_API_KEY',
  'GOOGLE_API_KEY',
  'GOOGLE_GEMINI_BASE_URL',
  'GOOGLE_VERTEX_BASE_URL',
  'GOOGLE_GENAI_USE_GCA',
  'GOOGLE_GENAI_USE_VERTEXAI',
  'GEMINI_CLI_USE_COMPUTE_ADC',
  'GEMINI_CLI_CUSTOM_HEADERS',
  'GEMINI_API_KEY_AUTH_MECHANISM',
  'GOOGLE_GENAI_API_VERSION',
  'GOOGLE_CLOUD_PROJECT',
  'GOOGLE_CLOUD_PROJECT_ID',
  'GOOGLE_CLOUD_LOCATION',
];

const setGeminiCliAuthSelectedType = (
  settingsConfig: unknown,
  selectedType: 'oauth-personal' | 'gemini-api-key',
): GeminiCliSettingsConfig => {
  const nextConfig = (isRecord(settingsConfig) ? { ...settingsConfig } : {}) as GeminiCliSettingsConfig;
  const config = isRecord(nextConfig.config) ? { ...nextConfig.config } : {};
  const security = isRecord(config.security) ? { ...config.security } : {};
  const auth = isRecord(security.auth) ? { ...security.auth } : {};
  auth.selectedType = selectedType;
  security.auth = auth;
  config.security = security;
  nextConfig.config = config;

  return nextConfig;
};

const normalizeOfficialSettingsConfig = (settingsConfig: unknown): GeminiCliSettingsConfig => {
  const nextConfig = setGeminiCliAuthSelectedType(settingsConfig, 'oauth-personal');
  const nextEnv = isRecord(nextConfig.env) ? { ...nextConfig.env } : {};

  OFFICIAL_PROVIDER_REMOVED_ENV_KEYS.forEach((key) => {
    delete nextEnv[key];
  });

  nextConfig.env = nextEnv;

  return nextConfig;
};

const normalizeCustomSettingsConfig = (settingsConfig: unknown): GeminiCliSettingsConfig => (
  setGeminiCliAuthSelectedType(settingsConfig, 'gemini-api-key')
);

const normalizeSettingsConfigForCategory = (
  settingsConfig: unknown,
  category: string,
): GeminiCliSettingsConfig => (
  category === 'official'
    ? normalizeOfficialSettingsConfig(settingsConfig)
    : normalizeCustomSettingsConfig(settingsConfig)
);

const parseSettingsConfig = (rawConfig?: unknown): GeminiCliSettingsConfig => {
  if (isRecord(rawConfig)) {
    return rawConfig as GeminiCliSettingsConfig;
  }

  if (typeof rawConfig !== 'string' || !rawConfig.trim()) {
    return defaultCustomConfig;
  }

  try {
    const parsed = JSON.parse(rawConfig) as GeminiCliSettingsConfig;
    return parsed && typeof parsed === 'object' ? parsed : defaultCustomConfig;
  } catch {
    return defaultCustomConfig;
  }
};

const isRecord = (value: unknown): value is Record<string, unknown> => (
  typeof value === 'object' && value !== null && !Array.isArray(value)
);

const extractGeminiEnvModelName = (settingsConfig: unknown): string => {
  if (!isRecord(settingsConfig) || !isRecord(settingsConfig.env)) {
    return '';
  }

  const modelName = settingsConfig.env.GEMINI_MODEL;
  return typeof modelName === 'string' ? modelName.trim() : '';
};

const extractGeminiApiKey = (settingsConfig: unknown): string => (
  extractEnvString(settingsConfig, 'GEMINI_API_KEY') || extractEnvString(settingsConfig, 'GOOGLE_API_KEY')
);

const extractEnvString = (settingsConfig: unknown, key: string): string => {
  if (!isRecord(settingsConfig) || !isRecord(settingsConfig.env)) {
    return '';
  }

  const value = settingsConfig.env[key];
  return typeof value === 'string' ? value.trim() : '';
};

const resolveFetchModelsConfig = (settingsConfig: unknown) => {
  const configuredBaseUrl = extractEnvString(settingsConfig, 'GOOGLE_GEMINI_BASE_URL');
  const baseUrl = configuredBaseUrl || DEFAULT_GEMINI_MODELS_BASE_URL;
  const apiKey = extractEnvString(settingsConfig, 'GEMINI_API_KEY') || extractEnvString(settingsConfig, 'GOOGLE_API_KEY');

  return { baseUrl, apiKey, hasConfiguredBaseUrl: Boolean(configuredBaseUrl) };
};

const cloneSettingsConfigWithEnv = (settingsConfig: unknown) => {
  const nextConfig = (isRecord(settingsConfig) ? { ...settingsConfig } : {}) as GeminiCliSettingsConfig;
  const nextEnv = isRecord(nextConfig.env) ? { ...nextConfig.env } : {};

  return { nextConfig, nextEnv };
};

const updateGeminiEnvValue = (
  settingsConfig: unknown,
  key: string,
  value: string,
): GeminiCliSettingsConfig => {
  const { nextConfig, nextEnv } = cloneSettingsConfigWithEnv(settingsConfig);
  const trimmedValue = value.trim();

  if (trimmedValue) {
    nextEnv[key] = trimmedValue;
  } else {
    delete nextEnv[key];
  }

  nextConfig.env = nextEnv;
  return nextConfig;
};

const updateGeminiEnvModelName = (
  settingsConfig: unknown,
  modelName: string,
): GeminiCliSettingsConfig => {
  const { nextConfig, nextEnv } = cloneSettingsConfigWithEnv(settingsConfig);
  const trimmedModelName = modelName.trim();

  if (trimmedModelName) {
    nextEnv.GEMINI_MODEL = trimmedModelName;
  } else {
    delete nextEnv.GEMINI_MODEL;
  }

  nextConfig.env = nextEnv;
  return nextConfig;
};

const updateGeminiApiKey = (
  settingsConfig: unknown,
  apiKey: string,
): GeminiCliSettingsConfig => {
  const { nextConfig, nextEnv } = cloneSettingsConfigWithEnv(settingsConfig);
  const trimmedApiKey = apiKey.trim();

  if (trimmedApiKey) {
    nextEnv.GEMINI_API_KEY = trimmedApiKey;
  } else {
    delete nextEnv.GEMINI_API_KEY;
  }

  delete nextEnv.GOOGLE_API_KEY;
  nextConfig.env = nextEnv;
  return nextConfig;
};

const syncDedicatedFieldsToSettingsConfig = (
  settingsConfig: unknown,
  fields: {
    apiKey?: unknown;
    baseUrl?: unknown;
    modelName?: unknown;
  },
): GeminiCliSettingsConfig | unknown => {
  let nextConfig = settingsConfig;

  if (typeof fields.baseUrl === 'string') {
    nextConfig = updateGeminiEnvValue(nextConfig, 'GOOGLE_GEMINI_BASE_URL', fields.baseUrl);
  }

  if (typeof fields.apiKey === 'string') {
    nextConfig = updateGeminiApiKey(nextConfig, fields.apiKey);
  }

  if (typeof fields.modelName === 'string') {
    nextConfig = updateGeminiEnvModelName(nextConfig, fields.modelName);
  }

  return nextConfig;
};

const GeminiCliProviderFormModal: React.FC<GeminiCliProviderFormModalProps> = ({
  open,
  provider,
  isCopy = false,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [settingsConfigValue, setSettingsConfigValue] = React.useState<unknown>(defaultCustomConfig);
  const [settingsConfigValid, setSettingsConfigValid] = React.useState(true);
  const [fetchedModels, setFetchedModels] = React.useState<FetchedModel[]>([]);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);
  const [selectedProviderCategory, setSelectedProviderCategory] = React.useState<string>('custom');
  const [billingConfig, setBillingConfig] = React.useState(() => getBillingConfigFromMeta(provider?.meta));
  const [customHeaders, setCustomHeaders] = React.useState(() => getCustomHeadersFromMeta(provider?.meta));
  const [modelRewrites, setModelRewrites] = React.useState<ModelRewritesState>(() => getModelRewritesFromMeta(provider?.meta));
  const selectedChannel = Form.useWatch('channel', form) as string | undefined;
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const apiFormatOptions = React.useMemo(() => [
    {
      value: 'gemini_native',
      label: t('geminicli.provider.apiFormatGeminiNative'),
    },
    {
      value: 'openai_chat',
      label: t('geminicli.provider.apiFormatOpenAIChat'),
    },
    {
      value: 'openai_responses',
      label: t('geminicli.provider.apiFormatOpenAIResponses'),
    },
    {
      value: 'anthropic',
      label: t('geminicli.provider.apiFormatAnthropic'),
    },
  ], [t]);
  const sectionWrapperCol = { span: 24 };
  const notesCollapseResetKey = `${open ? 'open' : 'closed'}:${provider?.id ?? 'new'}:${isCopy ? 'copy' : 'normal'}`;
  const isEdit = Boolean(provider && !isCopy);
  const canSelectProviderCategory = !provider;
  const providerCategory = provider ? (provider.category || 'custom') : selectedProviderCategory;
  const isOfficialMode = providerCategory === 'official';
  const selectedIsCustomProviderProfile = !selectedChannel || selectedChannel === CUSTOM_PROVIDER_ENDPOINT_KEY;
  // A protocol selector only means something when the gateway can take this
  // CLI over; otherwise the upstream format is fixed by the channel and the
  // dropdown is a no-op. `undefined` while the support list loads.
  const { isGatewaySupported } = useGatewaySupportedCliKeys();
  const gatewaySupportsThisCli = isGatewaySupported('gemini') !== false;

  const channelOptions = React.useMemo(() => (
    isOfficialMode && !canSelectProviderCategory
      ? [
          {
            value: OFFICIAL_GEMINI_CHANNEL_KEY,
            label: t('geminicli.provider.providerProfileOfficial'),
          },
        ]
      : [
          {
            value: CUSTOM_PROVIDER_ENDPOINT_KEY,
            label: t('geminicli.provider.providerProfileCustom'),
          },
          ...(canSelectProviderCategory
            ? [
                {
                  value: OFFICIAL_GEMINI_CHANNEL_KEY,
                  label: t('geminicli.provider.providerProfileOfficial'),
                },
              ]
            : []),
          ...getGatewayProviderProfilesForTool('gemini').flatMap((profile) => {
            const endpoints = profile.tools.gemini?.endpoints || [];
            return endpoints.map((endpoint) => ({
              value: toGatewayProviderEndpointKey(profile.id, endpoint.id),
              label: `${profile.label} / ${endpoint.label}`,
            }));
          }),
        ]
  ), [canSelectProviderCategory, gatewayProviderProfilesVersion, isOfficialMode, t]);
  const activeFormKey = React.useMemo(() => {
    if (!open) {
      return null;
    }

    const mode = provider ? (isCopy ? 'copy' : 'edit') : 'create';
    return [
      mode,
      provider?.id || '__new__',
      provider?.updatedAt || '',
      provider?.settingsConfig || '',
    ].join(':');
  }, [isCopy, open, provider]);
  const [initializedFormKey, setInitializedFormKey] = React.useState<string | null>(null);
  const isFormReady = activeFormKey !== null && initializedFormKey === activeFormKey;

  const modelOptions = React.useMemo(() => {
    const seenIds = new Set<string>();
    const options: { label: string; value: string }[] = [];

    DEFAULT_GEMINI_MODEL_OPTIONS.forEach((option) => {
      if (!seenIds.has(option.value)) {
        seenIds.add(option.value);
        options.push(option);
      }
    });

    fetchedModels.forEach((model) => {
      if (seenIds.has(model.id)) {
        return;
      }

      seenIds.add(model.id);
      const displayName = model.name || model.id;
      options.push({
        label: displayName && displayName !== model.id ? `${displayName} (${model.id})` : model.id,
        value: model.id,
      });
    });

    return options;
  }, [fetchedModels]);

  React.useEffect(() => {
    if (!open) {
      setInitializedFormKey(null);
      return;
    }

    const initialCategory = provider?.category || 'custom';
    const rawInitialConfig = provider
      ? parseSettingsConfig(provider.settingsConfig)
      : initialCategory === 'official'
        ? defaultOfficialConfig
        : defaultCustomConfig;
    const initialConfig = normalizeSettingsConfigForCategory(
      rawInitialConfig,
      initialCategory,
    ) as GeminiCliSettingsConfig;
    const initialBaseUrl = extractEnvString(initialConfig, 'GOOGLE_GEMINI_BASE_URL');
    const providerEndpointSelection = initialCategory === 'official'
      ? {
          providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
          providerEndpointId: undefined,
        }
      : inferGatewayProviderEndpointSelection({
          tool: 'gemini',
          meta: provider?.meta,
          providerType: provider?.meta?.providerType,
          apiFormat: provider?.meta?.apiFormat,
        });
    const providerEndpoint = providerEndpointSelection.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
      ? undefined
      : findGatewayProviderEndpoint(
          providerEndpointSelection.providerProfileId,
          'gemini',
          providerEndpointSelection.providerEndpointId,
        );
    const selectedBaseUrl = initialBaseUrl || providerEndpoint?.baseUrl || '';
    const selectedApiFormat = initialCategory === 'official'
      ? DEFAULT_GEMINI_API_FORMAT
      : providerEndpoint
        ? normalizeGeminiApiFormat(providerEndpoint.apiFormat)
        : normalizeGeminiApiFormat(provider?.meta?.apiFormat);

    setSelectedProviderCategory(initialCategory);
    setBillingConfig(getBillingConfigFromMeta(provider?.meta));
    setCustomHeaders(getCustomHeadersFromMeta(provider?.meta));
    setModelRewrites(getModelRewritesFromMeta(provider?.meta));
    setSettingsConfigValue(initialConfig);
    setSettingsConfigValid(true);
    setFetchedModels([]);
    form.setFieldsValue({
      name: isCopy ? `${provider?.name || ''} ${t('common.copy')}`.trim() : provider?.name,
      category: initialCategory,
      channel: initialCategory === 'official'
        ? OFFICIAL_GEMINI_CHANNEL_KEY
        : toGatewayProviderEndpointKey(
            providerEndpointSelection.providerProfileId,
            providerEndpointSelection.providerEndpointId,
          ),
      apiKey: extractGeminiApiKey(initialConfig),
      baseUrl: selectedBaseUrl,
      modelName: extractGeminiEnvModelName(initialConfig),
      apiFormat: selectedApiFormat,
      settingsConfig: initialConfig,
      notes: provider?.notes || '',
    });
    setInitializedFormKey(activeFormKey);
  }, [activeFormKey, form, isCopy, open, provider, t]);

  const handleCategoryChange = (category: string) => {
    if (!canSelectProviderCategory) {
      return;
    }

    setSelectedProviderCategory(category);
    const nextConfig = normalizeSettingsConfigForCategory(
      category === 'official' ? defaultOfficialConfig : defaultCustomConfig,
      category,
    );
    setSettingsConfigValue(nextConfig);
    setSettingsConfigValid(true);
    setFetchedModels([]);
    form.setFieldsValue({
      category,
      channel: category === 'official'
        ? OFFICIAL_GEMINI_CHANNEL_KEY
        : CUSTOM_PROVIDER_ENDPOINT_KEY,
      apiKey: extractGeminiApiKey(nextConfig),
      baseUrl: extractEnvString(nextConfig, 'GOOGLE_GEMINI_BASE_URL'),
      modelName: extractGeminiEnvModelName(nextConfig),
      apiFormat: DEFAULT_GEMINI_API_FORMAT,
      settingsConfig: nextConfig,
    });
  };

  const handleChannelChange = (channel: string) => {
    if (channel === OFFICIAL_GEMINI_CHANNEL_KEY) {
      if (!canSelectProviderCategory) {
        return;
      }
      handleCategoryChange('official');
      return;
    }

    const { providerProfileId, providerEndpointId } = parseGatewayProviderEndpointKey(channel);
    if (canSelectProviderCategory && providerCategory !== 'custom') {
      setSelectedProviderCategory('custom');
      form.setFieldsValue({ category: 'custom' });
    }

    if (providerProfileId === CUSTOM_PROVIDER_PROFILE_ID) {
      form.setFieldsValue({
        channel: CUSTOM_PROVIDER_ENDPOINT_KEY,
        apiFormat: form.getFieldValue('apiFormat') || DEFAULT_GEMINI_API_FORMAT,
      });
      return;
    }

    const endpoint = findGatewayProviderEndpoint(providerProfileId, 'gemini', providerEndpointId);
    if (!endpoint) {
      return;
    }

    const nextModelName = endpoint.model?.trim() || form.getFieldValue('modelName') || '';
    const nextConfig = normalizeSettingsConfigForCategory(
      syncDedicatedFieldsToSettingsConfig(
        settingsConfigValue || {},
        {
          baseUrl: endpoint.baseUrl,
          modelName: nextModelName,
        },
      ),
      'custom',
    );
    setSettingsConfigValue(nextConfig);
    setSettingsConfigValid(true);
    setFetchedModels([]);
    form.setFieldsValue({
      category: 'custom',
      channel: toGatewayProviderEndpointKey(providerProfileId, endpoint.id),
      apiFormat: normalizeGeminiApiFormat(endpoint.apiFormat),
      baseUrl: endpoint.baseUrl,
      modelName: nextModelName,
      settingsConfig: nextConfig,
    });
  };

  const handleApiKeyChange = (apiKey: string) => {
    const nextConfig = normalizeSettingsConfigForCategory(
      updateGeminiApiKey(settingsConfigValue, apiKey),
      'custom',
    );
    setSettingsConfigValue(nextConfig);
    setSettingsConfigValid(true);
    setFetchedModels([]);
    form.setFieldsValue({
      apiKey,
      settingsConfig: nextConfig,
    });
  };

  const handleBaseUrlChange = (baseUrl: string) => {
    const nextConfig = normalizeSettingsConfigForCategory(
      updateGeminiEnvValue(settingsConfigValue, 'GOOGLE_GEMINI_BASE_URL', baseUrl),
      'custom',
    );
    setSettingsConfigValue(nextConfig);
    setSettingsConfigValid(true);
    setFetchedModels([]);
    form.setFieldsValue({
      baseUrl,
      settingsConfig: nextConfig,
    });
  };

  const handleModelNameChange = (modelName: string) => {
    const nextConfig = normalizeSettingsConfigForCategory(
      updateGeminiEnvModelName(settingsConfigValue, modelName),
      providerCategory,
    );
    setSettingsConfigValue(nextConfig);
    setSettingsConfigValid(true);
    form.setFieldsValue({
      modelName,
      settingsConfig: nextConfig,
    });
  };

  const handleSettingsConfigChange = (value: unknown, isValid: boolean) => {
    const nextValue = isValid
      ? normalizeSettingsConfigForCategory(value, isOfficialMode ? 'official' : 'custom')
      : value;
    setSettingsConfigValue(nextValue);
    setSettingsConfigValid(isValid);

    if (isValid) {
      form.setFieldsValue({
        apiKey: extractGeminiApiKey(nextValue),
        baseUrl: extractEnvString(nextValue, 'GOOGLE_GEMINI_BASE_URL'),
        modelName: extractGeminiEnvModelName(nextValue),
      });
    }
  };

  const handleFetchModels = async () => {
    if (!settingsConfigValid) {
      message.warning(t('geminicli.fetchModels.configInvalid'));
      return;
    }

    if (isOfficialMode) {
      setLoadingModels(true);
      try {
        const response = await invoke<GeminiCliOfficialModelsResponse>('fetch_gemini_cli_official_models');
        const models = response.models.length > 0 ? response.models : GEMINI_CLI_OFFICIAL_MODELS;
        setFetchedModels(models);

        if (response.source === 'bundled') {
          message.info(t('geminicli.fetchModels.officialBundled', { count: models.length }));
        } else {
          message.success(t('geminicli.fetchModels.officialUpdated', { count: models.length }));
        }
      } catch (error) {
        console.error('Failed to fetch Gemini CLI official models:', error);
        setFetchedModels(GEMINI_CLI_OFFICIAL_MODELS);
        message.info(t('geminicli.fetchModels.officialBundled', { count: GEMINI_CLI_OFFICIAL_MODELS.length }));
      } finally {
        setLoadingModels(false);
      }
      return;
    }

    const latestSettingsConfig = normalizeSettingsConfigForCategory(
      syncDedicatedFieldsToSettingsConfig(
        settingsConfigValue || {},
        form.getFieldsValue(['apiKey', 'baseUrl', 'modelName']),
      ),
      'custom',
    );
    setSettingsConfigValue(latestSettingsConfig);
    const { baseUrl, apiKey, hasConfiguredBaseUrl } = resolveFetchModelsConfig(latestSettingsConfig);
    const selectedApiFormat = normalizeGeminiApiFormat(form.getFieldValue('apiFormat'));
    if (selectedApiFormat === 'gemini_native' && !apiKey && !hasConfiguredBaseUrl) {
      message.warning(t('geminicli.fetchModels.apiKeyRequired'));
      return;
    }
    if (selectedApiFormat !== 'gemini_native' && !hasConfiguredBaseUrl) {
      message.warning(t('geminicli.fetchModels.baseUrlRequired'));
      return;
    }

    setLoadingModels(true);
    try {
      const modelFetchRequest = selectedApiFormat === 'gemini_native'
        ? {
            baseUrl,
            apiKey: apiKey || undefined,
            apiType: 'native',
            sdkType: '@ai-sdk/google',
          }
        : selectedApiFormat === 'anthropic'
          ? {
              baseUrl,
              apiKey: apiKey || undefined,
              apiType: 'native',
              sdkType: '@ai-sdk/anthropic',
            }
          : {
              baseUrl,
              apiKey: apiKey || undefined,
              apiType: 'openai_compat',
              sdkType: '@ai-sdk/openai',
            };
      const response = await invoke<FetchModelsResponse>('fetch_provider_models', {
        request: {
          ...modelFetchRequest,
          headers: getEffectiveCustomHeaders(customHeaders),
        },
      });

      setFetchedModels(response.models);
      if (response.models.length > 0) {
        message.success(t('geminicli.fetchModels.success', { count: response.models.length }));
      } else {
        message.info(t('geminicli.fetchModels.noModels'));
      }
    } catch (error) {
      console.error('Failed to fetch Gemini CLI models:', error);
      message.error(t('geminicli.fetchModels.failed'));
    } finally {
      setLoadingModels(false);
    }
  };

  const handleSubmit = async () => {
    if (!settingsConfigValid || !isFormReady) {
      return;
    }

    const values = await form.validateFields();
    setLoading(true);
    try {
      const selectedCategory = (canSelectProviderCategory ? values.category : providerCategory) === 'official'
        ? 'official'
        : 'custom';
      const selectedChannel = typeof values.channel === 'string' ? values.channel : CUSTOM_PROVIDER_ENDPOINT_KEY;
      const { providerProfileId, providerEndpointId } = parseGatewayProviderEndpointKey(selectedChannel);
      const selectedEndpoint = selectedCategory === 'official' || providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(providerProfileId, 'gemini', providerEndpointId);
      const selectedApiFormat = selectedCategory === 'official'
        ? undefined
        : selectedEndpoint
          ? normalizeGeminiApiFormat(selectedEndpoint.apiFormat)
          : normalizeGeminiApiFormat(values.apiFormat);
      const gatewayProfile = selectedEndpoint
        ? toGatewayProviderProfileReference('gemini', providerProfileId, selectedEndpoint.id)
        : undefined;
      const latestSettingsConfig = syncDedicatedFieldsToSettingsConfig(
        settingsConfigValue || {},
        values,
      );
      const settingsConfigPayload = normalizeSettingsConfigForCategory(
        latestSettingsConfig,
        selectedCategory,
      );
      const settingsConfig = JSON.stringify(settingsConfigPayload, null, 2);
      await onSubmit({
        name: values.name,
        category: selectedCategory,
        settingsConfig,
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
        notes: values.notes,
      });
    } finally {
      setLoading(false);
    }
  };

  return (
    <Modal
      title={isEdit ? t('geminicli.provider.editProvider') : t('geminicli.provider.addProvider')}
      open={open}
      onCancel={onCancel}
      onOk={() => {
        void handleSubmit();
      }}
      confirmLoading={loading}
      okButtonProps={{ disabled: !settingsConfigValid || !isFormReady }}
      width={820}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      {isFormReady && (
        <Form form={form} layout="horizontal" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <Form.Item
            label={t('geminicli.provider.providerProfile')}
            required
            help={<Text type="secondary" style={{ fontSize: 12 }}>{t('geminicli.provider.providerProfileHelp')}</Text>}
          >
            <div className={isOfficialMode ? undefined : styles.providerProfileRow}>
              <Form.Item
                name="channel"
                noStyle
                rules={[{ required: true, message: t('common.error') }]}
              >
                <Select
                  options={channelOptions}
                  disabled={isOfficialMode && !canSelectProviderCategory}
                  onChange={handleChannelChange}
                />
              </Form.Item>
              {!isOfficialMode && (
                <Form.Item
                  name="apiFormat"
                  noStyle
                  initialValue={DEFAULT_GEMINI_API_FORMAT}
                >
                  <Select options={apiFormatOptions} disabled={!gatewaySupportsThisCli || !selectedIsCustomProviderProfile} />
                </Form.Item>
              )}
            </div>
          </Form.Item>

          <Form.Item
            name="name"
            label={t('geminicli.provider.formName')}
            rules={[{ required: true, message: t('geminicli.provider.nameRequired') }]}
          >
            <Input placeholder={t('geminicli.provider.namePlaceholder')} />
          </Form.Item>

          {!isOfficialMode && (
            <>
              <Form.Item
                name="baseUrl"
                label={t('geminicli.provider.baseUrl')}
                help={<Text type="secondary" style={{ fontSize: 12 }}>{t('geminicli.provider.baseUrlHelp')}</Text>}
              >
                <Input
                  placeholder={t('geminicli.provider.baseUrlPlaceholder')}
                  onChange={(event) => handleBaseUrlChange(event.target.value)}
                />
              </Form.Item>

              <Form.Item
                name="apiKey"
                label={t('geminicli.provider.apiKey')}
              >
                <Input
                  type={showApiKey ? 'text' : 'password'}
                  placeholder={t('geminicli.provider.apiKeyPlaceholder')}
                  onChange={(event) => handleApiKeyChange(event.target.value)}
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
            </>
          )}

          <Form.Item
            label={t('geminicli.provider.modelName')}
            help={<Text type="secondary" style={{ fontSize: 12 }}>{t('geminicli.provider.modelNameHelp')}</Text>}
          >
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%' }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <Form.Item name="modelName" noStyle>
                  <ImeSafeAutoComplete
                    allowClear
                    options={modelOptions}
                    placeholder={t('geminicli.provider.modelNamePlaceholder')}
                    filterOption={(inputValue, option) =>
                      (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
                        option?.value.toLowerCase().includes(inputValue.toLowerCase())) ?? false
                    }
                    onChange={handleModelNameChange}
                  />
                </Form.Item>
              </div>
              <Button
                icon={<CloudDownloadOutlined />}
                loading={loadingModels}
                onClick={handleFetchModels}
                disabled={!settingsConfigValid}
              >
                {t('geminicli.fetchModels.button')}
              </Button>
              {fetchedModels.length > 0 && (
                <Text type="secondary" style={{ whiteSpace: 'nowrap' }}>
                  {t('geminicli.fetchModels.loaded', { count: fetchedModels.length })}
                </Text>
              )}
            </div>
          </Form.Item>

          <Form.Item label={t('geminicli.provider.settingsConfig')} required>
            <JsonEditor
              key={`${activeFormKey}:${providerCategory}`}
              value={settingsConfigValue}
              onChange={handleSettingsConfigChange}
              height={180}
            />
          </Form.Item>

          {!settingsConfigValid && (
            <Alert
              type="error"
              showIcon
              message={t('geminicli.provider.settingsConfigInvalid')}
              style={{ marginBottom: 16 }}
            />
          )}

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
              title={t('geminicli.provider.notes')}
              placeholder={t('geminicli.provider.notesPlaceholder')}
              rows={3}
              resetKey={notesCollapseResetKey}
            />
          </Form.Item>
        </Form>
      )}
    </Modal>
  );
};

export default GeminiCliProviderFormModal;
