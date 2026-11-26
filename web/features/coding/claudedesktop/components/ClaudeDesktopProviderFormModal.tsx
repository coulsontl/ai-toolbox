import React from 'react';
import { Modal, Form, Input, Button, Checkbox, Space, Dropdown, Select, message, Tooltip } from 'antd';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';
import {
  EyeInvisibleOutlined,
  EyeOutlined,
  ThunderboltOutlined,
  CloudDownloadOutlined,
  DownOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import ProviderNotesCollapse from '@/features/coding/shared/providerConfig/ProviderNotesCollapse';
import CustomHeadersCollapse from '@/features/coding/shared/providerHeaders/CustomHeadersCollapse';
import {
  getCustomHeadersFromMeta,
  getEffectiveCustomHeaders,
  type CustomHeadersState,
} from '@/features/coding/shared/providerHeaders/customHeadersUtils';
import ModelRewritesCollapse from '@/features/coding/shared/providerModelRewrites/ModelRewritesCollapse';
import { useGatewaySupportedCliKeys } from '@/features/coding/shared/gateway/useGatewaySupportedCliKeys';
import {
  getModelRewritesFromMeta,
  type ModelRewritesState,
} from '@/features/coding/shared/providerModelRewrites/modelRewritesUtils';
import { useAppStore } from '@/stores';
import type {
  ClaudeDesktopFormValues,
  ClaudeDesktopProvider,
} from '@/types/claudedesktop';
import {
  CUSTOM_PROVIDER_ENDPOINT_KEY,
  CUSTOM_PROVIDER_PROFILE_ID,
  findGatewayProviderEndpoint,
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
  parseClaudeSettingsConfig,
  type ClaudeModelRole,
} from '../../claudecode/utils/claudeModelConfig';
import styles from './ClaudeDesktopProviderFormModal.module.less';

interface ClaudeDesktopProviderFormModalProps {
  open: boolean;
  provider?: ClaudeDesktopProvider | null;
  isCopy?: boolean;
  onCancel: () => void;
  onSubmit: (values: ClaudeDesktopFormValues) => Promise<void>;
}

// Fetch-models API response type.
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
  modelField: 'sonnetModel' | 'opusModel' | 'fableModel' | 'haikuModel';
  displayNameField: 'sonnetModelName' | 'opusModelName' | 'fableModelName' | 'haikuModelName';
  tierAliasField: 'sonnetTierAlias' | 'opusTierAlias' | 'fableTierAlias' | 'haikuTierAlias';
  tierAlias: string;
  supportsOneM: boolean;
}

/** Claude Desktop `anthropicFamilyTier` legal values. */
const TIER_ALIAS_OPTIONS: Array<{ value: string; labelKey: string }> = [
  { value: '', labelKey: 'claudecode.model.tierAliasNone' },
  { value: 'sonnet', labelKey: 'claudecode.model.roleSonnet' },
  { value: 'opus', labelKey: 'claudecode.model.roleOpus' },
  { value: 'fable', labelKey: 'claudecode.model.roleFable' },
  { value: 'haiku', labelKey: 'claudecode.model.roleHaiku' },
  { value: 'mythos', labelKey: 'claudecode.model.roleMythos' },
];

type ClaudeDesktopApiFormat = 'anthropic' | 'openai_chat' | 'openai_responses' | 'gemini_native';

const DEFAULT_API_FORMAT: ClaudeDesktopApiFormat = 'anthropic';
const OFFICIAL_PROVIDER_ENDPOINT_KEY = '__official__:';

function normalizeApiFormat(value?: string): ClaudeDesktopApiFormat {
  if (value === 'openai_chat' || value === 'openai_responses' || value === 'gemini_native') {
    return value;
  }
  return DEFAULT_API_FORMAT;
}

/** claude-safe route_id per role, mirroring cc-switch CLAUDE_DESKTOP_ROLE_ROUTE_IDS. */
const CLAUDE_DESKTOP_ROLE_ROUTE_IDS: Record<string, string> = {
  sonnet: 'claude-sonnet-5',
  opus: 'claude-opus-5',
  fable: 'claude-fable-5',
  haiku: 'claude-haiku-4-5',
};

const ClaudeDesktopProviderFormModal: React.FC<ClaudeDesktopProviderFormModalProps> = ({
  open,
  provider,
  isCopy = false,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const language = useAppStore((state) => state.language);
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);
  const [fetchedModels, setFetchedModels] = React.useState<FetchedModel[]>([]);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [fetchApiType, setFetchApiType] = React.useState<'openai_compat' | 'native'>('native');
  const [providerCategory, setProviderCategory] = React.useState<'official' | 'custom'>('custom');
  const [customHeaders, setCustomHeaders] = React.useState<CustomHeadersState>(() =>
    getCustomHeadersFromMeta(provider?.meta),
  );
  const [modelRewrites, setModelRewrites] = React.useState<ModelRewritesState>(() =>
    getModelRewritesFromMeta(provider?.meta),
  );
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );

  const labelCol = { span: language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };
  const sectionWrapperCol = { span: 24 };
  const notesCollapseResetKey = `${open ? 'open' : 'closed'}:${provider?.id ?? 'new'}:${isCopy ? 'copy' : 'normal'}`;

  const isEdit = !!provider && !isCopy;
  const isOfficialMode = providerCategory === 'official';

  const apiFormatOptions = React.useMemo(() => [
    { value: 'anthropic', label: t('claudecode.provider.apiFormatAnthropic') },
    { value: 'openai_chat', label: t('claudecode.provider.apiFormatOpenAIChat') },
    { value: 'openai_responses', label: t('claudecode.provider.apiFormatOpenAIResponses') },
    { value: 'gemini_native', label: t('claudecode.provider.apiFormatGeminiNative') },
  ], [t]);

  /**
   * 「官方渠道」 is not a choice: the official row is a fixed list member the
   * backend seeds (see `ensure_official_provider_row`), not something the user
   * creates. The form therefore offers only custom/gateway channels when
   * creating. Editing an existing official row still shows its channel, as a
   * single read-only option — dropping it would leave the required select
   * empty, which is exactly the "disabled + required" trap that made official
   * saves fail before.
   */
  const providerEndpointOptions = React.useMemo(() => (
    isOfficialMode
      ? [{ value: OFFICIAL_PROVIDER_ENDPOINT_KEY, label: t('claudecode.provider.providerProfileOfficial') }]
      : [
          { value: CUSTOM_PROVIDER_ENDPOINT_KEY, label: t('claudecode.provider.providerProfileCustom') },
          ...getGatewayProviderProfilesForTool('claude_desktop').flatMap((profile) => {
            const endpoints = profile.tools.claude_desktop?.endpoints || [];
            return endpoints.map((endpoint) => ({
              value: toGatewayProviderEndpointKey(profile.id, endpoint.id),
              label: `${profile.label} / ${endpoint.label}`,
            }));
          }),
        ]
  ), [gatewayProviderProfilesVersion, isOfficialMode, t]);

  const watchOptions = React.useMemo(() => ({ form, preserve: true }), [form]);
  const selectedProviderProfileId = Form.useWatch('providerProfileId', watchOptions) as string | undefined;
  const selectedIsCustomProviderProfile = (selectedProviderProfileId || CUSTOM_PROVIDER_PROFILE_ID) === CUSTOM_PROVIDER_PROFILE_ID;
  // A protocol selector only means something when the gateway can take this
  // CLI over; otherwise the upstream format is fixed by the channel.
  const { isGatewaySupported } = useGatewaySupportedCliKeys();
  const gatewaySupportsThisCli = isGatewaySupported('claude_desktop') !== false;

  const fallbackModel = Form.useWatch('model', watchOptions) || '';
  const sonnetModel = Form.useWatch('sonnetModel', watchOptions) || '';
  const sonnetModelName = Form.useWatch('sonnetModelName', watchOptions) || '';
  const opusModel = Form.useWatch('opusModel', watchOptions) || '';
  const opusModelName = Form.useWatch('opusModelName', watchOptions) || '';
  const fableModel = Form.useWatch('fableModel', watchOptions) || '';
  const fableModelName = Form.useWatch('fableModelName', watchOptions) || '';
  const haikuModel = Form.useWatch('haikuModel', watchOptions) || '';
  const haikuModelName = Form.useWatch('haikuModelName', watchOptions) || '';
  const sonnetTierAlias = Form.useWatch('sonnetTierAlias', watchOptions) || '';
  const opusTierAlias = Form.useWatch('opusTierAlias', watchOptions) || '';
  const fableTierAlias = Form.useWatch('fableTierAlias', watchOptions) || '';
  const haikuTierAlias = Form.useWatch('haikuTierAlias', watchOptions) || '';

  const modelRoleRows: ModelRoleRow[] = React.useMemo(() => [
    {
      role: 'sonnet',
      label: t('claudecode.model.roleSonnet'),
      model: sonnetModel,
      displayName: sonnetModelName,
      modelField: 'sonnetModel',
      displayNameField: 'sonnetModelName',
      tierAliasField: 'sonnetTierAlias',
      tierAlias: sonnetTierAlias,
      supportsOneM: true,
    },
    {
      role: 'opus',
      label: t('claudecode.model.roleOpus'),
      model: opusModel,
      displayName: opusModelName,
      modelField: 'opusModel',
      displayNameField: 'opusModelName',
      tierAliasField: 'opusTierAlias',
      tierAlias: opusTierAlias,
      supportsOneM: true,
    },
    {
      role: 'fable',
      label: t('claudecode.model.roleFable'),
      model: fableModel,
      displayName: fableModelName,
      modelField: 'fableModel',
      displayNameField: 'fableModelName',
      tierAliasField: 'fableTierAlias',
      tierAlias: fableTierAlias,
      supportsOneM: true,
    },
    {
      role: 'haiku',
      label: t('claudecode.model.roleHaiku'),
      model: haikuModel,
      displayName: haikuModelName,
      modelField: 'haikuModel',
      displayNameField: 'haikuModelName',
      tierAliasField: 'haikuTierAlias',
      tierAlias: haikuTierAlias,
      supportsOneM: true,
    },
  ], [fableModel, fableModelName, fableTierAlias, haikuModel, haikuModelName, haikuTierAlias, opusModel, opusModelName, opusTierAlias, sonnetModel, sonnetModelName, sonnetTierAlias, t]);

  // Initialize the form when the modal opens.
  React.useEffect(() => {
    if (!open) {
      return;
    }
    setFetchedModels([]);
    if (provider) {
      const settingsConfig = parseClaudeSettingsConfig(provider.settingsConfig);
      const baseUrl = settingsConfig.env?.ANTHROPIC_BASE_URL || '';
      const apiKey = settingsConfig.env?.ANTHROPIC_AUTH_TOKEN || settingsConfig.env?.ANTHROPIC_API_KEY || '';
      const routes = provider.meta?.claudeDesktopModelRoutes;
      const modelConfig = getClaudeProviderModelConfig(settingsConfig);
      // Prefer the role model mapping from meta.claudeDesktopModelRoutes (the
      // shape Claude Desktop consumes); env model fields backfill older providers
      // created before the routes-based shape.
      const roleModel = (role: 'sonnet' | 'opus' | 'fable' | 'haiku') => {
        const route = routes?.[CLAUDE_DESKTOP_ROLE_ROUTE_IDS[role]];
        if (route) {
          // Stored route carries 1M intent as `supports1m` (model is stripped of
          // the [1m] marker); rebuild the marker so the checkbox reflects state.
          return route.supports1m ? setClaudeOneMMarker(route.model, true) : route.model;
        }
        return modelConfig.roles[role].model;
      };
      const roleModelName = (role: 'sonnet' | 'opus' | 'fable' | 'haiku') => {
        const route = routes?.[CLAUDE_DESKTOP_ROLE_ROUTE_IDS[role]];
        return route?.labelOverride || route?.model || modelConfig.roles[role].displayName;
      };
      const roleTierAlias = (role: 'sonnet' | 'opus' | 'fable' | 'haiku') => {
        const route = routes?.[CLAUDE_DESKTOP_ROLE_ROUTE_IDS[role]];
        return route?.tierAlias || '';
      };
      const nextProviderCategory = provider.category === 'official' ? 'official' : 'custom';
      const providerEndpointSelection = nextProviderCategory === 'official'
        ? { providerProfileId: CUSTOM_PROVIDER_PROFILE_ID, providerEndpointId: undefined }
        : inferGatewayProviderEndpointSelection({
            tool: 'claude_desktop',
            meta: provider.meta,
            providerType: provider.meta?.providerType,
            apiFormat: provider.meta?.apiFormat,
          });
      const providerEndpoint = providerEndpointSelection.providerProfileId === CUSTOM_PROVIDER_PROFILE_ID
        ? undefined
        : findGatewayProviderEndpoint(
            providerEndpointSelection.providerProfileId,
            'claude_desktop',
            providerEndpointSelection.providerEndpointId,
          );
      const selectedBaseUrl = baseUrl || providerEndpoint?.baseUrl || '';
      const selectedApiFormat = providerEndpoint
        ? normalizeApiFormat(providerEndpoint.apiFormat)
        : normalizeApiFormat(provider.meta?.apiFormat);
      setProviderCategory(nextProviderCategory);
      setCustomHeaders(getCustomHeadersFromMeta(provider.meta));
      setModelRewrites(getModelRewritesFromMeta(provider.meta));

      form.setFieldsValue({
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
        apiKey,
        apiFormat: selectedApiFormat,
        model: modelConfig.fallbackModel,
        haikuModel: roleModel('haiku'),
        haikuModelName: roleModelName('haiku'),
        sonnetModel: roleModel('sonnet'),
        sonnetModelName: roleModelName('sonnet'),
        opusModel: roleModel('opus'),
        opusModelName: roleModelName('opus'),
        fableModel: roleModel('fable'),
        fableModelName: roleModelName('fable'),
        sonnetTierAlias: roleTierAlias('sonnet'),
        opusTierAlias: roleTierAlias('opus'),
        fableTierAlias: roleTierAlias('fable'),
        haikuTierAlias: roleTierAlias('haiku'),
        notes: provider.notes,
      });
    } else {
      form.resetFields();
      setProviderCategory('custom');
      setCustomHeaders(getCustomHeadersFromMeta(undefined));
      setModelRewrites(getModelRewritesFromMeta(undefined));
      form.setFieldsValue({
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiFormat: DEFAULT_API_FORMAT,
      });
    }
  }, [open, provider, form]);

  const handleProviderEndpointChange = (selectionKey: string) => {
    if (selectionKey === OFFICIAL_PROVIDER_ENDPOINT_KEY) {
      setProviderCategory('official');
      setFetchedModels([]);
      setCustomHeaders(getCustomHeadersFromMeta(undefined));
      setModelRewrites(getModelRewritesFromMeta(undefined));
      form.setFieldsValue({
        baseUrl: undefined,
        apiKey: undefined,
        providerEndpointKey: OFFICIAL_PROVIDER_ENDPOINT_KEY,
        providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
        providerEndpointId: undefined,
        apiFormat: DEFAULT_API_FORMAT,
        model: undefined,
        haikuModel: undefined,
        haikuModelName: undefined,
        sonnetModel: undefined,
        sonnetModelName: undefined,
        opusModel: undefined,
        opusModelName: undefined,
        fableModel: undefined,
        fableModelName: undefined,
        sonnetTierAlias: undefined,
        opusTierAlias: undefined,
        fableTierAlias: undefined,
        haikuTierAlias: undefined,
      });
      return;
    }

    setProviderCategory('custom');
    const { providerProfileId, providerEndpointId } = parseGatewayProviderEndpointKey(selectionKey);
    if (providerProfileId === CUSTOM_PROVIDER_PROFILE_ID) {
      form.setFieldsValue({
        providerEndpointKey: CUSTOM_PROVIDER_ENDPOINT_KEY,
        providerProfileId,
        providerEndpointId: undefined,
        apiFormat: form.getFieldValue('apiFormat') || DEFAULT_API_FORMAT,
      });
      return;
    }

    const endpoint = findGatewayProviderEndpoint(providerProfileId, 'claude_desktop', providerEndpointId);
    if (!endpoint) {
      return;
    }
    const endpointModel = endpoint.model?.trim();
    const nextModel = endpoint.models?.primary ?? endpointModel ?? form.getFieldValue('model');
    const nextHaikuModel = endpoint.models?.haiku ?? endpointModel ?? form.getFieldValue('haikuModel');
    const nextSonnetModel = endpoint.models?.sonnet ?? endpointModel ?? form.getFieldValue('sonnetModel');
    const nextOpusModel = endpoint.models?.opus ?? endpointModel ?? form.getFieldValue('opusModel');
    const nextFableModel = endpoint.models?.fable ?? '';
    form.setFieldsValue({
      providerEndpointKey: toGatewayProviderEndpointKey(providerProfileId, endpoint.id),
      providerProfileId,
      providerEndpointId: endpoint.id,
      apiFormat: normalizeApiFormat(endpoint.apiFormat),
      baseUrl: endpoint.baseUrl,
      model: nextModel,
      haikuModel: nextHaikuModel,
      haikuModelName: nextHaikuModel,
      sonnetModel: nextSonnetModel,
      sonnetModelName: nextSonnetModel,
      opusModel: nextOpusModel,
      opusModelName: nextOpusModel,
      fableModel: nextFableModel,
      fableModelName: nextFableModel,
    });
  };

  const handleFetchModels = async () => {
    const baseUrl = form.getFieldValue('baseUrl');
    const apiKey = form.getFieldValue('apiKey');

    if (!baseUrl) {
      message.warning(t('claudecode.fetchModels.baseUrlRequired'));
      return;
    }

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
          headers: getEffectiveCustomHeaders(customHeaders),
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

  const modelOptions = React.useMemo(() => {
    return fetchedModels.map((model) => {
      const name = model.name || model.id;
      return {
        label: name && name !== model.id ? `${name} (${model.id})` : model.id,
        value: model.id,
      };
    });
  }, [fetchedModels]);

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
    const sourceModel = fallbackModel || sonnetModel || opusModel || fableModel || haikuModel;
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
  }, [fableModel, fallbackModel, form, haikuModel, modelRoleRows, opusModel, sonnetModel, t]);

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

  const handleSubmit = async () => {
    let values;
    try {
      values = await form.validateFields();
    } catch {
      return;
    }

    setLoading(true);
    try {
      await onSubmit({
        name: values.name,
        category: providerCategory,
        providerEndpointKey: values.providerEndpointKey,
        providerProfileId: values.providerProfileId,
        providerEndpointId: values.providerEndpointId,
        apiFormat: providerCategory === 'official' ? undefined : normalizeApiFormat(values.apiFormat),
        baseUrl: values.baseUrl?.trim() || undefined,
        apiKey: values.apiKey?.trim() || undefined,
        model: values.model,
        haikuModel: values.haikuModel,
        haikuModelName: values.haikuModelName,
        sonnetModel: values.sonnetModel,
        sonnetModelName: values.sonnetModelName,
        opusModel: values.opusModel,
        opusModelName: values.opusModelName,
        fableModel: values.fableModel,
        fableModelName: values.fableModelName,
        sonnetTierAlias: values.sonnetTierAlias,
        opusTierAlias: values.opusTierAlias,
        fableTierAlias: values.fableTierAlias,
        haikuTierAlias: values.haikuTierAlias,
        notes: values.notes,
        customHeaders: providerCategory === 'official'
          ? { enabled: false, headers: [] }
          : customHeaders,
        modelRewrites: providerCategory === 'official'
          ? { enabled: false, rewrites: [] }
          : modelRewrites,
      });
      form.resetFields();
      setFetchedModels([]);
      onCancel();
    } catch (error) {
      console.error('Failed to save Claude Desktop provider:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const renderModelMappingSection = () => (
    <Form.Item wrapperCol={sectionWrapperCol}>
      <section className={styles.modelMappingSection}>
        <div className={styles.modelMappingHeader}>
          <div className={styles.modelMappingTitleBlock}>
            <div className={styles.modelMappingTitle}>{t('claudecode.model.mappingTitle')}</div>
            <div className={styles.modelMappingHint}>{t('claudecode.model.mappingHint')}</div>
          </div>
          <div className={styles.modelMappingActions}>
            {!isOfficialMode && (
              <>
                <Tooltip title={t('claudecode.model.quickSetTooltip')}>
                  <Button
                    size="small"
                    icon={<ThunderboltOutlined />}
                    disabled={!fallbackModel && !sonnetModel && !opusModel && !fableModel && !haikuModel}
                    onClick={handleQuickSetModels}
                  >
                    {t('claudecode.model.quickSetModels')}
                  </Button>
                </Tooltip>
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
                {fetchedModels.length > 0 && (
                  <span className={styles.modelLoadedText}>
                    {t('claudecode.fetchModels.loaded', { count: fetchedModels.length })}
                  </span>
                )}
              </>
            )}
          </div>
        </div>

        <div className={styles.modelGridHeader}>
          <span>{t('claudecode.model.roleHeader')}</span>
          <span>{t('claudecode.model.displayNameHeader')}</span>
          <span>{t('claudecode.model.requestModelHeader')}</span>
          <span>{t('claudecode.model.oneMHeader')}</span>
          <span>{t('claudecode.model.tierAliasHeader')}</span>
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
                  // The input edits the stripped base only; the 1M state lives in the
                  // checkbox. Rendering the stored raw value would let trailing edits
                  // (e.g. backspacing the "]") re-append the marker and pile up
                  // garbage like `xxx[1M][1M]`.
                  getValueProps={(value) => ({
                    value: stripClaudeOneMMarker(typeof value === 'string' ? value : ''),
                  })}
                  getValueFromEvent={(value: string) => {
                    const previousModelBase = stripClaudeOneMMarker(row.model).trim();
                    const nextModelBase = stripClaudeOneMMarker(value).trim();
                    const nextModel = row.supportsOneM
                      ? setClaudeOneMMarker(nextModelBase, hasClaudeOneMMarker(row.model))
                      : nextModelBase;
                    const shouldSyncDisplayName =
                      !row.displayName.trim() || row.displayName.trim() === previousModelBase;

                    if (shouldSyncDisplayName) {
                      // Defer the display-name sync so it does not fight the current input.
                      setTimeout(() => {
                        form.setFieldsValue({
                          [row.displayNameField]: nextModelBase,
                        });
                      }, 0);
                    }

                    return nextModel;
                  }}
                >
                  <ImeSafeAutoComplete
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
                <Form.Item name={row.tierAliasField} noStyle initialValue="">
                  <Select
                    options={TIER_ALIAS_OPTIONS.map((option) => ({
                      value: option.value,
                      label: t(option.labelKey),
                    }))}
                    style={{ width: '100%' }}
                  />
                </Form.Item>
              </div>
            );
          })}
        </div>

        <div className={styles.fallbackModel}>
          <div className={styles.fallbackModelLabel}>{t('claudecode.model.fallbackModel')}</div>
          <div className={styles.fallbackModelInput}>
            <Form.Item name="model" noStyle>
              <ImeSafeAutoComplete
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

  /**
   * The official row's explanation.
   *
   * Its wording is Claude Desktop's own now. The shared `claudecode.*` copy
   * says "use the account Claude Code is currently signed in with", which is
   * the wrong CLI and the wrong files — Desktop's 1P mode is restored by
   * rewriting `deploymentMode` and dropping the 3P profile, not by reading a
   * CLI login.
   */
  const renderOfficialModeNotice = () => (
    <Form.Item wrapperCol={{ offset: labelCol.span, span: wrapperCol.span }}>
      <div className={styles.officialModeNotice}>
        <div className={styles.officialModeAccent} aria-hidden="true" />
        <div className={styles.officialModeContent}>
          <div className={styles.officialModeTitle}>
            {t('claudedesktop.provider.officialModeTitle')}
          </div>
          <div className={styles.officialModeDescription}>
            {t('claudedesktop.provider.officialModeDescription')}
          </div>
        </div>
      </div>
    </Form.Item>
  );

  return (
    <Modal
      title={
        isEdit
          ? '编辑 Claude Desktop 供应商'
          : `新增 Claude Desktop 供应商${isCopy ? '（复制）' : ''}`
      }
      open={open}
      onCancel={onCancel}
      onOk={handleSubmit}
      confirmLoading={loading}
      width={800}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={labelCol}
        wrapperCol={wrapperCol}
      >
        <Form.Item
          label={t('claudecode.provider.providerProfile')}
          required
          // In official mode the select holds the row's fixed identity, not a
          // choice, so it explains nothing about channels.
          help={isOfficialMode ? undefined : (
            <span style={{ fontSize: 12, color: 'var(--color-text-secondary)' }}>
              {t('claudecode.provider.providerProfileHelp')}
            </span>
          )}
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
                disabled={isEdit || isOfficialMode}
                onChange={handleProviderEndpointChange}
              />
            </Form.Item>
            {!isOfficialMode && (
              <Form.Item
                name="apiFormat"
                noStyle
                initialValue={DEFAULT_API_FORMAT}
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

        {isOfficialMode && renderOfficialModeNotice()}

        {/* Editable in official mode too: the card titles the row 官方账号, and
            whatever the user names it here is what the tray menu and the
            favorite history show. It must never be `disabled` while required —
            that combination can only fail validation, which is how official
            saves used to end in 「操作失败」. */}
        <Form.Item
          name="name"
          label="名称"
          rules={[{ required: true, message: t('claudedesktop.provider.nameRequired') }]}
        >
          <Input placeholder="供应商名称" />
        </Form.Item>

        {!isOfficialMode && (
          <>
            <Form.Item
              name="baseUrl"
              label="Base URL"
              rules={[{ required: true, message: t('claudedesktop.provider.baseUrlRequired') }]}
            >
              <Input placeholder="https://api.anthropic.com" />
            </Form.Item>

            <Form.Item
              name="apiKey"
              label="API Key"
              rules={[{ required: true, message: t('claudedesktop.provider.apiKeyRequired') }]}
            >
              <Input
                type={showApiKey ? 'text' : 'password'}
                placeholder="API Key"
                addonAfter={
                  <button
                    type="button"
                    onClick={() => setShowApiKey(!showApiKey)}
                    style={{
                      border: 'none',
                      background: 'transparent',
                      cursor: 'pointer',
                      padding: 0,
                      color: 'var(--color-text-secondary)',
                    }}
                  >
                    {showApiKey ? <EyeInvisibleOutlined /> : <EyeOutlined />}
                  </button>
                }
              />
            </Form.Item>
          </>
        )}

        {!isOfficialMode && renderModelMappingSection()}

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
            title={'备注'}
            placeholder="可选备注"
            rows={3}
            resetKey={notesCollapseResetKey}
          />
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default ClaudeDesktopProviderFormModal;