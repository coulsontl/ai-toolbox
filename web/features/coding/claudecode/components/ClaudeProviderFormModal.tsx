import React from 'react';
import { Modal, Form, Input, Select, Space, Button, Alert, message, AutoComplete, Checkbox, Dropdown } from 'antd';
import { EyeInvisibleOutlined, EyeOutlined, CloudDownloadOutlined, DownOutlined, ThunderboltOutlined } from '@ant-design/icons';
import { Settings2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import JsonEditor from '@/components/common/JsonEditor';
import { useAppStore } from '@/stores';
import type { ClaudeApiFormat, ClaudeCodeProvider, ClaudeProviderFormValues, ClaudeSettingsConfig, GatewayProviderMeta } from '@/types/claudecode';
import { readCurrentOpenCodeProviders } from '@/services/opencodeApi';
import BillingConfigCollapse from '@/features/coding/shared/providerBilling/BillingConfigCollapse';
import ProviderConfigCollapse from '@/features/coding/shared/providerConfig/ProviderConfigCollapse';
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
} from '@/features/coding/shared/gateway/providerProfiles';
import {
  getClaudeProviderModelConfig,
  hasClaudeOneMMarker,
  setClaudeOneMMarker,
  stripClaudeOneMMarker,
  type ClaudeModelRole,
} from '../utils/claudeModelConfig';
import styles from './ClaudeProviderFormModal.module.less';

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function toExtraSettingsEditorValue(rawConfig?: string): unknown {
  if (!rawConfig?.trim()) {
    return null;
  }

  try {
    const parsed = JSON.parse(rawConfig) as unknown;
    if (!isPlainObject(parsed)) {
      return rawConfig;
    }
    return parsed;
  } catch {
    return rawConfig;
  }
}

function parseExtraSettingsConfig(rawConfig?: string): string | undefined {
  const trimmedConfig = rawConfig?.trim();
  if (!trimmedConfig) {
    return undefined;
  }

  const parsed = JSON.parse(trimmedConfig) as unknown;
  if (!isPlainObject(parsed)) {
    throw new Error('Expected JSON object');
  }

  return JSON.stringify(parsed);
}

function hasNonEmptyExtraSettingsObject(rawConfig?: string): boolean {
  const trimmedConfig = rawConfig?.trim();
  if (!trimmedConfig) {
    return false;
  }

  try {
    const parsed = JSON.parse(trimmedConfig) as unknown;
    return isPlainObject(parsed) && Object.keys(parsed).length > 0;
  } catch {
    return false;
  }
}

const DEFAULT_CLAUDE_API_FORMAT: ClaudeApiFormat = 'anthropic';
const OFFICIAL_PROVIDER_ENDPOINT_KEY = '__official__:';

function normalizeClaudeApiFormat(value?: string): ClaudeApiFormat {
  if (value === 'openai_chat' || value === 'openai_responses' || value === 'gemini_native') {
    return value;
  }
  return DEFAULT_CLAUDE_API_FORMAT;
}

function mergeGatewayMetaIntoProviderMeta(
  meta: GatewayProviderMeta | undefined,
  apiFormat: ClaudeApiFormat | undefined,
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

// OpenCode 供应商展示类型
interface OpenCodeProviderDisplay {
  id: string;
  name: string;
  baseUrl: string | undefined;
  apiKey?: string;
  models: { id: string; name: string }[];
}

interface ClaudeProviderFormModalProps {
  open: boolean;
  provider?: ClaudeCodeProvider | null;
  isCopy?: boolean;
  mode?: 'manual' | 'import';
  onCancel: () => void;
  onSubmit: (values: ClaudeProviderFormValues) => Promise<void>;
}

// 获取模型 API 响应类型
interface FetchedModel {
  id: string;
  name?: string;
  ownedBy?: string;
}

interface FetchModelsResponse {
  models: FetchedModel[];
  total: number;
}

interface ModelRoleRow {
  role: ClaudeModelRole;
  label: string;
  model: string;
  displayName: string;
  modelField: 'sonnetModel' | 'opusModel' | 'haikuModel';
  displayNameField: 'sonnetModelName' | 'opusModelName' | 'haikuModelName';
  supportsOneM: boolean;
}

const ClaudeProviderFormModal: React.FC<ClaudeProviderFormModalProps> = ({
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

  // 从 OpenCode 导入相关状态
  const [openCodeProviders, setOpenCodeProviders] = React.useState<OpenCodeProviderDisplay[]>([]);
  const [selectedProvider, setSelectedProvider] = React.useState<OpenCodeProviderDisplay | null>(null);
  const [availableModels, setAvailableModels] = React.useState<{ id: string; name: string }[]>([]);
  const [loadingProviders, setLoadingProviders] = React.useState(false);
  const [processedBaseUrl, setProcessedBaseUrl] = React.useState<string>('');
  // 动态获取的模型列表
  const [fetchedModels, setFetchedModels] = React.useState<FetchedModel[]>([]);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [fetchApiType, setFetchApiType] = React.useState<'openai_compat' | 'native'>('native');
  // 当前表单的 baseUrl（用于匹配供应商）
  const [currentBaseUrl, setCurrentBaseUrl] = React.useState<string>('');
  const [providerCategory, setProviderCategory] = React.useState<'official' | 'custom'>('custom');
  const [extraSettingsValue, setExtraSettingsValue] = React.useState<unknown>(null);
  const [extraSettingsError, setExtraSettingsError] = React.useState<string>();
  const [advancedSettingsExpanded, setAdvancedSettingsExpanded] = React.useState(false);
  const [billingConfig, setBillingConfig] = React.useState(() => getBillingConfigFromMeta(provider?.meta));
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const extraSettingsRawRef = React.useRef('');
  const apiFormatOptions = React.useMemo(() => [
    {
      value: 'anthropic',
      label: t('claudecode.provider.apiFormatAnthropic'),
    },
    {
      value: 'openai_chat',
      label: t('claudecode.provider.apiFormatOpenAIChat'),
    },
    {
      value: 'openai_responses',
      label: t('claudecode.provider.apiFormatOpenAIResponses'),
    },
    {
      value: 'gemini_native',
      label: t('claudecode.provider.apiFormatGeminiNative'),
    },
  ], [t]);

  const isEdit = !!provider && !isCopy;
  const canSelectProviderCategory = !provider && mode === 'manual';
  const isOfficialMode = providerCategory === 'official';
  const watchOptions = React.useMemo(() => ({ form, preserve: true }), [form]);
  const selectedProviderProfileId = Form.useWatch('providerProfileId', watchOptions) as string | undefined;
  const selectedIsCustomProviderProfile = (selectedProviderProfileId || CUSTOM_PROVIDER_PROFILE_ID) === CUSTOM_PROVIDER_PROFILE_ID;
  const fallbackModel = Form.useWatch('model', watchOptions) || '';
  const sonnetModel = Form.useWatch('sonnetModel', watchOptions) || '';
  const sonnetModelName = Form.useWatch('sonnetModelName', watchOptions) || '';
  const opusModel = Form.useWatch('opusModel', watchOptions) || '';
  const opusModelName = Form.useWatch('opusModelName', watchOptions) || '';
  const haikuModel = Form.useWatch('haikuModel', watchOptions) || '';
  const haikuModelName = Form.useWatch('haikuModelName', watchOptions) || '';

  const modelRoleRows: ModelRoleRow[] = React.useMemo(() => [
    {
      role: 'sonnet',
      label: t('claudecode.model.roleSonnet'),
      model: sonnetModel,
      displayName: sonnetModelName,
      modelField: 'sonnetModel',
      displayNameField: 'sonnetModelName',
      supportsOneM: true,
    },
    {
      role: 'opus',
      label: t('claudecode.model.roleOpus'),
      model: opusModel,
      displayName: opusModelName,
      modelField: 'opusModel',
      displayNameField: 'opusModelName',
      supportsOneM: true,
    },
    {
      role: 'haiku',
      label: t('claudecode.model.roleHaiku'),
      model: haikuModel,
      displayName: haikuModelName,
      modelField: 'haikuModel',
      displayNameField: 'haikuModelName',
      supportsOneM: false,
    },
  ], [haikuModel, haikuModelName, opusModel, opusModelName, sonnetModel, sonnetModelName, t]);

  const providerEndpointOptions = React.useMemo(() => {
    if (isOfficialMode && !canSelectProviderCategory) {
      return [{
        value: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        label: t('claudecode.provider.providerProfileOfficial'),
      }];
    }

    return [
      {
      value: CUSTOM_PROVIDER_ENDPOINT_KEY,
      label: t('claudecode.provider.providerProfileCustom'),
      },
      ...(canSelectProviderCategory ? [{
        value: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        label: t('claudecode.provider.providerProfileOfficial'),
      }] : []),
      ...getGatewayProviderProfilesForTool('claude').flatMap((profile) => {
        const endpoints = profile.tools.claude?.endpoints || [];
        return endpoints.map((endpoint) => ({
          value: toGatewayProviderEndpointKey(profile.id, endpoint.id),
          label: `${profile.label} / ${endpoint.label}`,
        }));
      }),
    ];
  }, [canSelectProviderCategory, gatewayProviderProfilesVersion, isOfficialMode, t]);

  const getExtraSettingsErrorMessage = React.useCallback((error: unknown) => {
    if (error instanceof SyntaxError) {
      return t('claudecode.provider.extraSettingsInvalidJsonDetailed', { message: error.message });
    }
    if (error instanceof Error && error.message === 'Expected JSON object') {
      return t('claudecode.provider.extraSettingsInvalidObject');
    }
    const messageText = error instanceof Error ? error.message : String(error);
    return t('claudecode.provider.extraSettingsInvalidJsonDetailed', { message: messageText });
  }, [t]);

  const validateExtraSettingsEditorValue = React.useCallback((): string | undefined => {
    try {
      setExtraSettingsError(undefined);
      return parseExtraSettingsConfig(extraSettingsRawRef.current);
    } catch (error) {
      setExtraSettingsError(getExtraSettingsErrorMessage(error));
      return undefined;
    }
  }, [getExtraSettingsErrorMessage]);

  const handleExtraSettingsChange = React.useCallback((_value: unknown, isValid: boolean) => {
    if (!isValid) {
      setExtraSettingsError(t('claudecode.provider.extraSettingsInvalidJson'));
      return;
    }
    validateExtraSettingsEditorValue();
  }, [t, validateExtraSettingsEditorValue]);

  const handleExtraSettingsBlur = React.useCallback((_value: unknown, isValid: boolean) => {
    if (!isValid) {
      try {
        parseExtraSettingsConfig(extraSettingsRawRef.current);
      } catch (error) {
        setExtraSettingsError(getExtraSettingsErrorMessage(error));
      }
      return;
    }
    const parsedExtraSettingsConfig = validateExtraSettingsEditorValue();
    if (parsedExtraSettingsConfig === undefined) {
      setExtraSettingsValue(null);
      return;
    }
    setExtraSettingsValue(JSON.parse(parsedExtraSettingsConfig) as unknown);
  }, [getExtraSettingsErrorMessage, validateExtraSettingsEditorValue]);

  const handleExtraSettingsRawChange = React.useCallback((value: string) => {
    extraSettingsRawRef.current = value;
    setExtraSettingsValue(value);
  }, []);

  // 加载 OpenCode 中的供应商列表
  React.useEffect(() => {
    if (mode === 'import' || isEdit) {
      loadOpenCodeProviders();
    }
  }, [mode, isEdit]);

  // 初始化表单（组件挂载时执行一次）
  React.useEffect(() => {
    if (provider) {
      let settingsConfig: ClaudeSettingsConfig = {};
      try {
        settingsConfig = JSON.parse(provider.settingsConfig);
      } catch (error) {
        console.error('Failed to parse settingsConfig:', error);
      }

      const baseUrl = settingsConfig.env?.ANTHROPIC_BASE_URL || '';
      const modelConfig = getClaudeProviderModelConfig(settingsConfig);
      const nextProviderCategory = provider.category === 'official' ? 'official' : 'custom';
      const providerEndpointSelection = nextProviderCategory === 'official'
        ? {
            providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
            providerEndpointId: undefined,
          }
        : inferGatewayProviderEndpointSelection({
            tool: 'claude',
            providerType: provider.meta?.providerType,
            apiFormat: provider.meta?.apiFormat,
            baseUrl,
          });
      const providerEndpoint = providerEndpointSelection.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            providerEndpointSelection.providerProfileId,
            'claude',
            providerEndpointSelection.providerEndpointId,
          );
      const selectedBaseUrl = baseUrl || providerEndpoint?.baseUrl || '';
      const selectedApiFormat = providerEndpoint
        ? normalizeClaudeApiFormat(providerEndpoint.apiFormat)
        : normalizeClaudeApiFormat(provider.meta?.apiFormat);
      setProviderCategory(nextProviderCategory);
      setCurrentBaseUrl(selectedBaseUrl);
      setBillingConfig(getBillingConfigFromMeta(provider.meta));
      const nextExtraSettingsRaw = nextProviderCategory === 'official'
        ? ''
        : provider.extraSettingsConfig || '';
      setExtraSettingsValue(toExtraSettingsEditorValue(nextExtraSettingsRaw));
      setExtraSettingsError(undefined);
      setAdvancedSettingsExpanded(nextProviderCategory !== 'official' && hasNonEmptyExtraSettingsObject(nextExtraSettingsRaw));
      extraSettingsRawRef.current = nextExtraSettingsRaw;

      form.setFieldsValue({
        category: nextProviderCategory,
        name: provider.name,
        providerEndpointKey: nextProviderCategory === 'official'
          ? OFFICIAL_PROVIDER_ENDPOINT_KEY
          : toGatewayProviderEndpointKey(
              providerEndpointSelection.providerProfileId,
              providerEndpointSelection.providerEndpointId,
            ),
        providerProfileId: providerEndpointSelection.providerProfileId,
        providerEndpointId: providerEndpointSelection.providerEndpointId,
        baseUrl: selectedBaseUrl,
        apiKey: settingsConfig.env?.ANTHROPIC_AUTH_TOKEN || settingsConfig.env?.ANTHROPIC_API_KEY,
        apiFormat: selectedApiFormat,
        model: modelConfig.fallbackModel,
        haikuModel: modelConfig.roles.haiku.model,
        haikuModelName: modelConfig.roles.haiku.displayName,
        sonnetModel: modelConfig.roles.sonnet.model,
        sonnetModelName: modelConfig.roles.sonnet.displayName,
        opusModel: modelConfig.roles.opus.model,
        opusModelName: modelConfig.roles.opus.displayName,
        notes: provider.notes,
      });
    } else {
      form.resetFields();
      setProviderCategory('custom');
      setCurrentBaseUrl('');
      setBillingConfig(getBillingConfigFromMeta(undefined));
      setExtraSettingsValue(null);
      setExtraSettingsError(undefined);
      setAdvancedSettingsExpanded(false);
      extraSettingsRawRef.current = '';
      form.setFieldsValue({
        category: 'custom',
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiFormat: DEFAULT_CLAUDE_API_FORMAT,
      });
    }
  }, [provider, form]);

  React.useEffect(() => {
    if (!open) {
      return;
    }

    if (!provider && mode === 'manual') {
      setProviderCategory('custom');
      setCurrentBaseUrl('');
      setBillingConfig(getBillingConfigFromMeta(undefined));
      setExtraSettingsValue(null);
      setExtraSettingsError(undefined);
      setAdvancedSettingsExpanded(false);
      extraSettingsRawRef.current = '';
      form.setFieldsValue({
        category: 'custom',
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiFormat: DEFAULT_CLAUDE_API_FORMAT,
      });
    }
  }, [form, mode, open, provider]);

  const loadOpenCodeProviders = async () => {
    setLoadingProviders(true);
    try {
      const providers = await readCurrentOpenCodeProviders();

      // 直接读取 OpenCode 当前配置，避免把“我使用过的供应商”历史库当作当前导入源。
      const anthropicProviders: OpenCodeProviderDisplay[] = Object.entries(providers)
        .filter(([, providerConfig]) => providerConfig.npm === '@ai-sdk/anthropic')
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

      setOpenCodeProviders(anthropicProviders);
    } catch (error) {
      console.error('Failed to load OpenCode providers:', error);
      message.error(t('common.error'));
    } finally {
      setLoadingProviders(false);
    }
  };

  // 获取模型列表（调用 fetch_provider_models API）
  const handleFetchModels = async () => {
    const baseUrl = form.getFieldValue('baseUrl');
    const apiKey = form.getFieldValue('apiKey');

    if (!baseUrl) {
      message.warning(t('claudecode.fetchModels.baseUrlRequired'));
      return;
    }

    // 构建 customUrl：在 baseUrl 后追加 /v1/models
    const base = baseUrl.replace(/\/$/, '');
    const customUrl = `${base}/v1/models`;

    setLoadingModels(true);
    try {
      const response = await invoke<FetchModelsResponse>('fetch_provider_models', {
        request: {
          baseUrl: `${base}/v1`,
          apiKey,
          apiType: fetchApiType,
          sdkType: '@ai-sdk/anthropic',
          customUrl,
        },
      });

      setFetchedModels(response.models);
      if (response.models.length > 0) {
        message.success(t('claudecode.fetchModels.success', { count: response.models.length }));
      } else {
        message.info(t('claudecode.fetchModels.noModels'));
      }
    } catch (error) {
      console.error('Failed to fetch models:', error);
      message.error(t('claudecode.fetchModels.failed'));
    } finally {
      setLoadingModels(false);
    }
  };

  const handleProviderSelect = (providerId: string) => {
    const providerData = openCodeProviders.find((p) => p.id === providerId);
    if (!providerData) return;

    setSelectedProvider(providerData);
    setAvailableModels(providerData.models);

    // 处理 baseUrl：去掉末尾的 /v1 和末尾的 /
    let processedUrl = providerData.baseUrl || '';
    // 去掉末尾的 /v1
    if (processedUrl.endsWith('/v1')) {
      processedUrl = processedUrl.slice(0, -3);
    }
    // 去掉末尾的 /
    if (processedUrl.endsWith('/')) {
      processedUrl = processedUrl.slice(0, -1);
    }
    setProcessedBaseUrl(processedUrl);
    setCurrentBaseUrl(processedUrl);

    // 自动填充表单
    form.setFieldsValue({
      name: providerData.name,
      providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
      providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
      providerEndpointId: undefined,
      baseUrl: processedUrl,
      apiKey: providerData.apiKey || '',
      apiFormat: DEFAULT_CLAUDE_API_FORMAT,
    });
  };

  const handleProviderEndpointChange = (selectionKey: string) => {
    if (selectionKey === OFFICIAL_PROVIDER_ENDPOINT_KEY) {
      if (!canSelectProviderCategory) {
        return;
      }
      handleCategoryChange('official');
      return;
    }

    const { providerProfileId, providerEndpointId } = parseGatewayProviderEndpointKey(selectionKey);
    if (canSelectProviderCategory && providerCategory !== 'custom') {
      setProviderCategory('custom');
      form.setFieldsValue({ category: 'custom' });
    }

    if (providerProfileId === CUSTOM_PROVIDER_PROFILE_ID) {
      form.setFieldsValue({
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId,
        providerEndpointId: undefined,
        apiFormat: form.getFieldValue('apiFormat') || DEFAULT_CLAUDE_API_FORMAT,
      });
      return;
    }

    const endpoint = findGatewayProviderEndpoint(providerProfileId, 'claude', providerEndpointId);
    if (!endpoint) {
      return;
    }

    const endpointModel = endpoint.model?.trim();
    const nextModel = endpoint.models?.primary ?? endpointModel ?? form.getFieldValue('model');
    const nextHaikuModel = endpoint.models?.haiku ?? endpointModel ?? form.getFieldValue('haikuModel');
    const nextSonnetModel = endpoint.models?.sonnet ?? endpointModel ?? form.getFieldValue('sonnetModel');
    const nextOpusModel = endpoint.models?.opus ?? endpointModel ?? form.getFieldValue('opusModel');

    form.setFieldsValue({
      providerEndpointKey: toGatewayProviderEndpointKey(providerProfileId, endpoint.id),
      providerProfileId,
      providerEndpointId: endpoint.id,
      apiFormat: normalizeClaudeApiFormat(endpoint.apiFormat),
      baseUrl: endpoint.baseUrl,
      model: nextModel,
      haikuModel: nextHaikuModel,
      haikuModelName: nextHaikuModel,
      sonnetModel: nextSonnetModel,
      sonnetModelName: nextSonnetModel,
      opusModel: nextOpusModel,
      opusModelName: nextOpusModel,
    });
    setCurrentBaseUrl(endpoint.baseUrl);
  };

  const handleSubmit = async () => {
    try {
      // 只验证当前模式需要的字段
      const fieldsToValidate = mode === 'import'
        ? ['sourceProvider', 'name', 'baseUrl', 'apiKey', 'apiFormat', 'model', 'haikuModel', 'haikuModelName', 'sonnetModel', 'sonnetModelName', 'opusModel', 'opusModelName', 'notes']
        : [...(canSelectProviderCategory ? ['category'] : []), 'name', ...(!isOfficialMode ? ['providerEndpointKey', 'baseUrl', 'apiKey', 'apiFormat'] : []), 'model', 'haikuModel', 'haikuModelName', 'sonnetModel', 'sonnetModelName', 'opusModel', 'opusModelName', 'notes'];
      
      const values = await form.validateFields(fieldsToValidate);
      const submittedValues = {
        ...(form.getFieldsValue(true) as ClaudeProviderFormValues),
        ...values,
      };
      
      setLoading(true);
      
      const normalizedBaseUrl = submittedValues.baseUrl?.trim() || undefined;
      const normalizedApiKey = submittedValues.apiKey?.trim() || undefined;
      const selectedCategory = mode === 'import'
        ? 'custom'
        : (providerCategory === 'official' ? 'official' : 'custom');
      const selectedEndpoint = selectedCategory === 'official' || submittedValues.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            submittedValues.providerProfileId,
            'claude',
            submittedValues.providerEndpointId,
          );
      const selectedProfile = selectedEndpoint
        ? findGatewayProviderProfile(submittedValues.providerProfileId)
        : undefined;
      const selectedApiFormat = selectedCategory === 'official'
        ? undefined
        : selectedEndpoint
          ? normalizeClaudeApiFormat(selectedEndpoint.apiFormat)
          : normalizeClaudeApiFormat(submittedValues.apiFormat);
      const selectedBaseUrl = selectedCategory === 'official'
        ? undefined
        : normalizedBaseUrl ?? selectedEndpoint?.baseUrl;
      let extraSettingsConfig: string | undefined;
      try {
        extraSettingsConfig = selectedCategory === 'official'
          ? undefined
          : parseExtraSettingsConfig(extraSettingsRawRef.current);
      } catch (error) {
        setExtraSettingsError(getExtraSettingsErrorMessage(error));
        setAdvancedSettingsExpanded(true);
        return;
      }
      const formValues: ClaudeProviderFormValues = {
        name: submittedValues.name,
        category: selectedCategory,
        providerEndpointKey: selectedEndpoint
          ? toGatewayProviderEndpointKey(selectedProfile?.id || submittedValues.providerProfileId || '', selectedEndpoint.id)
          : CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: selectedEndpoint
          ? selectedProfile?.id
          : CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: selectedEndpoint?.id,
        baseUrl: mode === 'import'
          ? normalizedBaseUrl
          : selectedBaseUrl,
        apiKey: mode === 'import'
          ? normalizedApiKey
          : (selectedCategory === 'official' ? undefined : normalizedApiKey),
        model: submittedValues.model,
        haikuModel: submittedValues.haikuModel,
        haikuModelName: submittedValues.haikuModelName,
        sonnetModel: submittedValues.sonnetModel,
        sonnetModelName: submittedValues.sonnetModelName,
        opusModel: submittedValues.opusModel,
        opusModelName: submittedValues.opusModelName,
        extraSettingsConfig,
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
      setExtraSettingsValue(null);
      setExtraSettingsError(undefined);
      setAdvancedSettingsExpanded(false);
      extraSettingsRawRef.current = '';
      setSelectedProvider(null);
      setAvailableModels([]);
      onCancel();
    } catch (error) {
      console.error('Form validation failed:', error);
    } finally {
      setLoading(false);
    }
  };

  // 根据 baseUrl 匹配供应商的模型列表
  // OpenCode 的 URL 可能包含 /v1，所以用包含匹配
  const matchedProviderModels = React.useMemo(() => {
    if (!currentBaseUrl || openCodeProviders.length === 0) {
      return [];
    }

    // 标准化 URL：去掉末尾的 / 和 /v1
    const normalizeUrl = (url: string) => {
      let normalized = url.replace(/\/$/, '');
      if (normalized.endsWith('/v1')) {
        normalized = normalized.slice(0, -3);
      }
      return normalized.toLowerCase();
    };

    const normalizedCurrentUrl = normalizeUrl(currentBaseUrl);

    // 查找匹配的供应商
    const matchedProvider = openCodeProviders.find((p) => {
      if (!p.baseUrl) return false;
      const normalizedProviderUrl = normalizeUrl(p.baseUrl);
      // OpenCode 的 URL 包含 ClaudeCode 的 URL，或者反过来
      return normalizedProviderUrl.includes(normalizedCurrentUrl) ||
             normalizedCurrentUrl.includes(normalizedProviderUrl);
    });

    return matchedProvider?.models || [];
  }, [currentBaseUrl, openCodeProviders]);

  // 计算 AutoComplete 选项（使用动态获取的模型列表）
  const modelOptions = React.useMemo(() => {
    const options: { label: string; value: string }[] = [];
    const seenIds = new Set<string>();

    // 1. 添加动态获取的模型
    fetchedModels.forEach((model) => {
      if (!seenIds.has(model.id)) {
        seenIds.add(model.id);
        const name = model.name || model.id;
        options.push({
          label: name && name !== model.id ? `${name} (${model.id})` : model.id,
          value: model.id,
        });
      }
    });

    // 2. 添加根据 URL 匹配的供应商模型
    matchedProviderModels.forEach((model) => {
      if (!seenIds.has(model.id)) {
        seenIds.add(model.id);
        options.push({
          label: model.name && model.name !== model.id ? `${model.name} (${model.id})` : model.id,
          value: model.id,
        });
      }
    });

    return options;
  }, [fetchedModels, matchedProviderModels]);

  const filterModelOption = React.useCallback((inputValue: string, option?: { label: unknown; value: unknown }) => {
    const normalizedInput = inputValue.toLowerCase();
    return [option?.label, option?.value]
      .filter((item): item is string | number => item !== undefined && item !== null)
      .some((item) => String(item).toLowerCase().includes(normalizedInput));
  }, []);

  const handleRoleOneMChange = React.useCallback((row: ModelRoleRow, enabled: boolean) => {
    if (!row.supportsOneM) {
      return;
    }

    const previousModelBase = stripClaudeOneMMarker(row.model).trim();
    const nextModel = setClaudeOneMMarker(row.model, enabled);
    const nextModelBase = stripClaudeOneMMarker(nextModel).trim();
    const shouldSyncDisplayName =
      !row.displayName.trim() || row.displayName.trim() === previousModelBase;

    form.setFieldsValue({
      [row.modelField]: nextModel,
      ...(shouldSyncDisplayName ? { [row.displayNameField]: nextModelBase } : {}),
    });
  }, [form]);

  const handleQuickSetModels = React.useCallback(() => {
    const sourceModel = fallbackModel || sonnetModel || opusModel || haikuModel;
    const sourceModelBase = stripClaudeOneMMarker(sourceModel).trim();
    if (!sourceModelBase) {
      return;
    }

    const nextValues: Record<string, string> = {};
    modelRoleRows.forEach((row) => {
      const nextModel = row.supportsOneM
        ? setClaudeOneMMarker(sourceModel, hasClaudeOneMMarker(sourceModel))
        : sourceModelBase;
      nextValues[row.modelField] = nextModel;
      nextValues[row.displayNameField] = stripClaudeOneMMarker(nextModel).trim();
    });
    form.setFieldsValue(nextValues);
    message.success(t('claudecode.model.quickSetSuccess'));
  }, [fallbackModel, form, haikuModel, modelRoleRows, opusModel, sonnetModel, t]);

  const fetchApiTypeMenu = React.useMemo(() => ({
    selectedKeys: [fetchApiType],
    onClick: ({ key }: { key: string }) => {
      setFetchApiType(key === 'openai_compat' ? 'openai_compat' : 'native');
    },
    items: [
      {
        key: 'native',
        label: t('claudecode.fetchModels.native'),
      },
      {
        key: 'openai_compat',
        label: t('claudecode.fetchModels.openaiCompat'),
      },
    ],
  }), [fetchApiType, t]);

  const renderModelMappingSection = () => (
    <Form.Item wrapperCol={sectionWrapperCol}>
      <section className={styles.modelMappingSection}>
        <div className={styles.modelMappingHeader}>
          <div className={styles.modelMappingTitleBlock}>
            <div className={styles.modelMappingTitle}>{t('claudecode.model.mappingTitle')}</div>
            <div className={styles.modelMappingHint}>{t('claudecode.model.mappingHint')}</div>
          </div>
          <div className={styles.modelMappingActions}>
            <Button
              size="small"
              icon={<ThunderboltOutlined />}
              disabled={!fallbackModel && !sonnetModel && !opusModel && !haikuModel}
              onClick={handleQuickSetModels}
            >
              {t('claudecode.model.quickSetModels')}
            </Button>
            {!isOfficialMode && mode !== 'import' && (
              <Space.Compact>
                <Button
                  size="small"
                  icon={<CloudDownloadOutlined />}
                  loading={loadingModels}
                  onClick={handleFetchModels}
                >
                  {t('claudecode.fetchModels.button')}
                </Button>
                <Dropdown menu={fetchApiTypeMenu} trigger={['click']}>
                  <Button
                    size="small"
                    icon={<DownOutlined />}
                    aria-label={fetchApiType === 'native'
                      ? t('claudecode.fetchModels.native')
                      : t('claudecode.fetchModels.openaiCompat')}
                  />
                </Dropdown>
              </Space.Compact>
            )}
            {fetchedModels.length > 0 && mode !== 'import' && (
              <span className={styles.modelLoadedText}>
                {t('claudecode.fetchModels.loaded', { count: fetchedModels.length })}
              </span>
            )}
          </div>
        </div>

        <div className={styles.modelGridHeader}>
          <span>{t('claudecode.model.roleHeader')}</span>
          <span>{t('claudecode.model.displayNameHeader')}</span>
          <span>{t('claudecode.model.requestModelHeader')}</span>
          <span>{t('claudecode.model.oneMHeader')}</span>
        </div>

        <div className={styles.modelRows}>
          {modelRoleRows.map((row) => {
            const modelBase = stripClaudeOneMMarker(row.model);
            const usesOneM = row.supportsOneM && hasClaudeOneMMarker(row.model);
            return (
              <div key={row.role} className={styles.modelRow}>
                <div className={styles.modelRoleLabel}>{row.label}</div>
                <Form.Item name={row.displayNameField} noStyle>
                  <Input
                    placeholder={modelBase || t('claudecode.model.displayNamePlaceholder')}
                  />
                </Form.Item>
                <Form.Item
                  name={row.modelField}
                  noStyle
                  getValueFromEvent={(value: string) => {
                    const previousModelBase = stripClaudeOneMMarker(row.model).trim();
                    const nextModelBase = stripClaudeOneMMarker(value).trim();
                    const nextModel = row.supportsOneM
                      ? setClaudeOneMMarker(nextModelBase, hasClaudeOneMMarker(row.model))
                      : nextModelBase;
                    const shouldSyncDisplayName =
                      !row.displayName.trim() || row.displayName.trim() === previousModelBase;

                    if (shouldSyncDisplayName) {
                      // 使用 setTimeout 确保在下一个事件循环中更新，避免干扰当前输入
                      setTimeout(() => {
                        form.setFieldsValue({
                          [row.displayNameField]: nextModelBase,
                        });
                      }, 0);
                    }

                    return nextModel;
                  }}
                >
                  <AutoComplete
                    allowClear
                    options={modelOptions}
                    placeholder={t('claudecode.model.defaultModelPlaceholder')}
                    style={{ width: '100%' }}
                    filterOption={filterModelOption}
                    onClear={() => form.setFieldsValue({ [row.modelField]: '' })}
                  />
                </Form.Item>
                <div className={styles.oneMCell}>
                  {row.supportsOneM && (
                    <Checkbox
                      checked={usesOneM}
                      onChange={(event) => handleRoleOneMChange(row, event.target.checked)}
                    >
                      {t('claudecode.model.oneMLabel')}
                    </Checkbox>
                  )}
                </div>
              </div>
            );
          })}
        </div>

        <div className={styles.fallbackModel}>
          <div className={styles.fallbackModelLabel}>{t('claudecode.model.fallbackModel')}</div>
          <div className={styles.fallbackModelInput}>
            <Form.Item name="model" noStyle>
              <AutoComplete
                allowClear
                options={modelOptions}
                placeholder={t('claudecode.model.defaultModelPlaceholder')}
                style={{ width: '100%' }}
                filterOption={filterModelOption}
              />
            </Form.Item>
            <div className={styles.modelMappingHint}>
              {t('claudecode.model.fallbackModelHint')}
            </div>
          </div>
        </div>
      </section>
    </Form.Item>
  );

  const handleCategoryChange = (category: string) => {
    const nextCategory = category === 'official' ? 'official' : 'custom';
    setProviderCategory(nextCategory);

    if (nextCategory === 'official') {
      setCurrentBaseUrl('');
      setFetchedModels([]);
      setBillingConfig(getBillingConfigFromMeta(undefined));
      setExtraSettingsValue(null);
      setExtraSettingsError(undefined);
      setAdvancedSettingsExpanded(false);
      extraSettingsRawRef.current = '';
      form.setFieldsValue({
        category: 'official',
        baseUrl: undefined,
        apiKey: undefined,
        providerEndpointKey: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiFormat: DEFAULT_CLAUDE_API_FORMAT,
      });
    } else {
      form.setFieldsValue({
        category: 'custom',
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiFormat: form.getFieldValue('apiFormat') || DEFAULT_CLAUDE_API_FORMAT,
      });
    }
  };

  const renderManualTab = () => (
    <Form
      form={form}
      layout="horizontal"
      labelCol={labelCol}
      wrapperCol={wrapperCol}
    >
      <Form.Item
        label={t('claudecode.provider.providerProfile')}
        required
        help={<span style={{ fontSize: 12, color: 'var(--color-text-secondary)' }}>{t('claudecode.provider.providerProfileHelp')}</span>}
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
              initialValue={DEFAULT_CLAUDE_API_FORMAT}
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

      {isOfficialMode && (
        <Form.Item wrapperCol={{ offset: labelCol.span, span: wrapperCol.span }}>
          <div className={styles.officialModeNotice}>
            <div className={styles.officialModeAccent} aria-hidden="true" />
            <div className={styles.officialModeContent}>
              <div className={styles.officialModeTitle}>
                {t('claudecode.provider.officialModeTitle')}
              </div>
              <div className={styles.officialModeDescription}>
                {t('claudecode.provider.officialModeDescription')}
              </div>
            </div>
          </div>
        </Form.Item>
      )}

      <Form.Item
        name="name"
        label={t('claudecode.provider.formName')}
        rules={[{ required: true, message: t('common.error') }]}
      >
        <Input placeholder={t('claudecode.provider.namePlaceholder')} />
      </Form.Item>

      {!isOfficialMode && (
        <>
          <Form.Item
            name="baseUrl"
            label={t('claudecode.provider.baseUrl')}
            rules={[{ required: true, message: t('common.error') }]}
          >
            <Input
              placeholder={t('claudecode.provider.baseUrlPlaceholder')}
              onChange={(e) => setCurrentBaseUrl(e.target.value)}
            />
          </Form.Item>

          <Form.Item
            name="apiKey"
            label={t('claudecode.provider.apiKey')}
            rules={[{ required: true, message: t('common.error') }]}
          >
            <Input
              type={showApiKey ? 'text' : 'password'}
              placeholder={t('claudecode.provider.apiKeyPlaceholder')}
              addonAfter={
                <Button
                  type="text"
                  size="small"
                  icon={showApiKey ? <EyeInvisibleOutlined /> : <EyeOutlined />}
                  onClick={() => setShowApiKey(!showApiKey)}
                >
                  {showApiKey ? t('claudecode.provider.hideApiKey') : t('claudecode.provider.showApiKey')}
                </Button>
              }
            />
          </Form.Item>

        </>
      )}

      {renderModelMappingSection()}

      {!isOfficialMode && (
        <Form.Item wrapperCol={sectionWrapperCol}>
          <ProviderConfigCollapse
            title={t('claudecode.provider.advancedSettings')}
            expanded={advancedSettingsExpanded}
            onExpandedChange={setAdvancedSettingsExpanded}
            icon={<Settings2 />}
          >
            <div className={styles.extraSettingsContent}>
              <JsonEditor
                value={extraSettingsValue}
                onChange={handleExtraSettingsChange}
                onBlur={handleExtraSettingsBlur}
                onRawChange={handleExtraSettingsRawChange}
                onRawBlur={handleExtraSettingsRawChange}
                mode="text"
                height={180}
                minHeight={140}
                maxHeight={360}
                resizable
                className={styles.extraSettingsEditor}
                placeholder={t('claudecode.provider.extraSettingsPlaceholder')}
              />
              <div className={styles.extraSettingsHelp}>
                {extraSettingsError && (
                  <div className={styles.extraSettingsError}>
                    {extraSettingsError}
                  </div>
                )}
                <div className={styles.extraSettingsHint}>
                  {t('claudecode.provider.extraSettingsHint')}
                </div>
              </div>
            </div>
          </ProviderConfigCollapse>
        </Form.Item>
      )}

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
          title={t('claudecode.provider.notes')}
          placeholder={t('claudecode.provider.notesPlaceholder')}
          rows={3}
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
      >
        <Form.Item
          name="sourceProvider"
          label={t('claudecode.import.selectProvider')}
          rules={[{ required: true, message: t('common.error') }]}
        >
          <Select
            placeholder={t('claudecode.import.selectProviderPlaceholder')}
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
            message={t('claudecode.import.importInfo')}
            description={
              <Space orientation="vertical" size={4}>
                <div>• {t('claudecode.import.providerName')}: {selectedProvider.name}</div>
                <div>• {t('claudecode.import.baseUrl')}: {processedBaseUrl}</div>
                <div>• {t('claudecode.import.availableModels')}: {availableModels.length > 0 ? t('claudecode.import.modelsCount', { count: availableModels.length }) : '-'}</div>
              </Space>
            }
            type="success"
            showIcon
            style={{ marginBottom: 16 }}
          />
        )}

        <Form.Item name="name" label={t('claudecode.provider.formName')}>
          <Input placeholder={t('claudecode.provider.namePlaceholder')} disabled />
        </Form.Item>

        <Form.Item name="baseUrl" label={t('claudecode.provider.baseUrl')}>
          <Input disabled />
        </Form.Item>

        <Form.Item name="apiKey" label={t('claudecode.provider.apiKey')}>
          <Input type="password" disabled />
        </Form.Item>

        <Form.Item name="apiFormat" label={t('claudecode.provider.apiFormat')} initialValue={DEFAULT_CLAUDE_API_FORMAT}>
          <Select options={apiFormatOptions} disabled />
        </Form.Item>

        {availableModels.length > 0 && (
          <Alert
            message={t('claudecode.model.selectFromProvider')}
            type="info"
            showIcon
            style={{ marginBottom: 16 }}
          />
        )}

        {renderModelMappingSection()}

        <Form.Item name="notes" wrapperCol={sectionWrapperCol}>
          <ProviderNotesCollapse
            title={t('claudecode.provider.notes')}
            placeholder={t('claudecode.provider.notesPlaceholder')}
            rows={3}
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
          ? t('claudecode.provider.editProvider')
          : mode === 'import'
            ? t('claudecode.import.title')
            : t('claudecode.provider.addProvider')
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

export default ClaudeProviderFormModal;
