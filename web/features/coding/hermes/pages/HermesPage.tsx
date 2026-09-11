import React from 'react';
import { useProviderSharing } from '@/features/coding/shared/providerShare';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import {
  Button,
  Collapse,
  Empty,
  Form,
  Input,
  Modal,
  Select,
  Space,
  Spin,
  Tooltip,
  Typography,
  message,
} from 'antd';
import {
  ApiOutlined,
  CheckSquareOutlined,
  CloudDownloadOutlined,
  DatabaseOutlined,
  DeleteOutlined,
  DownOutlined,
  EditOutlined,
  EllipsisOutlined,
  EyeOutlined,
  FileTextOutlined,
  FolderOpenOutlined,
  GlobalOutlined,
  ImportOutlined,
  LinkOutlined,
  PlusOutlined,
  QuestionCircleOutlined,
  ReloadOutlined,
  RightOutlined,
  RobotOutlined,
  SettingOutlined,
  ThunderboltOutlined,
  ToolOutlined,
} from '@ant-design/icons';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import ProviderCard from '@/components/common/ProviderCard';
import type {
  ModelDisplayData,
  ProviderConnectivityStatusItem,
  ProviderDisplayData,
} from '@/components/common/ProviderCard/types';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import {
  PROVIDER_SORT_MODES_BASIC,
  backupProvidersBeforeDelete,
  ProviderBatchToolbar,
  ProviderSearchEmpty,
  ProviderSearchInput,
  ProviderSortDropdown,
  filterProviderItems,
  sortProviderItems,
  useProviderBatchSelection,
  useProviderListSort,
} from '@/features/coding/shared/providerList';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import CliManualPathSetting from '@/components/common/CliManualPathSetting';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import ProviderConnectivityTestModal from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import {
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import HermesMemoryPanel from '../components/HermesMemoryPanel';
import { hasAllApiHubExtension, refreshTrayMenu } from '@/services/appApi';
import {
  deleteHermesRuntimeProvider,
  getHermesSettingsConfig,
  launchHermesDashboard,
  listHermesAllApiHubProviders,
  openHermesWebUi,
  readHermesRuntimeConfig,
  resolveHermesAllApiHubProviders,
  saveHermesModelSettings,
  saveHermesModelsProvider,
  saveHermesOtherSettings,
  saveHermesSettingsConfig,
} from '@/services/hermesApi';
import {
  hasCcSwitchDb,
  type CcSwitchProviderCandidate,
} from '@/services/ccSwitchApi';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import ImportFromAllApiHubModalForTool from '@/features/coding/shared/allApiHub/ImportFromAllApiHubModalForTool';
import {
  buildFavoriteProviderOptions,
  buildFavoriteProviderStorageKey,
  extractFavoriteProviderRawId,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  type HermesFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import { upsertFavoriteProvider, type OpenCodeFavoriteProvider } from '@/services/opencodeApi';
import type { OpenCodeProvider } from '@/types/opencode';
import type { AllApiHubProviderItem } from '@/types/allApiHub';
import { extractHermesProviderFromCcSwitch } from '../utils/importMapping';
import { buildFetchedHermesModel } from '../utils/hermesFetchedModels';
import { findPresetModelById } from '@/constants/presetModels';
import { hermesPromptApi } from '@/services/hermesPromptApi';
import type {
  HermesRuntimeConfig,
  HermesRuntimeProviderView,
} from '@/types/hermes';
import { useSettingsStore } from '@/stores';

import JsonEditor from '@/components/common/JsonEditor';
import ModelFormModal from '@/components/common/ModelFormModal';
import type { ModelFormValues } from '@/components/common/ModelFormModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchModelsApplyResult } from '@/components/common/FetchModelsModal/types';
import {
  asRecord,
  buildHermesConnectivityProvider,
  getNumberField,
  hermesApiModeToSdkName,
  getProviderModelRecords,
  getStringField,
  isRecordEmpty,
  maskCredential,
  setOptionalStringField,
  HERMES_REASONING_LEVELS,
  parseReasoningEffort,
} from '../utils/hermesUtils';
import styles from './HermesPage.module.less';

const { Title, Text, Link } = Typography;

interface HermesProviderModalState {
  provider?: HermesRuntimeProviderView;
}

interface HermesModelModalState {
  provider: HermesRuntimeProviderView;
  modelId?: string;
  /** 以某模型为模板新建(复制):打开新增弹窗并预填该模型内容。 */
  copyFromId?: string;
}

/** Shape accepted by the shared root-directory hook (maps to Hermes `configDir`). */
interface HermesCommonConfigLike {
  config?: string;
  rootDir?: string | null;
}

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'hermes-model-settings': <RobotOutlined />,
  'hermes-providers': <DatabaseOutlined />,
  'hermes-global-prompt': <FileTextOutlined />,
  'hermes-memory': <EditOutlined />,
  'hermes-other-configuration': <ToolOutlined />,
};

const HERMES_API_MODE_OPTIONS = [
  'anthropic',
  'openai',
  'openai-completions',
  'openai-responses',
  'google',
  'gemini',
  'custom',
].map((value) => ({ value, label: value }));

/** Build the OpenCode-provider envelope that wraps a Hermes provider favorite. */
const buildHermesFavoriteProviderConfig = (
  providerKey: string,
  modelsProvider: Record<string, unknown>,
): OpenCodeProvider => {
  const displayName = getStringField(modelsProvider, 'display_name')
    || getStringField(modelsProvider, 'name')
    || providerKey;
  const apiKey = getStringField(modelsProvider, 'api_key');
  const baseUrl = getStringField(modelsProvider, 'base_url');
  const models = getProviderModelRecords(modelsProvider);
  const payload: HermesFavoriteProviderPayload = {
    providerKey,
    modelsProvider,
  };
  return buildFavoriteProviderOptions(
    {
      npm: hermesApiModeToSdkName(getStringField(modelsProvider, 'api_mode')),
      name: displayName,
      options: {
        ...(baseUrl ? { baseURL: baseUrl } : {}),
        ...(apiKey ? { apiKey } : {}),
      },
      models: Object.fromEntries(models.map((model) => [model.id, {}])),
    },
    payload,
  );
};

/** Resolve the favorite payload back into a Hermes-language provider import record. */
const resolveHermesFavoriteProviderPayload = (
  favoriteProvider: OpenCodeFavoriteProvider,
): HermesFavoriteProviderPayload => {
  const payload = getFavoriteProviderPayload<HermesFavoriteProviderPayload>(favoriteProvider);
  if (payload?.providerKey && payload.modelsProvider) {
    return payload;
  }
  return {
    providerKey: extractFavoriteProviderRawId('hermes', favoriteProvider.providerId),
    modelsProvider: payload?.modelsProvider ?? favoriteProvider.providerConfig,
  };
};

const HermesPage: React.FC = () => {
  const { t } = useTranslation();
  const { shareProvider, shareModal } = useProviderSharing('hermes', () => loadConfig());
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [runtimeConfig, setRuntimeConfig] = React.useState<HermesRuntimeConfig | null>(null);
  const [modelForm] = Form.useForm();
  const [providerModalForm] = Form.useForm();
  const [providerModal, setProviderModal] = React.useState<HermesProviderModalState | null>(null);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [batchDeleteProviderId, setBatchDeleteProviderId] = React.useState<string | null>(null);
  const [selectedModelIdsByProvider, setSelectedModelIdsByProvider] = React.useState<Record<string, string[]>>({});
  const [fetchModelsProviderKey, setFetchModelsProviderKey] = React.useState<string | null>(null);
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [providerJson, setProviderJson] = React.useState<Record<string, unknown>>({});
  const [providerJsonValid, setProviderJsonValid] = React.useState(true);
  const [providerAdvancedExpanded, setProviderAdvancedExpanded] = React.useState(false);
  const [modelModal, setModelModal] = React.useState<HermesModelModalState | null>(null);
  const [connectivityProviderId, setConnectivityProviderId] = React.useState<string | null>(null);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);
  const [otherSettings, setOtherSettings] = React.useState<Record<string, unknown>>({});
  const [otherSettingsValid, setOtherSettingsValid] = React.useState(true);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const modelSettingsSaveSeqRef = React.useRef(0);
  const sidebarHidden = sidebarHiddenByPage.hermes;

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'hermes-model-settings',
      title: t('hermes.modelSettings.title', { defaultValue: 'Model Settings' }),
      order: 1,
    },
    {
      id: 'hermes-providers',
      title: t('hermes.provider.title', { defaultValue: 'Providers' }),
      order: 2,
    },
    {
      id: 'hermes-global-prompt',
      title: t('hermes.prompt.title', { defaultValue: 'Global Prompt' }),
      order: 3,
    },
    {
      id: 'hermes-memory',
      title: t('hermes.memory.title', { defaultValue: 'Memory' }),
      order: 4,
    },
    {
      id: 'hermes-other-configuration',
      title: t('hermes.otherConfig.title', { defaultValue: 'Other Configuration' }),
      order: 5,
    },
    {
      id: 'hermes-session-manager',
      title: t('sessionManager.title'),
      order: 6,
    },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const config = await readHermesRuntimeConfig();
      setRuntimeConfig(config);
      setOtherSettings(config.otherSettings || {});
      const agent = asRecord(config.otherSettings?.agent);
      modelForm.setFieldsValue({
        defaultProvider: config.modelSettings.defaultProvider || undefined,
        defaultModel: config.modelSettings.defaultModel || undefined,
        reasoningEffort: parseReasoningEffort(agent.reasoning_effort) as string | undefined,
      });
    } catch (error) {
      console.error('Failed to load Hermes runtime config:', error);
      message.error(t('common.error'));
    } finally {
      if (!silent) {
        setLoading(false);
      }
    }
  }, [modelForm, t]);

  React.useEffect(() => {
    void loadConfig();
  }, [loadConfig]);

  React.useEffect(() => {
    const handleTrayConfigRefresh = (event: Event) => {
      event.preventDefault();
      void loadConfig(true);
    };

    window.addEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    return () => {
      window.removeEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    };
  }, [loadConfig]);

  // Root config-dir editing via the shared RootDirectoryModal. Hermes persists a
  // `configDir` in the DB (id "common"); the shared hook uses `rootDir` naming,
  // so we map between the two shapes.
  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig<HermesCommonConfigLike>({
    t,
    translationKeyPrefix: 'hermes',
    defaultConfig: '{}',
    loadConfig,
    getCommonConfig: async (): Promise<HermesCommonConfigLike | null> => {
      const config = await getHermesSettingsConfig();
      return { config: config?.configDir ?? '', rootDir: config?.configDir ?? null };
    },
    saveCommonConfig: async (input) => {
      if (input.clearRootDir || !input.rootDir) {
        await saveHermesSettingsConfig({ clearConfigDir: true });
        return;
      }
      await saveHermesSettingsConfig({ configDir: input.rootDir });
    },
  });

  const providerOptions = React.useMemo(() => {
    const options = new Map<string, string>();
    runtimeConfig?.providers.forEach((provider) => {
      options.set(provider.providerKey, `${provider.displayName} (${provider.providerKey})`);
    });
    runtimeConfig?.builtinProviders.forEach((provider) => {
      if (!options.has(provider.key)) {
        options.set(provider.key, `${provider.name} (${provider.key})`);
      }
    });
    const current = runtimeConfig?.modelSettings.defaultProvider;
    if (current && !options.has(current)) {
      options.set(current, current);
    }
    return Array.from(options.entries()).map(([value, label]) => ({ value, label }));
  }, [runtimeConfig]);

  const selectedProviderKey = Form.useWatch('defaultProvider', modelForm);
  const selectedDefaultModel = Form.useWatch('defaultModel', modelForm);
  const selectedProvider = runtimeConfig?.providers.find(
    (provider) => provider.providerKey === selectedProviderKey,
  );
  const modelOptions = React.useMemo(() => {
    const options = new Set<string>();
    selectedProvider?.modelIds?.forEach((modelId) => options.add(modelId));
    const current = selectedDefaultModel || runtimeConfig?.modelSettings.defaultModel;
    if (current) {
      options.add(current);
    }
    return Array.from(options).map((modelId) => ({ value: modelId, label: modelId }));
  }, [runtimeConfig?.modelSettings.defaultModel, selectedDefaultModel, selectedProvider?.modelIds]);

  const hermesProviders = React.useMemo(
    () => runtimeConfig?.providers ?? [],
    [runtimeConfig?.providers],
  );

  const existingFavoriteProviderIds = React.useMemo(
    () => hermesProviders.map((provider) => buildFavoriteProviderStorageKey('hermes', provider.providerKey)),
    [hermesProviders],
  );

  /** Persist a Hermes provider into the shared favorite-provider history. Errors are swallowed
   *  so a favorite write failure never blocks the primary save/import flow. */
  const upsertHermesFavoriteProvider = React.useCallback(
    (providerKey: string, modelsProvider: Record<string, unknown>) => {
      return upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('hermes', providerKey),
        buildHermesFavoriteProviderConfig(providerKey, modelsProvider),
      ).catch((error) => {
        console.error('Failed to save Hermes favorite provider:', error);
      });
    },
    [],
  );

  const connectivityInfo = React.useMemo(() => {
    if (!connectivityProviderId) {
      return null;
    }
    const provider = hermesProviders.find((item) => item.providerKey === connectivityProviderId);
    if (!provider) {
      return null;
    }
    return {
      providerId: provider.providerKey,
      providerName: provider.displayName,
      providerConfig: buildHermesConnectivityProvider(provider),
      modelIds: provider.modelIds ?? [],
    };
  }, [connectivityProviderId, hermesProviders]);

  const handleModelSettingsChange = async (
    changedValues: Record<string, unknown>,
    allValues: Record<string, unknown>,
  ) => {
    const reasoningChanged = 'reasoningEffort' in changedValues;
    const modelChanged = ('defaultProvider' in changedValues) || ('defaultModel' in changedValues);
    if (!runtimeConfig) {
      return;
    }

    if (reasoningChanged) {
      const currentAgent = asRecord(runtimeConfig.otherSettings.agent);
      const nextLevel = parseReasoningEffort(allValues.reasoningEffort);
      // 未变化不写
      if ((currentAgent.reasoning_effort ?? '') === (nextLevel ?? '')) {
        return;
      }
      const nextAgent = { ...currentAgent };
      if (nextLevel) {
        nextAgent.reasoning_effort = nextLevel;
      } else {
        delete nextAgent.reasoning_effort;
      }
      setSaving(true);
      try {
        const nextConfig = await saveHermesOtherSettings({
          ...runtimeConfig.otherSettings,
          agent: nextAgent,
        });
        setRuntimeConfig(nextConfig);
        setOtherSettings(nextConfig.otherSettings || {});
        await refreshTrayMenu();
      } catch (error) {
        console.error('Failed to save Hermes reasoning effort:', error);
        message.error(t('common.error'));
      } finally {
        setSaving(false);
      }
      return;
    }

    if (!modelChanged) {
      return;
    }

    const nextProvider = (allValues.defaultProvider ?? '') as string;
    const nextModel = (allValues.defaultModel ?? '') as string;
    const currentSettings = runtimeConfig.modelSettings;
    if (
      (currentSettings.defaultProvider ?? '') === nextProvider
      && (currentSettings.defaultModel ?? '') === nextModel
    ) {
      return;
    }

    const saveSeq = modelSettingsSaveSeqRef.current + 1;
    modelSettingsSaveSeqRef.current = saveSeq;
    setSaving(true);
    try {
      const nextConfig = await saveHermesModelSettings({
        defaultProvider: nextProvider,
        defaultModel: nextModel,
      });
      if (modelSettingsSaveSeqRef.current === saveSeq) {
        setRuntimeConfig(nextConfig);
        setOtherSettings(nextConfig.otherSettings || {});
      }
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Hermes model settings:', error);
      if (modelSettingsSaveSeqRef.current === saveSeq) {
        message.error(t('common.error'));
      }
    } finally {
      if (modelSettingsSaveSeqRef.current === saveSeq) {
        setSaving(false);
      }
    }
  };

  // Initialize the inlined provider edit/save modal whenever it opens.
  React.useEffect(() => {
    if (!providerModal) {
      return;
    }
    const nextProviderJson = providerModal.provider?.provider
      ? asRecord(providerModal.provider.provider)
      : {};
    setProviderJson(nextProviderJson);
    setProviderJsonValid(true);
    setProviderAdvancedExpanded(false);
    providerModalForm.setFieldsValue({
      providerKey: providerModal.provider?.providerKey,
      apiMode: providerModal.provider?.apiMode || getStringField(nextProviderJson, 'api_mode'),
      baseUrl: getStringField(nextProviderJson, 'base_url')
        || getStringField(nextProviderJson, 'baseUrl'),
      providerApiKey: getStringField(nextProviderJson, 'api_key')
        || getStringField(nextProviderJson, 'apiKey'),
      providerDisplayName: getStringField(nextProviderJson, 'display_name'),
    });
  }, [providerModal, providerModalForm]);

  const handleSaveProviderModal = async () => {
    if (!providerJsonValid) {
      message.error(t('hermes.invalidJson', { defaultValue: 'Invalid JSON.' }));
      return;
    }
    const values = await providerModalForm.validateFields();
    const providerKey = values.providerKey?.trim();
    if (!providerKey) {
      message.error(t('hermes.provider.providerKeyRequired', { defaultValue: '请输入供应商 Key' }));
      return;
    }

    const nextProviderJson = { ...providerJson };
    setOptionalStringField(nextProviderJson, 'api_mode', values.apiMode);
    setOptionalStringField(nextProviderJson, 'base_url', values.baseUrl);
    setOptionalStringField(nextProviderJson, 'api_key', values.providerApiKey);
    setOptionalStringField(nextProviderJson, 'display_name', values.providerDisplayName);
    await handleSaveProvider({ providerKey, provider: nextProviderJson });
  };

  const handleSaveProvider = async (value: {
    providerKey: string;
    provider: Record<string, unknown>;
  }) => {
    setSaving(true);
    try {
      const nextConfig = await saveHermesModelsProvider({
        providerKey: value.providerKey,
        provider: value.provider,
      });
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      setProviderModal(null);
      void upsertHermesFavoriteProvider(value.providerKey, value.provider);
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save Hermes provider:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteProvider = (provider: HermesRuntimeProviderView) => {
    Modal.confirm({
      title: t('hermes.provider.deleteConfirmTitle', { defaultValue: 'Delete provider?' }),
      content: t('hermes.provider.deleteConfirmContent', {
        defaultValue: 'Remove "{{name}}" from custom_providers in config.yaml?',
        name: provider.displayName,
      }),
      okButtonProps: { danger: true },
      onOk: async () => {
        setSaving(true);
        try {
          // Back up the provider into the favorite-provider history before deletion
          // so it can still be re-imported later from "导入我使用过的供应商".
          if (provider.provider) {
            try {
              await upsertFavoriteProvider(
                buildFavoriteProviderStorageKey('hermes', provider.providerKey),
                buildHermesFavoriteProviderConfig(provider.providerKey, provider.provider),
              );
            } catch (error) {
              console.error('Failed to preserve Hermes favorite provider before deletion:', error);
            }
          }
          const nextConfig = await deleteHermesRuntimeProvider(provider.providerKey);
          setRuntimeConfig(nextConfig);
          setOtherSettings(nextConfig.otherSettings || {});
          await refreshTrayMenu();
          message.success(t('common.success'));
          // The remove only ever touches custom_providers. If the same key is a
          // read-only Hermes built-in (providers: dict), a card legitimately
          // remains — make that clear so users don't think the delete failed.
          const readOnlyRemaining = nextConfig.providers.find(
            (item) => item.providerKey === provider.providerKey && item.isReadOnly,
          );
          if (readOnlyRemaining) {
            message.info(
              t('hermes.provider.readOnlyRemains', {
                name: readOnlyRemaining.displayName || readOnlyRemaining.providerKey,
              }),
            );
          }
        } catch (error) {
          console.error('Failed to delete Hermes provider:', error);
          message.error(t('common.error'));
        } finally {
          setSaving(false);
        }
      },
    });
  };

  const canBatchDeleteProvider = React.useCallback(
    (provider: HermesRuntimeProviderView) => !provider.isReadOnly && !provider.isDefault,
    [],
  );

  const handleBatchDeleteProviders = React.useCallback(
    async (providerKeys: string[]): Promise<boolean> => {
      if (!runtimeConfig) return false;
      const providersToDelete = hermesProviders.filter(
        (provider) => providerKeys.includes(provider.providerKey) && canBatchDeleteProvider(provider),
      );
      if (providersToDelete.length === 0) return false;
      setSaving(true);
      try {
        await backupProvidersBeforeDelete(
          providersToDelete,
          (provider) => upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('hermes', provider.providerKey),
            buildHermesFavoriteProviderConfig(provider.providerKey, provider.provider ?? {}),
          ),
          (provider) => t('common.batch.backupFailed', { name: provider.displayName || provider.providerKey }),
        );
        let nextConfig: HermesRuntimeConfig | null = null;
        for (const provider of providersToDelete) {
          nextConfig = await deleteHermesRuntimeProvider(provider.providerKey);
        }
        if (nextConfig) {
          setRuntimeConfig(nextConfig);
          setOtherSettings(nextConfig.otherSettings || {});
        }
        await refreshTrayMenu();
        message.success(t('common.success'));
        const readOnlyRemaining = nextConfig?.providers.filter(
          (provider) => providerKeys.includes(provider.providerKey) && provider.isReadOnly,
        ) ?? [];
        if (readOnlyRemaining.length > 0) {
          message.info(t('hermes.provider.readOnlyRemains', {
            name: readOnlyRemaining.map((provider) => provider.displayName || provider.providerKey).join(', '),
          }));
        }
        return true;
      } catch (error) {
        console.error('Failed to batch delete Hermes providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig(true);
        return false;
      } finally {
        setSaving(false);
      }
    },
    [hermesProviders, runtimeConfig, canBatchDeleteProvider, loadConfig, t],
  );

  const handleSaveModel = async (values: ModelFormValues) => {
    if (!modelModal) {
      return;
    }
    const modelId = values.id?.trim();
    if (!modelId) {
      message.error(t('hermes.model.idRequired', { defaultValue: 'Model ID is required.' }));
      return;
    }

    const currentProvider = runtimeConfig?.providers.find(
      (provider) => provider.providerKey === modelModal.provider.providerKey,
    ) ?? modelModal.provider;
    const existingModels = getProviderModelRecords(currentProvider.provider);
    const duplicateModel = existingModels.some((entry) => (
      entry.id === modelId && entry.id !== modelModal.modelId
    ));
    if (duplicateModel) {
      message.error(t('hermes.model.idExists', { defaultValue: 'Model ID already exists.' }));
      return;
    }

    const nextModel = { ...(existingModels.find((entry) => entry.id === modelModal.modelId)?.model ?? {}) };
    setOptionalStringField(nextModel, 'id', modelId);
    setOptionalStringField(nextModel, 'name', values.name);
    if (typeof values.contextLimit === 'number') {
      nextModel.context_length = values.contextLimit;
    } else {
      delete nextModel.context_length;
    }
    if (typeof values.outputLimit === 'number') {
      nextModel.max_tokens = values.outputLimit;
    } else {
      delete nextModel.max_tokens;
    }
    if (typeof values.reasoning === 'boolean') {
      nextModel.reasoning = values.reasoning;
    } else {
      delete nextModel.reasoning;
    }

    // 额外参数：以编辑器内容为准，移除旧的未知字段后再合并（与 OpenClaw 一致）。
    for (const key of Object.keys(nextModel)) {
      if (!HERMES_KNOWN_MODEL_FIELDS.has(key)) {
        delete nextModel[key];
      }
    }
    if (values.extraParams && typeof values.extraParams === 'object') {
      Object.assign(nextModel, values.extraParams);
    }

    let modelWasReplaced = false;
    const nextModels = existingModels.map((entry) => {
      if (entry.id === modelModal.modelId) {
        modelWasReplaced = true;
        return nextModel;
      }
      return entry.model;
    });
    if (!modelWasReplaced) {
      nextModels.push(nextModel);
    }

    setSaving(true);
    try {
      const nextProviderConfig = {
        ...(currentProvider.provider ?? {}),
        models: nextModels,
      };
      const nextConfig = await saveHermesModelsProvider({
        providerKey: currentProvider.providerKey,
        provider: nextProviderConfig,
      });

      // per-model 思考等级覆盖写入 agent.reasoning_overrides["providerKey/modelId"]
      const oldOverrideKey = modelModal.modelId
        ? `${currentProvider.providerKey}/${modelModal.modelId}`
        : null;
      const newOverrideKey = `${currentProvider.providerKey}/${modelId}`;
      const desiredLevel = parseReasoningEffort(values.thinkingLevel);
      const freshAgent = asRecord(nextConfig.otherSettings?.agent);
      const currentOverrides = asRecord(freshAgent.reasoning_overrides);
      const nextOverrides = { ...currentOverrides };
      // 重命名模型时清理旧的覆盖键
      if (oldOverrideKey && oldOverrideKey !== newOverrideKey) {
        delete nextOverrides[oldOverrideKey];
      }
      if (desiredLevel) {
        nextOverrides[newOverrideKey] = desiredLevel;
      } else {
        delete nextOverrides[newOverrideKey];
      }

      const overridesChanged = JSON.stringify(nextOverrides) !== JSON.stringify(currentOverrides);
      let finalConfig = nextConfig;
      if (overridesChanged) {
        const nextAgent = { ...freshAgent };
        if (Object.keys(nextOverrides).length > 0) {
          nextAgent.reasoning_overrides = nextOverrides;
        } else {
          delete nextAgent.reasoning_overrides;
        }
        finalConfig = await saveHermesOtherSettings({
          ...nextConfig.otherSettings,
          agent: nextAgent,
        });
      }
      setRuntimeConfig(finalConfig);
      setOtherSettings(finalConfig.otherSettings || {});
      setModelModal(null);
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save Hermes model:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('hermes');
  const [providerKeyword, setProviderKeyword] = React.useState('');
  const visibleProviders = React.useMemo(
    () =>
      sortProviderItems(
        filterProviderItems(hermesProviders, providerKeyword, (provider) => [
          provider.providerKey,
          provider.displayName,
          ...(provider.modelIds ?? []),
        ]),
        sortMode,
        { name: (provider) => provider.displayName || provider.providerKey },
        (provider) => lastUsedAt(provider.providerKey),
      ),
    [hermesProviders, providerKeyword, sortMode, lastUsedAt],
  );

  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter(canBatchDeleteProvider).map((provider) => provider.providerKey),
    [visibleProviders, canBatchDeleteProvider],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });

  const handleSetDefaultModel = async (provider: HermesRuntimeProviderView, modelId: string) => {
    setSaving(true);
    try {
      const nextConfig = await saveHermesModelSettings({
        defaultProvider: provider.providerKey,
        defaultModel: modelId,
      });
      noteProviderUsed(provider.providerKey);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      modelForm.setFieldsValue({
        defaultProvider: provider.providerKey,
        defaultModel: modelId,
      });
      await refreshTrayMenu();
      message.success(t('hermes.model.setAsDefaultSuccess', { defaultValue: 'Set as default.' }));
    } catch (error) {
      console.error('Failed to set Hermes default model:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleOpenConnectivityTest = (providerKey: string) => {
    setConnectivityProviderId(providerKey);
    setConnectivityModalOpen(true);
  };

  const handleRemoveConnectivityModels = React.useCallback(async (modelIdsToRemove: string[]) => {
    if (!connectivityProviderId || modelIdsToRemove.length === 0) {
      return;
    }
    const provider = hermesProviders.find((item) => item.providerKey === connectivityProviderId);
    if (!provider || provider.isReadOnly) {
      return;
    }
    const selectedModelIdSet = new Set(modelIdsToRemove);
    const nextModels = getProviderModelRecords(provider.provider)
      .filter((entry) => !selectedModelIdSet.has(entry.id))
      .map((entry) => entry.model);

    setSaving(true);
    try {
      const nextProviderConfig = { ...(provider.provider ?? {}), models: nextModels };
      const nextConfig = await saveHermesModelsProvider({
        providerKey: provider.providerKey,
        provider: nextProviderConfig,
      });
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
    } catch (error) {
      console.error('Failed to remove Hermes models from connectivity test:', error);
      throw error;
    } finally {
      setSaving(false);
    }
  }, [connectivityProviderId, hermesProviders]);

  /**
   * 从 agent.reasoning_overrides 中移除指定 model override 键并保存 otherSettings。
   * 删除模型时调用,避免遗留 override 在新模型复用同 id 时被静默应用。
   * 无 override 变更时直接返回原 config(不发起额外保存)。
   */
  const removeReasoningOverrides = async (
    config: HermesRuntimeConfig,
    overrideKeys: string[]
  ): Promise<HermesRuntimeConfig> => {
    if (overrideKeys.length === 0) {
      return config;
    }
    const freshAgent = asRecord(config.otherSettings?.agent);
    const currentOverrides = asRecord(freshAgent.reasoning_overrides);
    const nextOverrides = { ...currentOverrides };
    let changed = false;
    for (const key of overrideKeys) {
      if (nextOverrides[key] !== undefined) {
        delete nextOverrides[key];
        changed = true;
      }
    }
    if (!changed) {
      return config;
    }
    const nextAgent = { ...freshAgent };
    if (Object.keys(nextOverrides).length > 0) {
      nextAgent.reasoning_overrides = nextOverrides;
    } else {
      delete nextAgent.reasoning_overrides;
    }
    return saveHermesOtherSettings({
      ...config.otherSettings,
      agent: nextAgent,
    });
  };

  const handleDeleteModel = React.useCallback(
    async (providerKey: string, modelId: string) => {
      const provider = hermesProviders.find((item) => item.providerKey === providerKey);
      if (!provider || provider.isReadOnly) {
        return;
      }
      const nextModels = getProviderModelRecords(provider.provider)
        .filter((entry) => entry.id !== modelId)
        .map((entry) => entry.model);
      try {
        const nextConfig = await saveHermesModelsProvider({
          providerKey,
          provider: { ...(provider.provider ?? {}), models: nextModels },
        });
        const finalConfig = await removeReasoningOverrides(nextConfig, [`${providerKey}/${modelId}`]);
        setRuntimeConfig(finalConfig);
        setOtherSettings(finalConfig.otherSettings || {});
        message.success(t('hermes.model.batchDeleteSuccess', { count: 1 }));
      } catch (error) {
        console.error('Failed to delete Hermes model:', error);
        message.error(t('common.error'));
      }
    },
    [hermesProviders, t],
  );

  const handleToggleBatchDeleteMode = (providerKey: string) => {
    setBatchDeleteProviderId((prev) => {
      const next = prev === providerKey ? null : providerKey;
      // 清理涉及到的 provider 的勾选:关闭时清当前,切换到另一个 provider 时清前一个,
      // 否则旧 provider 的勾选会残留,重入时被静默预选导致误删本次会话从未选中的模型。
      const keysToClean = next === null ? [providerKey] : prev && prev !== providerKey ? [prev] : [];
      if (keysToClean.length > 0) {
        setSelectedModelIdsByProvider((selected) => {
          const copy = { ...selected };
          keysToClean.forEach((k) => delete copy[k]);
          return copy;
        });
      }
      return next;
    });
  };

  const handleToggleModelSelection = (providerKey: string, modelId: string, selected: boolean) => {
    setSelectedModelIdsByProvider((prev) => {
      const current = prev[providerKey] ?? [];
      if (selected) {
        return { ...prev, [providerKey]: current.includes(modelId) ? current : [...current, modelId] };
      }
      return { ...prev, [providerKey]: current.filter((id) => id !== modelId) };
    });
  };

  const handleBatchDeleteModels = React.useCallback(
    async (providerKey: string) => {
      const provider = hermesProviders.find((item) => item.providerKey === providerKey);
      if (!provider || provider.isReadOnly) {
        setBatchDeleteProviderId(null);
        return;
      }
      const selected = selectedModelIdsByProvider[providerKey] ?? [];
      if (selected.length === 0) {
        return;
      }
      const selectedSet = new Set(selected);
      const nextModels = getProviderModelRecords(provider.provider)
        .filter((entry) => !selectedSet.has(entry.id))
        .map((entry) => entry.model);

      try {
        const nextConfig = await saveHermesModelsProvider({
          providerKey,
          provider: { ...(provider.provider ?? {}), models: nextModels },
        });
        const overrideKeys = selected.map((modelId) => `${providerKey}/${modelId}`);
        const finalConfig = await removeReasoningOverrides(nextConfig, overrideKeys);
        setRuntimeConfig(finalConfig);
        setOtherSettings(finalConfig.otherSettings || {});
        message.success(t('hermes.model.batchDeleteSuccess', { count: selected.length }));
      } catch (error) {
        console.error('Failed to batch delete Hermes models:', error);
        message.error(t('common.error'));
      } finally {
        setBatchDeleteProviderId(null);
        setSelectedModelIdsByProvider((prev) => {
          const copy = { ...prev };
          delete copy[providerKey];
          return copy;
        });
      }
    },
    [hermesProviders, selectedModelIdsByProvider, t],
  );

  const handleBatchTestProviders = React.useCallback(async () => {
    const targets = hermesProviders.map((provider) => (
      buildProviderConnectivityBatchTarget(
        {
          providerId: provider.providerKey,
          providerName: provider.displayName,
          providerConfig: buildHermesConnectivityProvider(provider),
          modelIds: provider.modelIds ?? [],
        },
        {
          requireBaseUrl: true,
          requireApiKey: false,
          errorMessages: {
            missingBaseUrl: t('common.baseUrlMissing'),
            missingApiKey: t('common.apiKeyMissing'),
            missingModel: t('common.modelMissing'),
          },
        },
      )
    ));

    setConnectivityStatuses(
      Object.fromEntries(hermesProviders.map((provider) => [
        provider.providerKey,
        { status: 'running' as const },
      ])),
    );
    setBatchTestingProviders(true);

    try {
      await runProviderConnectivityBatch(targets, (providerKey, status) => {
        const nextStatus = status.status === 'success'
          ? {
              ...status,
              tooltipMessage: status.totalMs !== undefined
                ? t('common.connectivityBatchSuccessWithTiming', {
                    model: status.modelId || t('common.notSet'),
                    totalMs: status.totalMs,
                  })
                : t('common.connectivityBatchSuccess', {
                    model: status.modelId || t('common.notSet'),
                  }),
            }
          : status;
        setConnectivityStatuses((previousStatuses) => ({
          ...previousStatuses,
          [providerKey]: nextStatus,
        }));
      });
    } catch (error) {
      console.error('Failed to batch test Hermes providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [hermesProviders, t]);

  const handleOtherSettingsBlur = async (value: unknown, isValid: boolean) => {
    if (!isValid || !otherSettingsValid) {
      message.error(t('hermes.invalidJson', { defaultValue: 'Invalid JSON.' }));
      return;
    }
    const nextOtherSettings = value && typeof value === 'object' && !Array.isArray(value)
      ? value as Record<string, unknown>
      : {};
    setSaving(true);
    try {
      const nextConfig = await saveHermesOtherSettings(nextOtherSettings);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save Hermes other settings:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleOpenRootFolder = async () => {
    if (runtimeConfig?.rootPathInfo.path) {
      await revealItemInDir(runtimeConfig.rootPathInfo.path);
    }
  };

  const handleRefreshConfig = () => {
    void loadConfig(true);
    void refreshTrayMenu();
  };

  const handleOpenWebUi = async () => {
    try {
      await openHermesWebUi();
    } catch {
      Modal.confirm({
        title: t('hermes.openWebUi', { defaultValue: 'Open Web UI' }),
        content: t('hermes.openWebUiOffline', {
          defaultValue: 'Hermes Web UI is not running. Launch the dashboard service?',
        }),
        okText: t('hermes.launchDashboard', { defaultValue: 'Launch Dashboard' }),
        onOk: async () => {
          try {
            await launchHermesDashboard();
            message.success(
              t('hermes.dashboardLaunched', {
                defaultValue: 'Hermes dashboard launched — retry "Open Web UI" shortly.',
              })
            );
          } catch {
            message.error(t('common.error'));
          }
        },
      });
    }
  };

  const handleImportFromAllApiHub = React.useCallback(
    async (imported: AllApiHubProviderItem[]) => {
      const existingKeys = new Set(hermesProviders.map((provider) => provider.providerKey));
      const toImport = imported.filter((item) => !existingKeys.has(item.providerId));

      let ok = 0;
      let fail = 0;
      for (const item of toImport) {
        try {
          await saveHermesModelsProvider({ providerKey: item.providerId, provider: item.config });
          void upsertHermesFavoriteProvider(item.providerId, item.config);
          ok += 1;
        } catch (error) {
          console.error('Failed to import All API Hub provider:', item.providerId, error);
          fail += 1;
        }
      }

      setAllApiHubImportModalOpen(false);
      if (ok > 0 && fail === 0) {
        message.success(t('common.allApiHub.importSuccess', { count: ok }));
      } else if (ok > 0 && fail > 0) {
        message.warning(t('common.allApiHub.importPartial', { ok, fail }));
      } else if (fail > 0) {
        message.error(t('common.error'));
      }

      void loadConfig(true);
      void refreshTrayMenu();
    },
    [hermesProviders, loadConfig, t],
  );

  const handleImportFromCcSwitch = React.useCallback(
    async (imported: CcSwitchProviderCandidate[]) => {
      const existingKeys = new Set(hermesProviders.map((provider) => provider.providerKey));
      let ok = 0;
      let fail = 0;
      for (const candidate of imported) {
        if (existingKeys.has(candidate.providerId)) {
          continue;
        }
        const provider = extractHermesProviderFromCcSwitch(candidate);
        if (!provider) {
          continue;
        }
        try {
          await saveHermesModelsProvider({ providerKey: candidate.providerId, provider });
          void upsertHermesFavoriteProvider(candidate.providerId, provider);
          ok += 1;
        } catch (error) {
          console.error('Failed to import CC Switch provider:', candidate.providerId, error);
          fail += 1;
        }
      }

      setCcSwitchImportModalOpen(false);
      if (ok > 0 && fail === 0) {
        message.success(t('common.ccSwitch.importSuccess', { count: ok }));
      } else if (ok > 0 && fail > 0) {
        message.warning(t('common.ccSwitch.importPartial', { ok, fail }));
      } else if (fail > 0) {
        message.error(t('common.error'));
      }

      void loadConfig(true);
      void refreshTrayMenu();
    },
    [hermesProviders, loadConfig, t],
  );

  const handleImportFavoriteProviders = React.useCallback(
    async (providersToImport: OpenCodeFavoriteProvider[]) => {
      const existingKeys = new Set(hermesProviders.map((provider) => provider.providerKey));
      let importedCount = 0;
      for (const favoriteProvider of providersToImport) {
        const { providerKey, modelsProvider } = resolveHermesFavoriteProviderPayload(favoriteProvider);
        if (!providerKey || existingKeys.has(providerKey)) {
          continue;
        }
        try {
          await saveHermesModelsProvider({ providerKey, provider: modelsProvider });
          existingKeys.add(providerKey);
          importedCount += 1;
        } catch (error) {
          console.error('Failed to import Hermes favorite provider:', providerKey, error);
        }
      }

      if (importedCount > 0) {
        setImportModalOpen(false);
        message.success(t('opencode.provider.importSuccess', { count: importedCount }));
        void loadConfig(true);
        void refreshTrayMenu();
      }
    },
    [hermesProviders, loadConfig, t],
  );

  React.useEffect(() => {
    const checkAllApiHub = async () => {
      try {
        setAllApiHubAvailable(await hasAllApiHubExtension());
      } catch {
        setAllApiHubAvailable(false);
      }
    };
    void checkAllApiHub();
    const checkCcSwitch = async () => {
      try {
        setCcSwitchAvailable(await hasCcSwitchDb());
      } catch {
        setCcSwitchAvailable(false);
      }
    };
    void checkCcSwitch();
  }, []);

  // Derived values for the inlined shared ModelFormModal (mirrors the old HermesModelModal).
  const modelModalProvider = modelModal?.provider ?? hermesProviders[0];
  const modelModalTargetId = modelModal?.modelId ?? modelModal?.copyFromId;
  const modelModalRecord = modelModalTargetId
    ? getProviderModelRecords(modelModalProvider?.provider).find(
      (entry) => entry.id === modelModalTargetId,
    )?.model
    : undefined;
  const modelModalExistingIds = getProviderModelRecords(modelModalProvider?.provider).map((entry) => entry.id);
  const modelModalRecordSafe = asRecord(modelModalRecord);
  // 提取已知字段之外的额外参数，原样保留在配置文件中（与 OpenClaw extraParams 一致）。
  const HERMES_KNOWN_MODEL_FIELDS = new Set(['id', 'name', 'context_length', 'max_tokens', 'reasoning']);
  const modelModalExtraParams = (() => {
    const extra: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(modelModalRecordSafe)) {
      if (!HERMES_KNOWN_MODEL_FIELDS.has(key) && value !== undefined) {
        extra[key] = value;
      }
    }
    return Object.keys(extra).length > 0 ? extra : undefined;
  })();
  // per-model 思考等级覆盖键 agent.reasoning_overrides["providerKey/modelId"]
  const modelModalOverrideKey = modelModalProvider?.providerKey && modelModalTargetId
    ? `${modelModalProvider.providerKey}/${modelModalTargetId}`
    : null;
  const modelModalThinkingLevel = (() => {
    if (!modelModalOverrideKey) {
      return undefined;
    }
    const agent = asRecord(otherSettings.agent);
    const overrides = asRecord(agent.reasoning_overrides);
    return parseReasoningEffort(overrides[modelModalOverrideKey]) as string | undefined;
  })();
  const modelModalInitialValues = {
    id: modelModal?.modelId ?? getStringField(modelModalRecordSafe, 'id'),
    name: getStringField(modelModalRecordSafe, 'name'),
    reasoning: typeof modelModalRecordSafe.reasoning === 'boolean' ? modelModalRecordSafe.reasoning : undefined,
    contextLimit: getNumberField(modelModalRecordSafe, 'context_length'),
    outputLimit: getNumberField(modelModalRecordSafe, 'max_tokens'),
    thinkingLevel: modelModalThinkingLevel,
    extraParams: modelModalExtraParams,
  };

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!fetchModelsProviderKey) {
      return null;
    }
    const provider = hermesProviders.find((item) => item.providerKey === fetchModelsProviderKey);
    if (!provider) {
      return null;
    }
    const view = buildHermesConnectivityProvider(provider);
    return {
      providerId: provider.providerKey,
      providerName: provider.displayName,
      baseUrl: view.options?.baseURL || '',
      apiKey: view.options?.apiKey,
      sdkType: view.npm,
      existingModelIds: provider.modelIds ?? [],
    };
  }, [fetchModelsProviderKey, hermesProviders]);

  const handleOpenFetchModels = (providerKey: string) => {
    setFetchModelsProviderKey(providerKey);
    setFetchModelsModalOpen(true);
  };

  const handleFetchModelsSuccess = async ({ selectedModels, removedModelIds }: FetchModelsApplyResult) => {
    if (!fetchModelsProviderKey) {
      return;
    }
    const provider = hermesProviders.find((item) => item.providerKey === fetchModelsProviderKey);
    if (!provider || provider.isReadOnly) {
      return;
    }
    const removedSet = new Set(removedModelIds);
    const nextModels = getProviderModelRecords(provider.provider)
      .filter((entry) => !removedSet.has(entry.id))
      .map((entry) => entry.model);
    const currentIds = new Set(nextModels.map((model) => getStringField(model, 'id')));
    for (const model of selectedModels) {
      if (currentIds.has(model.id)) {
        continue;
      }
      const matchedPreset = findPresetModelById(model.id, fetchModelsProviderInfo?.sdkType);
      nextModels.push(buildFetchedHermesModel(model, matchedPreset));
    }

    try {
      const nextConfig = await saveHermesModelsProvider({
        providerKey: provider.providerKey,
        provider: { ...(provider.provider ?? {}), models: nextModels },
      });
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      message.success(t('hermes.model.fetchModels', { defaultValue: 'Fetch Models' }));
    } catch (error) {
      console.error('Failed to apply fetched Hermes models:', error);
      message.error(t('common.error'));
    } finally {
      setFetchModelsModalOpen(false);
    }
  };

  const renderProvider = (provider: HermesRuntimeProviderView) => {
    const providerConfig = asRecord(provider.provider);
    const credentialPreview = maskCredential(provider.credential);
    const baseUrl = getStringField(providerConfig, 'base_url')
      || getStringField(providerConfig, 'baseUrl');
    const hasModelIds = (provider.modelIds?.length ?? 0) > 0;
    const connectivityTooltip = !baseUrl
      ? t('common.baseUrlMissing')
      : !hasModelIds
        ? t('common.modelMissing')
        : '';
    const isReadOnly = provider.isReadOnly;

    const deleteDisabledReason = !isReadOnly && provider.isDefault
      ? t('hermes.provider.deleteDisabledDefault', { defaultValue: '该渠道已设为默认，不可删除' })
      : undefined;

    const providerDisplay: ProviderDisplayData = {
      id: provider.providerKey,
      name: provider.displayName,
      sdkName: provider.apiMode || 'hermes',
      baseUrl: baseUrl || credentialPreview || `${provider.providerKey} (${t('hermes.provider.readOnly', { defaultValue: 'read-only' })})`,
    };
    const modelDisplayList: ModelDisplayData[] = getProviderModelRecords(provider.provider).map((entry) => ({
      id: entry.id,
      name: getStringField(entry.model, 'name') || entry.id,
      isPrimary: provider.isDefault && runtimeConfig?.modelSettings.defaultModel === entry.id,
      contextLimit: getNumberField(entry.model, 'context_length'),
      outputLimit: getNumberField(entry.model, 'max_tokens'),
    }));
    const isBatchDeleteMode = batchDeleteProviderId === provider.providerKey;
    const selectedModelIds = selectedModelIdsByProvider[provider.providerKey] ?? [];
    const selectedModelCount = selectedModelIds.length;

    return (
      <ProviderCard
        key={provider.providerKey}
        provider={providerDisplay}
        models={modelDisplayList}
        onEdit={isReadOnly ? undefined : () => setProviderModal({ provider })}
        onCopy={isReadOnly ? undefined : () => setProviderModal({ provider: undefined })}
        onShare={() => shareProvider({
          id: provider.providerKey, name: provider.displayName, category: 'custom',
          settingsConfig: JSON.stringify({ ...provider.provider, api_mode: provider.apiMode ?? provider.provider?.api_mode }),
          credential: provider.credential,
          credentialUnavailable: provider.isBuiltin,
          defaultModel: provider.isDefault ? runtimeConfig?.modelSettings.defaultModel ?? undefined : undefined,
        })}
        onDelete={isReadOnly ? undefined : () => handleDeleteProvider(provider)}
        deleteConfirm={false}
        deleteDisabledReason={deleteDisabledReason}
        selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.providerKey)}
        selected={providerBatch.selectedIds.has(provider.providerKey)}
        onSelectChange={(checked) => providerBatch.toggleSelect(provider.providerKey, checked)}
        connectivityStatus={connectivityStatuses[provider.providerKey]}
        extraActions={
          <>
            {!isReadOnly && (
              <>
                <Button
                  size="small"
                  type="text"
                  style={{ fontSize: 12 }}
                  onClick={() => handleToggleBatchDeleteMode(provider.providerKey)}
                >
                  <DeleteOutlined style={{ marginRight: 4 }} />
                  {isBatchDeleteMode
                    ? t('hermes.model.cancelBatchDelete', { defaultValue: '退出批量删除' })
                    : t('hermes.model.batchDelete', { defaultValue: '批量删除' })}
                </Button>
                {isBatchDeleteMode && (
                  <Button
                    size="small"
                    danger
                    style={{ fontSize: 12 }}
                    disabled={selectedModelCount === 0}
                    onClick={() => {
                      Modal.confirm({
                        title: t('hermes.model.batchDeleteConfirmTitle', { defaultValue: '批量删除模型' }),
                        content: t('hermes.model.batchDeleteConfirmContent', { count: selectedModelCount }),
                        okText: t('common.delete', { defaultValue: '删除' }),
                        cancelText: t('common.cancel'),
                        onOk: () => handleBatchDeleteModels(provider.providerKey),
                      });
                    }}
                  >
                    {t('hermes.model.deleteSelected', { count: selectedModelCount })}
                  </Button>
                )}
              </>
            )}
            <Tooltip title={connectivityTooltip}>
              <span>
                <Button
                  size="small"
                  type="text"
                  style={{ fontSize: 12 }}
                  onClick={() => handleOpenConnectivityTest(provider.providerKey)}
                  disabled={!baseUrl || !hasModelIds}
                >
                  <ApiOutlined style={{ marginRight: 4 }} />
                  {t('hermes.connectivity.button', { defaultValue: 'Test' })}
                </Button>
              </span>
            </Tooltip>
            {!isReadOnly && (
              <Button
                size="small"
                type="text"
                style={{ fontSize: 12 }}
                onClick={() => handleOpenFetchModels(provider.providerKey)}
                disabled={!baseUrl}
              >
                <CloudDownloadOutlined style={{ marginRight: 4 }} />
                {t('hermes.model.fetchModels', { defaultValue: 'Fetch Models' })}
              </Button>
            )}
          </>
        }
        modelSelectionMode={isBatchDeleteMode}
        selectedModelIds={selectedModelIds}
        onToggleModelSelection={(modelId, selected) => handleToggleModelSelection(provider.providerKey, modelId, selected)}
        modelsDraggable={!isBatchDeleteMode}
        onAddModel={isReadOnly ? undefined : () => setModelModal({ provider })}
        onEditModel={isReadOnly ? undefined : (modelId) => setModelModal({ provider, modelId })}
        onCopyModel={isReadOnly ? undefined : (modelId) => setModelModal({ provider, copyFromId: modelId })}
        onDeleteModel={isReadOnly ? undefined : (modelId) => handleDeleteModel(provider.providerKey, modelId)}
        onSetPrimaryModel={isReadOnly ? undefined : (modelId) => handleSetDefaultModel(provider, modelId)}
        i18nPrefix="pi"
      />
    );
  };

  return (
    <Spin spinning={loading}>
      <SectionSidebarLayout
        sidebarTitle={t('hermes.title', { defaultValue: 'Hermes' })}
        sidebarHidden={sidebarHidden}
        sections={sidebarSections}
        markerAttr="data-hermes-sidebar-section"
        getIcon={(id) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
      >
        <div className={styles.pageContent}>
          <div className={styles.pageHeader}>
            <div>
              <div className={styles.titleRow}>
                <Title level={4} className={styles.pageTitle}>
                  {t('hermes.title', { defaultValue: 'Hermes' })}
                </Title>
                <Link
                  type="secondary"
                  className={styles.headerLink}
                  onClick={(event) => {
                    event.stopPropagation();
                    void openUrl('https://hermes-agent.nousresearch.com/docs/');
                  }}
                >
                  <LinkOutlined /> {t('hermes.viewDocs', { defaultValue: '官方文档' })}
                </Link>
                <Link
                  type="secondary"
                  className={styles.headerLink}
                  onClick={(event) => {
                    event.stopPropagation();
                    setPreviewModalOpen(true);
                  }}
                >
                  <EyeOutlined /> {t('common.previewConfig')}
                </Link>
              </div>
              <Space className={styles.pathToolbar} wrap>
                <Text type="secondary" className={styles.pathLabel}>
                  {t('hermes.configPath', { defaultValue: 'Config path' })}:
                </Text>
                <Text code className={styles.pathText}>
                  {runtimeConfig?.rootPathInfo.path}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => setRootDirectoryModalOpen(true)}
                  className={styles.textAction}
                >
                  {t('hermes.rootPathSource.customize', { defaultValue: 'Customize' })}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenRootFolder}
                  className={styles.textAction}
                >
                  {t('hermes.openFolder', { defaultValue: 'Open folder' })}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<ReloadOutlined />}
                  onClick={handleRefreshConfig}
                  className={styles.textAction}
                >
                  {t('hermes.refreshConfig', { defaultValue: 'Refresh' })}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<GlobalOutlined />}
                  onClick={handleOpenWebUi}
                  className={styles.textAction}
                >
                  {t('hermes.openWebUi', { defaultValue: 'Open Web UI' })}
                </Button>
              </Space>
            </div>
            <Button type="text" icon={<EllipsisOutlined />} onClick={() => setSettingsModalOpen(true)}>
              {t('common.moreOptions')}
            </Button>
          </div>
          <div className={styles.pageHint}>
            {t('hermes.pageHint', {
              defaultValue: 'Hermes reads a single config.yaml; provider facts come from the runtime file. Built-in (read-only) providers are managed by the providers: dict.',
            })}
          </div>

          <div
            id="hermes-model-settings"
            className={styles.hermesSection}
            data-hermes-sidebar-section="true"
            data-sidebar-title={t('hermes.modelSettings.title', { defaultValue: 'Model Settings' })}
          >
            <div className={styles.modelCard}>
              <Title level={5} className={styles.modelCardTitle}>
                <RobotOutlined style={{ marginRight: 8 }} />
                {t('hermes.modelSettings.title', { defaultValue: 'Model Settings' })}
              </Title>
              <div className={styles.modelCardContent}>
                <Form
                  form={modelForm}
                  layout="vertical"
                  onValuesChange={handleModelSettingsChange}
                >
                  <div className={styles.modelSettingsGrid}>
                    <Form.Item label={t('hermes.modelSettings.defaultProvider', { defaultValue: 'Default provider' })} name="defaultProvider">
                      <Select
                        allowClear
                        showSearch
                        options={providerOptions}
                        placeholder={t('hermes.modelSettings.defaultProviderPlaceholder', { defaultValue: 'Select a provider' })}
                      />
                    </Form.Item>
                    <Form.Item label={t('hermes.modelSettings.defaultModel', { defaultValue: 'Default model' })} name="defaultModel">
                      <Select
                        allowClear
                        showSearch
                        options={modelOptions}
                        placeholder={t('hermes.modelSettings.defaultModelPlaceholder', { defaultValue: 'Select a model' })}
                      />
                    </Form.Item>
                    <Form.Item label={t('hermes.modelSettings.reasoningEffort', { defaultValue: 'Default reasoning level' })} name="reasoningEffort">
                      <Select
                        allowClear
                        options={HERMES_REASONING_LEVELS.map((level) => ({ value: level, label: level }))}
                        placeholder={t('hermes.modelSettings.reasoningEffortPlaceholder', { defaultValue: 'Default: medium' })}
                      />
                    </Form.Item>
                  </div>
                </Form>
              </div>
            </div>
          </div>

          <div
            id="hermes-providers"
            className={styles.hermesSection}
            data-hermes-sidebar-section="true"
            data-sidebar-title={t('hermes.provider.title', { defaultValue: 'Providers' })}
          >
            <Collapse
              className={styles.collapseCard}
              items={[
                {
                  key: 'providers',
                  label: (
                    <Space>
                      <ApiOutlined />
                      <Text strong>{t('hermes.provider.title', { defaultValue: 'Providers' })}</Text>
                    </Space>
                  ),
                  extra: (
                    <Space size={4} wrap onClick={(event) => event.stopPropagation()}>
                      <Button
                        type="link"
                        size="small"
                        style={{ fontSize: 12, display: 'inline-flex', alignItems: 'center', gap: 4 }}
                        onClick={(e) => {
                          e.stopPropagation();
                          if (providerBatch.selectionMode) {
                            providerBatch.exitSelection();
                          } else {
                            providerBatch.enterSelection();
                          }
                        }}
                      >
                        <CheckSquareOutlined style={{ fontSize: 13, lineHeight: 1 }} />
                        <span>
                          {providerBatch.selectionMode
                            ? t('common.batch.exit')
                            : t('common.batch.manage')}
                        </span>
                      </Button>
                      {providerBatch.selectionMode && (
                        <ProviderBatchToolbar
                          hasSelection={providerBatch.hasSelection}
                          visibleCount={batchSelectableIds.length}
                          isAllSelected={providerBatch.isAllSelected}
                          indeterminate={providerBatch.indeterminate}
                          onSelectAll={providerBatch.selectAllFiltered}
                          onBatchDelete={providerBatch.batchDelete}
                          disabled={loading}
                        />
                      )}
                      <ProviderSearchInput value={providerKeyword} onChange={setProviderKeyword} />
                      <ProviderSortDropdown
                        mode={sortMode}
                        modes={PROVIDER_SORT_MODES_BASIC}
                        onChange={setSortMode}
                      />
                      <Button
                        type="link"
                        size="small"
                        style={{ fontSize: 12 }}
                        icon={<ThunderboltOutlined />}
                        loading={batchTestingProviders}
                        onClick={handleBatchTestProviders}
                      >
                        {t('common.batchTest')}
                      </Button>
                      <Button
                        type="link"
                        size="small"
                        style={{ fontSize: 12 }}
                        icon={<PlusOutlined />}
                        onClick={() => setProviderModal({})}
                      >
                        {t('hermes.provider.addSupplier', { defaultValue: 'Add provider' })}
                      </Button>
                    </Space>
                  ),
                  children: (
                    <div>
                      {runtimeConfig?.providers.length
                        ? (
                            <div className={styles.providerList}>
                              {visibleProviders.length ? (
                                visibleProviders.map(renderProvider)
                              ) : (
                                <ProviderSearchEmpty />
                              )}
                            </div>
                          )
                        : <Empty description={t('hermes.provider.emptyText', { defaultValue: 'No providers configured.' })} />}
                      <div style={{ marginTop: 12 }}>
                        <Space wrap>
                          <Button
                            type="dashed"
                            icon={<ImportOutlined />}
                            onClick={() => setImportModalOpen(true)}
                          >
                            {t('opencode.provider.importFavorite')}
                          </Button>
                          {allApiHubAvailable && (
                            <Button
                              type="dashed"
                              icon={<AllApiHubIcon />}
                              onClick={() => setAllApiHubImportModalOpen(true)}
                            >
                              {t('common.allApiHub.importFromAllApiHub')}
                            </Button>
                          )}
                          {ccSwitchAvailable && (
                            <Button
                              type="dashed"
                              icon={<ImportOutlined />}
                              onClick={() => setCcSwitchImportModalOpen(true)}
                            >
                              {t('common.ccSwitch.importFromCcSwitch')}
                            </Button>
                          )}
                        </Space>
                      </div>
                    </div>
                  ),
                },
              ]}
            />
          </div>

          <div
            id="hermes-global-prompt"
            className={`${styles.hermesSection} ${styles.promptSection}`}
            data-hermes-sidebar-section="true"
            data-sidebar-title={t('hermes.prompt.title', { defaultValue: 'Global Prompt' })}
          >
            <GlobalPromptSettings
              translationKeyPrefix="hermes.prompt"
              service={hermesPromptApi}
              collapseKey="hermes-prompt"
              onUpdated={async () => {
                await loadConfig(true);
                await refreshTrayMenu();
              }}
            />
          </div>

          <div
            id="hermes-memory"
            className={styles.hermesSection}
            data-hermes-sidebar-section="true"
            data-sidebar-title={t('hermes.memory.title', { defaultValue: 'Memory' })}
          >
            <HermesMemoryPanel />
          </div>

          <div
            id="hermes-other-configuration"
            className={styles.hermesSection}
            data-hermes-sidebar-section="true"
            data-sidebar-title={t('hermes.otherConfig.title', { defaultValue: 'Other Configuration' })}
          >
            <Collapse
              style={{ marginBottom: 0 }}
              items={[
                {
                  key: 'other',
                  label: (
                    <Space>
                      <SettingOutlined />
                      <Text strong>
                        {t('hermes.otherConfig.title', { defaultValue: 'Other Configuration' })}
                      </Text>
                    </Space>
                  ),
                  children: (
                    <Form.Item
                      help={
                        <span>
                          <Text type="secondary">
                            {t('hermes.otherConfig.hint', {
                              defaultValue: 'Top-level config.yaml keys not managed by this page (agent, etc.).',
                            })}
                          </Text>
                          ，<span style={{ color: 'var(--ant-color-primary)' }}>{t('hermes.otherConfig.autoSaveHint', { defaultValue: 'Auto-saves on blur. Keys removed in the editor are kept on disk.' })}</span>
                        </span>
                      }
                      style={{ marginBottom: 0 }}
                    >
                      <JsonEditor
                        value={otherSettings}
                        height={260}
                        onChange={(nextValue, nextIsValid) => {
                          setOtherSettings(
                            nextValue && typeof nextValue === 'object' && !Array.isArray(nextValue)
                              ? nextValue as Record<string, unknown>
                              : {},
                          );
                          setOtherSettingsValid(nextIsValid);
                        }}
                        onBlur={handleOtherSettingsBlur}
                      />
                    </Form.Item>
                  ),
                },
              ]}
            />
          </div>

          <div
            id="hermes-session-manager"
            className={styles.hermesSection}
            data-hermes-sidebar-section="true"
            data-sidebar-title={t('sessionManager.title')}
          >
            <SessionManagerPanel tool="hermes" />
          </div>
        </div>

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(runtimeConfig?.rootPathInfo || null)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        <Modal
          title={providerModal?.provider
            ? t('hermes.provider.editSupplierTitle', { defaultValue: '编辑供应商', name: providerModal.provider.displayName })
            : t('hermes.provider.addSupplierTitle', { defaultValue: '添加供应商' })}
          open={!!providerModal}
          width={720}
          confirmLoading={saving}
          onCancel={() => setProviderModal(null)}
          onOk={handleSaveProviderModal}
          destroyOnHidden
        >
          <Form form={providerModalForm} layout="vertical" className={styles.providerForm}>
            <div className={styles.modalSection}>
              <div className={styles.modalGrid}>
                <Form.Item
                  label={t('hermes.provider.providerKey', { defaultValue: '供应商 Key' })}
                  name="providerKey"
                  rules={[{ required: true, message: t('hermes.provider.providerKeyRequired', { defaultValue: '请输入供应商 Key' }) }]}
                >
                  <Input
                    disabled={!!providerModal?.provider}
                    placeholder={t('hermes.provider.providerKeyPlaceholder', { defaultValue: '如 anthropic' })}
                  />
                </Form.Item>
                <Form.Item
                  label={t('hermes.provider.displayName', { defaultValue: '显示名称' })}
                  name="providerDisplayName"
                >
                  <Input
                    placeholder={t('hermes.provider.displayNamePlaceholder', { defaultValue: '供应商显示名称' })}
                  />
                </Form.Item>
                <Form.Item label={t('hermes.provider.apiMode', { defaultValue: 'API mode' })} name="apiMode">
                  <Select
                    allowClear
                    showSearch
                    options={HERMES_API_MODE_OPTIONS}
                    placeholder={t('hermes.provider.apiModePlaceholder', { defaultValue: 'anthropic / openai / ...' })}
                  />
                </Form.Item>
                <Form.Item label={t('hermes.provider.baseUrl', { defaultValue: 'Base URL' })} name="baseUrl">
                  <Input placeholder="https://api.anthropic.com" />
                </Form.Item>
                <Form.Item
                  label={t('hermes.provider.apiKey', { defaultValue: 'API key' })}
                  name="providerApiKey"
                >
                  <Input.Password autoComplete="off" />
                </Form.Item>
              </div>
            </div>

            <div className={styles.advancedToggle}>
              <Button
                type="link"
                onClick={() => setProviderAdvancedExpanded(!providerAdvancedExpanded)}
                className={styles.advancedToggleButton}
              >
                {providerAdvancedExpanded ? <DownOutlined /> : <RightOutlined />}
                <span>{t('common.advancedSettings', { defaultValue: 'Advanced settings' })}</span>
                <Tooltip title={t('hermes.provider.advancedHint', { defaultValue: 'Full provider config that is written to config.yaml (name is set from the provider name).' })}>
                  <QuestionCircleOutlined style={{ color: 'var(--color-text-tertiary)' }} />
                </Tooltip>
              </Button>
            </div>
            {providerAdvancedExpanded && (
              <div className={styles.modalSection}>
                <div className={styles.advancedEditor}>
                  <Text type="secondary">
                    <FileTextOutlined style={{ marginRight: 4 }} />
                    {t('hermes.provider.configJson', { defaultValue: 'Provider config (JSON)' })}
                  </Text>
                  <JsonEditor
                    value={isRecordEmpty(providerJson) ? undefined : providerJson}
                    height={240}
                    mode="text"
                    onChange={(value, isValid) => {
                      if (isValid) {
                        setProviderJson(asRecord(value));
                      }
                      setProviderJsonValid(isValid);
                    }}
                  />
                </div>
              </div>
            )}
          </Form>
        </Modal>

        <ModelFormModal
          open={!!modelModal}
          width={560}
          isEdit={!!modelModal?.modelId}
          initialValues={modelModalInitialValues}
          existingIds={modelModal?.modelId ? [] : modelModalExistingIds}
          showOptions={false}
          showVariants={false}
          showModalities={false}
          showInputTypes={false}
          showApi={false}
          showReasoning
          showThinkingLevelMap={false}
          showThinkingLevel
          thinkingLevelOptions={HERMES_REASONING_LEVELS.map((level) => ({ value: level, label: level }))}
          showOmpThinking={false}
          showCompat={false}
          showCost={false}
          showExtraParams
          limitRequired={false}
          nameRequired={false}
          npmType={modelModalProvider?.apiMode ? hermesApiModeToSdkName(modelModalProvider.apiMode) : undefined}
          onCancel={() => setModelModal(null)}
          onSuccess={handleSaveModel}
          onDuplicateId={() => message.error(t('hermes.model.idExists', { defaultValue: 'Model ID already exists.' }))}
          i18nPrefix="pi"
        />

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
          removableModelIds={connectivityInfo?.modelIds}
          onRemoveModels={handleRemoveConnectivityModels}
          onCancel={() => setConnectivityModalOpen(false)}
        />

        {fetchModelsProviderInfo && (
          <FetchModelsModal
            open={fetchModelsModalOpen}
            providerId={fetchModelsProviderInfo.providerId}
            providerName={fetchModelsProviderInfo.providerName}
            baseUrl={fetchModelsProviderInfo.baseUrl}
            apiKey={fetchModelsProviderInfo.apiKey}
            sdkType={fetchModelsProviderInfo.sdkType}
            existingModelIds={fetchModelsProviderInfo.existingModelIds}
            onCancel={() => setFetchModelsModalOpen(false)}
            onSuccess={handleFetchModelsSuccess}
          />
        )}

        <ImportProviderModal
          open={importModalOpen}
          onClose={() => setImportModalOpen(false)}
          onImport={handleImportFavoriteProviders}
          existingProviderIds={existingFavoriteProviderIds}
          providerFilter={(provider) => isFavoriteProviderForSource('hermes', provider)}
        />

        {allApiHubAvailable && (
          <ImportFromAllApiHubModalForTool
            open={allApiHubImportModalOpen}
            existingProviderIds={hermesProviders.map((provider) => provider.providerKey)}
            onCancel={() => setAllApiHubImportModalOpen(false)}
            onImport={handleImportFromAllApiHub}
            listProviders={listHermesAllApiHubProviders}
            resolveProviders={resolveHermesAllApiHubProviders}
          />
        )}

        {ccSwitchAvailable && (
          <ImportFromCcSwitchModal
            open={ccSwitchImportModalOpen}
            appType="claude"
            existingProviderIds={hermesProviders.map((provider) => provider.providerKey)}
            onClose={() => setCcSwitchImportModalOpen(false)}
            onImport={handleImportFromCcSwitch}
          />
        )}

        <FileConfigPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('hermes.preview.title', { defaultValue: 'Preview config' })}
          files={[
            {
              key: 'config',
              label: runtimeConfig?.configPath?.split(/[\\/]/).pop() || 'config.yaml',
              content: runtimeConfig?.configContent ?? runtimeConfig?.config,
              language: 'yaml',
            },
            {
              key: 'prompt',
              label: runtimeConfig?.promptPath?.split(/[\\/]/).pop() || 'SOUL.md',
              content: runtimeConfig?.promptContent,
              language: 'markdown',
            },
          ]}
        />

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={async (visible) => {
            await setSidebarHidden('hermes', !visible);
          }}
        >
          <CliManualPathSetting commandName="hermes" labelKey="subModules.hermes" />
        </SidebarSettingsModal>

        {shareModal}
      </SectionSidebarLayout>
    </Spin>
  );
};

export default HermesPage;
