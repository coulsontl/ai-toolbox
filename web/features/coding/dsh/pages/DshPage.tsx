import React from 'react';
import { useProviderSharing } from '@/features/coding/shared/providerShare';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import {
  Alert,
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
  CloudSyncOutlined,
  DatabaseOutlined,
  FileTextOutlined,
  GlobalOutlined,
  MessageOutlined,
  ImportOutlined,
  RobotOutlined,
  SettingOutlined,
  ToolOutlined,
} from '@ant-design/icons';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';

import JsonEditor from '@/components/common/JsonEditor';
import DshConfigPreviewModal from '@/components/common/DshConfigPreviewModal';
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
  buildFetchedDshModel,
  dshApiToSdkName,
} from '../utils/dshFetchedModels';
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
import DshProviderCard from '../components/DshProviderCard';
import DshMigrationNotice from '../components/DshMigrationNotice';
import DshProviderFormModal, {
  type DshProviderDraft,
} from '../components/DshProviderFormModal';
import {
  fetchRemotePresetModels,
  hasAllApiHubExtension,
  refreshTrayMenu,
} from '@/services/appApi';
import { useSettingsStore } from '@/stores';
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
  type DshFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import { upsertFavoriteProvider, type OpenCodeFavoriteProvider } from '@/services/opencodeApi';
import type { AllApiHubProviderItem } from '@/types/allApiHub';
import {
  buildDshProviderFromAllApiHub,
  extractDshProviderFromCcSwitch,
} from '../utils/importMapping';
import {
  buildDshProviderDeletionPlan,
  canDeleteDshProvider,
  credentialRefFromProviderKey,
} from '../utils/providerDeletion';
import {
  asRecord,
  getDshModelRecords,
  getNumberField,
  getProviderModelRecords,
  getStringField,
} from '../utils/providerModels';
import {
  createDefaultProviderConfig,
  hasProviderConfigContent,
  isCatalogOwnedRoute,
  isRecordEmpty,
  setOptionalStringField,
} from '../utils/providerConfig';
import {
  checkDshAgentInstructions,
  deleteDshCredential,
  deleteDshRuntimeProvider,
  enableDshAgentInstructions,
  getDshSettingsConfig,
  launchDshDashboard,
  listDshAllApiHubProviders,
  openDshWebUi,
  readDshRuntimeConfig,
  resolveDshAllApiHubProviders,
  saveDshCredential,
  saveDshModelSettings,
  saveDshModelsProvider,
  reorderDshProviders,
  saveDshOtherSettings,
  saveDshSettingsConfig,
} from '@/services/dshApi';
import { dshPromptApi } from '@/services/dshPromptApi';
import type { OpenCodeProvider } from '@/types/opencode';
import type {
  DshDeleteScope,
  DshRuntimeConfig,
  DshRuntimeProviderView,
} from '@/types/dsh';

import styles from './DshPage.module.less';

const { Title, Text } = Typography;

interface ProviderJsonModalState {
  provider?: DshRuntimeProviderView;
  /** Pre-fill from `provider`, save under `<key>_copy`. */
  copy?: boolean;
}

interface DshModelModalState {
  provider: DshRuntimeProviderView;
  modelId?: string;
  model?: Record<string, unknown>;
}

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'dsh-model-settings': <RobotOutlined />,
  'dsh-providers': <DatabaseOutlined />,
  'dsh-global-prompt': <FileTextOutlined />,
  'dsh-other-configuration': <ToolOutlined />,
  'dsh-session-manager': <MessageOutlined />,
};

/// Raw dsh provider dict from `llm-pi-ai.providers.<route>`; `{}` when absent.
const providerRawConfig = (provider: DshRuntimeProviderView): Record<string, unknown> =>
  provider.provider ?? {};

/**
 * 模型字段中由弹窗专用控件管理的键；其余字段视为额外参数原样保留。
 *
 * `input` / `compat` / `reasoningEfforts` 是上游 llm-pi-ai 的模型字段
 * （`packages/llm/llm-pi-ai/src/config.ts` 的 modelFields）。`reasoning` 与
 * `cost` 不是上游 schema 字段：schemastery 对未知键透传，它们只是本页展示层
 * 的兼容字段，保留以免保存时洗掉历史数据。
 */
const DSH_KNOWN_MODEL_FIELDS = new Set([
  'id', 'name', 'contextWindow', 'maxTokens', 'input', 'reasoningEfforts', 'compat',
  'reasoning', 'cost',
]);

/** 解析模型弹窗的输入类型字符串（JSON 数组）。 */
const parseDshInputTypes = (value: string | undefined): string[] => {
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

/** 解析模型弹窗的 compat JSON 字符串。 */
const parseDshCompat = (value: string | undefined): Record<string, unknown> => {
  if (!value) {
    return {};
  }
  try {
    return asRecord(JSON.parse(value));
  } catch {
    return {};
  }
};

/** 把字符串数组序列化成模型弹窗的输入类型字段。 */
const stringifyDshInputTypes = (value: unknown): string | undefined => {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const values = value.filter((entry): entry is string => typeof entry === 'string');
  return values.length > 0 ? JSON.stringify(values) : undefined;
};

/** 把对象序列化成模型弹窗的 compat 字段。 */
const stringifyDshRecord = (value: unknown): string | undefined => {
  const record = asRecord(value);
  return isRecordEmpty(record) ? undefined : JSON.stringify(record, null, 2);
};

/** 提取模型已知字段之外的额外参数（与 OpenClaw extraParams 一致）。 */
const extractDshExtraParams = (model: unknown): Record<string, unknown> | undefined => {
  const record = asRecord(model);
  const extra: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(record)) {
    if (!DSH_KNOWN_MODEL_FIELDS.has(key) && value !== undefined) {
      extra[key] = value;
    }
  }
  return Object.keys(extra).length > 0 ? extra : undefined;
};

const asStringRecord = (value: unknown): Record<string, string> => {
  const record = asRecord(value);
  return Object.fromEntries(
    Object.entries(record).filter((entry): entry is [string, string] => typeof entry[1] === 'string'),
  );
};

const buildDshOpenCodeProvider = (
  provider: DshRuntimeProviderView,
  providerConfig: Record<string, unknown> = providerRawConfig(provider) ?? {},
  apiKeyValue = '',
): OpenCodeProvider => {
  // Same explicit-models → bundled-catalog fallback as the card and the probes;
  // a second copy here would let sharing and display disagree about a route.
  const records = getDshModelRecords(provider);
  const models = Object.fromEntries(
    records.map((entry) => [
      entry.id,
      {
        ...entry.model,
        id: getStringField(entry.model, 'id') || entry.id,
        name: getStringField(entry.model, 'name') || entry.id,
      },
    ]),
  );
  const headers = asStringRecord(providerConfig.headers);
  return {
    npm: dshApiToSdkName(getStringField(providerConfig, 'api')),
    name: provider.displayName,
    options: {
      baseURL: getStringField(providerConfig, 'baseURL'),
      ...(provider.apiKey || apiKeyValue ? { apiKey: provider.apiKey || apiKeyValue } : {}),
      ...(isRecordEmpty(headers) ? {} : { headers }),
    },
    models,
  };
};

/** Build the OpenCode-provider envelope that wraps a DSH provider favorite. The credential
 *  (API key) is kept alongside the route definition so a re-import can replay both writes. */
const buildDshFavoriteProviderConfig = (
  providerKey: string,
  modelsProvider: Record<string, unknown>,
  credential?: { refName: string; value: string },
): OpenCodeProvider => {
  const headers = asStringRecord(modelsProvider.headers);
  const records = getProviderModelRecords(modelsProvider);
  const apiKey = credential?.value;
  const payload: DshFavoriteProviderPayload = {
    providerKey,
    ...(credential && credential.value ? { credential } : {}),
    modelsProvider,
  };
  return buildFavoriteProviderOptions(
    {
      npm: dshApiToSdkName(getStringField(modelsProvider, 'api')),
      name: getStringField(modelsProvider, 'displayName') || providerKey,
      options: {
        baseURL: getStringField(modelsProvider, 'baseURL'),
        ...(apiKey ? { apiKey } : {}),
        ...(isRecordEmpty(headers) ? {} : { headers }),
      },
      models: Object.fromEntries(records.map((entry) => [entry.id, {}])),
    },
    payload,
  );
};

/** Resolve the favorite payload back into a DSH-language provider import record. */
const resolveDshFavoriteProviderPayload = (
  favoriteProvider: OpenCodeFavoriteProvider,
): DshFavoriteProviderPayload => {
  const payload = getFavoriteProviderPayload<DshFavoriteProviderPayload>(favoriteProvider);
  if (payload?.providerKey && payload.modelsProvider) {
    return payload;
  }
  return {
    providerKey: extractFavoriteProviderRawId('dsh', favoriteProvider.providerId),
    modelsProvider: payload?.modelsProvider ?? favoriteProvider.providerConfig,
  };
};

const DshPage: React.FC = () => {
  const { t } = useTranslation();
  const { shareProvider, shareModal } = useProviderSharing('dsh', () => loadConfig());
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [refreshingModels, setRefreshingModels] = React.useState(false);
  const [runtimeConfig, setRuntimeConfig] = React.useState<DshRuntimeConfig | null>(null);
  const [modelForm] = Form.useForm();
  const [providerModal, setProviderModal] = React.useState<ProviderJsonModalState | null>(null);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [dshModelModal, setDshModelModal] = React.useState<DshModelModalState | null>(null);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const { viewMode, setViewMode } = useProviderListViewPrefs('dsh');
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const [batchDeleteProviderId, setBatchDeleteProviderId] = React.useState<string | null>(null);
  const [selectedModelIdsByProvider, setSelectedModelIdsByProvider] = React.useState<Record<string, string[]>>({});
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string | null>(null);
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [connectivityProviderId, setConnectivityProviderId] = React.useState<string | null>(null);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);
  const [otherSettings, setOtherSettings] = React.useState<Record<string, unknown>>({});
  const [otherSettingsValid, setOtherSettingsValid] = React.useState(true);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const [deleteScopeProvider, setDeleteScopeProvider] = React.useState<DshRuntimeProviderView | null>(null);
  /** "打开 Web UI" 离线回退 Modal 的阶段:null=关闭,'initial'=启动 dsh web,'npx'=改用 npx。 */
  const [launchModalStage, setLaunchModalStage] = React.useState<'initial' | 'npx' | null>(null);
  const [launchingDashboard, setLaunchingDashboard] = React.useState(false);
  const [agentInstructionsEnabled, setAgentInstructionsEnabled] = React.useState(true);
  const [enablingAgentInstructions, setEnablingAgentInstructions] = React.useState(false);
  const modelSettingsSaveSeqRef = React.useRef(0);
  const sidebarHidden = sidebarHiddenByPage.dsh;

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'dsh-model-settings',
      title: t('dsh.modelSettings.title', { defaultValue: '默认模型' }),
      order: 1,
    },
    {
      id: 'dsh-providers',
      title: t('dsh.provider.title', { defaultValue: '供应商列表' }),
      order: 2,
    },
    {
      id: 'dsh-global-prompt',
      title: t('common.prompt.title'),
      order: 3,
    },
    {
      id: 'dsh-other-configuration',
      title: t('dsh.otherConfig.title', { defaultValue: '其他配置' }),
      order: 4,
    },
    {
      id: 'dsh-session-manager',
      title: t('sessionManager.title'),
      order: 5,
    },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const config = await readDshRuntimeConfig();
      setRuntimeConfig(config);
      setOtherSettings(config.otherSettings || {});
      modelForm.setFieldsValue({
        defaultProvider: config.modelSettings.provider || undefined,
        defaultModel: config.modelSettings.model || undefined,
        defaultReasoningEffort: config.modelSettings.reasoningEffort || undefined,
      });
    } catch (error) {
      console.error('Failed to load dsh runtime config:', error);
      message.error(t('common.error'));
    } finally {
      if (!silent) {
        setLoading(false);
      }
    }
  }, [modelForm, t]);

  const checkAgentInstructions = React.useCallback(async () => {
    try {
      const status = await checkDshAgentInstructions();
      setAgentInstructionsEnabled(status.enabled);
    } catch (error) {
      console.error('Failed to check dsh agent-instructions:', error);
    }
  }, []);

  const handleEnableAgentInstructions = async () => {
    setEnablingAgentInstructions(true);
    try {
      await enableDshAgentInstructions();
      await checkAgentInstructions();
      message.success(t('dsh.agentInstructions.enableSuccess'));
    } catch (error) {
      console.error('Failed to enable dsh agent-instructions:', error);
      message.error(t('common.error'));
    } finally {
      setEnablingAgentInstructions(false);
    }
  };

  React.useEffect(() => {
    checkAgentInstructions();
  }, [checkAgentInstructions]);

  React.useEffect(() => {
    loadConfig();
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

  const [pinnedProfile, setPinnedProfile] = React.useState<string | undefined>(undefined);
  React.useEffect(() => {
    let cancelled = false;
    getDshSettingsConfig()
      .then((cfg) => {
        if (!cancelled) {
          setPinnedProfile(cfg?.profile || undefined);
        }
      })
      .catch((error) => {
        console.error('Failed to load dsh settings config:', error);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig({
    t,
    translationKeyPrefix: 'dsh',
    defaultConfig: '{}',
    loadConfig,
    getCommonConfig: async () => {
      const cfg = await getDshSettingsConfig();
      return { config: cfg?.configDir ?? '', rootDir: cfg?.configDir ?? null };
    },
    saveCommonConfig: saveDshSettingsConfig,
  });

  /**
   * `null` clears the pin (back to auto-detection), a name pins that profile.
   * The backend keeps the current pin when `profile` is omitted, so an empty
   * string is the explicit "clear" signal.
   */
  const handleSelectProfile = async (profile: string | null) => {
    try {
      const cfg = await getDshSettingsConfig();
      await saveDshSettingsConfig({
        config: cfg?.configDir ?? '',
        rootDir: cfg?.configDir ?? null,
        profile: profile ?? '',
      });
      setPinnedProfile(profile ?? undefined);
      await loadConfig(true);
    } catch (error) {
      console.error('Failed to save dsh profile:', error);
      message.error(t('common.error'));
    }
  };

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
    const current = runtimeConfig?.modelSettings.provider;
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
    const current = selectedDefaultModel || runtimeConfig?.modelSettings.model;
    if (current) {
      options.add(current);
    }
    return Array.from(options).map((modelId) => ({ value: modelId, label: modelId }));
  }, [runtimeConfig?.modelSettings.model, selectedDefaultModel, selectedProvider?.modelIds]);

  const defaultProviderId = React.useMemo(
    () => (runtimeConfig?.providers ?? []).find((provider) => provider.isDefault)?.providerKey ?? '',
    [runtimeConfig?.providers],
  );
  const dshProviders = React.useMemo(
    () => runtimeConfig?.providers ?? [],
    [runtimeConfig?.providers],
  );

  const existingFavoriteProviderIds = React.useMemo(
    () => dshProviders.map((provider) => buildFavoriteProviderStorageKey('dsh', provider.providerKey)),
    [dshProviders],
  );

  /** Persist a DSH provider route-plus-credential pair into the shared favorite-provider
   *  history. Errors are swallowed so a favorite failure never blocks the primary flow. */
  const upsertDshFavoriteProvider = React.useCallback(
    (providerKey: string, modelsProvider: Record<string, unknown>, credential?: { refName: string; value: string }) => {
      return upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('dsh', providerKey),
        buildDshFavoriteProviderConfig(providerKey, modelsProvider, credential),
      ).catch((error) => {
        console.error('Failed to save DSH favorite provider:', error);
      });
    },
    [],
  );

  // Reasoning-effort options follow the selected default model's
  // `reasoningEfforts` keys (e.g. off/low/high/max) instead of a fixed subset,
  // so levels like `max` stay selectable once the model declares them.
  const reasoningEffortOptions = React.useMemo(() => {
    const provider = selectedProviderKey
      ? dshProviders.find((item) => item.providerKey === selectedProviderKey)
      : undefined;
    const modelId = selectedDefaultModel || runtimeConfig?.modelSettings.model;
    const model = provider
      ? getDshModelRecords(provider)
        .find((entry) => entry.id === modelId)?.model
      : undefined;
    const efforts = asRecord(model?.reasoningEfforts);
    const levels = Object.keys(efforts).filter((key) => {
      const value = efforts[key];
      return value !== null && value !== undefined && value !== '';
    });
    const effective = levels.length > 0 ? [...new Set(levels)] : ['low', 'medium', 'high'];
    return effective.map((value) => ({ value, label: value }));
  }, [selectedProviderKey, selectedDefaultModel, dshProviders, runtimeConfig?.modelSettings.model]);

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!fetchModelsProviderId) {
      return null;
    }
    const provider = dshProviders.find((item) => item.providerKey === fetchModelsProviderId);
    if (!provider) {
      return null;
    }
    const providerConfig = providerRawConfig(provider) ?? {};
    return {
      providerId: provider.providerKey,
      name: provider.displayName,
      baseUrl: getStringField(providerConfig, 'baseURL'),
      apiKey: provider.apiKey || '',
      headers: asStringRecord(providerConfig.headers),
      sdkName: dshApiToSdkName(getStringField(providerConfig, 'api')),
      existingModelIds: getDshModelRecords(provider).map((entry) => entry.id),
    };
  }, [fetchModelsProviderId, dshProviders]);

  const connectivityInfo = React.useMemo(() => {
    if (!connectivityProviderId) {
      return null;
    }
    const provider = dshProviders.find((item) => item.providerKey === connectivityProviderId);
    if (!provider) {
      return null;
    }
    const providerConfig = providerRawConfig(provider) ?? {};
    const modelIds = getDshModelRecords(provider).map((entry) => entry.id);
    return {
      providerId: provider.providerKey,
      providerName: provider.displayName,
      providerConfig: buildDshOpenCodeProvider(provider, providerConfig),
      modelIds,
    };
  }, [connectivityProviderId, dshProviders]);

  const handleModelSettingsChange = async (
    changedValues: Record<string, unknown>,
    allValues: {
      defaultProvider?: string;
      defaultModel?: string;
      defaultReasoningEffort?: string;
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

    const currentSettings = runtimeConfig.modelSettings;
    const nextDefaultProvider = nextValues.defaultProvider ?? '';
    const nextDefaultModel = nextValues.defaultModel ?? '';
    const nextDefaultReasoningEffort = nextValues.defaultReasoningEffort ?? '';
    if (
      (currentSettings.provider ?? '') === nextDefaultProvider
      && (currentSettings.model ?? '') === nextDefaultModel
      && (currentSettings.reasoningEffort ?? '') === nextDefaultReasoningEffort
    ) {
      return;
    }

    const saveSeq = modelSettingsSaveSeqRef.current + 1;
    modelSettingsSaveSeqRef.current = saveSeq;
    setSaving(true);
    try {
      const nextConfig = await saveDshModelSettings({
        provider: nextDefaultProvider,
        model: nextDefaultModel,
        reasoningEffort: nextDefaultReasoningEffort,
      });
      if (modelSettingsSaveSeqRef.current === saveSeq) {
        setRuntimeConfig(nextConfig);
        setOtherSettings(nextConfig.otherSettings || {});
      }
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save dsh model settings:', error);
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
    provider?: DshRuntimeProviderView,
    options?: { copy?: boolean },
  ) => {
    setProviderModal({ provider, copy: options?.copy === true });
  };

  /**
   * Save what the modal handed back.
   *
   * dsh splits a route across two files: the provider entry
   * (`llm-pi-ai.providers.<route>`) and the secret (`.credentials.yaml`, under a
   * `REF` name). An API key typed in the form writes the credential; clearing it
   * on an existing provider deletes the credential. The provider entry is
   * written whenever the route exists or the form says something — a catalog
   * route can carry a credential without ever getting a provider entry.
   */
  const handleSaveProviderModal = async (draft: DshProviderDraft) => {
    if (!providerModal) {
      return;
    }
    const providerKey = draft.values.providerKey?.trim();
    if (!providerKey) {
      message.error(t('dsh.provider.providerKeyRequired', { defaultValue: '请输入供应商 Key' }));
      return;
    }

    setSaving(true);
    try {
      const existingProvider = runtimeConfig?.providers.find(
        (provider) => provider.providerKey === providerKey,
      );
      const hadCredential = !!existingProvider?.credentialExists;
      const nextApiKey = typeof draft.values.providerApiKey === 'string'
        ? draft.values.providerApiKey.trim()
        : '';
      const credentialRef = providerModal.provider?.apiKeyEnv
        || credentialRefFromProviderKey(providerKey);
      const shouldSaveCredential = nextApiKey !== '';
      const shouldDeleteCredential = hadCredential && nextApiKey === '';
      // Copy, never alias: `asRecord` returns the live `runtimeConfig` object,
      // and the `delete`s below would mutate it before the command runs. A
      // failed save would then leave the rejected edit in state, and the next
      // write that spreads this record would persist a route missing its
      // `baseURL` / `displayName`.
      const nextProviderConfigJson = providerModal.provider?.provider
        ? { ...asRecord(providerModal.provider.provider) }
        : createDefaultProviderConfig();
      // Built-in/catalog channels keep display name and wire protocol from the
      // catalog (official llm-pi-ai behavior); only custom routes own them.
      const editingBuiltIn = isCatalogOwnedRoute(providerModal.provider, providerModal.copy === true);
      if (editingBuiltIn) {
        delete nextProviderConfigJson.displayName;
        delete nextProviderConfigJson.api;
      } else {
        setOptionalStringField(nextProviderConfigJson, 'displayName', draft.values.displayName);
        setOptionalStringField(nextProviderConfigJson, 'api', draft.values.api);
      }
      setOptionalStringField(nextProviderConfigJson, 'baseURL', draft.values.baseUrl);
      // dsh has no authHeader flag; drop any stale key written by older builds.
      delete nextProviderConfigJson.authHeader;
      if (isRecordEmpty(draft.modelOverrides)) {
        delete nextProviderConfigJson.modelOverrides;
      } else {
        nextProviderConfigJson.modelOverrides = draft.modelOverrides;
      }

      let nextConfig: DshRuntimeConfig | null = null;
      if (shouldSaveCredential) {
        nextConfig = await saveDshCredential({ refName: credentialRef, value: nextApiKey });
        setOptionalStringField(nextProviderConfigJson, 'apiKeyEnv', credentialRef);
      } else if (shouldDeleteCredential) {
        nextConfig = await deleteDshCredential(credentialRef);
      }
      const shouldSaveProviderConfig = !providerModal.provider
        || !!providerModal.provider.provider
        || hasProviderConfigContent(nextProviderConfigJson);
      if (shouldSaveProviderConfig) {
        nextConfig = await saveDshModelsProvider({
          providerKey,
          provider: nextProviderConfigJson,
        });
      } else if (!shouldSaveCredential && !shouldDeleteCredential) {
        message.error(t('dsh.provider.selectAtLeastOneSection', { defaultValue: '请至少填写一个配置区块' }));
        return;
      }
      if (!nextConfig) {
        return;
      }
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      setProviderModal(null);
      void upsertDshFavoriteProvider(
        providerKey,
        nextProviderConfigJson,
        shouldSaveCredential ? { refName: credentialRef, value: nextApiKey } : undefined,
      );
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save dsh provider:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const openDshModelModal = (
    provider: DshRuntimeProviderView,
    modelId?: string,
    options?: { copy?: boolean },
  ) => {
    const model = modelId
      ? getDshModelRecords(provider).find((entry) => entry.id === modelId)?.model
      : undefined;
    const isCopy = options?.copy === true;
    const nextModel = model ? { ...model } : undefined;
    if (isCopy && nextModel && modelId) {
      nextModel.id = `${modelId}_copy`;
    }

    setDshModelModal({ provider, modelId: isCopy ? undefined : modelId, model: nextModel });
  };

  const handleSaveDshModel = async (values: ModelFormValues) => {
    if (!dshModelModal) {
      return;
    }
    const modelId = values.id?.trim();
    if (!modelId) {
      message.error(t('dsh.model.idRequired', { defaultValue: '请输入模型 ID' }));
      return;
    }

    const currentProvider = runtimeConfig?.providers.find(
      (provider) => provider.providerKey === dshModelModal.provider.providerKey,
    ) ?? dshModelModal.provider;
    const existingModels = getDshModelRecords(currentProvider);
    const duplicateModel = existingModels.some((entry) => (
      entry.id === modelId && entry.id !== dshModelModal.modelId
    ));
    if (duplicateModel) {
      message.error(t('dsh.model.idExists', { defaultValue: '模型 ID 已存在' }));
      return;
    }

    const nextModel = { ...(dshModelModal.model ?? {}) };
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
    // reasoningEfforts from the thinkingLevelMap JSON editor. Preset default maps
    // fill every level with `null` where the preset does not support it; drop
    // null/empty levels so only real effort mappings are persisted.
    if (typeof values.thinkingLevelMap === 'string' && values.thinkingLevelMap.trim()) {
      try {
        const parsed = JSON.parse(values.thinkingLevelMap);
        const entries = parsed && typeof parsed === 'object'
          ? Object.entries(parsed as Record<string, unknown>)
            .filter(([, value]) => value !== null && value !== undefined && value !== '')
          : [];
        if (entries.length > 0) {
          nextModel.reasoningEfforts = Object.fromEntries(entries);
        } else {
          delete nextModel.reasoningEfforts;
        }
      } catch {
        delete nextModel.reasoningEfforts;
      }
    } else {
      delete nextModel.reasoningEfforts;
    }
    // 输入类型与 compat 是上游模型字段；空值必须删除键，而不是写空数组/空对象
    // （空数组表示“未声明”，空 compat 还会被上游当成一次无效配置）。
    const inputTypes = parseDshInputTypes(values.inputTypes);
    if (inputTypes.length > 0) {
      nextModel.input = inputTypes;
    } else {
      delete nextModel.input;
    }
    const compat = parseDshCompat(values.compat);
    if (!isRecordEmpty(compat)) {
      nextModel.compat = compat;
    } else {
      delete nextModel.compat;
    }

    // 额外参数：以编辑器内容为准，移除旧的未知字段后再合并（与 OpenClaw 一致）。
    for (const key of Object.keys(nextModel)) {
      if (!DSH_KNOWN_MODEL_FIELDS.has(key)) {
        delete nextModel[key];
      }
    }
    if (values.extraParams && typeof values.extraParams === 'object') {
      Object.assign(nextModel, values.extraParams);
    }

    let modelWasReplaced = false;
    const nextModels = existingModels.map((entry) => {
      if (entry.id === dshModelModal.modelId) {
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
        ...providerRawConfig(currentProvider),
        models: nextModels,
      };
      const nextConfig = await saveDshModelsProvider({
        providerKey: currentProvider.providerKey,
        provider: nextProviderConfig,
      });
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      setDshModelModal(null);
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save dsh model:', error);
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
    provider: DshRuntimeProviderView,
    nextModels: Record<string, unknown>[],
  ) => {
    const nextProviderConfig = {
      ...(providerRawConfig(provider) ?? {}),
      models: nextModels,
    };
    const nextConfig = await saveDshModelsProvider({
      providerKey: provider.providerKey,
      provider: nextProviderConfig,
    });
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

  const handleBatchDeleteModels = async (provider: DshRuntimeProviderView) => {
    const selectedModelIds = selectedModelIdsByProvider[provider.providerKey] ?? [];
    if (selectedModelIds.length === 0) {
      return;
    }

    setSaving(true);
    try {
      const selectedModelIdSet = new Set(selectedModelIds);
      const nextModels = getDshModelRecords(provider)
        .filter((entry) => !selectedModelIdSet.has(entry.id))
        .map((entry) => entry.model);
      const nextConfig = await saveProviderModels(provider, nextModels);
      if (
        provider.isDefault
        && nextConfig.modelSettings.model
        && selectedModelIdSet.has(nextConfig.modelSettings.model)
      ) {
        const updatedConfig = await saveDshModelSettings({
          provider: nextConfig.modelSettings.provider ?? provider.providerKey,
          model: '',
          reasoningEffort: '',
        });
        setRuntimeConfig(updatedConfig);
        setOtherSettings(updatedConfig.otherSettings || {});
        modelForm.setFieldValue('defaultModel', undefined);
      }
      clearBatchDeleteState(provider.providerKey);
      message.success(t('dsh.model.batchDeleteSuccess', {
        defaultValue: '已删除 {{count}} 个模型',
        count: selectedModelIds.length,
      }));
    } catch (error) {
      console.error('Failed to batch delete dsh models:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleReorderModels = async (provider: DshRuntimeProviderView, modelIds: string[]) => {
    const currentModelMap = new Map(
      getDshModelRecords(provider).map((entry) => [entry.id, entry.model]),
    );
    const nextModels = modelIds
      .map((modelId) => currentModelMap.get(modelId))
      .filter((model): model is Record<string, unknown> => !!model);

    setSaving(true);
    try {
      await saveProviderModels(provider, nextModels);
    } catch (error) {
      console.error('Failed to reorder dsh models:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('dsh');
  const [providerKeyword, setProviderKeyword] = React.useState('');
  const visibleProviders = React.useMemo(
    () =>
      sortProviderItems(
        filterProviderItems(dshProviders, providerKeyword, (provider) => [
          provider.providerKey,
          provider.displayName,
          ...(provider.modelIds ?? []),
        ]),
        sortMode,
        { name: (provider) => provider.displayName || provider.providerKey },
        (provider) => lastUsedAt(provider.providerKey),
      ),
    [dshProviders, providerKeyword, sortMode, lastUsedAt],
  );

  const handleSetPrimaryModel = async (provider: DshRuntimeProviderView, modelId: string) => {
    setSaving(true);
    try {
      const nextConfig = await saveDshModelSettings({
        provider: provider.providerKey,
        model: modelId,
        reasoningEffort: runtimeConfig?.modelSettings.reasoningEffort ?? '',
      });
      noteProviderUsed(provider.providerKey);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      modelForm.setFieldsValue({
        defaultProvider: provider.providerKey,
        defaultModel: modelId,
        defaultReasoningEffort: nextConfig.modelSettings.reasoningEffort || undefined,
      });
      await refreshTrayMenu();
      message.success(t('dsh.model.setAsPrimarySuccess', {
        defaultValue: '已将 {{name}} 设为默认模型',
        name: modelId,
      }));
    } catch (error) {
      console.error('Failed to set dsh default model:', error);
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
    const provider = dshProviders.find((item) => item.providerKey === fetchModelsProviderId);
    if (!provider) {
      return;
    }

    const removedModelIdSet = new Set(removedModelIds);
    const currentModels = getDshModelRecords(provider)
      .filter((entry) => !removedModelIdSet.has(entry.id))
      .map((entry) => entry.model);
    const currentModelIds = new Set(currentModels.map((model) => getStringField(model, 'id')));
    const providerApi = getStringField(providerRawConfig(provider) ?? {}, 'api');
    selectedModels.forEach((model) => {
      if (!currentModelIds.has(model.id)) {
        const matchedPresetModel = findPresetModelById(model.id, dshApiToSdkName(providerApi));
        currentModels.push(buildFetchedDshModel(model, matchedPresetModel));
      }
    });

    setSaving(true);
    try {
      await saveProviderModels(provider, currentModels);
      clearBatchDeleteState(provider.providerKey);
      setFetchModelsModalOpen(false);
      message.success(t('dsh.fetchModels.applySuccess', {
        defaultValue: '已应用 {{addCount}} 个模型，移除 {{removeCount}} 个',
        addCount: selectedModels.length,
        removeCount: removedModelIds.length,
      }));
    } catch (error) {
      console.error('Failed to apply fetched dsh models:', error);
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

    const provider = dshProviders.find((item) => item.providerKey === connectivityProviderId);
    if (!provider) {
      return;
    }

    const selectedModelIdSet = new Set(modelIdsToRemove);
    const nextModels = getDshModelRecords(provider)
      .filter((entry) => !selectedModelIdSet.has(entry.id))
      .map((entry) => entry.model);

    setSaving(true);
    try {
      const nextConfig = await saveProviderModels(provider, nextModels);
      if (
        provider.isDefault
        && nextConfig.modelSettings.model
        && selectedModelIdSet.has(nextConfig.modelSettings.model)
      ) {
        const updatedConfig = await saveDshModelSettings({
          provider: nextConfig.modelSettings.provider ?? provider.providerKey,
          model: '',
          reasoningEffort: '',
        });
        setRuntimeConfig(updatedConfig);
        setOtherSettings(updatedConfig.otherSettings || {});
        modelForm.setFieldValue('defaultModel', undefined);
      }
      clearBatchDeleteState(provider.providerKey);
    } catch (error) {
      console.error('Failed to remove dsh models from connectivity test:', error);
      throw error;
    } finally {
      setSaving(false);
    }
  }, [clearBatchDeleteState, connectivityProviderId, modelForm, dshProviders]);

  const handleBatchTestProviders = React.useCallback(async () => {
    const targets = dshProviders.map((provider) => {
      const providerConfig = buildDshOpenCodeProvider(provider, providerRawConfig(provider));
      const modelIds = getDshModelRecords(provider).map((entry) => entry.id);
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
      Object.fromEntries(dshProviders.map((provider) => [
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
      console.error('Failed to batch test dsh providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [dshProviders, t]);

  const handleDeleteDshModel = async (provider: DshRuntimeProviderView, modelId: string) => {
    setSaving(true);
    try {
      const nextModels = getDshModelRecords(provider)
        .filter((entry) => entry.id !== modelId)
        .map((entry) => entry.model);
      const nextConfig = await saveProviderModels(provider, nextModels);
      if (provider.isDefault && nextConfig.modelSettings.model === modelId) {
        const updatedConfig = await saveDshModelSettings({
          provider: nextConfig.modelSettings.provider ?? provider.providerKey,
          model: '',
          reasoningEffort: '',
        });
        setRuntimeConfig(updatedConfig);
        setOtherSettings(updatedConfig.otherSettings || {});
        modelForm.setFieldValue('defaultModel', undefined);
        await refreshTrayMenu();
      }
      clearBatchDeleteState(provider.providerKey);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete dsh model:', error);
      message.error(t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteProvider = (
    provider: DshRuntimeProviderView,
    scope: DshDeleteScope,
    skipConfirm = false,
  ) => {
    const performDelete = async () => {
      setSaving(true);
      try {
        let nextConfig: DshRuntimeConfig;
        const credentialRef = provider.apiKeyEnv || credentialRefFromProviderKey(provider.providerKey);
        // Back up the provider into the favorite-provider history before deletion so it can be
        // re-imported later from "导入我使用过的供应商". Only persist what's being deleted.
        try {
          const credentialForFavorite =
            provider.credentialExists && provider.apiKey
              ? { refName: credentialRef, value: provider.apiKey }
              : undefined;
          if ((scope === 'provider' || scope === 'both') && provider.provider) {
            await upsertFavoriteProvider(
              buildFavoriteProviderStorageKey('dsh', provider.providerKey),
              buildDshFavoriteProviderConfig(provider.providerKey, provider.provider, credentialForFavorite),
            );
          }
        } catch (error) {
          console.error('Failed to preserve DSH favorite provider before deletion:', error);
        }
        if (scope === 'credential') {
          nextConfig = await deleteDshCredential(credentialRef);
        } else if (scope === 'provider') {
          nextConfig = await deleteDshRuntimeProvider(provider.providerKey);
        } else {
          if (provider.credentialExists) {
            nextConfig = await deleteDshCredential(credentialRef);
          }
          nextConfig = await deleteDshRuntimeProvider(provider.providerKey);
        }
        setRuntimeConfig(nextConfig);
        setOtherSettings(nextConfig.otherSettings || {});
        await refreshTrayMenu();
        message.success(t('common.success'));
      } catch (error) {
        console.error('Failed to delete dsh provider:', error);
        message.error(t('common.error'));
      } finally {
        setSaving(false);
      }
    };

    // deleteScopeProvider 弹窗已二次确认过,直接执行;直接删除路径仍走 Modal.confirm。
    if (skipConfirm) {
      void performDelete();
      return;
    }

    Modal.confirm({
      title: t('dsh.provider.deleteConfirmTitle', { defaultValue: '删除供应商' }),
      content: t('dsh.provider.deleteConfirmContent', {
        defaultValue: '确定删除供应商 {{providerKey}}（{{scope}}）吗？',
        providerKey: provider.providerKey,
        scope: t(`dsh.provider.deleteScope.${scope}`),
      }),
      okButtonProps: { danger: true },
      onOk: performDelete,
    });
  };

  const handleDeleteSupplier = (provider: DshRuntimeProviderView) => {
    const hasCredential = provider.credentialExists;
    const hasProviderConfig = !!provider.provider;
    if (hasCredential && hasProviderConfig) {
      setDeleteScopeProvider(provider);
      return;
    }
    const scope: DshDeleteScope = hasCredential ? 'credential' : 'provider';
    handleDeleteProvider(provider, scope);
  };

  const handleDeleteScopeSelect = (scope: DshDeleteScope) => {
    const provider = deleteScopeProvider;
    setDeleteScopeProvider(null);
    if (provider) {
      handleDeleteProvider(provider, scope, true);
    }
  };

  const handleBatchDeleteProviders = React.useCallback(
    async (providerKeys: string[]): Promise<boolean> => {
      if (!runtimeConfig) return false;
      const deletionPlan = buildDshProviderDeletionPlan(runtimeConfig.providers, providerKeys);
      if (deletionPlan.length === 0) return false;
      setSaving(true);
      try {
        await backupProvidersBeforeDelete(
          deletionPlan,
          ({ provider, credential }) => upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('dsh', provider.providerKey),
            buildDshFavoriteProviderConfig(provider.providerKey, provider.provider ?? {}, credential),
          ),
          ({ provider }) => t('common.batch.backupFailed', { name: provider.displayName || provider.providerKey }),
        );
        let nextConfig = runtimeConfig;
        for (const target of deletionPlan) {
          if (target.deleteCredential) {
            nextConfig = await deleteDshCredential(target.credentialRef);
          }
          if (target.provider.provider) {
            nextConfig = await deleteDshRuntimeProvider(target.provider.providerKey);
          }
          clearBatchDeleteState(target.provider.providerKey);
        }
        setRuntimeConfig(nextConfig);
        setOtherSettings(nextConfig.otherSettings || {});
        await refreshTrayMenu();
        message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete dsh providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig(true);
        return false;
      } finally {
        setSaving(false);
      }
    },
    [runtimeConfig, loadConfig, clearBatchDeleteState, t],
  );

  /** Why a row cannot be picked, for the disabled checkbox's tooltip. */
  const batchBlockReason = React.useCallback(
    (provider: DshRuntimeProviderView): ProviderSelectionBlockReason | undefined => {
      if (provider.isDefault) return 'default';
      if (provider.isBuiltin || provider.modelSource === 'builtin') return 'readOnly';
      if (!provider.credentialExists && !provider.provider) return 'noRecord';
      return undefined;
    },
    [],
  );

  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter(canDeleteDshProvider).map((provider) => provider.providerKey),
    [visibleProviders],
  );

  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });

  const handleOtherSettingsBlur = async (value: unknown, isValid: boolean) => {
    if (!isValid || !otherSettingsValid) {
      message.error(t('dsh.invalidJson', { defaultValue: 'JSON 格式不正确' }));
      return;
    }
    const nextOtherSettings = value && typeof value === 'object' && !Array.isArray(value)
      ? value as Record<string, unknown>
      : {};
    setSaving(true);
    try {
      const nextConfig = await saveDshOtherSettings(nextOtherSettings);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      await refreshTrayMenu();
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save dsh other settings:', error);
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

  const profileOptions = React.useMemo(
    () => [
      {
        value: '',
        // Name the profile auto-detection lands on, but only while it is really
        // auto-detecting; under a pin that name would describe the pin, not
        // what clearing it would pick.
        label:
          !pinnedProfile && runtimeConfig?.profile
            ? t('dsh.profile.auto', {
                defaultValue: 'Auto-detect ({{profile}})',
                profile: runtimeConfig.profile,
              })
            : t('dsh.profile.autoDetecting', { defaultValue: 'Auto-detect' }),
      },
      ...(runtimeConfig?.profiles ?? []).map((profile) => ({
        value: profile,
        label: profile,
      })),
    ],
    [pinnedProfile, runtimeConfig?.profile, runtimeConfig?.profiles, t],
  );

  const handleImportFromAllApiHub = React.useCallback(
    async (imported: AllApiHubProviderItem[]) => {
      const existingKeys = new Set(dshProviders.map((provider) => provider.providerKey));
      let ok = 0;
      let fail = 0;
      for (const item of imported) {
        if (existingKeys.has(item.providerId)) {
          continue;
        }
        const { providerKey, provider, apiKey, credentialRef } = buildDshProviderFromAllApiHub(item);
        try {
          // 先保存 provider route,再保存凭据:若 provider 落盘失败,凭据尚未写入,
          // 不会留下无人引用的孤立 credential(凭据在 UI 上无法单独清理)。
          await saveDshModelsProvider({ providerKey, provider });
          if (apiKey) {
            await saveDshCredential({ refName: credentialRef, value: apiKey });
          }
          void upsertDshFavoriteProvider(providerKey, provider, apiKey ? { refName: credentialRef, value: apiKey } : undefined);
          ok += 1;
        } catch (error) {
          console.error('Failed to import All API Hub provider:', item.name, error);
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

      await loadConfig(true);
      void refreshTrayMenu();
    },
    [dshProviders, loadConfig, t],
  );

  const handleImportFromCcSwitch = async (imported: CcSwitchProviderCandidate[]) => {
    const existingKeys = new Set(dshProviders.map((provider) => provider.providerKey));
    let ok = 0;
    let fail = 0;
    for (const candidate of imported) {
      if (existingKeys.has(candidate.providerId)) {
        continue;
      }
      const mapped = extractDshProviderFromCcSwitch(candidate);
      if (!mapped) {
        continue;
      }
      try {
        // 先保存 provider route,再保存凭据:若 provider 落盘失败,凭据尚未写入,
        // 不会留下无人引用的孤立 credential。
        await saveDshModelsProvider({ providerKey: candidate.providerId, provider: mapped.provider });
        if (mapped.apiKey) {
          await saveDshCredential({ refName: mapped.credentialRef, value: mapped.apiKey });
        }
        void upsertDshFavoriteProvider(
          candidate.providerId,
          mapped.provider,
          mapped.apiKey ? { refName: mapped.credentialRef, value: mapped.apiKey } : undefined,
        );
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

    await loadConfig(true);
    void refreshTrayMenu();
  };

  const handleImportFavoriteProviders = React.useCallback(
    async (providersToImport: OpenCodeFavoriteProvider[]) => {
      const existingKeys = new Set(dshProviders.map((provider) => provider.providerKey));
      let importedCount = 0;
      for (const favoriteProvider of providersToImport) {
        const { providerKey, credential, modelsProvider } = resolveDshFavoriteProviderPayload(favoriteProvider);
        if (!providerKey || existingKeys.has(providerKey)) {
          continue;
        }
        try {
          if (credential?.value) {
            await saveDshCredential({ refName: credential.refName, value: credential.value });
          }
          await saveDshModelsProvider({ providerKey, provider: modelsProvider });
          existingKeys.add(providerKey);
          importedCount += 1;
        } catch (error) {
          console.error('Failed to import DSH favorite provider:', providerKey, error);
        }
      }

      if (importedCount > 0) {
        setImportModalOpen(false);
        message.success(t('opencode.provider.importSuccess', { count: importedCount }));
        await loadConfig(true);
        void refreshTrayMenu();
      }
    },
    [dshProviders, loadConfig, t],
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

  const handleRefreshConfig = () => {
    void loadConfig(true);
    void refreshTrayMenu();
  };

  const handleRefreshModelsCache = async () => {
    setRefreshingModels(true);
    try {
      await fetchRemotePresetModels();
      message.success(t('dsh.modelsRefreshSuccess', { defaultValue: '预设模型已刷新' }));
    } catch (error) {
      console.error('Failed to refresh dsh preset models:', error);
      message.error(t('common.error'));
    } finally {
      setRefreshingModels(false);
    }
  };

  const handleOpenWebUi = async () => {
    try {
      await openDshWebUi();
    } catch {
      setLaunchModalStage('initial');
    }
  };

  const handleLaunchDashboard = async () => {
    const stage = launchModalStage;
    const useNpx = stage === 'npx';
    setLaunchingDashboard(true);
    try {
      await launchDshDashboard(useNpx);
      message.success(useNpx
        ? t('dsh.dashboardLaunchedNpx', { defaultValue: '已通过 npx 启动 DSh Web UI,稍后再次点击"打开 Web UI"' })
        : t('dsh.dashboardLaunched', { defaultValue: 'DSh Web UI 已启动,稍后再次点击"打开 Web UI"' }));
      setLaunchModalStage(null);
    } catch (error) {
      console.error('Failed to launch dsh dashboard:', error);
      if (useNpx) {
        message.error(t('common.error'));
        setLaunchModalStage(null);
      } else {
        // 全局 dsh 缺失等失败:切到 npx 回退态,让用户确认改用 npx 启动。
        setLaunchModalStage('npx');
      }
    } finally {
      setLaunchingDashboard(false);
    }
  };

  /**
   * One mapper for every layout. `ProviderListSection` renders the same node in
   * the list and the grid, so the two cannot offer different actions.
   */
  const renderProviderCard = (provider: DshRuntimeProviderView) => {
    const hasCredential = provider.credentialExists;
    const hasProviderConfig = !!provider.provider;
    const providerConfig = providerRawConfig(provider);
    const isBatchDeleteMode = batchDeleteProviderId === provider.providerKey;
    const selectedModelIds = selectedModelIdsByProvider[provider.providerKey] ?? [];
    const providerBaseUrl = getStringField(providerConfig, 'baseURL');
    const hasModelIds = getDshModelRecords(provider).length > 0;
    // Official llm-pi-ai behavior: a catalog-backed/built-in channel is not
    // removable and owns no route-level protocol or display-name (those come
    // from the catalog). The predicate is shared with the editor modal and the
    // card — see `isCatalogOwnedRoute`.
    const isBuiltInChannel = isCatalogOwnedRoute(provider, false);
    const canDeleteProvider = !isBuiltInChannel && (hasCredential || hasProviderConfig);

    return (
      <DshProviderCard
        key={provider.providerKey}
        provider={provider}
        defaultModelId={provider.isDefault
          ? runtimeConfig?.modelSettings.model ?? undefined
          : undefined}
        viewMode={viewMode}
        selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.providerKey)}
        selected={providerBatch.selectedIds.has(provider.providerKey)}
        onSelectChange={(checked) => providerBatch.toggleSelect(provider.providerKey, checked)}
        selectDisabledReason={providerBatch.selectionMode
          ? providerSelectionDisabledReason(t, batchBlockReason(provider))
          : undefined}
        connectivityStatus={connectivityStatuses[provider.providerKey]}
        // Only a route with its own `providers` entry has a stored position; a
        // route that exists only as the `agent-default-model` pointer is
        // appended by the reader, so its handle would drag nothing.
        dragDisabled={providerBatchDragDisabled || !hasProviderConfig}
        onEdit={() => openProviderModal(provider)}
        onCopy={() => openProviderModal(provider, { copy: true })}
        onShare={() => shareProvider({
          id: provider.providerKey, name: provider.displayName, category: 'custom',
          settingsConfig: JSON.stringify({ ...providerConfig, api: provider.api ?? providerConfig.api,
            models: getDshModelRecords(provider).map((entry) => ({ ...entry.model, id: entry.id })) }),
          credential: provider.apiKey,
          credentialUnavailable: (provider.credentialExists || Boolean(provider.apiKeyEnv)) && !provider.apiKey,
          defaultModel: provider.isDefault ? runtimeConfig?.modelSettings.model ?? undefined : undefined,
        })}
        onDelete={canDeleteProvider ? () => handleDeleteSupplier(provider) : undefined}
        deleteDisabledReason={canDeleteProvider && provider.isDefault
          ? t('dsh.provider.deleteDisabledDefault', { defaultValue: '该渠道已设为默认，不可删除' })
          : undefined}
        onAddModel={() => openDshModelModal(provider)}
        onEditModel={(modelId) => openDshModelModal(provider, modelId)}
        onCopyModel={(modelId) => openDshModelModal(provider, modelId, { copy: true })}
        onDeleteModel={(modelId) => handleDeleteDshModel(provider, modelId)}
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

  // Dragging rewrites the key order of `llm-pi-ai.providers`. Search and
  // non-`custom` sort modes show an order the file does not contain, so a drop
  // there would persist something the user is not looking at.
  const providerDragDisabled = sortMode !== 'custom' || providerKeyword.trim() !== '';
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  const handleProviderDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) {
      return;
    }
    // Only routes that live in the dict have a position; a catalog-backed
    // default route without a provider entry is appended by the reader.
    const providerKeys = dshProviders
      .filter((provider) => Boolean(provider.provider))
      .map((provider) => provider.providerKey);
    const oldIndex = providerKeys.indexOf(active.id as string);
    const newIndex = providerKeys.indexOf(over.id as string);
    if (oldIndex < 0 || newIndex < 0) {
      return;
    }
    const movedKeys = arrayMove(providerKeys, oldIndex, newIndex);
    const previousConfig = runtimeConfig;
    try {
      const nextConfig = await reorderDshProviders(movedKeys);
      setRuntimeConfig(nextConfig);
      setOtherSettings(nextConfig.otherSettings || {});
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to reorder dsh providers:', error);
      setRuntimeConfig(previousConfig);
      message.error(t('common.error'));
    }
  };

  return (
    <Spin spinning={loading}>
      <SectionSidebarLayout
        sidebarTitle={t('dsh.title', { defaultValue: 'DeepSeek Harness' })}
        sidebarHidden={sidebarHidden}
        sections={sidebarSections}
        markerAttr="data-dsh-sidebar-section"
        getIcon={(id) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
      >
        <div className={styles.pageContent}>
          <CodingPageHeader
            title={t('dsh.title', { defaultValue: 'DeepSeek Harness' })}
            docsUrl="https://deepseek-harness.github.io/deepseek-harness/guide/quickstart"
            onPreviewConfig={() => setPreviewModalOpen(true)}
            configPath={runtimeConfig?.rootPathInfo.path ?? ''}
            // The root is the config directory; the active file inside it is
            // `profiles/<profile>/cordis.patch.yml` or legacy `settings.yaml`.
            configPathKind="directory"
            onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
            onOpenFolder={handleOpenRootFolder}
            onRefresh={handleRefreshConfig}
            onMoreOptions={() => setSettingsModalOpen(true)}
            extraActions={(
              <Space size={4} wrap>
                <Button
                  type="text"
                  size="small"
                  icon={<CloudSyncOutlined />}
                  onClick={handleRefreshModelsCache}
                  loading={refreshingModels}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('dsh.syncModels', { defaultValue: '同步模型' })}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<GlobalOutlined />}
                  onClick={handleOpenWebUi}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('dsh.openWebUi', { defaultValue: '打开 Web UI' })}
                </Button>
                {/* Only while a profile layer is actually in use. A profile
                    directory can exist without dsh having migrated (0.1.5
                    provisions `profiles/web` but still reads `settings.yaml`),
                    and pinning one there changes nothing — the backend keeps
                    reading the legacy file. Showing the picker anyway would
                    promise a switch that does not happen. */}
                {runtimeConfig?.profiles.length && runtimeConfig?.profile ? (
                  <>
                    <Text type="secondary" style={{ fontSize: 12, marginLeft: 4 }}>
                      {t('dsh.profile.label', { defaultValue: '配置 Profile' })}:
                    </Text>
                    <Select
                      size="small"
                      value={pinnedProfile ?? ''}
                      options={profileOptions}
                      onChange={(value) => {
                        void handleSelectProfile(value ? value : null);
                      }}
                      style={{ minWidth: 180 }}
                    />
                  </>
                ) : null}
              </Space>
            )}
          />

          {/* The notice owns its own dismissal state, so the page cannot show
              it for a path the user already closed. It is a one-time event:
              a banner that stays forever stops being information and becomes
              furniture. */}
          {runtimeConfig?.migratedFromSettings ? (
            <DshMigrationNotice
              patchPath={runtimeConfig.configPath}
              importedPath={`${runtimeConfig.settingsYamlPath}.imported`}
            />
          ) : null}

          <div
            id="dsh-model-settings"
            className={styles.dshSection}
            data-dsh-sidebar-section="true"
            data-sidebar-title={t('dsh.modelSettings.title', { defaultValue: '默认模型' })}
          >
            <div className={styles.modelCard}>
              <Title level={5} className={styles.modelCardTitle}>
                <RobotOutlined style={{ marginRight: 8 }} />
                {t('dsh.modelSettings.title', { defaultValue: '默认模型' })}
              </Title>
              <div className={styles.modelCardContent}>
                <Form
                  form={modelForm}
                  layout="vertical"
                  onValuesChange={handleModelSettingsChange}
                >
                  <div className={styles.modelSettingsGrid}>
                    <Form.Item label={t('dsh.modelSettings.defaultProvider', { defaultValue: '默认供应商' })} name="defaultProvider">
                      <Select
                        allowClear
                        showSearch
                        options={providerOptions}
                        placeholder={t('dsh.modelSettings.defaultProviderPlaceholder', { defaultValue: '选择默认供应商' })}
                      />
                    </Form.Item>
                    <Form.Item label={t('dsh.modelSettings.defaultModel', { defaultValue: '默认模型' })} name="defaultModel">
                      <Select
                        allowClear
                        showSearch
                        options={modelOptions}
                        placeholder={t('dsh.modelSettings.defaultModelPlaceholder', { defaultValue: '选择默认模型' })}
                      />
                    </Form.Item>
                    <Form.Item label={t('dsh.modelSettings.reasoningEffort', { defaultValue: '推理强度' })} name="defaultReasoningEffort">
                      <Select
                        allowClear
                        options={reasoningEffortOptions}
                        placeholder={t('dsh.modelSettings.reasoningEffortPlaceholder', { defaultValue: '选择推理强度' })}
                      />
                    </Form.Item>
                  </div>
                </Form>
              </div>
            </div>
          </div>

          <ProviderListSection
            sectionId="dsh-providers"
            collapsed={providerListCollapsed}
            onCollapsedChange={setProviderListCollapsed}
            loading={loading}
            providerCount={dshProviders.length}
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
            hint={(
              <div>
                {t('dsh.pageHint', {
                  defaultValue: '管理 dsh（DeepSeek Harness）的模型供应商、默认模型与全局提示词。',
                })}
              </div>
            )}
            footer={(
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
            )}
          >
            {/* One `DndContext` serves both layouts; only the sorting strategy
                and the drag modifier change with the view. The order it writes
                is `llm-pi-ai.providers`' key order, which is also what
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
            id="dsh-global-prompt"
            className={`${styles.dshSection} ${styles.promptSection}`}
            data-dsh-sidebar-section="true"
            data-sidebar-title={t('common.prompt.title')}
          >
            {!agentInstructionsEnabled ? (
              <Alert
                type="warning"
                showIcon
                style={{ marginBottom: 12 }}
                title={t('dsh.agentInstructions.disabledWarning')}
                // One fact, one key, two render positions: the scope note also
                // shows below when the toggle is already on, because a user who
                // enabled it still needs to know the Web surface is not covered.
                description={t('dsh.agentInstructions.scopeNote')}
                action={
                  <Button
                    size="small"
                    type="primary"
                    loading={enablingAgentInstructions}
                    onClick={handleEnableAgentInstructions}
                  >
                    {t('dsh.agentInstructions.enable')}
                  </Button>
                }
              />
            ) : (
              <div className={styles.agentInstructionsScope}>
                {t('dsh.agentInstructions.scopeNote')}
              </div>
            )}
            <GlobalPromptSettings
              toolName="dsh"
              promptFileName="AGENTS.md"
              service={dshPromptApi}
              collapseKey="dsh-prompt"
              onUpdated={async () => {
                await loadConfig(true);
                await refreshTrayMenu();
              }}
            />
          </div>

          <div
            id="dsh-other-configuration"
            className={styles.dshSection}
            data-dsh-sidebar-section="true"
            data-sidebar-title={t('dsh.otherConfig.title', { defaultValue: '其他配置' })}
          >
            <Collapse
              className={styles.collapseCard}
              items={[
                {
                  key: 'other',
                  label: (
                    <Space>
                      <SettingOutlined />
                      <Text strong>{t('dsh.otherConfig.title', { defaultValue: '其他配置' })}</Text>
                    </Space>
                  ),
                  children: runtimeConfig?.profile ? (
                    // The profile patch carries every dsh plugin row; a free-form
                    // JSON editor would let a rewrite drop unrelated plugins.
                    <Alert
                      type="info"
                      showIcon
                      title={t('dsh.profile.otherSettingsFrozenTitle', {
                        defaultValue: '其他配置已由 dsh 自身管理',
                      })}
                      description={t('dsh.profile.otherSettingsFrozenDesc', {
                        defaultValue:
                          'profile 补丁文件里是 dsh 全部插件的配置行，直接编辑容易破坏其它插件。请使用 dsh 设置页（设置 → 插件）维护这些配置；本页只管理供应商与默认模型。',
                      })}
                    />
                  ) : (
                    <Form.Item
                      help={
                        <span>
                          <Text type="secondary">{t('dsh.otherConfig.hint', { defaultValue: '以 JSON 维护 dsh 的其他配置项，失焦自动保存' })}，</Text>
                          <span style={{ color: 'var(--ant-color-primary)' }}>
                            {t('dsh.otherConfig.autoSaveHint', { defaultValue: '自动保存' })}
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
            id="dsh-session-manager"
            className={styles.dshSection}
            data-dsh-sidebar-section="true"
            data-sidebar-title={t('sessionManager.title')}
          >
            <SessionManagerPanel tool="dsh" />
          </div>
        </div>

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(runtimeConfig?.rootPathInfo || null)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        <DshProviderFormModal
          open={!!providerModal}
          provider={providerModal?.provider}
          copy={providerModal?.copy}
          saving={saving}
          onCancel={() => setProviderModal(null)}
          onSubmit={(draft) => void handleSaveProviderModal(draft)}
        />

        <Modal
          title={t('dsh.provider.deleteScopeModalTitle', { defaultValue: '删除供应商' })}
          open={!!deleteScopeProvider}
          onCancel={() => setDeleteScopeProvider(null)}
          footer={deleteScopeProvider ? [
            <Button key="cancel" onClick={() => setDeleteScopeProvider(null)}>
              {t('common.cancel')}
            </Button>,
            <Button
              key="both"
              danger
              type="primary"
              onClick={() => handleDeleteScopeSelect('both')}
            >
              {t('dsh.provider.confirmDelete', { defaultValue: '确认删除' })}
            </Button>,
          ] : null}
          destroyOnHidden
        >
          <Text>
            {t('dsh.provider.deleteScopeModalContent', {
              defaultValue: '该供应商同时包含供应商配置与 API 凭证，删除后将一并清除且无法恢复。确定删除供应商 {{providerKey}} 吗？',
              providerKey: deleteScopeProvider?.providerKey,
            })}
          </Text>
        </Modal>

        <ModelFormModal
          open={!!dshModelModal}
          width={700}
          isEdit={!!dshModelModal?.modelId}
          initialValues={dshModelModal ? {
            id: dshModelModal.modelId ?? getStringField(dshModelModal.model ?? {}, 'id'),
            name: getStringField(dshModelModal.model ?? {}, 'name'),
            reasoning: typeof dshModelModal.model?.reasoning === 'boolean'
              ? dshModelModal.model.reasoning
              : undefined,
            contextLimit: typeof dshModelModal.model?.contextWindow === 'number'
              ? dshModelModal.model.contextWindow
              : undefined,
            outputLimit: typeof dshModelModal.model?.maxTokens === 'number'
              ? dshModelModal.model.maxTokens
              : undefined,
            inputTypes: stringifyDshInputTypes(dshModelModal.model?.input),
            compat: stringifyDshRecord(dshModelModal.model?.compat),
            costInput: getNumberField(asRecord(dshModelModal.model?.cost), 'input'),
            costOutput: getNumberField(asRecord(dshModelModal.model?.cost), 'output'),
            costCacheRead: getNumberField(asRecord(dshModelModal.model?.cost), 'cacheRead'),
            costCacheWrite: getNumberField(asRecord(dshModelModal.model?.cost), 'cacheWrite'),
            extraParams: extractDshExtraParams(dshModelModal.model),
            thinkingLevelMap: dshModelModal.model?.reasoningEfforts && typeof dshModelModal.model.reasoningEfforts === 'object'
              ? JSON.stringify(
                  Object.fromEntries(
                    Object.entries(dshModelModal.model.reasoningEfforts as Record<string, unknown>)
                      .filter(([, value]) => value !== null && value !== undefined && value !== ''),
                  ),
                  null,
                  2,
                )
              : undefined,
          } : undefined}
          existingIds={dshModelModal && !dshModelModal.modelId
            ? getDshModelRecords(dshModelModal.provider).map((entry) => entry.id)
            : []}
          npmType={dshModelModal
            ? dshApiToSdkName(getStringField(providerRawConfig(dshModelModal.provider), 'api'))
            : undefined}
          showOptions={false}
          showVariants={false}
          showModalities={false}
          showInputTypes
          showReasoning={false}
          showCost={false}
          showThinkingLevelMap={true}
          showCompat
          showExtraParams
          limitRequired={false}
          nameRequired={false}
          messageOverrides={{
            idPlaceholder: t('dsh.model.idPlaceholder'),
            extraParamsHint: t('dsh.model.extraParamsHint'),
            thinkingLevelMapHint: t('dsh.model.thinkingLevelMapHint'),
          }}
          onCancel={() => setDshModelModal(null)}
          onSuccess={handleSaveDshModel}
          onDuplicateId={() => message.error(t('dsh.model.idExists', { defaultValue: '模型 ID 已存在' }))}
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

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
          removableModelIds={connectivityInfo?.modelIds}
          onRemoveModels={handleRemoveConnectivityModels}
          onCancel={() => setConnectivityModalOpen(false)}
        />

        <ImportProviderModal
          open={importModalOpen}
          onClose={() => setImportModalOpen(false)}
          onImport={handleImportFavoriteProviders}
          toolName="dsh"
          existingProviderIds={existingFavoriteProviderIds}
          providerFilter={(provider) => isFavoriteProviderForSource('dsh', provider)}
        />

        {allApiHubAvailable && (
          <ImportFromAllApiHubModalForTool
            open={allApiHubImportModalOpen}
            existingProviderIds={dshProviders.map((provider) => provider.providerKey)}
            onCancel={() => setAllApiHubImportModalOpen(false)}
            onImport={handleImportFromAllApiHub}
            listProviders={listDshAllApiHubProviders}
            resolveProviders={resolveDshAllApiHubProviders}
          />
        )}

        {ccSwitchAvailable && (
          <ImportFromCcSwitchModal
            open={ccSwitchImportModalOpen}
            appType="claude"
            existingProviderIds={dshProviders.map((provider) => provider.providerKey)}
            onClose={() => setCcSwitchImportModalOpen(false)}
            onImport={handleImportFromCcSwitch}
          />
        )}

        <DshConfigPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('dsh.preview.title', { defaultValue: '配置预览' })}
          data={runtimeConfig}
        />

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={async (visible) => {
            await setSidebarHidden('dsh', !visible);
          }}
        >
          <CliManualPathSetting commandName="dsh" labelKey="subModules.dsh" toolNameKey="subModules.dshFull" />
        </SidebarSettingsModal>

        <Modal
          title={launchModalStage === 'npx'
            ? t('dsh.openWebUi', { defaultValue: '打开 Web UI' })
            : t('dsh.openWebUi', { defaultValue: '打开 Web UI' })}
          open={launchModalStage !== null}
          confirmLoading={launchingDashboard}
          okText={launchModalStage === 'npx'
            ? t('dsh.launchWithNpx', { defaultValue: '使用 npx 启动' })
            : t('dsh.launchDashboard', { defaultValue: '启动 dsh web' })}
          cancelText={t('common.cancel')}
          onCancel={() => setLaunchModalStage(null)}
          onOk={handleLaunchDashboard}
          destroyOnHidden
        >
          {launchModalStage === 'npx'
            ? t('dsh.useNpxConfirm', {
                defaultValue: '未检测到 dsh CLI,是否使用 `npx @deepseek-ai/dsh` 启动?',
              })
            : t('dsh.openWebUiOffline', {
                defaultValue: 'DSh Web UI 未运行。启动 dsh web 服务后,稍后再次点击"打开 Web UI"。',
              })}
        </Modal>

        {shareModal}
      </SectionSidebarLayout>
    </Spin>
  );
};

export default DshPage;
