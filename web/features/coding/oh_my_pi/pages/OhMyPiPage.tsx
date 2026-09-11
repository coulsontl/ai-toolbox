import React from 'react';
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
  Switch,
  Tooltip,
  Typography,
  message,
} from 'antd';
import {
  ApiOutlined,
  AppstoreAddOutlined,
  CheckSquareOutlined,
  CloudDownloadOutlined,
  CloudSyncOutlined,
  DatabaseOutlined,
  DeleteOutlined,
  DownOutlined,
  EditOutlined,
  EllipsisOutlined,
  EyeOutlined,
  FileTextOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  MessageOutlined,
  PlusOutlined,
  QuestionCircleOutlined,
  ReloadOutlined,
  RightOutlined,
  RobotOutlined,
  SettingOutlined,
  ThunderboltOutlined,
  ToolOutlined,
  ImportOutlined,
} from '@ant-design/icons';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';

import { useProviderSharing } from '@/features/coding/shared/providerShare';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import JsonEditor from '@/components/common/JsonEditor';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import ProviderCard from '@/components/common/ProviderCard';
import type {
  ModelDisplayData,
  ProviderConnectivityStatusItem,
  ProviderDisplayData,
} from '@/components/common/ProviderCard/types';
import ModelFormModal from '@/components/common/ModelFormModal';
import type { ModelFormValues } from '@/components/common/ModelFormModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchModelsApplyResult } from '@/components/common/FetchModelsModal/types';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import CliManualPathSetting from '@/components/common/CliManualPathSetting';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import { findPresetModelById } from '@/constants/presetModels';
import {
  buildFetchedOmpModel,
  ompApiToSdkName,
} from '../utils/ompFetchedModels';
import ProviderConnectivityTestModal from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import {
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
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
import {
  fetchRemotePresetModels,
  hasAllApiHubExtension,
  refreshTrayMenu,
} from '@/services/appApi';
import {
  buildFavoriteProviderOptions,
  buildFavoriteProviderStorageKey,
  extractFavoriteProviderRawId,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  type PiFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import {
  upsertFavoriteProvider,
  type OpenCodeAllApiHubProvider,
  type OpenCodeFavoriteProvider,
} from '@/services/opencodeApi';
import { useSettingsStore } from '@/stores';
import {
  PI_INPUT_TYPES,
  PI_THINKING_LEVEL_KEYS,
  buildOmpThinkingFromPreset,
  getOmpModelDefaultThinkingLevel,
  getOmpModelThinkingLevels,
} from '@/utils/ompModelMetadata';
import {
  deleteOmpRuntimeProvider,
  getOmpSettingsConfig,
  readOmpRuntimeConfig,
  saveOmpModelSettings,
  saveOmpModelsProvider,
  saveOmpOtherSettings,
  saveOmpSettingsConfig,
} from '@/services/ohMyPiApi';
import { ohMyPiPromptApi } from '@/services/ohMyPiPromptApi';
import type {
  OmpRuntimeConfig,
  OmpRuntimeProviderView,
} from '@/types/ohMyPi';
import type { OpenCodeModel, OpenCodeProvider } from '@/types/opencode';

import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import { extractOmpProviderFromCcSwitch } from '../utils/importMapping';
import OmpExtensionsSection from '../components/OmpExtensionsSection';
import styles from './OhMyPiPage.module.less';

const { Title, Text, Link } = Typography;

// OMP 与 Pi 复用同一套收藏供应商结构。
type OmpFavoriteProviderPayload = PiFavoriteProviderPayload;

interface ProviderJsonModalState {
  provider?: OmpRuntimeProviderView;
}

interface OmpModelModalState {
  provider: OmpRuntimeProviderView;
  modelId?: string;
  model?: Record<string, unknown>;
}

const PI_API_OPTIONS = [
  'openai-completions',
  'openai-responses',
  'anthropic-messages',
  'google-generative-ai',
].map((value) => ({ value, label: value }));

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'pi-model-settings': <RobotOutlined />,
  'pi-providers': <DatabaseOutlined />,
  'pi-extensions': <AppstoreAddOutlined />,
  'pi-global-prompt': <FileTextOutlined />,
  'pi-other-configuration': <ToolOutlined />,
  'pi-session-manager': <MessageOutlined />,
};

const asRecord = (value: unknown): Record<string, unknown> => (
  value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
);

const getStringField = (value: Record<string, unknown>, key: string): string => {
  const fieldValue = value[key];
  return typeof fieldValue === 'string' ? fieldValue : '';
};

const getNumberField = (value: Record<string, unknown>, key: string): number | undefined => {
  const fieldValue = value[key];
  return typeof fieldValue === 'number' && Number.isFinite(fieldValue) ? fieldValue : undefined;
};

const stringifyRecordField = (value: unknown): string | undefined => {
  const record = asRecord(value);
  return isRecordEmpty(record) ? undefined : JSON.stringify(record, null, 2);
};

const stringifyStringArrayField = (value: unknown): string | undefined => {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const strings = value.filter((entry): entry is string => typeof entry === 'string');
  return strings.length > 0 ? JSON.stringify(strings) : undefined;
};

const parseJsonRecord = (value: string | undefined): Record<string, unknown> => {
  if (!value) {
    return {};
  }
  try {
    return asRecord(JSON.parse(value));
  } catch {
    return {};
  }
};

const parseStringArray = (value: string | undefined): string[] => {
  if (!value) {
    return [];
  }
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed)
      ? parsed.filter((entry): entry is string => typeof entry === 'string')
      : [];
  } catch {
    return [];
  }
};

const getProviderModelRecords = (
  providerConfig: Record<string, unknown> | undefined,
): Array<{ id: string; model: Record<string, unknown> }> => {
  if (!providerConfig) {
    return [];
  }
  const models = providerConfig.models;
  if (!Array.isArray(models)) {
    return [];
  }
  return models
    .map((model) => {
      if (typeof model === 'string') {
        return { id: model, model: { id: model } };
      }
      if (model && typeof model === 'object' && typeof (model as Record<string, unknown>).id === 'string') {
        return {
          id: (model as Record<string, string>).id,
          model: model as Record<string, unknown>,
        };
      }
      return null;
    })
    .filter((entry): entry is { id: string; model: Record<string, unknown> } => !!entry);
};

const setOptionalStringField = (
  target: Record<string, unknown>,
  key: string,
  value: unknown,
) => {
  if (typeof value === 'string' && value.trim()) {
    target[key] = value.trim();
  } else {
    delete target[key];
  }
};

const isRecordEmpty = (value: Record<string, unknown>): boolean => Object.keys(value).length === 0;

const createDefaultProviderConfig = (): Record<string, unknown> => ({
  api: 'openai-completions',
  baseUrl: '',
  models: [],
});

const hasProviderConfigContent = (providerConfig: Record<string, unknown>): boolean => (
  Object.values(providerConfig).some((value) => {
    if (value === null || value === undefined) {
      return false;
    }
    if (typeof value === 'string') {
      return value.trim() !== '';
    }
    if (Array.isArray(value)) {
      return value.length > 0;
    }
    if (typeof value === 'object') {
      return !isRecordEmpty(asRecord(value));
    }
    return true;
  })
);

const getOmpModelThinkingLevelOptions = (
  model: Record<string, unknown> | undefined,
): Array<{ value: string; label: string }> => {
  const levels = getOmpModelThinkingLevels(model);
  if (levels.length === 0) {
    return [];
  }
  const levelSet = new Set(levels);
  const optionSet = new Set<string>();
  const options: Array<{ value: string; label: string }> = [];
  // `off`(关闭思考)是独立于级别区间的选项,恒可为 defaultThinkingLevel。
  options.push({ value: 'off', label: 'off' });
  optionSet.add('off');
  // 标准级别始终打头,再附上模型声明的扩展级别(去重、保序)。
  for (const levelKey of PI_THINKING_LEVEL_KEYS) {
    if (levelSet.has(levelKey) && !optionSet.has(levelKey)) {
      optionSet.add(levelKey);
      options.push({ value: levelKey, label: levelKey });
    }
  }
  for (const levelKey of levels) {
    if (levelSet.has(levelKey) && !optionSet.has(levelKey)) {
      optionSet.add(levelKey);
      options.push({ value: levelKey, label: levelKey });
    }
  }
  // OMP 支持 `auto`(自动选择思考级别)作为全局默认思考级别选项。
  options.push({ value: 'auto', label: 'auto' });
  return options;
};

const isOmpThinkingLevelSupported = (
  thinkingLevel: string | undefined,
  model: Record<string, unknown> | undefined,
): boolean => {
  if (!thinkingLevel) {
    return true;
  }
  return getOmpModelThinkingLevelOptions(model).some((option) => option.value === thinkingLevel);
};

const asStringRecord = (value: unknown): Record<string, string> => {
  const record = asRecord(value);
  return Object.fromEntries(
    Object.entries(record).filter((entry): entry is [string, string] => typeof entry[1] === 'string'),
  );
};

const sdkNameToOmpApi = (sdkName?: string): string => {
  switch (sdkName) {
    case '@ai-sdk/anthropic':
      return 'anthropic-messages';
    case '@ai-sdk/google':
      return 'google-generative-ai';
    default:
      return 'openai-completions';
  }
};

const buildOmpModelFromOpenCodeModel = (
  modelId: string,
  model: OpenCodeModel,
  api?: string,
): Record<string, unknown> => {
  const inputTypes = (model.modalities?.input ?? []).filter((inputType) => PI_INPUT_TYPES.has(inputType));
  const thinking = buildOmpThinkingFromPreset(model.variants, api);

  return {
    id: model.id || modelId,
    name: model.name || modelId,
    ...(typeof model.reasoning === 'boolean' ? { reasoning: model.reasoning } : {}),
    ...(inputTypes.length > 0 ? { input: inputTypes } : {}),
    ...(typeof model.limit?.context === 'number' ? { contextWindow: model.limit.context } : {}),
    ...(typeof model.limit?.output === 'number' ? { maxTokens: model.limit.output } : {}),
    ...(thinking ? { thinking } : {}),
  };
};

const buildOmpModelsProviderFromOpenCodeProvider = (
  provider: OpenCodeProvider,
): Record<string, unknown> => {
  const options = provider.options ?? {};
  const headers = asStringRecord(options.headers);
  const api = sdkNameToOmpApi(provider.npm);
  const models = Object.entries(provider.models || {}).map(([modelId, model]) =>
    buildOmpModelFromOpenCodeModel(modelId, model, api),
  );

  return {
    ...(provider.name ? { name: provider.name } : {}),
    api,
    ...(options.baseURL ? { baseUrl: options.baseURL } : {}),
    ...(options.apiKey ? { apiKey: options.apiKey } : {}),
    ...(!isRecordEmpty(headers) ? { headers } : {}),
    models,
  };
};

const buildOmpOpenCodeProvider = (
  provider: OmpRuntimeProviderView,
  providerConfig: Record<string, unknown> = provider.modelsProvider ?? {},
): OpenCodeProvider => {
  const models = Object.fromEntries(
    getProviderModelRecords(providerConfig).map((entry) => [
      entry.id,
      {
        ...entry.model,
        id: undefined,
        name: getStringField(entry.model, 'name') || entry.id,
      },
    ]),
  );
  const api = getStringField(providerConfig, 'api');
  const headers = asStringRecord(providerConfig.headers);

  return {
    npm: ompApiToSdkName(api),
    name: provider.displayName,
    options: {
      baseURL: getStringField(providerConfig, 'baseUrl'),
      apiKey: getStringField(providerConfig, 'apiKey'),
      ...(isRecordEmpty(headers) ? {} : { headers }),
    },
    models,
  };
};

const buildOmpFavoriteProviderConfig = (
  providerKey: string,
  displayName: string,
  modelsProvider: Record<string, unknown>,
  credential?: Record<string, unknown>,
): OpenCodeProvider => {
  const favoriteProvider = buildOmpOpenCodeProvider({
    providerKey,
    displayName: displayName || getStringField(modelsProvider, 'name') || providerKey,
    sources: ['models_yml'],
    categories: ['custom'],
    credentialKind: credential && !isRecordEmpty(credential) ? 'api_key' : 'none',
    credential,
    modelsProvider,
    runtimeFiles: [],
    isBuiltin: false,
    isOverride: false,
    isDefault: false,
    modelIds: getProviderModelRecords(modelsProvider).map((entry) => entry.id),
  });

  const payload: OmpFavoriteProviderPayload = {
    providerKey,
    ...(credential && !isRecordEmpty(credential) ? { credential } : {}),
    modelsProvider,
  };

  return buildFavoriteProviderOptions(favoriteProvider, payload);
};

const resolveOmpFavoriteProviderPayload = (
  favoriteProvider: OpenCodeFavoriteProvider,
): OmpFavoriteProviderPayload => {
  const payload = getFavoriteProviderPayload<OmpFavoriteProviderPayload>(favoriteProvider);
  if (payload?.providerKey && payload.modelsProvider) {
    return payload;
  }

  return {
    providerKey: extractFavoriteProviderRawId('omp', favoriteProvider.providerId),
    modelsProvider: buildOmpModelsProviderFromOpenCodeProvider(favoriteProvider.providerConfig),
  };
};

const normalizeOmpFavoriteString = (value: unknown): string => (
  typeof value === 'string' ? value.trim().toLowerCase() : ''
);

const normalizeOmpFavoriteBaseUrl = (value: unknown): string => (
  normalizeOmpFavoriteString(value).replace(/\/+$/, '')
);

const buildStableObjectSignature = (value: unknown): unknown => {
  if (Array.isArray(value)) {
    return value.map((item) => buildStableObjectSignature(item));
  }
  if (value && typeof value === 'object') {
    return Object.keys(value as Record<string, unknown>)
      .sort()
      .reduce<Record<string, unknown>>((result, key) => {
        result[key] = buildStableObjectSignature((value as Record<string, unknown>)[key]);
        return result;
      }, {});
  }
  return value;
};

const getOmpFavoriteProviderIdentity = (favoriteProvider: OpenCodeFavoriteProvider): string => {
  const payload = resolveOmpFavoriteProviderPayload(favoriteProvider);
  const modelsProvider = payload.modelsProvider;
  const providerOptions = favoriteProvider.providerConfig.options ?? {};
  const api = getStringField(modelsProvider, 'api') || sdkNameToOmpApi(favoriteProvider.providerConfig.npm);
  const baseUrl = getStringField(modelsProvider, 'baseUrl') || providerOptions.baseURL;
  const apiKey = getStringField(modelsProvider, 'apiKey') || providerOptions.apiKey;
  const headers = isRecordEmpty(asRecord(modelsProvider.headers))
    ? asRecord(providerOptions.headers)
    : asRecord(modelsProvider.headers);
  if (!baseUrl && !apiKey && isRecordEmpty(headers)) {
    return `provider-key:${payload.providerKey}`;
  }

  return JSON.stringify({
    api: normalizeOmpFavoriteString(api),
    baseUrl: normalizeOmpFavoriteBaseUrl(baseUrl),
    apiKey: normalizeOmpFavoriteString(apiKey),
    headers: buildStableObjectSignature(headers),
  });
};

const getOmpFavoriteProviderModelCount = (favoriteProvider: OpenCodeFavoriteProvider): number => {
  const payload = resolveOmpFavoriteProviderPayload(favoriteProvider);
  return getProviderModelRecords(payload.modelsProvider).length;
};

const dedupeOmpFavoriteProviders = (
  favoriteProviders: OpenCodeFavoriteProvider[],
  currentStorageKeys: Set<string>,
): OpenCodeFavoriteProvider[] => {
  const providerByIdentity = new Map<string, OpenCodeFavoriteProvider>();

  favoriteProviders.forEach((favoriteProvider) => {
    const identity = getOmpFavoriteProviderIdentity(favoriteProvider);
    const existingProvider = providerByIdentity.get(identity);
    if (!existingProvider) {
      providerByIdentity.set(identity, favoriteProvider);
      return;
    }

    const existingIsCurrent = currentStorageKeys.has(existingProvider.providerId);
    const nextIsCurrent = currentStorageKeys.has(favoriteProvider.providerId);
    const existingModelCount = getOmpFavoriteProviderModelCount(existingProvider);
    const nextModelCount = getOmpFavoriteProviderModelCount(favoriteProvider);
    const shouldReplaceExisting =
      (!existingIsCurrent && nextIsCurrent) ||
      (existingIsCurrent === nextIsCurrent && nextModelCount > existingModelCount) ||
      (existingIsCurrent === nextIsCurrent &&
        nextModelCount === existingModelCount &&
        favoriteProvider.updatedAt > existingProvider.updatedAt);

    if (shouldReplaceExisting) {
      providerByIdentity.set(identity, favoriteProvider);
    }
  });

  return Array.from(providerByIdentity.values());
};

const OhMyPiPage: React.FC = () => {
  const { t } = useTranslation();
  const { shareProvider, shareModal } = useProviderSharing('omp', () => loadConfig());
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [refreshingModels, setRefreshingModels] = React.useState(false);
  const [extensionsRefreshKey, setExtensionsRefreshKey] = React.useState(0);
  const [runtimeConfig, setRuntimeConfig] = React.useState<OmpRuntimeConfig | null>(null);
  const [modelForm] = Form.useForm();
  const [providerModal, setProviderModal] = React.useState<ProviderJsonModalState | null>(null);
  const [providerModalForm] = Form.useForm();
  const [providerConfigJson, setProviderConfigJson] = React.useState<Record<string, unknown>>({});
  const [providerHeadersJson, setProviderHeadersJson] = React.useState<Record<string, unknown>>({});
  const [providerCompatJson, setProviderCompatJson] = React.useState<Record<string, unknown>>({});
  const [providerModelOverridesJson, setProviderModelOverridesJson] = React.useState<Record<string, unknown>>({});
  const [providerConfigJsonValid, setProviderConfigJsonValid] = React.useState(true);
  const [providerHeadersJsonValid, setProviderHeadersJsonValid] = React.useState(true);
  const [providerCompatJsonValid, setProviderCompatJsonValid] = React.useState(true);
  const [providerModelOverridesJsonValid, setProviderModelOverridesJsonValid] = React.useState(true);
  const [providerAdvancedExpanded, setProviderAdvancedExpanded] = React.useState(false);
  const [ompModelModal, setOmpModelModal] = React.useState<OmpModelModalState | null>(null);
  const [batchDeleteProviderId, setBatchDeleteProviderId] = React.useState<string | null>(null);
  const [selectedModelIdsByProvider, setSelectedModelIdsByProvider] = React.useState<Record<string, string[]>>({});
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string | null>(null);
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [connectivityProviderId, setConnectivityProviderId] = React.useState<string | null>(null);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);
  const [otherSettings, setOtherSettings] = React.useState<Record<string, unknown>>({});
  const [otherSettingsValid, setOtherSettingsValid] = React.useState(true);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
    const modelSettingsSaveSeqRef = React.useRef(0);
  const sidebarHidden = sidebarHiddenByPage.pi;

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'pi-model-settings',
      title: t('ohMyPi.modelSettings.title'),
      order: 1,
    },
    {
      id: 'pi-providers',
      title: t('ohMyPi.provider.title'),
      order: 2,
    },
    {
      id: 'pi-extensions',
      title: t('ohMyPi.extensions.title'),
      order: 3,
    },
    {
      id: 'pi-global-prompt',
      title: t('ohMyPi.prompt.title'),
      order: 4,
    },
    {
      id: 'pi-other-configuration',
      title: t('ohMyPi.otherConfig.title'),
      order: 5,
    },
    {
      id: 'pi-session-manager',
      title: t('sessionManager.title'),
      order: 6,
    },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const config = await readOmpRuntimeConfig();
      setRuntimeConfig(config);
      setOtherSettings(config.otherSettings || {});
      modelForm.setFieldsValue({
        defaultProvider: config.modelSettings.providerKey || undefined,
        defaultModel: config.modelSettings.modelId || undefined,
        defaultThinkingLevel: config.modelSettings.thinkingLevel || undefined,
      });
    } catch (error) {
      console.error('Failed to load Pi runtime config:', error);
      message.error(t('common.error'));
    } finally {
      if (!silent) {
        setLoading(false);
      }
    }
  }, [modelForm, t]);

  React.useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  React.useEffect(() => {
    const checkAllApiHubAvailability = async () => {
      try {
        const available = await hasAllApiHubExtension();
        setAllApiHubAvailable(available);
      } catch (error) {
        console.error('Failed to check All API Hub availability:', error);
        setAllApiHubAvailable(false);
      }
    };

    checkAllApiHubAvailability();

    const checkCcSwitchAvailability = async () => {
      try {
        setCcSwitchAvailable(await hasCcSwitchDb());
      } catch (error) {
        console.error('Failed to check CC Switch availability:', error);
        setCcSwitchAvailable(false);
      }
    };

    checkCcSwitchAvailability();
  }, []);

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

  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig({
    t,
    translationKeyPrefix: 'ohMyPi',
    defaultConfig: '{}',
    loadConfig,
    getCommonConfig: getOmpSettingsConfig,
    saveCommonConfig: saveOmpSettingsConfig,
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
    const current = runtimeConfig?.modelSettings.providerKey;
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
  const selectedModelRecord = React.useMemo(() => {
    if (!selectedProvider || !selectedDefaultModel) {
      return undefined;
    }
    return getProviderModelRecords(selectedProvider.modelsProvider).find(
      (entry) => entry.id === selectedDefaultModel,
    )?.model;
  }, [selectedDefaultModel, selectedProvider]);
  const thinkingLevelOptions = React.useMemo(
    () => getOmpModelThinkingLevelOptions(selectedModelRecord),
    [selectedModelRecord],
  );
  const modelOptions = React.useMemo(() => {
    const options = new Set<string>();
    selectedProvider?.modelIds?.forEach((modelId) => options.add(modelId));
    const current = selectedDefaultModel || runtimeConfig?.modelSettings.modelId;
    if (current) {
      options.add(current);
    }
    return Array.from(options).map((modelId) => ({ value: modelId, label: modelId }));
  }, [runtimeConfig?.modelSettings.modelId, selectedDefaultModel, selectedProvider?.modelIds]);

  const ompProviders = React.useMemo(
    () => runtimeConfig?.providers ?? [],
    [runtimeConfig?.providers],
  );
  const existingProviderIds = React.useMemo(
    () => ompProviders.map((provider) => provider.providerKey),
    [ompProviders],
  );
  const existingFavoriteProviderIds = React.useMemo(
    () => existingProviderIds.map((providerId) => buildFavoriteProviderStorageKey('omp', providerId)),
    [existingProviderIds],
  );
  const transformOmpFavoriteProviders = React.useCallback(
    (providers: OpenCodeFavoriteProvider[]) =>
      dedupeOmpFavoriteProviders(providers, new Set(existingFavoriteProviderIds)),
    [existingFavoriteProviderIds],
  );

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!fetchModelsProviderId) {
      return null;
    }
    const provider = ompProviders.find((item) => item.providerKey === fetchModelsProviderId);
    if (!provider) {
      return null;
    }
    const providerConfig = provider.modelsProvider ?? {};
    const api = getStringField(providerConfig, 'api');
    return {
      providerId: provider.providerKey,
      name: provider.displayName,
      baseUrl: getStringField(providerConfig, 'baseUrl'),
      apiKey: getStringField(providerConfig, 'apiKey'),
      headers: asStringRecord(providerConfig.headers),
      sdkName: ompApiToSdkName(api),
      existingModelIds: getProviderModelRecords(provider.modelsProvider).map((entry) => entry.id),
    };
  }, [fetchModelsProviderId, ompProviders]);

  const connectivityInfo = React.useMemo(() => {
    if (!connectivityProviderId) {
      return null;
    }
    const provider = ompProviders.find((item) => item.providerKey === connectivityProviderId);
    if (!provider) {
      return null;
    }
    const providerConfig = provider.modelsProvider ?? {};
    const modelIds = getProviderModelRecords(provider.modelsProvider).map((entry) => entry.id);
    return {
      providerId: provider.providerKey,
      providerName: provider.displayName,
      providerConfig: buildOmpOpenCodeProvider(provider, providerConfig),
      modelIds,
    };
  }, [connectivityProviderId, ompProviders]);

  const translateRuntimeLabel = React.useCallback((prefix: string, value: string): string => (
    t(`${prefix}.${value}`, { defaultValue: value })
  ), [t]);

  const upsertOmpFavoriteProvider = React.useCallback(async (
    providerKey: string,
    modelsProvider: Record<string, unknown>,
    credential?: unknown,
    displayName?: string,
  ) => {
    const credentialRecord = asRecord(credential);
    const favoriteConfig = buildOmpFavoriteProviderConfig(
      providerKey,
      displayName || getStringField(modelsProvider, 'name') || providerKey,
      modelsProvider,
      isRecordEmpty(credentialRecord) ? undefined : credentialRecord,
    );
    await upsertFavoriteProvider(
      buildFavoriteProviderStorageKey('omp', providerKey),
      favoriteConfig,
    );
  }, []);

  const handleModelSettingsChange = async (
    changedValues: Record<string, unknown>,
    allValues: {
      defaultProvider?: string;
      defaultModel?: string;
      defaultThinkingLevel?: string;
    },
  ) => {
    if (!runtimeConfig) {
      return;
    }

    const nextValues = { ...allValues };
    const nextProvider = runtimeConfig.providers.find(
      (provider) => provider.providerKey === nextValues.defaultProvider,
    );
    if (Object.prototype.hasOwnProperty.call(changedValues, 'defaultProvider')) {
      if (
        nextValues.defaultModel
        && nextProvider?.modelIds?.length
        && !nextProvider.modelIds.includes(nextValues.defaultModel)
      ) {
        nextValues.defaultModel = undefined;
        modelForm.setFieldValue('defaultModel', undefined);
      }
    }
    const nextModel = nextProvider && nextValues.defaultModel
      ? getProviderModelRecords(nextProvider.modelsProvider).find(
        (entry) => entry.id === nextValues.defaultModel,
      )?.model
      : undefined;
    const unsupportedThinkingCleared = Boolean(
      nextValues.defaultThinkingLevel
      && !isOmpThinkingLevelSupported(nextValues.defaultThinkingLevel, nextModel),
    );
    if (unsupportedThinkingCleared) {
      nextValues.defaultThinkingLevel = undefined;
      modelForm.setFieldValue('defaultThinkingLevel', undefined);
    }

    // Only treat this as an explicit clear when the thinking-level control was
    // part of this change and ended up empty/undefined, or when the previous
    // value is unsupported for the newly selected model. Changing provider or
    // model alone must not wipe the global defaultThinkingLevel.
    const clearThinkingLevel = unsupportedThinkingCleared || (
      Object.prototype.hasOwnProperty.call(
        changedValues,
        'defaultThinkingLevel',
      ) && !nextValues.defaultThinkingLevel
    );

    const currentSettings = runtimeConfig.modelSettings;
    const nextDefaultProvider = nextValues.defaultProvider ?? '';
    const nextDefaultModel = nextValues.defaultModel ?? '';
    const nextDefaultThinkingLevel = nextValues.defaultThinkingLevel ?? '';
    if (
      (currentSettings.providerKey ?? '') === nextDefaultProvider
      && (currentSettings.modelId ?? '') === nextDefaultModel
      && (currentSettings.thinkingLevel ?? '') === nextDefaultThinkingLevel
    ) {
      return;
    }

    const saveSeq = modelSettingsSaveSeqRef.current + 1;
    modelSettingsSaveSeqRef.current = saveSeq;
    setSaving(true);
    try {
      const nextConfig = await saveOmpModelSettings({
        defaultProvider: nextDefaultProvider,
        defaultModel: nextDefaultModel,
        defaultThinkingLevel: nextDefaultThinkingLevel,
        clearThinkingLevel,
      });
      if (modelSettingsSaveSeqRef.current === saveSeq) {
        setRuntimeConfig(nextConfig);
        setOtherSettings(nextConfig.otherSettings || {});
      }
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Pi model settings:', error);
      if (modelSettingsSaveSeqRef.current === saveSeq) {
        message.error(t('common.error'));
      }
    } finally {
      if (modelSettingsSaveSeqRef.current === saveSeq) {
        setSaving(false);
      }
    }
  };

  const openProviderModal = (
    provider?: OmpRuntimeProviderView,
    options?: { copy?: boolean },
  ) => {
    const isCopy = options?.copy === true;
    const isExistingProviderEdit = !!provider && !isCopy;
    const nextProviderConfigJson = provider?.modelsProvider
      ? asRecord(provider.modelsProvider)
      : isExistingProviderEdit
        ? {}
        : createDefaultProviderConfig();

    setProviderModal({ provider: isCopy ? undefined : provider });
    setProviderConfigJson(nextProviderConfigJson);
    setProviderHeadersJson(asRecord(nextProviderConfigJson.headers));
    setProviderCompatJson(asRecord(nextProviderConfigJson.compat));
    setProviderModelOverridesJson(asRecord(nextProviderConfigJson.modelOverrides));
    setProviderConfigJsonValid(true);
    setProviderHeadersJsonValid(true);
    setProviderCompatJsonValid(true);
    setProviderModelOverridesJsonValid(true);
    setProviderAdvancedExpanded(false);
    providerModalForm.setFieldsValue({
      providerKey: isCopy && provider ? `${provider.providerKey}_copy` : provider?.providerKey,
      displayName: getStringField(nextProviderConfigJson, 'name'),
      api: getStringField(nextProviderConfigJson, 'api') || undefined,
      baseUrl: getStringField(nextProviderConfigJson, 'baseUrl'),
      providerApiKey: getStringField(nextProviderConfigJson, 'apiKey'),
      authHeader: typeof nextProviderConfigJson.authHeader === 'boolean'
        ? nextProviderConfigJson.authHeader
        : undefined,
    });
  };

  const handleSaveProviderModal = async () => {
    if (
      !providerModal
      || !providerConfigJsonValid
      || !providerHeadersJsonValid
      || !providerCompatJsonValid
      || !providerModelOverridesJsonValid
    ) {
      return;
    }
    const values = await providerModalForm.validateFields();
    const providerKey = values.providerKey?.trim();
    if (!providerKey) {
      message.error(t('ohMyPi.provider.providerKeyRequired'));
      return;
    }

    setSaving(true);
    try {
      let nextConfig: OmpRuntimeConfig | null = null;
      const nextProviderConfigJson = { ...providerConfigJson };
      setOptionalStringField(nextProviderConfigJson, 'name', values.displayName);
      setOptionalStringField(nextProviderConfigJson, 'api', values.api);
      setOptionalStringField(nextProviderConfigJson, 'baseUrl', values.baseUrl);
      setOptionalStringField(nextProviderConfigJson, 'apiKey', values.providerApiKey);
      if (
        typeof values.authHeader === 'boolean'
        && (
          values.authHeader
          || Object.prototype.hasOwnProperty.call(providerConfigJson, 'authHeader')
        )
      ) {
        nextProviderConfigJson.authHeader = values.authHeader;
      } else {
        delete nextProviderConfigJson.authHeader;
      }
      if (isRecordEmpty(providerHeadersJson)) {
        delete nextProviderConfigJson.headers;
      } else {
        nextProviderConfigJson.headers = providerHeadersJson;
      }
      if (isRecordEmpty(providerCompatJson)) {
        delete nextProviderConfigJson.compat;
      } else {
        nextProviderConfigJson.compat = providerCompatJson;
      }
      if (isRecordEmpty(providerModelOverridesJson)) {
        delete nextProviderConfigJson.modelOverrides;
      } else {
        nextProviderConfigJson.modelOverrides = providerModelOverridesJson;
      }
      const shouldSaveProviderConfig = !providerModal.provider
        || providerModal.provider.sources.includes('models_yml')
        || hasProviderConfigContent(nextProviderConfigJson);
      if (shouldSaveProviderConfig) {
        nextConfig = await saveOmpModelsProvider({ providerKey, provider: nextProviderConfigJson });
      }
      if (!nextConfig) {
        return;
      }
      if (shouldSaveProviderConfig) {
        try {
          await upsertOmpFavoriteProvider(providerKey, nextProviderConfigJson, undefined, values.displayName);
        } catch (error) {
          console.error('Failed to save Pi favorite provider:', error);
        }
      }
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      setProviderModal(null);
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save Pi provider:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const openOmpModelModal = (
    provider: OmpRuntimeProviderView,
    modelId?: string,
    options?: { copy?: boolean },
  ) => {
    const model = modelId
      ? getProviderModelRecords(provider.modelsProvider).find((entry) => entry.id === modelId)?.model
      : undefined;
    const isCopy = options?.copy === true;
    const nextModel = model ? { ...model } : undefined;
    if (isCopy && nextModel && modelId) {
      nextModel.id = `${modelId}_copy`;
    }

    setOmpModelModal({ provider, modelId: isCopy ? undefined : modelId, model: nextModel });
  };

  const handleSaveOmpModel = async (values: ModelFormValues) => {
    if (!ompModelModal) {
      return;
    }
    const modelId = values.id?.trim();
    if (!modelId) {
      message.error(t('ohMyPi.model.idRequired'));
      return;
    }

    const currentProvider = runtimeConfig?.providers.find(
      (provider) => provider.providerKey === ompModelModal.provider.providerKey,
    ) ?? ompModelModal.provider;
    const existingModels = getProviderModelRecords(currentProvider.modelsProvider);
    const duplicateModel = existingModels.some((entry) => (
      entry.id === modelId && entry.id !== ompModelModal.modelId
    ));
    if (duplicateModel) {
      message.error(t('ohMyPi.model.idExists'));
      return;
    }

    const nextModel = { ...(ompModelModal.model ?? {}) };
    setOptionalStringField(nextModel, 'id', modelId);
    setOptionalStringField(nextModel, 'name', values.name);
    if (typeof values.contextLimit === 'number') {
      nextModel.contextWindow = values.contextLimit;
    } else {
      delete nextModel.contextWindow;
    }
    if (typeof values.outputLimit === 'number') {
      nextModel.maxTokens = values.outputLimit;
    } else {
      delete nextModel.maxTokens;
    }
    if (typeof values.reasoning === 'boolean') {
      nextModel.reasoning = values.reasoning;
    } else {
      delete nextModel.reasoning;
    }
    setOptionalStringField(nextModel, 'api', values.api);
    const inputTypes = parseStringArray(values.inputTypes);
    if (inputTypes.length > 0) {
      nextModel.input = inputTypes;
    } else {
      delete nextModel.input;
    }
    // OMP 用 `thinking` 结构(efforts/defaultLevel)表达思考级别,不识别 Pi 的
    // thinkingLevelMap。编辑框编辑 `thinking`,旧的 thinkingLevelMap 一律移除。
    delete nextModel.thinkingLevelMap;
    const ompThinking = parseJsonRecord(values.thinking);
    if (!isRecordEmpty(ompThinking)) {
      nextModel.thinking = ompThinking;
    } else {
      delete nextModel.thinking;
    }
    const compat = parseJsonRecord(values.compat);
    if (!isRecordEmpty(compat)) {
      nextModel.compat = compat;
    } else {
      delete nextModel.compat;
    }
    // OMP 的 cost 必须 input/output/cacheRead/cacheWrite 四字段齐全,
    // 否则整个 models.yml 校验失败、自定义 provider 被禁用。只填部分则整个 cost 不写。
    const costInput = values.costInput;
    const costOutput = values.costOutput;
    const costCacheRead = values.costCacheRead;
    const costCacheWrite = values.costCacheWrite;
    const costComplete = [costInput, costOutput, costCacheRead, costCacheWrite]
      .every((value) => typeof value === 'number' && Number.isFinite(value));
    if (costComplete) {
      nextModel.cost = {
        input: costInput,
        output: costOutput,
        cacheRead: costCacheRead,
        cacheWrite: costCacheWrite,
      };
    } else {
      delete nextModel.cost;
    }

    let modelWasReplaced = false;
    const nextModels = existingModels.map((entry) => {
      if (entry.id === ompModelModal.modelId) {
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
        ...(currentProvider.modelsProvider ?? {}),
        models: nextModels,
      };
      const nextConfig = await saveOmpModelsProvider({
        providerKey: currentProvider.providerKey,
        provider: nextProviderConfig,
      });
      try {
        await upsertOmpFavoriteProvider(
          currentProvider.providerKey,
          nextProviderConfig,
          currentProvider.credential,
          currentProvider.displayName,
        );
      } catch (error) {
        console.error('Failed to save Pi favorite provider:', error);
      }
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      setOmpModelModal(null);
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save Pi model:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const clearBatchDeleteState = React.useCallback((providerId?: string) => {
    if (providerId) {
      setSelectedModelIdsByProvider((previousState) => {
        if (!previousState[providerId]) {
          return previousState;
        }
        const nextState = { ...previousState };
        delete nextState[providerId];
        return nextState;
      });
      setBatchDeleteProviderId((currentProviderId) => (
        currentProviderId === providerId ? null : currentProviderId
      ));
      return;
    }

    setSelectedModelIdsByProvider({});
    setBatchDeleteProviderId(null);
  }, []);

  const saveProviderModels = async (
    provider: OmpRuntimeProviderView,
    nextModels: Record<string, unknown>[],
  ) => {
    const nextProviderConfig = {
      ...(provider.modelsProvider ?? {}),
      models: nextModels,
    };
    const nextConfig = await saveOmpModelsProvider({
      providerKey: provider.providerKey,
      provider: nextProviderConfig,
    });
    try {
      await upsertOmpFavoriteProvider(
        provider.providerKey,
        nextProviderConfig,
        provider.credential,
        provider.displayName,
      );
    } catch (error) {
      console.error('Failed to save Pi favorite provider:', error);
    }
    setRuntimeConfig(nextConfig);
    setOtherSettings(nextConfig.otherSettings || {});
    await refreshTrayMenu();
    return nextConfig;
  };

  const handleToggleBatchDeleteMode = (providerKey: string) => {
    if (batchDeleteProviderId === providerKey) {
      clearBatchDeleteState(providerKey);
      return;
    }
    setSelectedModelIdsByProvider({});
    setBatchDeleteProviderId(providerKey);
  };

  const handleToggleModelSelection = (providerKey: string, modelId: string, selected: boolean) => {
    setSelectedModelIdsByProvider((previousState) => {
      const currentModelIds = previousState[providerKey] ?? [];
      const nextModelIds = selected
        ? Array.from(new Set([...currentModelIds, modelId]))
        : currentModelIds.filter((id) => id !== modelId);

      if (nextModelIds.length === 0) {
        const nextState = { ...previousState };
        delete nextState[providerKey];
        return nextState;
      }

      return {
        ...previousState,
        [providerKey]: nextModelIds,
      };
    });
  };

  const handleBatchDeleteModels = async (provider: OmpRuntimeProviderView) => {
    const selectedModelIds = selectedModelIdsByProvider[provider.providerKey] ?? [];
    if (selectedModelIds.length === 0) {
      return;
    }

    setSaving(true);
    try {
      const selectedModelIdSet = new Set(selectedModelIds);
      const nextModels = getProviderModelRecords(provider.modelsProvider)
        .filter((entry) => !selectedModelIdSet.has(entry.id))
        .map((entry) => entry.model);
      const nextConfig = await saveProviderModels(provider, nextModels);
      if (
        provider.isDefault
        && nextConfig.modelSettings.modelId
        && selectedModelIdSet.has(nextConfig.modelSettings.modelId)
      ) {
        const updatedConfig = await saveOmpModelSettings({
          defaultProvider: nextConfig.modelSettings.providerKey ?? provider.providerKey,
          defaultModel: '',
          defaultThinkingLevel: '',
        });
        setRuntimeConfig(updatedConfig);
        setOtherSettings(updatedConfig.otherSettings || {});
        modelForm.setFieldValue('defaultModel', undefined);
      }
      clearBatchDeleteState(provider.providerKey);
      message.success(t('ohMyPi.model.batchDeleteSuccess', { count: selectedModelIds.length }));
    } catch (error) {
      console.error('Failed to batch delete Pi models:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleReorderModels = async (provider: OmpRuntimeProviderView, modelIds: string[]) => {
    const currentModelMap = new Map(
      getProviderModelRecords(provider.modelsProvider).map((entry) => [entry.id, entry.model]),
    );
    const nextModels = modelIds
      .map((modelId) => currentModelMap.get(modelId))
      .filter((model): model is Record<string, unknown> => !!model);

    setSaving(true);
    try {
      await saveProviderModels(provider, nextModels);
    } catch (error) {
      console.error('Failed to reorder Pi models:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('omp');
  const [providerKeyword, setProviderKeyword] = React.useState('');
  const visibleProviders = React.useMemo(
    () =>
      sortProviderItems(
        filterProviderItems(ompProviders, providerKeyword, (provider) => [
          provider.providerKey,
          provider.displayName,
          ...(provider.modelIds ?? []),
        ]),
        sortMode,
        { name: (provider) => provider.displayName || provider.providerKey },
        (provider) => lastUsedAt(provider.providerKey),
      ),
    [ompProviders, providerKeyword, sortMode, lastUsedAt],
  );

  const canBatchDeleteProvider = React.useCallback(
    (provider: OmpRuntimeProviderView) => !provider.isDefault
      && (provider.sources.includes('models_yml')
        || Object.prototype.hasOwnProperty.call(provider.modelsProvider ?? {}, 'apiKey')),
    [],
  );

  const handleBatchDeleteProviders = React.useCallback(
    async (providerKeys: string[]): Promise<boolean> => {
      const providersToDelete = ompProviders.filter(
        (provider) => providerKeys.includes(provider.providerKey) && canBatchDeleteProvider(provider),
      );
      if (providersToDelete.length === 0) return false;
      setSaving(true);
      try {
        await backupProvidersBeforeDelete(
          providersToDelete,
          (provider) => upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('omp', provider.providerKey),
            buildOmpFavoriteProviderConfig(
              provider.providerKey, provider.displayName, provider.modelsProvider ?? {},
            ),
          ),
          (provider) => t('common.batch.backupFailed', { name: provider.displayName || provider.providerKey }),
        );
        let nextConfig: OmpRuntimeConfig | null = null;
        for (const provider of providersToDelete) {
          nextConfig = await deleteOmpRuntimeProvider(provider.providerKey);
          clearBatchDeleteState(provider.providerKey);
        }
        if (nextConfig) {
          setRuntimeConfig(nextConfig);
          setOtherSettings(nextConfig.otherSettings || {});
        }
        await refreshTrayMenu();
        message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete OMP providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig(true);
        return false;
      } finally {
        setSaving(false);
      }
    },
    [ompProviders, canBatchDeleteProvider, loadConfig, clearBatchDeleteState, t],
  );

  // Exclude non-deletable providers (no credential and no models_yml source) from batch selection.
  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter(canBatchDeleteProvider).map((provider) => provider.providerKey),
    [visibleProviders, canBatchDeleteProvider],
  );

  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });

  const handleSetPrimaryModel = async (provider: OmpRuntimeProviderView, modelId: string) => {
    const nextModel = getProviderModelRecords(provider.modelsProvider).find(
      (entry) => entry.id === modelId,
    )?.model;
    const currentThinkingLevel = runtimeConfig?.modelSettings.thinkingLevel ?? undefined;
    const nextThinkingLevel = isOmpThinkingLevelSupported(currentThinkingLevel, nextModel)
      ? currentThinkingLevel ?? ''
      : getOmpModelDefaultThinkingLevel(nextModel) ?? '';
    setSaving(true);
    try {
      const nextConfig = await saveOmpModelSettings({
        defaultProvider: provider.providerKey,
        defaultModel: modelId,
        defaultThinkingLevel: nextThinkingLevel,
      });
      noteProviderUsed(provider.providerKey);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      modelForm.setFieldsValue({
        defaultProvider: provider.providerKey,
        defaultModel: modelId,
        defaultThinkingLevel: nextConfig.modelSettings.thinkingLevel || undefined,
      });
      await refreshTrayMenu();
      message.success(t('ohMyPi.model.setAsPrimarySuccess', { name: modelId }));
    } catch (error) {
      console.error('Failed to set Pi default model:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleOpenFetchModels = (providerKey: string) => {
    setFetchModelsProviderId(providerKey);
    setFetchModelsModalOpen(true);
  };

  const handleFetchModelsSuccess = async ({ selectedModels, removedModelIds }: FetchModelsApplyResult) => {
    if (!fetchModelsProviderId) {
      return;
    }
    const provider = ompProviders.find((item) => item.providerKey === fetchModelsProviderId);
    if (!provider) {
      return;
    }

    const removedModelIdSet = new Set(removedModelIds);
    const currentModels = getProviderModelRecords(provider.modelsProvider)
      .filter((entry) => !removedModelIdSet.has(entry.id))
      .map((entry) => entry.model);
    const currentModelIds = new Set(currentModels.map((model) => getStringField(model, 'id')));
    const providerApi = getStringField(provider.modelsProvider ?? {}, 'api');
    selectedModels.forEach((model) => {
      if (!currentModelIds.has(model.id)) {
        const matchedPresetModel = findPresetModelById(model.id, ompApiToSdkName(providerApi));
        currentModels.push(buildFetchedOmpModel(model, matchedPresetModel, providerApi));
      }
    });

    setSaving(true);
    try {
      await saveProviderModels(provider, currentModels);
      clearBatchDeleteState(provider.providerKey);
      setFetchModelsModalOpen(false);
      message.success(t('ohMyPi.fetchModels.applySuccess', {
        addCount: selectedModels.length,
        removeCount: removedModelIds.length,
      }));
    } catch (error) {
      console.error('Failed to apply fetched Pi models:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const saveImportedOmpProviders = async (
    providers: Array<{
      providerKey: string;
      modelsProvider: Record<string, unknown>;
      credential?: Record<string, unknown>;
      displayName?: string;
    }>,
  ) => {
    const existingProviderIdSet = new Set(existingProviderIds);
    let nextConfig: OmpRuntimeConfig | null = null;
    let importedCount = 0;

    setSaving(true);
    try {
      for (const provider of providers) {
        if (!provider.providerKey || existingProviderIdSet.has(provider.providerKey)) {
          continue;
        }

        // OMP 没有 auth.json;凭据直接作为 provider 配置的一部分写入 models.yml。
        nextConfig = await saveOmpModelsProvider({
          providerKey: provider.providerKey,
          provider: provider.modelsProvider,
        });
        existingProviderIdSet.add(provider.providerKey);
        importedCount += 1;

        try {
          await upsertOmpFavoriteProvider(
            provider.providerKey,
            provider.modelsProvider,
            provider.credential,
            provider.displayName,
          );
        } catch (error) {
          console.error('Failed to save imported Pi favorite provider:', error);
        }
      }

      if (nextConfig) {
        setRuntimeConfig(nextConfig);
        setOtherSettings(nextConfig.otherSettings || {});
      }
      if (importedCount > 0) {
        await refreshTrayMenu();
      }
      message.success(t('ohMyPi.provider.importSuccess', { count: importedCount }));
      return importedCount;
    } catch (error) {
      console.error('Failed to import Pi providers:', error);
      message.error(t('common.error'));
      return 0;
    } finally {
      setSaving(false);
    }
  };

  const handleImportProviders = async (providers: OpenCodeFavoriteProvider[]) => {
    const importedCount = await saveImportedOmpProviders(
      providers.map((provider) => {
        const payload = resolveOmpFavoriteProviderPayload(provider);
        return {
          providerKey: payload.providerKey,
          modelsProvider: payload.modelsProvider,
          credential: payload.credential,
          displayName: provider.providerConfig.name,
        };
      }),
    );
    if (importedCount > 0) {
      setImportModalOpen(false);
    }
  };

  const handleImportAllApiHubProviders = async (providers: OpenCodeAllApiHubProvider[]) => {
    const importedCount = await saveImportedOmpProviders(
      providers.map((provider) => ({
        providerKey: provider.providerId,
        modelsProvider: buildOmpModelsProviderFromOpenCodeProvider(provider.providerConfig),
        displayName: provider.name,
      })),
    );
    if (importedCount > 0) {
      setAllApiHubImportModalOpen(false);
    }
  };

  const handleImportFromCcSwitch = async (imported: CcSwitchProviderCandidate[]) => {
    const importedCount = await saveImportedOmpProviders(
      imported
        .map((candidate) => extractOmpProviderFromCcSwitch(candidate))
        .filter((entry): entry is NonNullable<typeof entry> => entry !== null),
    );
    if (importedCount > 0) {
      setCcSwitchImportModalOpen(false);
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

    const provider = ompProviders.find((item) => item.providerKey === connectivityProviderId);
    if (!provider) {
      return;
    }

    const selectedModelIdSet = new Set(modelIdsToRemove);
    const nextModels = getProviderModelRecords(provider.modelsProvider)
      .filter((entry) => !selectedModelIdSet.has(entry.id))
      .map((entry) => entry.model);

    setSaving(true);
    try {
      const nextConfig = await saveProviderModels(provider, nextModels);
      if (
        provider.isDefault
        && nextConfig.modelSettings.modelId
        && selectedModelIdSet.has(nextConfig.modelSettings.modelId)
      ) {
        const updatedConfig = await saveOmpModelSettings({
          defaultProvider: nextConfig.modelSettings.providerKey ?? provider.providerKey,
          defaultModel: '',
          defaultThinkingLevel: '',
        });
        setRuntimeConfig(updatedConfig);
        setOtherSettings(updatedConfig.otherSettings || {});
        modelForm.setFieldValue('defaultModel', undefined);
      }
      clearBatchDeleteState(provider.providerKey);
    } catch (error) {
      console.error('Failed to remove Pi models from connectivity test:', error);
      throw error;
    } finally {
      setSaving(false);
    }
  }, [clearBatchDeleteState, connectivityProviderId, modelForm, ompProviders]);

  const handleBatchTestProviders = React.useCallback(async () => {
    const targets = ompProviders.map((provider) => {
      const providerConfig = buildOmpOpenCodeProvider(provider);
      const modelIds = getProviderModelRecords(provider.modelsProvider).map((entry) => entry.id);
      return buildProviderConnectivityBatchTarget(
        {
          providerId: provider.providerKey,
          providerName: provider.displayName,
          providerConfig,
          modelIds,
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
      );
    });

    setConnectivityStatuses(
      Object.fromEntries(ompProviders.map((provider) => [
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
      console.error('Failed to batch test Pi providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [ompProviders, t]);

  const handleDeleteOmpModel = async (provider: OmpRuntimeProviderView, modelId: string) => {
    setSaving(true);
    try {
      const nextModels = getProviderModelRecords(provider.modelsProvider)
        .filter((entry) => entry.id !== modelId)
        .map((entry) => entry.model);
      const nextConfig = await saveProviderModels(provider, nextModels);
      if (provider.isDefault && nextConfig.modelSettings.modelId === modelId) {
        const updatedConfig = await saveOmpModelSettings({
          defaultProvider: nextConfig.modelSettings.providerKey ?? provider.providerKey,
          defaultModel: '',
          defaultThinkingLevel: '',
        });
        setRuntimeConfig(updatedConfig);
        setOtherSettings(updatedConfig.otherSettings || {});
        modelForm.setFieldValue('defaultModel', undefined);
        await refreshTrayMenu();
      }
      clearBatchDeleteState(provider.providerKey);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete Pi model:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteProvider = (provider: OmpRuntimeProviderView) => {
    Modal.confirm({
      title: t('ohMyPi.provider.deleteConfirmTitle'),
      content: t('ohMyPi.provider.deleteConfirmContent', {
        providerKey: provider.providerKey,
        scope: t('ohMyPi.provider.deleteScope.provider_config'),
      }),
      okButtonProps: { danger: true },
      onOk: async () => {
        setSaving(true);
        try {
          if (provider.modelsProvider && !isRecordEmpty(provider.modelsProvider)) {
            try {
              await upsertOmpFavoriteProvider(
                provider.providerKey,
                provider.modelsProvider,
                undefined,
                provider.displayName,
              );
            } catch (error) {
              console.error('Failed to preserve OMP favorite provider before deletion:', error);
            }
          }
          const nextConfig = await deleteOmpRuntimeProvider(provider.providerKey);
          setRuntimeConfig(nextConfig);
          setOtherSettings(nextConfig.otherSettings || {});
          await refreshTrayMenu();
          message.success(t('common.success'));
        } catch (error) {
          console.error('Failed to delete OMP provider:', error);
          message.error(t('common.error'));
        } finally {
          setSaving(false);
        }
      },
    });
  };

  const handleDeleteSupplier = (provider: OmpRuntimeProviderView) => {
    handleDeleteProvider(provider);
  };

  const handleOtherSettingsBlur = async (value: unknown, isValid: boolean) => {
    if (!isValid || !otherSettingsValid) {
      message.error(t('ohMyPi.invalidJson'));
      return;
    }
    const nextOtherSettings = value && typeof value === 'object' && !Array.isArray(value)
      ? value as Record<string, unknown>
      : {};
    setSaving(true);
    try {
      const nextConfig = await saveOmpOtherSettings(nextOtherSettings);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save Pi other settings:', error);
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
    setExtensionsRefreshKey((currentRefreshKey) => currentRefreshKey + 1);
    void refreshTrayMenu();
  };

  const handleRefreshModelsCache = async () => {
    setRefreshingModels(true);
    try {
      await fetchRemotePresetModels();
      message.success(t('ohMyPi.modelsRefreshSuccess'));
    } catch (error) {
      console.error('Failed to refresh Pi preset models:', error);
      message.error(t('common.error'));
    } finally {
      setRefreshingModels(false);
    }
  };

  const renderProvider = (provider: OmpRuntimeProviderView) => {
    // OMP 没有 auth.json,凭据(apiKey)直接写在 models.yml 的 provider 配置里。
    const providerConfig = provider.modelsProvider ?? {};
    const hasCredential = Object.prototype.hasOwnProperty.call(providerConfig, 'apiKey')
      && !isRecordEmpty({ apiKey: providerConfig.apiKey });
    const hasProviderConfig = provider.sources.includes('models_yml');
    const canDeleteProvider = hasCredential || hasProviderConfig;
    const deleteDisabledReason = canDeleteProvider && provider.isDefault
      ? t('ohMyPi.provider.deleteDisabledDefault', { defaultValue: '该渠道已设为默认，不可删除' })
      : undefined;
    const isBatchDeleteMode = batchDeleteProviderId === provider.providerKey;
    const selectedModelIds = selectedModelIdsByProvider[provider.providerKey] ?? [];
    const selectedModelCount = selectedModelIds.length;
    const providerBaseUrl = getStringField(providerConfig, 'baseUrl');
    const hasModelIds = getProviderModelRecords(provider.modelsProvider).length > 0;
    const connectivityTooltip = !providerBaseUrl
      ? t('common.baseUrlMissing')
      : !hasModelIds
        ? t('common.modelMissing')
        : '';
    const fetchModelsTooltip = !providerBaseUrl ? t('common.baseUrlMissing') : '';
    const providerDisplay: ProviderDisplayData = {
      id: provider.providerKey,
      name: provider.displayName,
      sdkName: getStringField(providerConfig, 'api') || provider.categories.join(', ') || 'omp',
      baseUrl: providerBaseUrl
        || provider.sources.map((source) => translateRuntimeLabel('ohMyPi.sourceLabels', source)).join(' / ')
        || t('ohMyPi.provider.builtinHint'),
    };
    const modelDisplayList: ModelDisplayData[] = getProviderModelRecords(provider.modelsProvider).map((entry) => ({
      id: entry.id,
      name: getStringField(entry.model, 'name') || entry.id,
      isPrimary: provider.isDefault && runtimeConfig?.modelSettings.modelId === entry.id,
    }));

    return (
      <ProviderCard
        key={provider.providerKey}
        provider={providerDisplay}
        models={modelDisplayList}
        onEdit={() => openProviderModal(provider)}
        onCopy={() => openProviderModal(provider, { copy: true })}
        onShare={() => shareProvider({
          id: provider.providerKey, name: provider.displayName, category: 'custom',
          settingsConfig: JSON.stringify(providerConfig), credential: provider.credential,
          credentialUnavailable: provider.credentialKind === 'oauth' || provider.credentialKind === 'env_possible',
          defaultModel: provider.isDefault ? runtimeConfig?.modelSettings.modelId ?? undefined : undefined,
        })}
        onDelete={canDeleteProvider ? () => handleDeleteSupplier(provider) : undefined}
        deleteConfirm={false}
        deleteDisabledReason={deleteDisabledReason}
        selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.providerKey)}
        selected={providerBatch.selectedIds.has(provider.providerKey)}
        onSelectChange={(checked) => providerBatch.toggleSelect(provider.providerKey, checked)}
        connectivityStatus={connectivityStatuses[provider.providerKey]}
        extraActions={
          <Space size={0}>
            <Button
              size="small"
              type="text"
              icon={<DeleteOutlined />}
              style={{ fontSize: 12 }}
              onClick={() => handleToggleBatchDeleteMode(provider.providerKey)}
            >
              {isBatchDeleteMode
                ? t('ohMyPi.model.cancelBatchDelete')
                : t('ohMyPi.model.batchDelete')}
            </Button>
            {isBatchDeleteMode && (
              <Button
                size="small"
                type="text"
                danger
                style={{ fontSize: 12 }}
                disabled={selectedModelCount === 0}
                onClick={() => {
                  Modal.confirm({
                    title: t('ohMyPi.model.batchDeleteConfirmTitle'),
                    content: t('ohMyPi.model.batchDeleteConfirmContent', { count: selectedModelCount }),
                    okText: t('common.confirm'),
                    cancelText: t('common.cancel'),
                    onOk: async () => {
                      await handleBatchDeleteModels(provider);
                    },
                  });
                }}
              >
                {t('ohMyPi.model.deleteSelected', { count: selectedModelCount })}
              </Button>
            )}
            <Tooltip title={connectivityTooltip}>
              <span>
                <Button
                  size="small"
                  type="text"
                  style={{ fontSize: 12 }}
                  onClick={() => handleOpenConnectivityTest(provider.providerKey)}
                  disabled={!providerBaseUrl || !hasModelIds}
                >
                  <ApiOutlined style={{ marginRight: 4 }} />
                  {t('ohMyPi.connectivity.button')}
                </Button>
              </span>
            </Tooltip>
            <Tooltip title={fetchModelsTooltip}>
              <span>
                <Button
                  size="small"
                  type="text"
                  style={{ fontSize: 12 }}
                  onClick={() => handleOpenFetchModels(provider.providerKey)}
                  disabled={!providerBaseUrl}
                >
                  <CloudDownloadOutlined style={{ marginRight: 4 }} />
                  {t('ohMyPi.fetchModels.button')}
                </Button>
              </span>
            </Tooltip>
          </Space>
        }
        onAddModel={() => openOmpModelModal(provider)}
        onEditModel={(modelId) => openOmpModelModal(provider, modelId)}
        onCopyModel={(modelId) => openOmpModelModal(provider, modelId, { copy: true })}
        onDeleteModel={(modelId) => handleDeleteOmpModel(provider, modelId)}
        onSetPrimaryModel={(modelId) => handleSetPrimaryModel(provider, modelId)}
        modelSelectionMode={isBatchDeleteMode}
        selectedModelIds={selectedModelIds}
        onToggleModelSelection={(modelId, selected) => handleToggleModelSelection(provider.providerKey, modelId, selected)}
        modelsDraggable={!isBatchDeleteMode}
        onReorderModels={(modelIds) => handleReorderModels(provider, modelIds)}
        i18nPrefix="ohMyPi"
      />
    );
  };

  return (
    <Spin spinning={loading}>
      <SectionSidebarLayout
        sidebarTitle={t('ohMyPi.title')}
        sidebarHidden={sidebarHidden}
        sections={sidebarSections}
        markerAttr="data-pi-sidebar-section"
        getIcon={(id) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
      >
        <div className={styles.pageContent}>
          <div className={styles.pageHeader}>
            <div>
              <div className={styles.titleRow}>
                <Title level={4} className={styles.pageTitle}>
                  {t('ohMyPi.title')}
                </Title>
                <Link
                  type="secondary"
                  className={styles.headerLink}
                  onClick={(event) => {
                    event.stopPropagation();
                    void openUrl('https://omp.sh/docs');
                  }}
                >
                  <LinkOutlined /> {t('ohMyPi.viewDocs')}
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
                  {t('ohMyPi.configPath')}:
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
                  {t('ohMyPi.rootPathSource.customize')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenRootFolder}
                  className={styles.textAction}
                >
                  {t('ohMyPi.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<ReloadOutlined />}
                  onClick={handleRefreshConfig}
                  className={styles.textAction}
                >
                  {t('ohMyPi.refreshConfig')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<CloudSyncOutlined />}
                  onClick={handleRefreshModelsCache}
                  loading={refreshingModels}
                  className={styles.textAction}
                >
                  {t('ohMyPi.syncModels')}
                </Button>
              </Space>
            </div>
            <Button type="text" icon={<EllipsisOutlined />} onClick={() => setSettingsModalOpen(true)}>
              {t('common.moreOptions')}
            </Button>
          </div>
          <div className={styles.pageHint}>
            {t('ohMyPi.pageHint')}
          </div>

          <div
            id="pi-model-settings"
            className={styles.ompSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('ohMyPi.modelSettings.title')}
          >
            <div className={styles.modelCard}>
              <Title level={5} className={styles.modelCardTitle}>
                <RobotOutlined style={{ marginRight: 8 }} />
                {t('ohMyPi.modelSettings.title')}
              </Title>
              <div className={styles.modelCardContent}>
                <Form
                  form={modelForm}
                  layout="vertical"
                  onValuesChange={handleModelSettingsChange}
                >
                  <div className={styles.modelSettingsGrid}>
                    <Form.Item label={t('ohMyPi.modelSettings.defaultProvider')} name="defaultProvider">
                      <Select
                        allowClear
                        showSearch
                        options={providerOptions}
                        placeholder={t('ohMyPi.modelSettings.defaultProviderPlaceholder')}
                      />
                    </Form.Item>
                    <Form.Item label={t('ohMyPi.modelSettings.defaultModel')} name="defaultModel">
                      <Select
                        allowClear
                        showSearch
                        options={modelOptions}
                        placeholder={t('ohMyPi.modelSettings.defaultModelPlaceholder')}
                      />
                    </Form.Item>
                    {thinkingLevelOptions.length > 0 ? (
                      <Form.Item label={t('ohMyPi.modelSettings.thinkingLevel')} name="defaultThinkingLevel">
                        <Select
                          allowClear
                          options={thinkingLevelOptions}
                          placeholder={t('ohMyPi.modelSettings.thinkingLevelPlaceholder')}
                        />
                      </Form.Item>
                    ) : null}
                  </div>
                </Form>
              </div>
            </div>
          </div>

          <div
            id="pi-providers"
            className={styles.ompSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('ohMyPi.provider.title')}
          >
            <Collapse
              className={styles.collapseCard}
              items={[
                {
                  key: 'providers',
                  label: (
                    <Space>
                      <ApiOutlined />
                      <Text strong>{t('ohMyPi.provider.title')}</Text>
                    </Space>
                  ),
                  extra: (
                    <Space onClick={(event) => event.stopPropagation()}>
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
                        onClick={() => openProviderModal()}
                      >
                        {t('ohMyPi.provider.addSupplier')}
                      </Button>
                    </Space>
                  ),
                  children: (
                    <div>
                      {runtimeConfig?.providers.length ? (
                        <div className={styles.providerList}>
                          {visibleProviders.length ? (
                            visibleProviders.map(renderProvider)
                          ) : (
                            <ProviderSearchEmpty />
                          )}
                        </div>
                      ) : (
                        <Empty description={t('ohMyPi.provider.emptyText')} />
                      )}
                      <div style={{ marginTop: 12 }}>
                        <Space wrap>
                          <Button
                            type="dashed"
                            icon={<ImportOutlined />}
                            onClick={() => setImportModalOpen(true)}
                          >
                            {t('ohMyPi.provider.importFavorite')}
                          </Button>
                          {allApiHubAvailable && (
                            <Button
                              type="dashed"
                              icon={<AllApiHubIcon />}
                              onClick={() => setAllApiHubImportModalOpen(true)}
                            >
                              {t('ohMyPi.provider.importAllApiHub')}
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
            id="pi-extensions"
            className={styles.ompSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('ohMyPi.extensions.title')}
          >
            <OmpExtensionsSection refreshKey={extensionsRefreshKey} />
          </div>

          <div
            id="pi-global-prompt"
            className={`${styles.ompSection} ${styles.promptSection}`}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('ohMyPi.prompt.title')}
          >
            <GlobalPromptSettings
              translationKeyPrefix="ohMyPi.prompt"
              service={ohMyPiPromptApi}
              collapseKey="pi-prompt"
              onUpdated={async () => {
                await loadConfig(true);
                await refreshTrayMenu();
              }}
            />
          </div>

          <div
            id="pi-other-configuration"
            className={styles.ompSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('ohMyPi.otherConfig.title')}
          >
            <Collapse
              className={styles.collapseCard}
              items={[
                {
                  key: 'other',
                  label: (
                    <Space>
                      <SettingOutlined />
                      <Text strong>{t('ohMyPi.otherConfig.title')}</Text>
                    </Space>
                  ),
                  children: (
                    <Form.Item
                      help={
                        <span>
                          <Text type="secondary">{t('ohMyPi.otherConfig.hint')}，</Text>
                          <span style={{ color: 'var(--ant-color-primary)' }}>
                            {t('ohMyPi.otherConfig.autoSaveHint')}
                          </span>
                        </span>
                      }
                      style={{ marginBottom: 0 }}
                    >
                      <JsonEditor
                        value={otherSettings}
                        height={260}
                        onChange={(value, isValid) => {
                          setOtherSettings((value && typeof value === 'object' && !Array.isArray(value))
                            ? value as Record<string, unknown>
                            : {});
                          setOtherSettingsValid(isValid);
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
            id="pi-session-manager"
            className={styles.ompSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('sessionManager.title')}
          >
            <SessionManagerPanel tool="oh_my_pi" />
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
            ? t('ohMyPi.provider.editSupplierTitle', { name: providerModal.provider.displayName })
            : t('ohMyPi.provider.addSupplierTitle')}
          open={!!providerModal}
          width={860}
          confirmLoading={saving}
          onCancel={() => setProviderModal(null)}
          onOk={handleSaveProviderModal}
          destroyOnHidden
        >
          <Form form={providerModalForm} layout="vertical" className={styles.providerForm}>
            <div className={styles.modalSection}>
              <div className={styles.modalGrid}>
                <Form.Item
                  label={t('ohMyPi.provider.providerKey')}
                  name="providerKey"
                  rules={[{ required: true, message: t('ohMyPi.provider.providerKeyRequired') }]}
                >
                  <Input
                    disabled={!!providerModal?.provider}
                    placeholder={t('ohMyPi.provider.providerKeyPlaceholder')}
                  />
                </Form.Item>
                <Form.Item label={t('ohMyPi.provider.displayName')} name="displayName">
                  <Input placeholder={t('ohMyPi.provider.displayNamePlaceholder')} />
                </Form.Item>
              </div>
            </div>

            <div className={styles.modalSection}>
              <Text strong>{t('ohMyPi.provider.configSection')}</Text>
              <div className={styles.modalGrid}>
                <Form.Item label={t('ohMyPi.provider.apiType')} name="api">
                  <Select
                    allowClear
                    showSearch
                    options={PI_API_OPTIONS}
                    placeholder={t('ohMyPi.provider.apiTypePlaceholder')}
                  />
                </Form.Item>
                <Form.Item label={t('ohMyPi.provider.baseUrl')} name="baseUrl">
                  <Input placeholder="https://api.example.com/v1" />
                </Form.Item>
                <Form.Item label={t('ohMyPi.provider.providerApiKey')} name="providerApiKey">
                  <Input.Password autoComplete="off" />
                </Form.Item>
                <Form.Item
                  label={(
                    <Space size={4}>
                      <span>{t('ohMyPi.provider.authHeader')}</span>
                      <Tooltip title={t('ohMyPi.provider.authHeaderHint')}>
                        <QuestionCircleOutlined style={{ color: 'var(--color-text-tertiary)' }} />
                      </Tooltip>
                    </Space>
                  )}
                  name="authHeader"
                  valuePropName="checked"
                >
                  <Switch />
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
                <span>{t('common.advancedSettings')}</span>
              </Button>
            </div>
            {providerAdvancedExpanded && (
              <div className={styles.modalSection}>
                <div className={styles.advancedEditor}>
                  <Text type="secondary">{t('ohMyPi.provider.headersJson')}</Text>
                  <JsonEditor
                    value={isRecordEmpty(providerHeadersJson) ? undefined : providerHeadersJson}
                    height={160}
                    onChange={(value, isValid) => {
                      if (isValid) {
                        setProviderHeadersJson(asRecord(value));
                      }
                      setProviderHeadersJsonValid(isValid);
                    }}
                  />
                </div>
                <div className={styles.advancedEditor}>
                  <Text type="secondary">{t('ohMyPi.provider.compatJson')}</Text>
                  <JsonEditor
                    value={isRecordEmpty(providerCompatJson) ? undefined : providerCompatJson}
                    height={180}
                    onChange={(value, isValid) => {
                      if (isValid) {
                        setProviderCompatJson(asRecord(value));
                      }
                      setProviderCompatJsonValid(isValid);
                    }}
                  />
                </div>
                <div className={styles.advancedEditor}>
                  <Text type="secondary">{t('ohMyPi.provider.modelOverridesJson')}</Text>
                  <JsonEditor
                    value={isRecordEmpty(providerModelOverridesJson) ? undefined : providerModelOverridesJson}
                    height={200}
                    onChange={(value, isValid) => {
                      if (isValid) {
                        setProviderModelOverridesJson(asRecord(value));
                      }
                      setProviderModelOverridesJsonValid(isValid);
                    }}
                  />
                </div>
              </div>
            )}
          </Form>
        </Modal>

        <ModelFormModal
          open={!!ompModelModal}
          width={700}
          isEdit={!!ompModelModal?.modelId}
          initialValues={ompModelModal ? {
            id: ompModelModal.modelId ?? getStringField(ompModelModal.model ?? {}, 'id'),
            name: getStringField(ompModelModal.model ?? {}, 'name'),
            api: getStringField(ompModelModal.model ?? {}, 'api'),
            reasoning: typeof ompModelModal.model?.reasoning === 'boolean'
              ? ompModelModal.model.reasoning
              : undefined,
            inputTypes: stringifyStringArrayField(ompModelModal.model?.input),
            thinking: typeof ompModelModal.model?.thinking === 'object'
              && ompModelModal.model.thinking !== null
              ? JSON.stringify(ompModelModal.model.thinking)
              : undefined,
            compat: stringifyRecordField(ompModelModal.model?.compat),
            contextLimit: typeof ompModelModal.model?.contextWindow === 'number'
              ? ompModelModal.model.contextWindow
              : undefined,
            outputLimit: typeof ompModelModal.model?.maxTokens === 'number'
              ? ompModelModal.model.maxTokens
              : undefined,
            costInput: getNumberField(asRecord(ompModelModal.model?.cost), 'input'),
            costOutput: getNumberField(asRecord(ompModelModal.model?.cost), 'output'),
            costCacheRead: getNumberField(asRecord(ompModelModal.model?.cost), 'cacheRead'),
            costCacheWrite: getNumberField(asRecord(ompModelModal.model?.cost), 'cacheWrite'),
          } : undefined}
          existingIds={ompModelModal && !ompModelModal.modelId
            ? getProviderModelRecords(ompModelModal.provider.modelsProvider).map((entry) => entry.id)
            : []}
          showOptions={false}
          showVariants={false}
          showModalities={false}
          showInputTypes
          showApi
          apiOptions={PI_API_OPTIONS}
          showReasoning
          showOmpThinking
          showCompat
          showCost
          limitRequired={false}
          nameRequired={false}
          npmType={ompModelModal
            ? ompApiToSdkName(getStringField(ompModelModal.provider.modelsProvider ?? {}, 'api'))
            : undefined}
          onCancel={() => setOmpModelModal(null)}
          onSuccess={handleSaveOmpModel}
          onDuplicateId={() => message.error(t('ohMyPi.model.idExists'))}
          i18nPrefix="ohMyPi"
        />

        {fetchModelsProviderInfo && (
          <FetchModelsModal
            open={fetchModelsModalOpen}
            providerId={fetchModelsProviderInfo.providerId}
            providerName={fetchModelsProviderInfo.name}
            baseUrl={fetchModelsProviderInfo.baseUrl}
            apiKey={fetchModelsProviderInfo.apiKey}
            headers={fetchModelsProviderInfo.headers}
            sdkType={fetchModelsProviderInfo.sdkName}
            existingModelIds={fetchModelsProviderInfo.existingModelIds}
            onCancel={() => setFetchModelsModalOpen(false)}
            onSuccess={handleFetchModelsSuccess}
          />
        )}

        <ImportProviderModal
          open={importModalOpen}
          onClose={() => setImportModalOpen(false)}
          onImport={handleImportProviders}
          existingProviderIds={existingFavoriteProviderIds}
          title={t('ohMyPi.provider.importModalTitle')}
          emptyDescription={t('ohMyPi.provider.noFavoriteProviders')}
          i18nPrefix="ohMyPi"
          providerFilter={(provider) => isFavoriteProviderForSource('omp', provider)}
          providerListTransform={transformOmpFavoriteProviders}
        />

        {allApiHubAvailable && (
          <ImportFromAllApiHubModal
            open={allApiHubImportModalOpen}
            onClose={() => setAllApiHubImportModalOpen(false)}
            onImport={handleImportAllApiHubProviders}
            existingProviderIds={existingProviderIds}
          />
        )}

        {ccSwitchAvailable && (
          <ImportFromCcSwitchModal
            open={ccSwitchImportModalOpen}
            appType="claude"
            existingProviderIds={existingProviderIds}
            onClose={() => setCcSwitchImportModalOpen(false)}
            onImport={handleImportFromCcSwitch}
          />
        )}

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
          removableModelIds={connectivityInfo?.modelIds}
          onRemoveModels={handleRemoveConnectivityModels}
          onCancel={() => setConnectivityModalOpen(false)}
        />

        <FileConfigPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('ohMyPi.preview.title')}
          files={[
            {
              key: 'config',
              label: runtimeConfig?.configPath?.split(/[\\/]/).pop() || 'config.yml',
              content: runtimeConfig?.configContent ?? runtimeConfig?.settings,
              language: 'yaml',
            },
            {
              key: 'models',
              label: runtimeConfig?.modelsPath?.split(/[\\/]/).pop() || 'models.yml',
              content: runtimeConfig?.modelsContent ?? runtimeConfig?.models,
              language: 'yaml',
            },
            {
              key: 'mcp',
              label: runtimeConfig?.mcpPath?.split(/[\\/]/).pop() || 'mcp.json',
              content: runtimeConfig?.mcpContent,
              language: 'json',
            },
            {
              key: 'prompt',
              label: runtimeConfig?.promptPath?.split(/[\\/]/).pop() || 'AGENTS.md',
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
            await setSidebarHidden('oh_my_pi', !visible);
          }}
        >
          <CliManualPathSetting commandName="omp" labelKey="subModules.ohMyPi" toolNameKey="subModules.ohMyPiFull" />
        </SidebarSettingsModal>

        {shareModal}
      </SectionSidebarLayout>
    </Spin>
  );
};

export default OhMyPiPage;
