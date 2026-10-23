import React from 'react';
import {
  Button,
  Collapse,
  Form,
  Modal,
  Select,
  Space,
  Spin,
  Typography,
  message,
} from 'antd';
import {
  AppstoreAddOutlined,
  CloudSyncOutlined,
  DatabaseOutlined,
  FileTextOutlined,
  MessageOutlined,
  RobotOutlined,
  SettingOutlined,
  ToolOutlined,
  ImportOutlined,
} from '@ant-design/icons';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';

import { useProviderSharing } from '@/features/coding/shared/providerShare';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import JsonEditor from '@/components/common/JsonEditor';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import type {
  ProviderConnectivityStatusItem,
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
  buildFetchedPiModel,
  piApiToSdkName,
} from '../utils/piFetchedModels';
import { PI_API_OPTIONS } from '../utils/piApiOptions';
import {
  createDefaultProviderConfig,
  hasProviderConfigContent,
  isRecordEmpty,
  resolvePiProviderApiKey,
  setOptionalStringField,
} from '../utils/providerConfig';
import {
  asRecord,
  getNumberField,
  getProviderModelRecords,
  getStringField,
} from '../utils/providerModels';
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
  filterProviderItems,
  sortProviderItems,
  providerSelectionDisabledReason,
  useProviderBatchSelection,
  useProviderListSort,
  useProviderListViewPrefs,
  type ProviderSelectionBlockReason,
} from '@/features/coding/shared/providerList';
import ProviderListSection from '@/features/coding/shared/ProviderListSection';
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import { ProviderCardGrid } from '@/features/coding/shared/providerCardView';
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  arrayMove,
  rectSortingStrategy,
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis, restrictToWindowEdges } from '@dnd-kit/modifiers';
import PiProviderCard from '../components/PiProviderCard';
import PiProviderFormModal, {
  type PiProviderDraft,
} from '../components/PiProviderFormModal';
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
  PI_THINKING_LEVEL_OPTIONS,
  buildPiThinkingLevelMapFromPreset,
  isPiThinkingLevelMapEntrySupported,
} from '@/utils/piModelMetadata';
import {
  deletePiRuntimeProvider,
  getPiSettingsConfig,
  readPiRuntimeConfig,
  reorderPiProviders,
  savePiAuthProvider,
  savePiModelSettings,
  savePiModelsProvider,
  savePiOtherSettings,
  savePiSettingsConfig,
} from '@/services/piApi';
import { piPromptApi } from '@/services/piPromptApi';
import type {
  PiDeleteScope,
  PiRuntimeConfig,
  PiRuntimeProviderView,
} from '@/types/pi';
import type { OpenCodeModel, OpenCodeProvider } from '@/types/opencode';

import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import { extractPiProviderFromCcSwitch } from '../utils/importMapping';
import PiExtensionsSection from '../components/PiExtensionsSection';
import styles from './PiPage.module.less';

const { Title, Text } = Typography;

interface ProviderJsonModalState {
  provider?: PiRuntimeProviderView;
  /** Pre-fill from `provider`, save under `<key>_copy`. */
  copy?: boolean;
}

interface PiModelModalState {
  provider: PiRuntimeProviderView;
  modelId?: string;
  model?: Record<string, unknown>;
}

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'pi-model-settings': <RobotOutlined />,
  'pi-providers': <DatabaseOutlined />,
  'pi-extensions': <AppstoreAddOutlined />,
  'pi-global-prompt': <FileTextOutlined />,
  'pi-other-configuration': <ToolOutlined />,
  'pi-session-manager': <MessageOutlined />,
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

const buildPiThinkingLevelOptionLabel = (levelKey: string, mappedValue: unknown): string => {
  if (typeof mappedValue === 'string' && mappedValue.trim() && mappedValue !== levelKey) {
    return `${levelKey} (${mappedValue})`;
  }
  return levelKey;
};

const getPiModelThinkingLevelOptions = (
  model: Record<string, unknown> | undefined,
): Array<{ value: string; label: string }> => {
  if (!model || model.reasoning === false) {
    return [];
  }

  const thinkingLevelMap = asRecord(model.thinkingLevelMap);
  if (!isRecordEmpty(thinkingLevelMap)) {
    return PI_THINKING_LEVEL_KEYS
      .filter((levelKey) => isPiThinkingLevelMapEntrySupported(levelKey, thinkingLevelMap))
      .map((levelKey) => ({
        value: levelKey,
        label: buildPiThinkingLevelOptionLabel(levelKey, thinkingLevelMap[levelKey]),
      }));
  }

  return model.reasoning === true ? PI_THINKING_LEVEL_OPTIONS : [];
};

const isPiThinkingLevelSupported = (
  thinkingLevel: string | undefined,
  model: Record<string, unknown> | undefined,
): boolean => {
  if (!thinkingLevel) {
    return true;
  }
  return getPiModelThinkingLevelOptions(model).some((option) => option.value === thinkingLevel);
};

const asStringRecord = (value: unknown): Record<string, string> => {
  const record = asRecord(value);
  return Object.fromEntries(
    Object.entries(record).filter((entry): entry is [string, string] => typeof entry[1] === 'string'),
  );
};

const sdkNameToPiApi = (sdkName?: string): string => {
  switch (sdkName) {
    case '@ai-sdk/anthropic':
      return 'anthropic-messages';
    case '@ai-sdk/google':
      return 'google-generative-ai';
    default:
      return 'openai-completions';
  }
};

const buildPiModelFromOpenCodeModel = (
  modelId: string,
  model: OpenCodeModel,
): Record<string, unknown> => {
  const inputTypes = (model.modalities?.input ?? []).filter((inputType) => PI_INPUT_TYPES.has(inputType));
  const thinkingLevelMap = buildPiThinkingLevelMapFromPreset(model.variants);

  return {
    id: model.id || modelId,
    name: model.name || modelId,
    ...(typeof model.reasoning === 'boolean' ? { reasoning: model.reasoning } : {}),
    ...(inputTypes.length > 0 ? { input: inputTypes } : {}),
    ...(typeof model.limit?.context === 'number' ? { contextWindow: model.limit.context } : {}),
    ...(typeof model.limit?.output === 'number' ? { maxTokens: model.limit.output } : {}),
    ...(!isRecordEmpty(thinkingLevelMap) ? { thinkingLevelMap } : {}),
  };
};

const buildPiModelsProviderFromOpenCodeProvider = (
  provider: OpenCodeProvider,
): Record<string, unknown> => {
  const options = provider.options ?? {};
  const headers = asStringRecord(options.headers);
  const models = Object.entries(provider.models || {}).map(([modelId, model]) =>
    buildPiModelFromOpenCodeModel(modelId, model),
  );

  return {
    ...(provider.name ? { name: provider.name } : {}),
    api: sdkNameToPiApi(provider.npm),
    ...(options.baseURL ? { baseUrl: options.baseURL } : {}),
    ...(options.apiKey ? { apiKey: options.apiKey } : {}),
    ...(!isRecordEmpty(headers) ? { headers } : {}),
    models,
  };
};

const buildPiOpenCodeProvider = (
  provider: PiRuntimeProviderView,
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
    npm: piApiToSdkName(api),
    name: provider.displayName,
    options: {
      baseURL: getStringField(providerConfig, 'baseUrl'),
      apiKey: getStringField(providerConfig, 'apiKey'),
      ...(isRecordEmpty(headers) ? {} : { headers }),
    },
    models,
  };
};

/**
 * Point an outbound OpenCode-shaped provider at the credential Pi will use.
 *
 * `buildPiOpenCodeProvider` stays the *stored channel* shape (favorites and
 * sharing dedupe on its `apiKey`), while requests must authenticate the way Pi
 * does: `auth.json` first, `models.json` second (issue #435).
 */
const withResolvedPiApiKey = (
  provider: PiRuntimeProviderView,
  openCodeProvider: OpenCodeProvider,
): OpenCodeProvider => {
  const apiKey = resolvePiProviderApiKey(provider);
  if (!apiKey) {
    return openCodeProvider;
  }
  return {
    ...openCodeProvider,
    options: { ...openCodeProvider.options, apiKey },
  };
};

const buildPiFavoriteProviderConfig = (
  providerKey: string,
  displayName: string,
  modelsProvider: Record<string, unknown>,
  credential?: Record<string, unknown>,
): OpenCodeProvider => {
  const favoriteProvider = buildPiOpenCodeProvider({
    providerKey,
    displayName: displayName || getStringField(modelsProvider, 'name') || providerKey,
    sources: ['models_json'],
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

  const payload: PiFavoriteProviderPayload = {
    providerKey,
    ...(credential && !isRecordEmpty(credential) ? { credential } : {}),
    modelsProvider,
  };

  return buildFavoriteProviderOptions(favoriteProvider, payload);
};

const resolvePiFavoriteProviderPayload = (
  favoriteProvider: OpenCodeFavoriteProvider,
): PiFavoriteProviderPayload => {
  const payload = getFavoriteProviderPayload<PiFavoriteProviderPayload>(favoriteProvider);
  if (payload?.providerKey && payload.modelsProvider) {
    return payload;
  }

  return {
    providerKey: extractFavoriteProviderRawId('pi', favoriteProvider.providerId),
    modelsProvider: buildPiModelsProviderFromOpenCodeProvider(favoriteProvider.providerConfig),
  };
};

const normalizePiFavoriteString = (value: unknown): string => (
  typeof value === 'string' ? value.trim().toLowerCase() : ''
);

const normalizePiFavoriteBaseUrl = (value: unknown): string => (
  normalizePiFavoriteString(value).replace(/\/+$/, '')
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

const getPiFavoriteProviderIdentity = (favoriteProvider: OpenCodeFavoriteProvider): string => {
  const payload = resolvePiFavoriteProviderPayload(favoriteProvider);
  const modelsProvider = payload.modelsProvider;
  const providerOptions = favoriteProvider.providerConfig.options ?? {};
  const api = getStringField(modelsProvider, 'api') || sdkNameToPiApi(favoriteProvider.providerConfig.npm);
  const baseUrl = getStringField(modelsProvider, 'baseUrl') || providerOptions.baseURL;
  const apiKey = getStringField(modelsProvider, 'apiKey') || providerOptions.apiKey;
  const headers = isRecordEmpty(asRecord(modelsProvider.headers))
    ? asRecord(providerOptions.headers)
    : asRecord(modelsProvider.headers);
  if (!baseUrl && !apiKey && isRecordEmpty(headers)) {
    return `provider-key:${payload.providerKey}`;
  }

  return JSON.stringify({
    api: normalizePiFavoriteString(api),
    baseUrl: normalizePiFavoriteBaseUrl(baseUrl),
    apiKey: normalizePiFavoriteString(apiKey),
    headers: buildStableObjectSignature(headers),
  });
};

const getPiFavoriteProviderModelCount = (favoriteProvider: OpenCodeFavoriteProvider): number => {
  const payload = resolvePiFavoriteProviderPayload(favoriteProvider);
  return getProviderModelRecords(payload.modelsProvider).length;
};

const dedupePiFavoriteProviders = (
  favoriteProviders: OpenCodeFavoriteProvider[],
  currentStorageKeys: Set<string>,
): OpenCodeFavoriteProvider[] => {
  const providerByIdentity = new Map<string, OpenCodeFavoriteProvider>();

  favoriteProviders.forEach((favoriteProvider) => {
    const identity = getPiFavoriteProviderIdentity(favoriteProvider);
    const existingProvider = providerByIdentity.get(identity);
    if (!existingProvider) {
      providerByIdentity.set(identity, favoriteProvider);
      return;
    }

    const existingIsCurrent = currentStorageKeys.has(existingProvider.providerId);
    const nextIsCurrent = currentStorageKeys.has(favoriteProvider.providerId);
    const existingModelCount = getPiFavoriteProviderModelCount(existingProvider);
    const nextModelCount = getPiFavoriteProviderModelCount(favoriteProvider);
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

const PiPage: React.FC = () => {
  const { t } = useTranslation();
  const { shareProvider, shareModal } = useProviderSharing('pi', () => loadConfig());
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [refreshingModels, setRefreshingModels] = React.useState(false);
  const [extensionsRefreshKey, setExtensionsRefreshKey] = React.useState(0);
  const [runtimeConfig, setRuntimeConfig] = React.useState<PiRuntimeConfig | null>(null);
  const [modelForm] = Form.useForm();
  const [providerModal, setProviderModal] = React.useState<ProviderJsonModalState | null>(null);
  const [piModelModal, setPiModelModal] = React.useState<PiModelModalState | null>(null);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const { viewMode, setViewMode } = useProviderListViewPrefs('pi');
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
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
  const [deleteScopeProvider, setDeleteScopeProvider] = React.useState<PiRuntimeProviderView | null>(null);
  const modelSettingsSaveSeqRef = React.useRef(0);
  const sidebarHidden = sidebarHiddenByPage.pi;

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'pi-model-settings',
      title: t('pi.modelSettings.title'),
      order: 1,
    },
    {
      id: 'pi-providers',
      title: t('pi.provider.title'),
      order: 2,
    },
    {
      id: 'pi-extensions',
      title: t('pi.extensions.title'),
      order: 3,
    },
    {
      id: 'pi-global-prompt',
      title: t('common.prompt.title'),
      order: 4,
    },
    {
      id: 'pi-other-configuration',
      title: t('pi.otherConfig.title'),
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
      const config = await readPiRuntimeConfig();
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
    translationKeyPrefix: 'pi',
    defaultConfig: '{}',
    loadConfig,
    getCommonConfig: getPiSettingsConfig,
    saveCommonConfig: savePiSettingsConfig,
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
    () => getPiModelThinkingLevelOptions(selectedModelRecord),
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

  const piProviders = React.useMemo(
    () => runtimeConfig?.providers ?? [],
    [runtimeConfig?.providers],
  );
  /**
   * The provider `settings.json`'s `defaultProvider` points at, or `''` when
   * nothing is applied. Passed even as `''`: the locate control then explains
   * that nothing is applied instead of silently disappearing.
   */
  const defaultProviderId = piProviders.find((provider) => provider.isDefault)?.providerKey ?? '';
  const existingProviderIds = React.useMemo(
    () => piProviders.map((provider) => provider.providerKey),
    [piProviders],
  );
  const existingFavoriteProviderIds = React.useMemo(
    () => existingProviderIds.map((providerId) => buildFavoriteProviderStorageKey('pi', providerId)),
    [existingProviderIds],
  );
  const transformPiFavoriteProviders = React.useCallback(
    (providers: OpenCodeFavoriteProvider[]) =>
      dedupePiFavoriteProviders(providers, new Set(existingFavoriteProviderIds)),
    [existingFavoriteProviderIds],
  );

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!fetchModelsProviderId) {
      return null;
    }
    const provider = piProviders.find((item) => item.providerKey === fetchModelsProviderId);
    if (!provider) {
      return null;
    }
    const providerConfig = provider.modelsProvider ?? {};
    const api = getStringField(providerConfig, 'api');
    return {
      providerId: provider.providerKey,
      name: provider.displayName,
      baseUrl: getStringField(providerConfig, 'baseUrl'),
      apiKey: resolvePiProviderApiKey(provider),
      headers: asStringRecord(providerConfig.headers),
      sdkName: piApiToSdkName(api),
      // Pi resolves `apiKey` / header values itself; the shared commands must
      // resolve the models.json config value syntax instead of sending templates.
      configValueMode: 'pi' as const,
      existingModelIds: getProviderModelRecords(provider.modelsProvider).map((entry) => entry.id),
    };
  }, [fetchModelsProviderId, piProviders]);

  const connectivityInfo = React.useMemo(() => {
    if (!connectivityProviderId) {
      return null;
    }
    const provider = piProviders.find((item) => item.providerKey === connectivityProviderId);
    if (!provider) {
      return null;
    }
    const providerConfig = provider.modelsProvider ?? {};
    const modelIds = getProviderModelRecords(provider.modelsProvider).map((entry) => entry.id);
    return {
      providerId: provider.providerKey,
      providerName: provider.displayName,
      providerConfig: withResolvedPiApiKey(provider, buildPiOpenCodeProvider(provider, providerConfig)),
      modelIds,
      configValueMode: 'pi' as const,
    };
  }, [connectivityProviderId, piProviders]);

  const upsertPiFavoriteProvider = React.useCallback(async (
    providerKey: string,
    modelsProvider: Record<string, unknown>,
    credential?: unknown,
    displayName?: string,
  ) => {
    const credentialRecord = asRecord(credential);
    const favoriteConfig = buildPiFavoriteProviderConfig(
      providerKey,
      displayName || getStringField(modelsProvider, 'name') || providerKey,
      modelsProvider,
      isRecordEmpty(credentialRecord) ? undefined : credentialRecord,
    );
    await upsertFavoriteProvider(
      buildFavoriteProviderStorageKey('pi', providerKey),
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
    if (
      nextValues.defaultThinkingLevel
      && !isPiThinkingLevelSupported(nextValues.defaultThinkingLevel, nextModel)
    ) {
      nextValues.defaultThinkingLevel = undefined;
      modelForm.setFieldValue('defaultThinkingLevel', undefined);
    }

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
      const nextConfig = await savePiModelSettings({
        defaultProvider: nextDefaultProvider,
        defaultModel: nextDefaultModel,
        defaultThinkingLevel: nextDefaultThinkingLevel,
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
    provider?: PiRuntimeProviderView,
    options?: { copy?: boolean },
  ) => {
    setProviderModal({ provider, copy: options?.copy === true });
  };

  /**
   * Save what the modal handed back.
   *
   * The credential (`auth.json`) and the channel (`models.json`) are separate
   * files behind separate commands: an empty credential editor means "do not
   * touch auth.json", while an emptied channel config still writes `models.json`
   * for a provider that already lives there.
   */
  const handleSaveProviderModal = async (draft: PiProviderDraft) => {
    if (!providerModal) {
      return;
    }
    const providerKey = draft.values.providerKey?.trim();
    if (!providerKey) {
      message.error(t('pi.provider.providerKeyRequired'));
      return;
    }

    setSaving(true);
    try {
      const shouldSaveCredential = Object.keys(draft.credential).length > 0;
      // Copy, never alias: `asRecord` returns the live `runtimeConfig` object,
      // and this block deletes keys from it. A failed save would then leave the
      // rejected edit in state, and the next successful write of any provider
      // would persist it.
      const nextProviderConfigJson = providerModal.provider?.modelsProvider
        ? { ...asRecord(providerModal.provider.modelsProvider) }
        : createDefaultProviderConfig();
      setOptionalStringField(nextProviderConfigJson, 'name', draft.values.displayName);
      setOptionalStringField(nextProviderConfigJson, 'api', draft.values.api);
      setOptionalStringField(nextProviderConfigJson, 'baseUrl', draft.values.baseUrl);
      setOptionalStringField(nextProviderConfigJson, 'apiKey', draft.values.providerApiKey);
      if (draft.values.authHeader === true) {
        nextProviderConfigJson.authHeader = true;
      } else {
        delete nextProviderConfigJson.authHeader;
      }
      if (isRecordEmpty(draft.headers)) {
        delete nextProviderConfigJson.headers;
      } else {
        nextProviderConfigJson.headers = draft.headers;
      }
      if (isRecordEmpty(draft.compat)) {
        delete nextProviderConfigJson.compat;
      } else {
        nextProviderConfigJson.compat = draft.compat;
      }
      if (isRecordEmpty(draft.modelOverrides)) {
        delete nextProviderConfigJson.modelOverrides;
      } else {
        nextProviderConfigJson.modelOverrides = draft.modelOverrides;
      }
      // An edit of a channel-only provider keeps writing models.json even when
      // every field was blanked (that is how the user deletes the channel);
      // adding a provider writes it only when something was filled in.
      const shouldSaveProviderConfig = !providerModal.provider
        || providerModal.provider.sources.includes('models_json')
        || hasProviderConfigContent(nextProviderConfigJson);
      if (!shouldSaveCredential && !shouldSaveProviderConfig) {
        message.error(t('pi.provider.selectAtLeastOneSection'));
        return;
      }

      let nextConfig: PiRuntimeConfig | null = null;
      if (shouldSaveCredential) {
        nextConfig = await savePiAuthProvider({ providerKey, credential: draft.credential });
      }
      if (shouldSaveProviderConfig) {
        nextConfig = await savePiModelsProvider({ providerKey, provider: nextProviderConfigJson });
      }
      if (!nextConfig) {
        return;
      }
      if (shouldSaveProviderConfig) {
        try {
          await upsertPiFavoriteProvider(
            providerKey,
            nextProviderConfigJson,
            draft.credential,
            draft.values.displayName,
          );
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

  const openPiModelModal = (
    provider: PiRuntimeProviderView,
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

    setPiModelModal({ provider, modelId: isCopy ? undefined : modelId, model: nextModel });
  };

  const handleSavePiModel = async (values: ModelFormValues) => {
    if (!piModelModal) {
      return;
    }
    const modelId = values.id?.trim();
    if (!modelId) {
      message.error(t('pi.model.idRequired'));
      return;
    }

    const currentProvider = runtimeConfig?.providers.find(
      (provider) => provider.providerKey === piModelModal.provider.providerKey,
    ) ?? piModelModal.provider;
    const existingModels = getProviderModelRecords(currentProvider.modelsProvider);
    const duplicateModel = existingModels.some((entry) => (
      entry.id === modelId && entry.id !== piModelModal.modelId
    ));
    if (duplicateModel) {
      message.error(t('pi.model.idExists'));
      return;
    }

    const nextModel = { ...(piModelModal.model ?? {}) };
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
    const thinkingLevelMap = parseJsonRecord(values.thinkingLevelMap);
    if (!isRecordEmpty(thinkingLevelMap)) {
      nextModel.thinkingLevelMap = thinkingLevelMap;
    } else {
      delete nextModel.thinkingLevelMap;
    }
    const compat = parseJsonRecord(values.compat);
    if (!isRecordEmpty(compat)) {
      nextModel.compat = compat;
    } else {
      delete nextModel.compat;
    }
    const nextCost = asRecord(nextModel.cost);
    const costFields: Array<[string, number | undefined]> = [
      ['input', values.costInput],
      ['output', values.costOutput],
      ['cacheRead', values.costCacheRead],
      ['cacheWrite', values.costCacheWrite],
    ];
    costFields.forEach(([key, value]) => {
      if (typeof value === 'number' && Number.isFinite(value)) {
        nextCost[key] = value;
      } else {
        delete nextCost[key];
      }
    });
    if (!isRecordEmpty(nextCost)) {
      nextModel.cost = nextCost;
    } else {
      delete nextModel.cost;
    }

    let modelWasReplaced = false;
    const nextModels = existingModels.map((entry) => {
      if (entry.id === piModelModal.modelId) {
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
      const nextConfig = await savePiModelsProvider({
        providerKey: currentProvider.providerKey,
        provider: nextProviderConfig,
      });
      try {
        await upsertPiFavoriteProvider(
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
      setPiModelModal(null);
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
    provider: PiRuntimeProviderView,
    nextModels: Record<string, unknown>[],
  ) => {
    const nextProviderConfig = {
      ...(provider.modelsProvider ?? {}),
      models: nextModels,
    };
    const nextConfig = await savePiModelsProvider({
      providerKey: provider.providerKey,
      provider: nextProviderConfig,
    });
    try {
      await upsertPiFavoriteProvider(
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

  const handleBatchDeleteModels = async (provider: PiRuntimeProviderView) => {
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
        const updatedConfig = await savePiModelSettings({
          defaultProvider: nextConfig.modelSettings.providerKey ?? provider.providerKey,
          defaultModel: '',
          defaultThinkingLevel: '',
        });
        setRuntimeConfig(updatedConfig);
        setOtherSettings(updatedConfig.otherSettings || {});
        modelForm.setFieldValue('defaultModel', undefined);
      }
      clearBatchDeleteState(provider.providerKey);
      message.success(t('pi.model.batchDeleteSuccess', { count: selectedModelIds.length }));
    } catch (error) {
      console.error('Failed to batch delete Pi models:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleReorderModels = async (provider: PiRuntimeProviderView, modelIds: string[]) => {
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

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('pi');
  const [providerKeyword, setProviderKeyword] = React.useState('');
  const visibleProviders = React.useMemo(
    () =>
      sortProviderItems(
        filterProviderItems(piProviders, providerKeyword, (provider) => [
          provider.providerKey,
          provider.displayName,
          ...(provider.modelIds ?? []),
        ]),
        sortMode,
        { name: (provider) => provider.displayName || provider.providerKey },
        (provider) => lastUsedAt(provider.providerKey),
      ),
    [piProviders, providerKeyword, sortMode, lastUsedAt],
  );

  const canBatchDeleteProvider = React.useCallback(
    (provider: PiRuntimeProviderView) => !provider.isDefault
      && (provider.sources.includes('auth_json') || provider.sources.includes('models_json')),
    [],
  );

  /** Why a row cannot be picked, for the disabled checkbox's tooltip. */
  const batchBlockReason = React.useCallback(
    (provider: PiRuntimeProviderView): ProviderSelectionBlockReason | undefined => {
      if (provider.isDefault) return 'default';
      if (!provider.sources.includes('auth_json') && !provider.sources.includes('models_json')) {
        return 'readOnly';
      }
      return undefined;
    },
    [],
  );

  const handleBatchDeleteProviders = React.useCallback(
    async (ids: string[]): Promise<boolean> => {
      const providersToDelete = piProviders.filter(
        (provider) => ids.includes(provider.providerKey) && canBatchDeleteProvider(provider),
      );
      if (providersToDelete.length === 0) return false;
      setSaving(true);
      try {
        await backupProvidersBeforeDelete(
          providersToDelete,
          (provider) => upsertPiFavoriteProvider(
            provider.providerKey,
            provider.modelsProvider ?? {},
            provider.credential,
            provider.displayName,
          ),
          (provider) => t('common.batch.backupFailed', { name: provider.displayName || provider.providerKey }),
        );
        let nextConfig: PiRuntimeConfig | null = null;
        for (const provider of providersToDelete) {
          const hasCredential = provider.sources.includes('auth_json');
          const hasProviderConfig = provider.sources.includes('models_json');
          const scope: PiDeleteScope = hasCredential && hasProviderConfig
            ? 'both'
            : hasCredential ? 'credential' : 'provider_config';
          nextConfig = await deletePiRuntimeProvider(provider.providerKey, scope);
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
        console.error('Failed to batch delete Pi providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig(true);
        return false;
      } finally {
        setSaving(false);
      }
    },
    [piProviders, canBatchDeleteProvider, upsertPiFavoriteProvider, loadConfig, clearBatchDeleteState, t],
  );

  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter(canBatchDeleteProvider).map((provider) => provider.providerKey),
    [visibleProviders, canBatchDeleteProvider],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });

  const handleSetPrimaryModel = async (provider: PiRuntimeProviderView, modelId: string) => {
    const nextModel = getProviderModelRecords(provider.modelsProvider).find(
      (entry) => entry.id === modelId,
    )?.model;
    const nextThinkingLevel = isPiThinkingLevelSupported(
      runtimeConfig?.modelSettings.thinkingLevel ?? undefined,
      nextModel,
    ) ? runtimeConfig?.modelSettings.thinkingLevel ?? '' : '';
    setSaving(true);
    try {
      const nextConfig = await savePiModelSettings({
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
      message.success(t('pi.model.setAsPrimarySuccess', { name: modelId }));
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
    const provider = piProviders.find((item) => item.providerKey === fetchModelsProviderId);
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
        const matchedPresetModel = findPresetModelById(model.id, piApiToSdkName(providerApi));
        currentModels.push(buildFetchedPiModel(model, matchedPresetModel));
      }
    });

    setSaving(true);
    try {
      await saveProviderModels(provider, currentModels);
      clearBatchDeleteState(provider.providerKey);
      setFetchModelsModalOpen(false);
      message.success(t('pi.fetchModels.applySuccess', {
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

  const saveImportedPiProviders = async (
    providers: Array<{
      providerKey: string;
      modelsProvider: Record<string, unknown>;
      credential?: Record<string, unknown>;
      displayName?: string;
    }>,
  ) => {
    const existingProviderIdSet = new Set(existingProviderIds);
    let nextConfig: PiRuntimeConfig | null = null;
    let importedCount = 0;

    setSaving(true);
    try {
      for (const provider of providers) {
        if (!provider.providerKey || existingProviderIdSet.has(provider.providerKey)) {
          continue;
        }

        if (provider.credential && !isRecordEmpty(provider.credential)) {
          nextConfig = await savePiAuthProvider({
            providerKey: provider.providerKey,
            credential: provider.credential,
          });
        }
        nextConfig = await savePiModelsProvider({
          providerKey: provider.providerKey,
          provider: provider.modelsProvider,
        });
        existingProviderIdSet.add(provider.providerKey);
        importedCount += 1;

        try {
          await upsertPiFavoriteProvider(
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
      message.success(t('pi.provider.importSuccess', { count: importedCount }));
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
    const importedCount = await saveImportedPiProviders(
      providers.map((provider) => {
        const payload = resolvePiFavoriteProviderPayload(provider);
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
    const importedCount = await saveImportedPiProviders(
      providers.map((provider) => ({
        providerKey: provider.providerId,
        modelsProvider: buildPiModelsProviderFromOpenCodeProvider(provider.providerConfig),
        displayName: provider.name,
      })),
    );
    if (importedCount > 0) {
      setAllApiHubImportModalOpen(false);
    }
  };

  const handleImportFromCcSwitch = async (imported: CcSwitchProviderCandidate[]) => {
    const importedCount = await saveImportedPiProviders(
      imported
        .map((candidate) => extractPiProviderFromCcSwitch(candidate))
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

    const provider = piProviders.find((item) => item.providerKey === connectivityProviderId);
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
        const updatedConfig = await savePiModelSettings({
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
  }, [clearBatchDeleteState, connectivityProviderId, modelForm, piProviders]);

  const handleBatchTestProviders = React.useCallback(async () => {
    const targets = piProviders.map((provider) => {
      const providerConfig = withResolvedPiApiKey(provider, buildPiOpenCodeProvider(provider));
      const modelIds = getProviderModelRecords(provider.modelsProvider).map((entry) => entry.id);
      return buildProviderConnectivityBatchTarget(
        {
          providerId: provider.providerKey,
          providerName: provider.displayName,
          providerConfig,
          modelIds,
          configValueMode: 'pi' as const,
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
      Object.fromEntries(piProviders.map((provider) => [
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
  }, [piProviders, t]);

  const handleDeletePiModel = async (provider: PiRuntimeProviderView, modelId: string) => {
    setSaving(true);
    try {
      const nextModels = getProviderModelRecords(provider.modelsProvider)
        .filter((entry) => entry.id !== modelId)
        .map((entry) => entry.model);
      const nextConfig = await saveProviderModels(provider, nextModels);
      if (provider.isDefault && nextConfig.modelSettings.modelId === modelId) {
        const updatedConfig = await savePiModelSettings({
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

  const handleDeleteProvider = (provider: PiRuntimeProviderView, scope: PiDeleteScope) => {
    Modal.confirm({
      title: t('pi.provider.deleteConfirmTitle'),
      content: t('pi.provider.deleteConfirmContent', {
        providerKey: provider.providerKey,
        scope: t(`pi.provider.deleteScope.${scope}`),
      }),
      okButtonProps: { danger: true },
      onOk: async () => {
        setSaving(true);
        try {
          if (provider.modelsProvider && !isRecordEmpty(provider.modelsProvider)) {
            try {
              await upsertPiFavoriteProvider(
                provider.providerKey,
                provider.modelsProvider,
                provider.credential,
                provider.displayName,
              );
            } catch (error) {
              console.error('Failed to preserve Pi favorite provider before deletion:', error);
            }
          }
          const nextConfig = await deletePiRuntimeProvider(provider.providerKey, scope);
          setRuntimeConfig(nextConfig);
          setOtherSettings(nextConfig.otherSettings || {});
          await refreshTrayMenu();
          message.success(t('common.success'));
        } catch (error) {
          console.error('Failed to delete Pi provider:', error);
          message.error(t('common.error'));
        } finally {
          setSaving(false);
        }
      },
    });
  };

  const handleDeleteSupplier = (provider: PiRuntimeProviderView) => {
    const hasCredential = provider.sources.includes('auth_json');
    const hasProviderConfig = provider.sources.includes('models_json');
    if (hasCredential && hasProviderConfig) {
      setDeleteScopeProvider(provider);
      return;
    }
    const scope: PiDeleteScope = hasCredential ? 'credential' : 'provider_config';
    handleDeleteProvider(provider, scope);
  };

  const handleDeleteScopeSelect = (scope: PiDeleteScope) => {
    const provider = deleteScopeProvider;
    setDeleteScopeProvider(null);
    if (provider) {
      handleDeleteProvider(provider, scope);
    }
  };

  const handleOtherSettingsBlur = async (value: unknown, isValid: boolean) => {
    if (!isValid || !otherSettingsValid) {
      message.error(t('pi.invalidJson'));
      return;
    }
    const nextOtherSettings = value && typeof value === 'object' && !Array.isArray(value)
      ? value as Record<string, unknown>
      : {};
    setSaving(true);
    try {
      const nextConfig = await savePiOtherSettings(nextOtherSettings);
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
      message.success(t('pi.modelsRefreshSuccess'));
    } catch (error) {
      console.error('Failed to refresh Pi preset models:', error);
      message.error(t('common.error'));
    } finally {
      setRefreshingModels(false);
    }
  };

  /**
   * One mapper for every layout. `ProviderListSection` renders the same node in
   * the list and the grid, so the two cannot offer different actions.
   */
  const renderProviderCard = (provider: PiRuntimeProviderView) => {
    const hasCredential = provider.sources.includes('auth_json');
    const hasProviderConfig = provider.sources.includes('models_json');
    const providerConfig = provider.modelsProvider ?? {};
    const canDeleteProvider = hasCredential || hasProviderConfig;
    const isBatchDeleteMode = batchDeleteProviderId === provider.providerKey;
    const selectedModelIds = selectedModelIdsByProvider[provider.providerKey] ?? [];
    const providerBaseUrl = getStringField(providerConfig, 'baseUrl');
    const hasModelIds = getProviderModelRecords(provider.modelsProvider).length > 0;

    return (
      <PiProviderCard
        key={provider.providerKey}
        provider={provider}
        defaultModelId={provider.isDefault
          ? runtimeConfig?.modelSettings.modelId ?? undefined
          : undefined}
        viewMode={viewMode}
        selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.providerKey)}
        selected={providerBatch.selectedIds.has(provider.providerKey)}
        onSelectChange={(checked) => providerBatch.toggleSelect(provider.providerKey, checked)}
        selectDisabledReason={providerBatch.selectionMode
          ? providerSelectionDisabledReason(t, batchBlockReason(provider))
          : undefined}
        connectivityStatus={connectivityStatuses[provider.providerKey]}
        // Only `models.json` entries have a stored position: an auth-only
        // provider is appended by the backend after them, so its handle would
        // drag nothing.
        dragDisabled={providerBatchDragDisabled || !hasProviderConfig}
        onEdit={() => openProviderModal(provider)}
        onCopy={() => openProviderModal(provider, { copy: true })}
        onShare={() => shareProvider({
          id: provider.providerKey, name: provider.displayName, category: 'custom',
          settingsConfig: JSON.stringify(providerConfig), credential: provider.credential,
          credentialUnavailable: provider.credentialKind === 'oauth' || provider.credentialKind === 'env_possible',
          defaultModel: provider.isDefault ? runtimeConfig?.modelSettings.modelId ?? undefined : undefined,
        })}
        // A provider that exists in neither runtime file has no delete path at
        // all; one that is the default keeps the button and explains itself.
        onDelete={canDeleteProvider ? () => handleDeleteSupplier(provider) : undefined}
        deleteDisabledReason={canDeleteProvider && provider.isDefault
          ? t('pi.provider.deleteDisabledDefault', { defaultValue: '该渠道已设为默认，不可删除' })
          : undefined}
        onAddModel={() => openPiModelModal(provider)}
        onEditModel={(modelId) => openPiModelModal(provider, modelId)}
        onCopyModel={(modelId) => openPiModelModal(provider, modelId, { copy: true })}
        onDeleteModel={(modelId) => handleDeletePiModel(provider, modelId)}
        onSetPrimaryModel={(modelId) => handleSetPrimaryModel(provider, modelId)}
        modelSelectionMode={isBatchDeleteMode}
        selectedModelIds={selectedModelIds}
        onToggleModelSelection={(modelId, selected) => handleToggleModelSelection(provider.providerKey, modelId, selected)}
        onReorderModels={(modelIds) => handleReorderModels(provider, modelIds)}
        onToggleBatchDeleteMode={() => handleToggleBatchDeleteMode(provider.providerKey)}
        onBatchDeleteModels={() => {
          Modal.confirm({
            title: t('common.model.batchDeleteConfirmTitle'),
            content: t('common.model.batchDeleteConfirmContent', { count: selectedModelIds.length }),
            okText: t('common.confirm'),
            cancelText: t('common.cancel'),
            onOk: async () => {
              await handleBatchDeleteModels(provider);
            },
          });
        }}
        onTestModels={() => handleOpenConnectivityTest(provider.providerKey)}
        testModelsDisabled={!providerBaseUrl || !hasModelIds}
        testModelsDisabledTooltip={!providerBaseUrl
          ? t('common.baseUrlMissing')
          : !hasModelIds
            ? t('common.modelMissing')
            : undefined}
        onFetchModels={() => handleOpenFetchModels(provider.providerKey)}
        fetchDisabled={!providerBaseUrl}
        fetchDisabledTooltip={!providerBaseUrl ? t('common.baseUrlMissing') : undefined}
      />
    );
  };

  // Dragging rewrites `models.json`'s `providers` key order. Search and
  // non-`custom` sort modes show an order the file does not contain, so a drop
  // there would persist something the user is not looking at.
  const providerDragDisabled = sortMode !== 'custom' || providerKeyword.trim() !== '';
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  const handleProviderDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) {
      return;
    }
    // Only keys that live in `models.json` have a position; auth-only providers
    // are appended by the backend and cannot be dragged.
    const providerKeys = piProviders
      .filter((provider) => provider.sources.includes('models_json'))
      .map((provider) => provider.providerKey);
    const oldIndex = providerKeys.indexOf(active.id as string);
    const newIndex = providerKeys.indexOf(over.id as string);
    if (oldIndex < 0 || newIndex < 0) {
      return;
    }
    const movedKeys = arrayMove(providerKeys, oldIndex, newIndex);
    const previousConfig = runtimeConfig;
    try {
      const nextConfig = await reorderPiProviders(movedKeys);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to reorder Pi providers:', error);
      setRuntimeConfig(previousConfig);
      message.error(t('common.error'));
    }
  };

  return (
    <Spin spinning={loading}>
      <SectionSidebarLayout
        sidebarTitle={t('pi.title')}
        sidebarHidden={sidebarHidden}
        sections={sidebarSections}
        markerAttr="data-pi-sidebar-section"
        getIcon={(id) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
      >
        <div className={styles.pageContent}>
          <CodingPageHeader
            title={t('pi.title')}
            docsUrl="https://pi.dev/docs/latest/quickstart"
            onPreviewConfig={() => setPreviewModalOpen(true)}
            configPath={runtimeConfig?.rootPathInfo.path ?? ''}
            // Pi's root is a directory (`auth.json` / `models.json` /
            // `settings.json` all derive from it), not one config file.
            configPathKind="directory"
            onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
            onOpenFolder={handleOpenRootFolder}
            onRefresh={handleRefreshConfig}
            onMoreOptions={() => setSettingsModalOpen(true)}
            extraActions={(
              <Button
                type="text"
                size="small"
                icon={<CloudSyncOutlined />}
                onClick={handleRefreshModelsCache}
                loading={refreshingModels}
                style={{ padding: 0, fontSize: 12 }}
              >
                {t('pi.syncModels')}
              </Button>
            )}
          />

          <div
            id="pi-model-settings"
            className={styles.piSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('pi.modelSettings.title')}
          >
            <div className={styles.modelCard}>
              <Title level={5} className={styles.modelCardTitle}>
                <RobotOutlined style={{ marginRight: 8 }} />
                {t('pi.modelSettings.title')}
              </Title>
              <div className={styles.modelCardContent}>
                <Form
                  form={modelForm}
                  layout="vertical"
                  onValuesChange={handleModelSettingsChange}
                >
                  <div className={styles.modelSettingsGrid}>
                    <Form.Item label={t('pi.modelSettings.defaultProvider')} name="defaultProvider">
                      <Select
                        allowClear
                        showSearch
                        options={providerOptions}
                        placeholder={t('pi.modelSettings.defaultProviderPlaceholder')}
                      />
                    </Form.Item>
                    <Form.Item label={t('pi.modelSettings.defaultModel')} name="defaultModel">
                      <Select
                        allowClear
                        showSearch
                        options={modelOptions}
                        placeholder={t('pi.modelSettings.defaultModelPlaceholder')}
                      />
                    </Form.Item>
                    {thinkingLevelOptions.length > 0 ? (
                      <Form.Item label={t('pi.modelSettings.thinkingLevel')} name="defaultThinkingLevel">
                        <Select
                          allowClear
                          options={thinkingLevelOptions}
                          placeholder={t('pi.modelSettings.thinkingLevelPlaceholder')}
                        />
                      </Form.Item>
                    ) : null}
                  </div>
                </Form>
              </div>
            </div>
          </div>

          <ProviderListSection
            sectionId="pi-providers"
            collapsed={providerListCollapsed}
            onCollapsedChange={setProviderListCollapsed}
            loading={loading}
            providerCount={piProviders.length}
            visibleCount={visibleProviders.length}
            batch={providerBatch}
            batchSelectableIds={batchSelectableIds}
            keyword={providerKeyword}
            onKeywordChange={setProviderKeyword}
            sortMode={sortMode}
            sortModes={PROVIDER_SORT_MODES_BASIC}
            onSortModeChange={setSortMode}
            dragDisabledBySort={providerDragDisabled}
            locateProviderId={defaultProviderId}
            viewMode={viewMode}
            onViewModeChange={setViewMode}
            onBatchTest={handleBatchTestProviders}
            batchTesting={batchTestingProviders}
            onAddProvider={() => openProviderModal()}
            hint={<div>{t('pi.pageHint')}</div>}
            footer={(
              <Space wrap>
                <Button
                  type="dashed"
                  icon={<ImportOutlined />}
                  onClick={() => setImportModalOpen(true)}
                >
                  {t('pi.provider.importFavorite')}
                </Button>
                {allApiHubAvailable && (
                  <Button
                    type="dashed"
                    icon={<AllApiHubIcon />}
                    onClick={() => setAllApiHubImportModalOpen(true)}
                  >
                    {t('pi.provider.importAllApiHub')}
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
            )}
          >
            {/* One `DndContext` serves both layouts; only the sorting strategy
                and the drag modifier change with the view. The order it writes
                is `models.json`'s `providers` key order, which is also what
                `build_provider_views` reads back. */}
            <DndContext
              sensors={providerBatchDragDisabled ? [] : sensors}
              collisionDetection={closestCenter}
              onDragEnd={handleProviderDragEnd}
              modifiers={[viewMode === 'card' ? restrictToWindowEdges : restrictToVerticalAxis]}
            >
              <SortableContext
                items={visibleProviders.map((provider) => provider.providerKey)}
                strategy={viewMode === 'card' ? rectSortingStrategy : verticalListSortingStrategy}
              >
                {viewMode === 'card' ? (
                  <ProviderCardGrid
                    items={visibleProviders}
                    getKey={(provider) => provider.providerKey}
                    renderItem={(provider) => renderProviderCard(provider)}
                  />
                ) : (
                  <div>
                    {visibleProviders.map((provider) => (
                      <React.Fragment key={provider.providerKey}>
                        {renderProviderCard(provider)}
                      </React.Fragment>
                    ))}
                  </div>
                )}
              </SortableContext>
            </DndContext>
          </ProviderListSection>

          <div
            id="pi-extensions"
            className={styles.piSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('pi.extensions.title')}
          >
            <PiExtensionsSection
              refreshKey={extensionsRefreshKey}
              // 扩展变更会改 `settings.json` 的 packages / extensions 过滤器，
              // 页面的其他部分（如 magic-context 区块）也读同一份文件。
              onChanged={async () => {
                await loadConfig(true);
                await refreshTrayMenu();
              }}
            />
          </div>

          <div
            id="pi-global-prompt"
            className={`${styles.piSection} ${styles.promptSection}`}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('common.prompt.title')}
          >
            <GlobalPromptSettings
              toolName="Pi"
              promptFileName="AGENTS.md"
              service={piPromptApi}
              collapseKey="pi-prompt"
              onUpdated={async () => {
                await loadConfig(true);
                await refreshTrayMenu();
              }}
            />
          </div>

          <div
            id="pi-other-configuration"
            className={styles.piSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('pi.otherConfig.title')}
          >
            <Collapse
              className={styles.collapseCard}
              items={[
                {
                  key: 'other',
                  label: (
                    <Space>
                      <SettingOutlined />
                      <Text strong>{t('pi.otherConfig.title')}</Text>
                    </Space>
                  ),
                  children: (
                    <Form.Item
                      help={
                        <span>
                          <Text type="secondary">{t('pi.otherConfig.hint')}，</Text>
                          <span style={{ color: 'var(--ant-color-primary)' }}>
                            {t('pi.otherConfig.autoSaveHint')}
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
            className={styles.piSection}
            data-pi-sidebar-section="true"
            data-sidebar-title={t('sessionManager.title')}
          >
            <SessionManagerPanel tool="pi" />
          </div>
        </div>

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(runtimeConfig?.rootPathInfo || null)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        <PiProviderFormModal
          open={!!providerModal}
          provider={providerModal?.provider}
          copy={providerModal?.copy}
          saving={saving}
          onCancel={() => setProviderModal(null)}
          onSubmit={(draft) => void handleSaveProviderModal(draft)}
        />

        <Modal
          title={t('pi.provider.deleteScopeModalTitle')}
          open={!!deleteScopeProvider}
          onCancel={() => setDeleteScopeProvider(null)}
          footer={deleteScopeProvider ? [
            <Button key="cancel" onClick={() => setDeleteScopeProvider(null)}>
              {t('common.cancel')}
            </Button>,
            <Button
              key="provider-config"
              danger
              onClick={() => handleDeleteScopeSelect('provider_config')}
            >
              {t('pi.provider.deleteProviderConfig')}
            </Button>,
            <Button
              key="credential"
              danger
              onClick={() => handleDeleteScopeSelect('credential')}
            >
              {t('pi.provider.deleteCredential')}
            </Button>,
            <Button
              key="both"
              danger
              type="primary"
              onClick={() => handleDeleteScopeSelect('both')}
            >
              {t('pi.provider.deleteBoth')}
            </Button>,
          ] : null}
          destroyOnHidden
        >
          <Text>
            {t('pi.provider.deleteScopeModalContent', {
              providerKey: deleteScopeProvider?.providerKey,
            })}
          </Text>
        </Modal>

        <ModelFormModal
          open={!!piModelModal}
          width={700}
          isEdit={!!piModelModal?.modelId}
          initialValues={piModelModal ? {
            id: piModelModal.modelId ?? getStringField(piModelModal.model ?? {}, 'id'),
            name: getStringField(piModelModal.model ?? {}, 'name'),
            api: getStringField(piModelModal.model ?? {}, 'api'),
            reasoning: typeof piModelModal.model?.reasoning === 'boolean'
              ? piModelModal.model.reasoning
              : undefined,
            inputTypes: stringifyStringArrayField(piModelModal.model?.input),
            thinkingLevelMap: stringifyRecordField(piModelModal.model?.thinkingLevelMap),
            compat: stringifyRecordField(piModelModal.model?.compat),
            contextLimit: typeof piModelModal.model?.contextWindow === 'number'
              ? piModelModal.model.contextWindow
              : undefined,
            outputLimit: typeof piModelModal.model?.maxTokens === 'number'
              ? piModelModal.model.maxTokens
              : undefined,
            costInput: getNumberField(asRecord(piModelModal.model?.cost), 'input'),
            costOutput: getNumberField(asRecord(piModelModal.model?.cost), 'output'),
            costCacheRead: getNumberField(asRecord(piModelModal.model?.cost), 'cacheRead'),
            costCacheWrite: getNumberField(asRecord(piModelModal.model?.cost), 'cacheWrite'),
          } : undefined}
          existingIds={piModelModal && !piModelModal.modelId
            ? getProviderModelRecords(piModelModal.provider.modelsProvider).map((entry) => entry.id)
            : []}
          showOptions={false}
          showVariants={false}
          showModalities={false}
          showInputTypes
          showApi
          apiOptions={PI_API_OPTIONS}
          showReasoning
          showThinkingLevelMap
          showCompat
          showCost
          limitRequired={false}
          nameRequired={false}
          npmType={piModelModal
            ? piApiToSdkName(getStringField(piModelModal.provider.modelsProvider ?? {}, 'api'))
            : undefined}
          toolName="Pi"
          messageOverrides={{
            name: t('pi.model.name'),
            namePlaceholder: t('pi.model.namePlaceholder'),
            nameOptionalPlaceholder: t('pi.model.nameOptionalPlaceholder'),
            idPlaceholder: t('pi.model.idPlaceholder'),
            contextLimit: t('pi.model.contextLimit'),
            contextLimitPlaceholder: t('pi.model.contextLimitPlaceholder'),
            outputLimit: t('pi.model.outputLimit'),
            outputLimitPlaceholder: t('pi.model.outputLimitPlaceholder'),
            reasoning: t('pi.model.reasoning'),
            costHint: t('pi.model.costHint'),
            thinkingLevelHint: t('pi.model.thinkingLevelHint'),
            thinkingLevelMapHint: t('pi.model.thinkingLevelMapHint'),
          }}
          onCancel={() => setPiModelModal(null)}
          onSuccess={handleSavePiModel}
          onDuplicateId={() => message.error(t('pi.model.idExists'))}
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
            configValueMode={fetchModelsProviderInfo.configValueMode}
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
          title={t('pi.provider.importModalTitle')}
          emptyDescription={t('pi.provider.noFavoriteProviders')}
          providerFilter={(provider) => isFavoriteProviderForSource('pi', provider)}
          providerListTransform={transformPiFavoriteProviders}
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
          title={t('pi.preview.title')}
          files={[
            { key: 'settings', label: 'settings.json', content: runtimeConfig?.settingsContent ?? runtimeConfig?.settings, language: 'json' },
            { key: 'auth', label: 'auth.json', content: runtimeConfig?.authContent ?? runtimeConfig?.auth, language: 'json' },
            { key: 'models', label: 'models.json', content: runtimeConfig?.modelsContent ?? runtimeConfig?.models, language: 'json' },
            { key: 'prompt', label: 'AGENTS.md', content: runtimeConfig?.promptContent, language: 'markdown' },
          ]}
        />

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={async (visible) => {
            await setSidebarHidden('pi', !visible);
          }}
        >
          <CliManualPathSetting commandName="pi" labelKey="subModules.pi" />
        </SidebarSettingsModal>

        {shareModal}
      </SectionSidebarLayout>
    </Spin>
  );
};

export default PiPage;
