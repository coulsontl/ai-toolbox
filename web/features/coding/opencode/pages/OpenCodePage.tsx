import React from 'react';
import { Button, Empty, Space, Typography, message, Spin, Select, Collapse, Form, Tooltip, Modal, Switch } from 'antd';
import {
  PlusOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  EyeOutlined,
  EllipsisOutlined,
  EditOutlined,
  CloudDownloadOutlined,
  CloudSyncOutlined,
  ReloadOutlined,
  FileOutlined,
  ImportOutlined,
  ApiOutlined,
  DeleteOutlined,
  SafetyCertificateOutlined,
  RobotOutlined,
  ToolOutlined,
  DatabaseOutlined,
  ThunderboltOutlined,
  FileTextOutlined,
  MessageOutlined,
} from '@ant-design/icons';

import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import { readOpenCodeConfigWithResult, saveOpenCodeConfig, getOpenCodeConfigPathInfo, getOpenCodeUnifiedModels, getOpenCodeAuthProviders, getOpenCodeAuthConfigPath, listFavoriteProviders, upsertFavoriteProvider, deleteFavoriteProvider, buildModelVariantsMap, getOpenCodeFreeModels, type ConfigPathInfo, type UnifiedModelOption, type GetAuthProvidersResponse, type OpenCodeFavoriteProvider, type OpenCodeDiagnosticsConfig } from '@/services/opencodeApi';
import { listOhMyOpenAgentConfigs, applyOhMyOpenAgentConfig } from '@/services/ohMyOpenAgentApi';
import { listOhMyOpenCodeSlimConfigs } from '@/services/ohMyOpenCodeSlimApi';
import { refreshTrayMenu, fetchRemotePresetModels, hasAllApiHubExtension } from '@/services/appApi';
import type { OpenCodeConfig, OpenCodeModel, OpenCodePluginEntry, OpenCodeProvider } from '@/types/opencode';
import {
  PRESET_MODELS,
  findPresetModelById,
  getPresetModelsVersion,
  subscribePresetModels,
  type PresetModel,
} from '@/constants/presetModels';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import type {
  ProviderDisplayData,
  ModelDisplayData,
  OfficialModelDisplayData,
  ProviderConnectivityStatusItem,
} from '@/components/common/ProviderCard/types';
import ProviderCard from '@/components/common/ProviderCard';
import OfficialProviderCard from '@/components/common/OfficialProviderCard';
import ProviderFormModal from '@/components/common/ProviderFormModal';
import type { ProviderFormValues } from '@/components/common/ProviderFormModal';
import ModelFormModal from '@/components/common/ModelFormModal';
import type { ModelFormValues } from '@/components/common/ModelFormModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import type { FetchModelsApplyResult, FetchedModel } from '@/components/common/FetchModelsModal/types';
import PluginSettings from '../components/PluginSettings';
import ConfigPathModal from '../components/ConfigPathModal';
import ConfigParseErrorAlert from '../components/ConfigParseErrorAlert';
import OhMyOpenAgentConfigSelector from '../components/OhMyOpenAgentConfigSelector';
import OhMyOpenCodeSlimConfigSelector from '../components/OhMyOpenCodeSlimConfigSelector';
import OhMyOpenAgentSettings from '../components/OhMyOpenAgentSettings';
import OhMyOpenCodeSlimSettings from '../components/OhMyOpenCodeSlimSettings';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import JsonEditor from '@/components/common/JsonEditor';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import ConnectivityTestModal from '../components/ConnectivityTestModal';
import { useRefreshStore } from '@/stores';
import { useSettingsStore } from '@/stores';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import { openCodePromptApi } from '@/services/openCodePromptApi';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import {
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import {
  buildFavoriteProviderStorageKey,
  dedupeOpenCodeFavoriteProviders,
  extractFavoriteProviderRawId,
  findDefaultTestModelIdForProvider,
  isFavoriteProviderForSource,
  needsFavoriteProviderMigration,
} from '@/features/coding/shared/favoriteProviders';
import {
  getOpenCodePluginPackageName,
  getOpenCodePluginName,
  sanitizeOpenCodePluginList,
} from '@/features/coding/opencode/utils/pluginNames';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';

import styles from './OpenCodePage.module.less';

const { Title, Text, Link } = Typography;

const isOhMyOpenAgentPlugin = (pluginName: string): boolean => {
  const baseName = getOpenCodePluginPackageName(pluginName);
  if (baseName.includes('oh-my-opencode-slim')) {
    return false;
  }
  return baseName.includes('oh-my-openagent') || baseName.includes('oh-my-opencode');
};

const buildManagedProviderOptions = (values: ProviderFormValues): OpenCodeProvider['options'] => {
  const nextOptions: Record<string, unknown> = {
    ...(values.extraOptions ?? {}),
  };

  if (values.baseUrl) {
    nextOptions.baseURL = values.baseUrl;
  }
  if (values.apiKey) {
    nextOptions.apiKey = values.apiKey;
  }
  if (values.headers && typeof values.headers === 'object' && Object.keys(values.headers).length > 0) {
    nextOptions.headers = values.headers;
  }
  if (values.disableTimeout) {
    nextOptions.timeout = false;
  } else if (values.timeout !== undefined) {
    nextOptions.timeout = values.timeout;
  }
  if (values.setCacheKey !== undefined) {
    nextOptions.setCacheKey = values.setCacheKey;
  }

  return Object.keys(nextOptions).length > 0
    ? nextOptions as OpenCodeProvider['options']
    : undefined;
};

const parseOptionalJsonValue = <T extends Record<string, unknown>>(jsonText?: string): T | undefined => {
  if (!jsonText) {
    return undefined;
  }

  const parsedValue = JSON.parse(jsonText) as T | null;
  if (!parsedValue) {
    return undefined;
  }

  return Object.keys(parsedValue).length > 0 ? parsedValue : undefined;
};

const buildManagedModelLimit = (
  baseModel: OpenCodeModel | null,
  values: ModelFormValues,
): OpenCodeModel['limit'] => {
  const nextLimit: OpenCodeModel['limit'] = {
    ...(baseModel?.limit ?? {}),
    ...(values.contextLimit !== undefined ? { context: values.contextLimit } : { context: undefined }),
    ...(values.outputLimit !== undefined ? { output: values.outputLimit } : { output: undefined }),
  };

  return nextLimit && Object.values(nextLimit).some((value) => value !== undefined)
    ? nextLimit
    : undefined;
};

// Helper function to convert OpenCodeProvider to ProviderDisplayData
const toProviderDisplayData = (id: string, provider: OpenCodeProvider): ProviderDisplayData => ({
  id,
  name: provider.name || id,
  sdkName: provider.npm || '@ai-sdk/openai-compatible',
  baseUrl: provider.options?.baseURL || '',
});

// Helper function to convert OpenCodeModel to ModelDisplayData
const toModelDisplayData = (id: string, model: OpenCodeModel): ModelDisplayData => ({
  id,
  name: model.name || id,
  contextLimit: model.limit?.context,
  outputLimit: model.limit?.output,
});

// Helper function to reorder object entries and return a new object
const reorderObject = <T,>(obj: Record<string, T>, newOrder: string[]): Record<string, T> => {
  const result: Record<string, T> = {};
  for (const key of newOrder) {
    if (obj[key]) {
      result[key] = obj[key];
    }
  }
  return result;
};

const buildUnifiedModelId = (providerId: string, modelId: string): string => `${providerId}/${modelId}`;

const sanitizeAgentsModelReferences = (
  agents: unknown,
  removedModelIdSet: Set<string>,
): unknown => {
  if (!agents || typeof agents !== 'object' || Array.isArray(agents)) {
    return agents;
  }

  let mutated = false;
  const nextAgents: Record<string, unknown> = {};

  for (const [agentName, agent] of Object.entries(agents as Record<string, unknown>)) {
    if (
      agent
      && typeof agent === 'object'
      && !Array.isArray(agent)
      && typeof (agent as Record<string, unknown>).model === 'string'
      && removedModelIdSet.has((agent as Record<string, string>).model)
    ) {
      const { model: _removedAgentModel, ...remainingAgent } = agent as Record<string, unknown>;
      nextAgents[agentName] = remainingAgent;
      mutated = true;
    } else {
      nextAgents[agentName] = agent;
    }
  }

  return mutated ? nextAgents : agents;
};

const SUPPORTED_PROVIDER_NPMS = new Set([
  '@ai-sdk/openai',
  '@ai-sdk/openai-compatible',
  '@ai-sdk/google',
  '@ai-sdk/anthropic',
]);

const OPENAI_COMPATIBLE_NPM = '@ai-sdk/openai-compatible';

const getFetchedModelDefaultModalities = (providerNpm?: string): OpenCodeModel['modalities'] => {
  const defaultInputModalities = providerNpm === OPENAI_COMPATIBLE_NPM ? ['text'] : ['text', 'image'];
  return {
    input: defaultInputModalities,
    output: ['text'],
  };
};

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'opencode-model-settings': <RobotOutlined />,
  'opencode-plugin-configuration': <ApiOutlined />,
  'opencode-omo-configuration': <ThunderboltOutlined />,
  'opencode-omo-slim-configuration': <ThunderboltOutlined />,
  'opencode-providers': <DatabaseOutlined />,
  'opencode-global-prompt': <FileTextOutlined />,
  'opencode-official-auth-channels': <SafetyCertificateOutlined />,
  'opencode-other-configuration': <ToolOutlined />,
  'opencode-session-manager': <MessageOutlined />,
};

const buildOpenCodeModelFromPreset = (preset: PresetModel, fallbackName: string): OpenCodeModel => ({
  name: preset.name || fallbackName,
  ...(preset.contextLimit || preset.outputLimit
    ? {
      limit: {
        ...(preset.contextLimit ? { context: preset.contextLimit } : {}),
        ...(preset.outputLimit ? { output: preset.outputLimit } : {}),
      },
    }
    : {}),
  ...(preset.modalities ? { modalities: preset.modalities } : {}),
  ...(preset.reasoning !== undefined ? { reasoning: preset.reasoning } : {}),
  ...(preset.attachment !== undefined ? { attachment: preset.attachment } : {}),
  ...(preset.tool_call !== undefined ? { tool_call: preset.tool_call } : {}),
  ...(preset.temperature !== undefined ? { temperature: preset.temperature } : {}),
  ...(preset.options && Object.keys(preset.options).length > 0 ? { options: preset.options } : {}),
  ...(preset.variants && Object.keys(preset.variants).length > 0 ? { variants: preset.variants } : {}),
});

const buildFetchedOpenCodeModel = (
  fetchedModel: FetchedModel,
  providerNpm?: string,
): OpenCodeModel => {
  const matchedPresetModel = findPresetModelById(fetchedModel.id, providerNpm);

  if (matchedPresetModel) {
    return buildOpenCodeModelFromPreset(matchedPresetModel, fetchedModel.name || fetchedModel.id);
  }

  return {
    name: fetchedModel.name || fetchedModel.id,
    modalities: getFetchedModelDefaultModalities(providerNpm),
    reasoning: true,
    tool_call: true,
  };
};

const OpenCodePage: React.FC = () => {
  const { t } = useTranslation();
  const { openCodeConfigRefreshKey, omoConfigRefreshKey, omosConfigRefreshKey, incrementOpenCodeConfigRefresh, incrementOmoConfigRefresh, incrementOmosConfigRefresh } = useRefreshStore();
  const {
    sidebarHiddenByPage,
    setSidebarHidden,
    opencodeAllowClearAppliedOhMyConfig,
    setOpencodeAllowClearAppliedOhMyConfig,
  } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [config, setConfig] = React.useState<OpenCodeConfig | null>(null);
  const [configPathInfo, setConfigPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [parseError, setParseError] = React.useState<{
    path: string;
    error: string;
    contentPreview?: string;
  } | null>(null);

  // Preview modal state
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<unknown>(null);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const sidebarHidden = sidebarHiddenByPage.opencode;

  // Provider modal state
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [currentProviderId, setCurrentProviderId] = React.useState<string>('');
  const [providerInitialValues, setProviderInitialValues] = React.useState<Partial<ProviderFormValues> | undefined>();
  const [baseProviderSnapshot, setBaseProviderSnapshot] = React.useState<OpenCodeProvider | null>(null);

  // Model modal state
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [currentModelProviderId, setCurrentModelProviderId] = React.useState<string>('');
  const [currentModelId, setCurrentModelId] = React.useState<string>('');
  const [modelInitialValues, setModelInitialValues] = React.useState<Partial<ModelFormValues> | undefined>();
  const [baseModelSnapshot, setBaseModelSnapshot] = React.useState<OpenCodeModel | null>(null);

  // Fetch models modal state
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string>('');

  // Import provider modal state
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);

  const [favoriteProviders, setFavoriteProviders] = React.useState<OpenCodeFavoriteProvider[]>([]);
  const [batchDeleteProviderId, setBatchDeleteProviderId] = React.useState<string | null>(null);
  const [selectedModelIdsByProvider, setSelectedModelIdsByProvider] = React.useState<Record<string, string[]>>({});

  // Connectivity test modal state
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityProviderId, setConnectivityProviderId] = React.useState<string>('');
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);


  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [officialProvidersCollapsed, setOfficialProvidersCollapsed] = React.useState(false);
  const [pathModalOpen, setPathModalOpen] = React.useState(false);
  const [otherConfigCollapsed, setOtherConfigCollapsed] = React.useState(true);
  const [unifiedModels, setUnifiedModels] = React.useState<UnifiedModelOption[]>([]);
  const [authProvidersData, setAuthProvidersData] = React.useState<GetAuthProvidersResponse | null>(null);
  const [authConfigPath, setAuthConfigPath] = React.useState<string>('');
  const resolvedAuthProviderIds = React.useMemo(
    () => new Set(authProvidersData?.resolvedAuthProviderIds ?? []),
    [authProvidersData],
  );

  // Use ref for validation state to avoid re-renders during editing
  const otherConfigJsonValidRef = React.useRef(true);
  const [pluginExpandNonce, setPluginExpandNonce] = React.useState(0);
  const [omoSettingsExpandNonce, setOmoSettingsExpandNonce] = React.useState(0);
  const [omoSlimSettingsExpandNonce, setOmoSlimSettingsExpandNonce] = React.useState(0);
  const [globalPromptExpandNonce, setGlobalPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);

  const [ohMyOpenAgentRefreshKey, setOhMyOpenAgentRefreshKey] = React.useState(0); // 用于触发 OhMyOpenAgentConfigSelector 刷新
  const [ohMyOpenAgentSettingsRefreshKey, setOhMyOpenAgentSettingsRefreshKey] = React.useState(0); // 用于触发 OhMyOpenAgentSettings 刷新
  const [omoConfigs, setOmoConfigs] = React.useState<Array<{ id: string; name: string; isApplied?: boolean }>>([]); // omo 配置列表
  const [omoSlimConfigs, setOmoSlimConfigs] = React.useState<Array<{ id: string; name: string; isApplied?: boolean }>>([]); // omo slim 配置列表

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const handleSidebarSelect = React.useCallback((id: string) => {
    // Keep expand logic local to this page.
    switch (id) {
      case 'opencode-providers':
        setProviderListCollapsed(false);
        break;
      case 'opencode-official-auth-channels':
        setOfficialProvidersCollapsed(false);
        break;
      case 'opencode-other-configuration':
        setOtherConfigCollapsed(false);
        break;
      case 'opencode-plugin-configuration':
        setPluginExpandNonce((v) => v + 1);
        break;
      case 'opencode-omo-configuration':
        setOmoSettingsExpandNonce((v) => v + 1);
        break;
      case 'opencode-omo-slim-configuration':
        setOmoSlimSettingsExpandNonce((v) => v + 1);
        break;
      case 'opencode-global-prompt':
        setGlobalPromptExpandNonce((v) => v + 1);
        break;
      case 'opencode-session-manager':
        setSessionManagerExpandNonce((value) => value + 1);
        break;
      default:
        break;
    }
  }, []);

  const loadConfig = React.useCallback(async (showSuccessMessage = false, silent = false) => {
    setLoading(true);
    setParseError(null);

    try {
      const pathInfo = await getOpenCodeConfigPathInfo();
      setConfigPathInfo(pathInfo);

      const result = await readOpenCodeConfigWithResult();

      switch (result.status) {
        case 'success':
          setConfig({
            ...result.config,
            plugin: result.config.plugin
              ? sanitizeOpenCodePluginList(result.config.plugin)
              : undefined,
          });
          if (showSuccessMessage) {
            message.success(t('opencode.refreshSuccess'));
          }
          break;

        case 'notFound':
          setConfig({
            $schema: 'https://opencode.ai/config.json',
            provider: {},
          });
          if (showSuccessMessage) {
            message.success(t('opencode.refreshSuccess'));
          }
          break;

        case 'parseError':
          setParseError({
            path: result.path,
            error: result.error,
            contentPreview: result.contentPreview,
          });
          setConfig({
            $schema: 'https://opencode.ai/config.json',
            provider: {},
          });
          break;

        case 'error':
          if (!silent) {
            message.error(result.error);
          }
          setConfig({
            $schema: 'https://opencode.ai/config.json',
            provider: {},
          });
          break;
      }
    } catch (error: unknown) {
      console.error('Failed to load config:', error);
      if (!silent) {
        const errorMessage = error instanceof Error ? error.message : t('common.error');
        message.error(errorMessage);
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    loadConfig();
    // Biome: make the dependency explicit for re-running on refresh key changes
    void openCodeConfigRefreshKey;
  }, [loadConfig, openCodeConfigRefreshKey]);

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
    // Biome: make the dependency explicit
    void openCodeConfigRefreshKey;
  }, [openCodeConfigRefreshKey]);

  // Reload config when MCP changes (from tray menu or MCP page)
  React.useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      unlisten = await listen('mcp-changed', () => {
        loadConfig(false, true);
      });
    };
    setup();
    return () => { unlisten?.(); };
  }, [loadConfig]);

  // Reload config when tray menu changes the active OpenCode config.
  React.useEffect(() => {
    const handleTrayConfigRefresh = (event: Event) => {
      event.preventDefault();
      void loadConfig(false, true);
    };

    window.addEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    return () => {
      window.removeEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    };
  }, [loadConfig]);

  // Check if the Oh My OpenAgent plugin is enabled.
  const omoPluginEnabled = config?.plugin?.some((p) => {
    return isOhMyOpenAgentPlugin(getOpenCodePluginName(p));
  }) ?? false;

  // Check if oh-my-opencode-slim plugin is enabled (use contains matching for fork versions)
  const omoSlimPluginEnabled = config?.plugin?.some((p) => {
    const baseName = getOpenCodePluginPackageName(getOpenCodePluginName(p));
    return baseName.includes('oh-my-opencode-slim');
  }) ?? false;

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => {
    const sections: SidebarSectionMarker[] = [
      {
        id: 'opencode-model-settings',
        title: t('opencode.modelSettings.title'),
        order: 1,
      },
      {
        id: 'opencode-plugin-configuration',
        title: t('opencode.plugin.title'),
        order: 2,
      },
    ];

    if (omoPluginEnabled || omoConfigs.length > 0) {
      sections.push({
        id: 'opencode-omo-configuration',
        title: t('opencode.ohMyOpenCode.title'),
        order: 3,
      });
    }

    if (omoSlimPluginEnabled || omoSlimConfigs.length > 0) {
      sections.push({
        id: 'opencode-omo-slim-configuration',
        title: t('opencode.ohMyOpenCodeSlim.title'),
        order: 4,
      });
    }

    sections.push(
      {
        id: 'opencode-providers',
        title: t('opencode.provider.title'),
        order: 5,
      },
      {
        id: 'opencode-official-auth-channels',
        title: t('opencode.official.title'),
        order: 6,
      },
      {
        id: 'opencode-global-prompt',
        title: t('opencode.prompt.title'),
        order: 7,
      },
      {
        id: 'opencode-other-configuration',
        title: t('opencode.otherConfig.title'),
        order: 8,
      },
      {
        id: 'opencode-session-manager',
        title: t('sessionManager.title'),
        order: 9,
      },
    );

    return sections;
  }, [
    omoConfigs.length,
    omoPluginEnabled,
    omoSlimConfigs.length,
    omoSlimPluginEnabled,
    t,
  ]);

  // Load omo config list
  React.useEffect(() => {
    // Biome: make the dependencies explicit
    void openCodeConfigRefreshKey;
    void omoConfigRefreshKey;
    void ohMyOpenAgentSettingsRefreshKey;
    const loadOmoConfigs = async () => {
      try {
        const configs = await listOhMyOpenAgentConfigs();
        setOmoConfigs(configs.map(c => ({ id: c.id, name: c.name, isApplied: c.isApplied })));
      } catch (error) {
        console.error('Failed to load omo configs:', error);
        setOmoConfigs([]);
      }
    };
    loadOmoConfigs();
  }, [openCodeConfigRefreshKey, omoConfigRefreshKey, ohMyOpenAgentSettingsRefreshKey]);

  // Load omo slim config list (used for visibility/filtering)
  React.useEffect(() => {
    // Biome: make the dependencies explicit
    void omosConfigRefreshKey;
    const loadOmoSlimConfigs = async () => {
      try {
        const configs = await listOhMyOpenCodeSlimConfigs();
        setOmoSlimConfigs(configs.map((c) => ({ id: c.id, name: c.name, isApplied: c.isApplied })));
      } catch (error) {
        console.error('Failed to load omo slim configs:', error);
        setOmoSlimConfigs([]);
      }
    };

    loadOmoSlimConfigs();
  }, [omosConfigRefreshKey]);

  // Auto-apply the applied config when plugin is enabled
  const prevOmoPluginEnabledRef = React.useRef(omoPluginEnabled);
  React.useEffect(() => {
    const autoApplyConfig = async () => {
      // Only auto-apply when plugin changes from disabled to enabled
      if (!prevOmoPluginEnabledRef.current && omoPluginEnabled && omoConfigs.length > 0) {
        // Find the applied config
        const appliedConfig = omoConfigs.find((c) => c.isApplied);
        if (appliedConfig) {
          try {
            await applyOhMyOpenAgentConfig(appliedConfig.id);
            console.log('Auto-applied omo config:', appliedConfig.name);
          } catch (error) {
            console.error('Failed to auto-apply omo config:', error);
          }
        }
      }
      prevOmoPluginEnabledRef.current = omoPluginEnabled;
    };
    autoApplyConfig();
  }, [omoPluginEnabled, omoConfigs]);

  // Load unified models (combining custom providers and official auth providers)
  React.useEffect(() => {
    // Biome: make the dependency explicit
    void openCodeConfigRefreshKey;
    const loadUnifiedModels = async () => {
      try {
        const models = await getOpenCodeUnifiedModels();
        setUnifiedModels(models);
      } catch (error) {
        console.error('Failed to load unified models:', error);
      }
    };

    loadUnifiedModels();
  }, [openCodeConfigRefreshKey]);

  const providerFilterOptions = React.useMemo(() => {
    if (!config || !currentProviderId) return [];
    const provider = config.provider[currentProviderId];
    if (!provider) return [];

    const modelMap = new Map<string, string>();

    if (provider.models) {
      Object.entries(provider.models).forEach(([id, model]) => {
        modelMap.set(id, model.name || id);
      });
    }

    const officialModels = authProvidersData?.mergedModels?.[currentProviderId] || [];
    officialModels.forEach((model) => {
      if (!modelMap.has(model.id)) {
        modelMap.set(model.id, model.name || model.id);
      }
    });

    return Array.from(modelMap.entries()).map(([id, name]) => ({
      label: `${name} (${id})`,
      value: id,
    }));
  }, [config, currentProviderId, authProvidersData]);

  // Load official auth providers data
  React.useEffect(() => {
    // Biome: make the dependency explicit
    void openCodeConfigRefreshKey;
    const loadAuthProviders = async () => {
      try {
        const data = await getOpenCodeAuthProviders();
        setAuthProvidersData(data);
      } catch (error) {
        console.error('Failed to load auth providers:', error);
      }
    };

    const loadAuthConfigPath = async () => {
      try {
        const path = await getOpenCodeAuthConfigPath();
        setAuthConfigPath(path);
      } catch (error) {
        console.error('Failed to load auth config path:', error);
      }
    };

    loadAuthProviders();
    loadAuthConfigPath();
  }, [openCodeConfigRefreshKey]);

  React.useEffect(() => {
    if (!config) return;
    // Biome: make the dependency explicit
    void omosConfigRefreshKey;
    const loadFavProviders = async () => {
      try {
        const allProviders = await listFavoriteProviders();
        const opencodeFavoriteProviders = allProviders.filter((provider) =>
          isFavoriteProviderForSource('opencode', provider),
        );

        for (const favoriteProvider of opencodeFavoriteProviders) {
          if (!needsFavoriteProviderMigration('opencode', favoriteProvider.providerId)) {
            continue;
          }

          const migratedStorageKey = buildFavoriteProviderStorageKey('opencode', favoriteProvider.providerId);
          await upsertFavoriteProvider(
            migratedStorageKey,
            favoriteProvider.providerConfig,
            favoriteProvider.diagnostics,
          );
          try {
            await deleteFavoriteProvider(favoriteProvider.providerId);
          } catch (error) {
            console.error('Failed to delete legacy OpenCode favorite provider during migration:', error);
          }
        }

        const migratedProviders = await listFavoriteProviders();
        const nextFavoriteProviders = migratedProviders.filter((provider) =>
          isFavoriteProviderForSource('opencode', provider),
        );
        const currentStorageKeys = new Set(
          Object.keys(config.provider || {}).map((providerId) =>
            buildFavoriteProviderStorageKey('opencode', providerId),
          ),
        );
        const { keptProviders, duplicateIds } = dedupeOpenCodeFavoriteProviders(
          nextFavoriteProviders,
          currentStorageKeys,
        );

        if (duplicateIds.length > 0) {
          await Promise.all(
            duplicateIds.map(async (providerId) => {
              try {
                await deleteFavoriteProvider(providerId);
              } catch (error) {
                console.error('Failed to delete duplicate OpenCode favorite provider:', error);
              }
            }),
          );
        }

        setFavoriteProviders(keptProviders);
      } catch (error) {
        console.error('Failed to load favorite providers:', error);
      }
    };
    loadFavProviders();
  }, [config, omosConfigRefreshKey]);

  // Open auth.json config file
  const handleOpenAuthConfig = async () => {
    if (!authConfigPath) {
      message.warning(t('opencode.official.configNotFound'));
      return;
    }
    try {
      await revealItemInDir(authConfigPath);
    } catch (error) {
      console.error('Failed to open auth config:', error);
      message.error(t('common.error'));
    }
  };

  const doSaveConfig = async (newConfig: OpenCodeConfig) => {
    try {
      await saveOpenCodeConfig(newConfig);
      setConfig(newConfig);
      message.success(t('common.success'));
    } catch {
      message.error(t('common.error'));
      throw new Error('Save failed');
    }
  };

  const sanitizeOpenCodeModelReferences = React.useCallback((
    currentConfig: OpenCodeConfig,
    removedUnifiedModelIds: string[],
  ): OpenCodeConfig => {
    if (removedUnifiedModelIds.length === 0) {
      return currentConfig;
    }

    const removedModelIdSet = new Set(removedUnifiedModelIds);
    const sanitizedAgents = sanitizeAgentsModelReferences(currentConfig.agents, removedModelIdSet);

    const nextConfig: OpenCodeConfig = {
      ...currentConfig,
      model: currentConfig.model && removedModelIdSet.has(currentConfig.model) ? undefined : currentConfig.model,
      small_model: currentConfig.small_model && removedModelIdSet.has(currentConfig.small_model) ? undefined : currentConfig.small_model,
    };

    if (sanitizedAgents !== currentConfig.agents) {
      nextConfig.agents = sanitizedAgents;
    }

    return nextConfig;
  }, []);

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

  const disabledProviderIds = React.useMemo(
    () => new Set(config?.disabled_providers ?? []),
    [config?.disabled_providers],
  );
  const providerEntries = React.useMemo(
    () => (config?.provider ? Object.entries(config.provider) : []),
    [config?.provider],
  );
  const existingProviderIds = React.useMemo(
    () => providerEntries.map(([id]) => id),
    [providerEntries],
  );
  const existingFavoriteProviderIds = React.useMemo(
    () => existingProviderIds.map((providerId) => buildFavoriteProviderStorageKey('opencode', providerId)),
    [existingProviderIds],
  );

  const handleToggleProviderDisabled = async (providerId: string) => {
    if (!config) return;

    const current = config.disabled_providers ?? [];
    const nextSet = new Set(current);
    if (nextSet.has(providerId)) {
      nextSet.delete(providerId); // enable
    } else {
      nextSet.add(providerId); // disable
    }

    const nextArr = Array.from(nextSet);
    try {
      await doSaveConfig({
        ...config,
        disabled_providers: nextArr.length > 0 ? nextArr : undefined,
      });
      await refreshTrayMenu();
      incrementOpenCodeConfigRefresh();
    } catch (e) {
      console.error('Failed to toggle provider disabled state:', e);
    }
  };

  const handleOpenConfigFolder = async () => {
    if (!configPathInfo?.path) return;

    try {
      // Try to reveal the file in explorer
      await revealItemInDir(configPathInfo.path);
    } catch {
      // If file doesn't exist, fallback to opening parent directory
      try {
        const parentDir = configPathInfo.path.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open folder:', error);
        message.error(t('common.error'));
      }
    }
  };

  const handlePathModalSuccess = () => {
    setPathModalOpen(false);
    loadConfig();
  };

  const handleParseErrorBackedUp = () => {
    setParseError(null);
    loadConfig();
  };

  // Force refresh models cache from API (with 1 minute rate limit)
  const [refreshingModels, setRefreshingModels] = React.useState(false);
  const lastModelsRefreshTimeRef = React.useRef<number>(0);
  const MODELS_REFRESH_INTERVAL_MS = 60 * 1000; // 1 minute

  const handleRefreshModelsCache = async () => {
    const now = Date.now();
    const timeSinceLastRefresh = now - lastModelsRefreshTimeRef.current;

    if (timeSinceLastRefresh < MODELS_REFRESH_INTERVAL_MS) {
      const secondsLeft = Math.ceil((MODELS_REFRESH_INTERVAL_MS - timeSinceLastRefresh) / 1000);
      message.info(t('opencode.modelsRefreshRateLimit', { seconds: secondsLeft }));
      return;
    }

    setRefreshingModels(true);
    try {
      await getOpenCodeFreeModels(true);
      const models = await getOpenCodeUnifiedModels();
      setUnifiedModels(models);
      const authData = await getOpenCodeAuthProviders();
      setAuthProvidersData(authData);
      await refreshTrayMenu();
      // Also refresh remote preset models
      fetchRemotePresetModels();
      lastModelsRefreshTimeRef.current = now;
      message.success(t('opencode.modelsRefreshSuccess'));
    } catch (error) {
      console.error('Failed to refresh models cache:', error);
      message.error(t('common.error'));
    } finally {
      setRefreshingModels(false);
    }
  };

  // Provider handlers
  const handleAddProvider = () => {
    setCurrentProviderId('');
    setBaseProviderSnapshot(null);
    setProviderInitialValues(undefined);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (providerId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    // 提取已知字段之外的额外参数
    const knownOptionKeys = ['baseURL', 'apiKey', 'headers', 'timeout', 'setCacheKey'];
    const extraOptions: Record<string, unknown> = {};
    if (provider.options) {
      const options = provider.options;
      Object.keys(options).forEach((key) => {
        if (!knownOptionKeys.includes(key)) {
          extraOptions[key] = options[key];
        }
      });
    }

    setCurrentProviderId(providerId);
    setBaseProviderSnapshot(provider);
    // Determine filter mode: use blacklist mode only if blacklist has items,
    // otherwise use whitelist mode (even if both arrays exist but blacklist is empty)
    const hasBlacklistItems = (provider.blacklist?.length ?? 0) > 0;
    const initialVals: Partial<ProviderFormValues> = {
      id: providerId,
      name: provider.name,
      sdkType: provider.npm || '@ai-sdk/openai-compatible',
      baseUrl: provider.options?.baseURL || '',
      apiKey: provider.options?.apiKey || '',
      headers: provider.options?.headers,
      timeout: provider.options?.timeout === false ? undefined : (provider.options?.timeout as number | undefined),
      disableTimeout: provider.options?.timeout === false,
      setCacheKey: provider.options?.setCacheKey,
      extraOptions: Object.keys(extraOptions).length > 0 ? extraOptions : undefined,
      filterMode: hasBlacklistItems ? 'blacklist' : 'whitelist',
      filterModels: hasBlacklistItems ? (provider.blacklist || []) : (provider.whitelist || []),
    };
    setProviderInitialValues(initialVals);
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (providerId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    // 提取已知字段之外的额外参数
    const knownOptionKeys = ['baseURL', 'apiKey', 'headers', 'timeout', 'setCacheKey'];
    const extraOptions: Record<string, unknown> = {};
    if (provider.options) {
      const options = provider.options;
      Object.keys(options).forEach((key) => {
        if (!knownOptionKeys.includes(key)) {
          extraOptions[key] = options[key];
        }
      });
    }

    setCurrentProviderId('');
    setBaseProviderSnapshot(null);
    // Determine filter mode: use blacklist mode only if blacklist has items
    const hasBlacklistItems = (provider.blacklist?.length ?? 0) > 0;
    setProviderInitialValues({
      id: `${providerId}_copy`,
      name: provider.name,
      sdkType: provider.npm,
      baseUrl: provider.options?.baseURL || '',
      apiKey: provider.options?.apiKey || '',
      headers: provider.options?.headers,
      timeout: provider.options?.timeout === false ? undefined : (provider.options?.timeout as number | undefined),
      disableTimeout: provider.options?.timeout === false,
      setCacheKey: provider.options?.setCacheKey,
      extraOptions: Object.keys(extraOptions).length > 0 ? extraOptions : undefined,
      filterMode: hasBlacklistItems ? 'blacklist' : 'whitelist',
      filterModels: hasBlacklistItems ? (provider.blacklist || []) : (provider.whitelist || []),
    });
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = async (providerId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    const performDelete = async () => {
      const newProviders = { ...config.provider };
      delete newProviders[providerId];

      const nextDisabledProviders = (config.disabled_providers ?? []).filter((id) => id !== providerId);
      const removedUnifiedModelIds = Object.keys(provider.models ?? {}).map((modelId) => buildUnifiedModelId(providerId, modelId));

      const nextConfig = sanitizeOpenCodeModelReferences({
        ...config,
        provider: newProviders,
        disabled_providers: nextDisabledProviders.length > 0 ? nextDisabledProviders : undefined,
      }, removedUnifiedModelIds);

      await doSaveConfig(nextConfig);
      clearBatchDeleteState(providerId);
    };

    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('opencode', providerId),
        provider,
      );
      await performDelete();
    } catch (favoriteError) {
      console.error('Failed to preserve favorite provider before deletion:', favoriteError);
      Modal.confirm({
        title: t('common.deleteWithoutBackupTitle'),
        content: t('common.deleteWithoutBackupContent'),
        okText: t('common.continueDelete'),
        cancelText: t('common.cancel'),
        onOk: async () => {
          await performDelete();
        },
      });
    }
  };

  const handleProviderSuccess = async (values: ProviderFormValues) => {
    if (!config) return;

    const filterMode = values.filterMode || 'whitelist';
    const filterModels = (values.filterModels || []).filter((modelId) => modelId);
    const shouldPersistFilter = filterModels.length > 0;
    const existingProvider = currentProviderId ? config.provider[currentProviderId] : undefined;
    const managedProviderOptions = buildManagedProviderOptions(values);
    const newProvider: OpenCodeProvider = {
      ...(baseProviderSnapshot ?? {}),
      ...(existingProvider ?? {}),
      npm: values.sdkType || '@ai-sdk/openai-compatible',
      name: values.name,
      options: managedProviderOptions,
      models: existingProvider?.models || baseProviderSnapshot?.models || {},
      whitelist: shouldPersistFilter && filterMode === 'whitelist' ? filterModels : undefined,
      blacklist: shouldPersistFilter && filterMode === 'blacklist' ? filterModels : undefined,
    };

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [values.id]: newProvider,
      },
    });

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('opencode', values.id),
        newProvider,
      );
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    setProviderModalOpen(false);
    setProviderInitialValues(undefined);
    setBaseProviderSnapshot(null);
  };

  const handleProviderDuplicateId = () => {
    message.error(t('opencode.provider.idExists'));
  };

  // Model handlers
  const handleAddModel = (providerId: string) => {
    setCurrentModelProviderId(providerId);
    setCurrentModelId('');
    setBaseModelSnapshot(null);
    setModelInitialValues(undefined);
    setModelModalOpen(true);
  };

  const handleEditModel = (providerId: string, modelId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;
    const model = provider.models[modelId];
    if (!model) return;

    setCurrentModelProviderId(providerId);
    setCurrentModelId(modelId);
    setBaseModelSnapshot(model);
    setModelInitialValues({
      id: modelId,
      name: model.name,
      contextLimit: model.limit?.context,
      outputLimit: model.limit?.output,
      options: model.options ? JSON.stringify(model.options) : undefined,
      variants: model.variants ? JSON.stringify(model.variants) : undefined,
      modalities: model.modalities ? JSON.stringify(model.modalities) : undefined,
      reasoning: model.reasoning,
      attachment: model.attachment,
      tool_call: model.tool_call,
      temperature: model.temperature,
    });
    setModelModalOpen(true);
  };

  const handleCopyModel = (providerId: string, modelId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;
    const model = provider.models[modelId];
    if (!model) return;

    setCurrentModelProviderId(providerId);
    setCurrentModelId('');
    setBaseModelSnapshot(null);
    setModelInitialValues({
      id: `${modelId}_copy`,
      name: model.name,
      contextLimit: model.limit?.context,
      outputLimit: model.limit?.output,
      options: model.options ? JSON.stringify(model.options) : undefined,
      variants: model.variants ? JSON.stringify(model.variants) : undefined,
      modalities: model.modalities ? JSON.stringify(model.modalities) : undefined,
      reasoning: model.reasoning,
      attachment: model.attachment,
      tool_call: model.tool_call,
      temperature: model.temperature,
    });
    setModelModalOpen(true);
  };

  const handleDeleteModel = async (providerId: string, modelId: string) => {
    if (!config) return;

    const provider = config.provider[providerId];
    if (!provider) return;

    const newModels = { ...provider.models };
    delete newModels[modelId];

    const updatedProvider: OpenCodeProvider = {
      ...provider,
      models: newModels,
    };

    const nextConfig = sanitizeOpenCodeModelReferences({
      ...config,
      provider: {
        ...config.provider,
        [providerId]: updatedProvider,
      },
    }, [buildUnifiedModelId(providerId, modelId)]);

    await doSaveConfig(nextConfig);

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('opencode', providerId),
        updatedProvider,
      );
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    // Refresh tray menu and model list after deleting model
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
    clearBatchDeleteState(providerId);
  };

  const handleSetPrimaryModel = async (providerId: string, modelId: string) => {
    if (!config) return;

    const provider = config.provider[providerId];
    if (!provider) return;

    const unifiedModelId = buildUnifiedModelId(providerId, modelId);
    if (config.model === unifiedModelId) {
      return;
    }

    await doSaveConfig({
      ...config,
      model: unifiedModelId,
    });
    await refreshTrayMenu();

    message.success(t('opencode.model.setAsPrimarySuccess', { name: provider.models[modelId]?.name || modelId }));
  };

  const handleToggleBatchDeleteMode = (providerId: string) => {
    if (batchDeleteProviderId === providerId) {
      clearBatchDeleteState(providerId);
      return;
    }

    setSelectedModelIdsByProvider({});
    setBatchDeleteProviderId(providerId);
  };

  const handleToggleModelSelection = (providerId: string, modelId: string, selected: boolean) => {
    setSelectedModelIdsByProvider((previousState) => {
      const currentModelIds = previousState[providerId] ?? [];
      const nextModelIds = selected
        ? Array.from(new Set([...currentModelIds, modelId]))
        : currentModelIds.filter((id) => id !== modelId);

      if (nextModelIds.length === 0) {
        const nextState = { ...previousState };
        delete nextState[providerId];
        return nextState;
      }

      return {
        ...previousState,
        [providerId]: nextModelIds,
      };
    });
  };

  const handleBatchDeleteModels = async (providerId: string) => {
    if (!config) return;

    const provider = config.provider[providerId];
    if (!provider) return;

    const selectedModelIds = selectedModelIdsByProvider[providerId] ?? [];
    if (selectedModelIds.length === 0) {
      return;
    }

    const nextModels = { ...provider.models };
    for (const modelId of selectedModelIds) {
      delete nextModels[modelId];
    }

    const updatedProvider: OpenCodeProvider = {
      ...provider,
      models: nextModels,
    };

    const removedUnifiedModelIds = selectedModelIds.map((modelId) => buildUnifiedModelId(providerId, modelId));
    const nextConfig = sanitizeOpenCodeModelReferences({
      ...config,
      provider: {
        ...config.provider,
        [providerId]: updatedProvider,
      },
    }, removedUnifiedModelIds);

    await doSaveConfig(nextConfig);

    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('opencode', providerId),
        updatedProvider,
      );
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    message.success(t('opencode.model.batchDeleteSuccess', { count: selectedModelIds.length }));
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
    clearBatchDeleteState(providerId);
  };

  const handleModelSuccess = async (values: ModelFormValues) => {
    if (!config) return;

    const provider = config.provider[currentModelProviderId];
    if (!provider) return;
    const newModel: OpenCodeModel = {
      ...(baseModelSnapshot ?? {}),
      ...(values.name ? { name: values.name } : { name: undefined }),
      limit: buildManagedModelLimit(baseModelSnapshot, values),
      modalities: values.modalities ? JSON.parse(values.modalities) : undefined,
      reasoning: values.reasoning,
      attachment: values.attachment,
      tool_call: values.tool_call,
      temperature: values.temperature,
      options: parseOptionalJsonValue(values.options),
      variants: parseOptionalJsonValue(values.variants),
    };

    const updatedProvider: OpenCodeProvider = {
      ...provider,
      models: {
        ...provider.models,
        [values.id]: newModel,
      },
    };

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [currentModelProviderId]: updatedProvider,
      },
    });

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('opencode', currentModelProviderId),
        updatedProvider,
      );
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    setModelModalOpen(false);
    setModelInitialValues(undefined);
    setBaseModelSnapshot(null);
    // Refresh tray menu and model list after adding/editing model
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  const handleModelDuplicateId = () => {
    message.error(t('opencode.model.idExists'));
  };

  // Fetch models handlers
  const handleOpenFetchModels = (providerId: string) => {
    setFetchModelsProviderId(providerId);
    setFetchModelsModalOpen(true);
  };

  const handleFetchModelsSuccess = async ({ selectedModels, removedModelIds }: FetchModelsApplyResult) => {
    if (!config || !fetchModelsProviderId) return;

    const provider = config.provider[fetchModelsProviderId];
    if (!provider) return;

    const newModels = { ...provider.models };
    removedModelIds.forEach((modelId) => {
      delete newModels[modelId];
    });
    selectedModels.forEach((model) => {
      newModels[model.id] = buildFetchedOpenCodeModel(model, provider.npm);
    });

    const updatedProvider: OpenCodeProvider = {
      ...provider,
      models: newModels,
    };

    const removedUnifiedModelIds = removedModelIds.map((modelId) => buildUnifiedModelId(fetchModelsProviderId, modelId));
    const nextConfig = sanitizeOpenCodeModelReferences({
      ...config,
      provider: {
        ...config.provider,
        [fetchModelsProviderId]: updatedProvider,
      },
    }, removedUnifiedModelIds);

    await doSaveConfig(nextConfig);

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('opencode', fetchModelsProviderId),
        updatedProvider,
      );
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    setFetchModelsModalOpen(false);
    message.success(t('opencode.fetchModels.applySuccess', {
      addCount: selectedModels.length,
      removeCount: removedModelIds.length,
    }));
    // Refresh tray menu and model list after fetching models
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
    clearBatchDeleteState(fetchModelsProviderId);
  };

  // Get current provider info for FetchModelsModal
  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!config || !fetchModelsProviderId) return null;
    const provider = config.provider[fetchModelsProviderId];
    if (!provider) return null;
    return {
      providerId: fetchModelsProviderId,
      name: provider.name || fetchModelsProviderId,
      baseUrl: provider.options?.baseURL || '',
      apiKey: provider.options?.apiKey,
      headers: provider.options?.headers as Record<string, string> | undefined,
      sdkName: provider.npm,
      existingModelIds: Object.keys(provider.models || {}),
    };
  }, [config, fetchModelsProviderId]);

  // Import provider handlers
  const handleImportProviders = async (providers: OpenCodeFavoriteProvider[]) => {
    if (!config) return;

    // Build new providers object
    const newProviders = { ...config.provider };
    providers.forEach((p) => {
      const rawProviderId = extractFavoriteProviderRawId('opencode', p.providerId);

      // Only add if not already exists
      if (!newProviders[rawProviderId]) {
        newProviders[rawProviderId] = p.providerConfig;
      }
    });

    await doSaveConfig({
      ...config,
      provider: newProviders,
    });

    setImportModalOpen(false);
    message.success(t('opencode.provider.importSuccess', { count: providers.length }));
    // Refresh tray menu and model list after importing
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  const handleImportAllApiHubProviders = async (providers: OpenCodeAllApiHubProvider[]) => {
    if (!config) return;

    const newProviders = { ...config.provider };
    providers.forEach((provider) => {
      if (!newProviders[provider.providerId]) {
        newProviders[provider.providerId] = provider.providerConfig;
      }
    });

    await doSaveConfig({
      ...config,
      provider: newProviders,
    });

    setAllApiHubImportModalOpen(false);
    message.success(t('opencode.provider.importSuccess', { count: providers.length }));
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  const favoriteProvidersMap = React.useMemo(() => {
    return new Map(
      favoriteProviders.map((item) => [extractFavoriteProviderRawId('opencode', item.providerId), item]),
    );
  }, [favoriteProviders]);

  const getProviderTestModelIds = React.useCallback((providerId: string, provider?: OpenCodeProvider) => {
    const modelIds = new Set<string>();

    Object.keys(provider?.models || {}).forEach((modelId) => {
      modelIds.add(modelId);
    });

    (authProvidersData?.mergedModels?.[providerId] || []).forEach((model) => {
      modelIds.add(model.id);
    });

    return Array.from(modelIds);
  }, [authProvidersData]);

  // Get current provider info for ConnectivityTestModal
  const connectivityProviderInfo = React.useMemo(() => {
    if (!config || !connectivityProviderId) return null;
    const provider = config.provider[connectivityProviderId];
    if (!provider) return null;
    return {
      name: provider.name || connectivityProviderId,
      config: provider,
      modelIds: getProviderTestModelIds(connectivityProviderId, provider),
      removableModelIds: provider.models ? Object.keys(provider.models) : [],
      diagnostics: favoriteProvidersMap.get(connectivityProviderId)?.diagnostics,
    };
  }, [config, connectivityProviderId, favoriteProvidersMap, getProviderTestModelIds]);

  const handleOpenConnectivityTest = (providerId: string) => {
    setConnectivityProviderId(providerId);
    setConnectivityModalOpen(true);
  };

  const handleBatchTestProviders = React.useCallback(async () => {
    if (providerEntries.length === 0) {
      return;
    }

    const targets = providerEntries.map(([providerId, provider]) => {
      const providerNpm = provider.npm || '@ai-sdk/openai-compatible';
      if (!SUPPORTED_PROVIDER_NPMS.has(providerNpm)) {
        return {
          providerId,
          errorMessage: t('common.unsupportedSdkType', { npm: providerNpm }),
        };
      }

      return buildProviderConnectivityBatchTarget(
        {
          providerId,
          providerName: provider.name || providerId,
          providerConfig: provider,
          modelIds: getProviderTestModelIds(providerId, provider),
        },
        {
          requireBaseUrl: !resolvedAuthProviderIds.has(providerId),
          requireApiKey: !resolvedAuthProviderIds.has(providerId),
          preferredModelId: findDefaultTestModelIdForProvider(favoriteProviders, 'opencode', providerId),
          errorMessages: {
            missingBaseUrl: t('common.baseUrlMissing'),
            missingApiKey: t('common.apiKeyMissing'),
            missingModel: t('common.modelMissing'),
          },
        },
      );
    });

    setConnectivityStatuses(
      Object.fromEntries(
        providerEntries.map(([providerId]) => [
          providerId,
          { status: 'running' as const },
        ]),
      ),
    );
    setBatchTestingProviders(true);

    try {
      await runProviderConnectivityBatch(targets, (providerId, status) => {
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
          [providerId]: nextStatus,
        }));
      });
    } catch (error) {
      console.error('Failed to batch test OpenCode providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [providerEntries, resolvedAuthProviderIds, t, getProviderTestModelIds, favoriteProviders]);

  const handleSaveDiagnostics = async (diagnostics: OpenCodeDiagnosticsConfig) => {
    if (!config || !connectivityProviderId) return;

    const provider = config.provider[connectivityProviderId];
    if (!provider) return;

    // Save diagnostics to favorite provider ONLY
    try {
      const updatedFav = await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('opencode', connectivityProviderId),
        provider,
        diagnostics,
      );

      // Update local state
      setFavoriteProviders((prev) => {
        const storageProviderId = buildFavoriteProviderStorageKey('opencode', connectivityProviderId);
        const index = prev.findIndex((p) => p.providerId === storageProviderId);
        if (index >= 0) {
          const newFavs = [...prev];
          newFavs[index] = updatedFav;
          return newFavs;
        } else {
          return [...prev, updatedFav];
        }
      });
    } catch (error) {
      console.error('Failed to save diagnostics:', error);
      message.error(t('common.error'));
    }
  };

  const handleRemoveModels = async (modelIdsToRemove: string[]) => {
    if (!config || !connectivityProviderId) return;

    const provider = config.provider[connectivityProviderId];
    if (!provider || !provider.models) return;

    // Create new models object without the removed models
    const newModels = { ...provider.models };
    for (const modelId of modelIdsToRemove) {
      delete newModels[modelId];
    }

    const newConfig = sanitizeOpenCodeModelReferences({
      ...config,
      provider: {
        ...config.provider,
        [connectivityProviderId]: {
          ...provider,
          models: newModels,
        },
      },
    }, modelIdsToRemove.map((modelId) => buildUnifiedModelId(connectivityProviderId, modelId)));

    await doSaveConfig(newConfig);
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
    clearBatchDeleteState(connectivityProviderId);
  };

  // Drag handlers

  const handleProviderDragEnd = async (event: DragEndEvent) => {
    if (!config) return;
    const { active, over } = event;

    if (over && active.id !== over.id) {
      const providerIds = Object.keys(config.provider);
      const oldIndex = providerIds.indexOf(active.id as string);
      const newIndex = providerIds.indexOf(over.id as string);

      const newOrder = arrayMove(providerIds, oldIndex, newIndex);
      const newProviders = reorderObject(config.provider, newOrder);

      await doSaveConfig({
        ...config,
        provider: newProviders,
      });
    }
  };

  const handleReorderModels = async (providerId: string, modelIds: string[]) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    const newModels = reorderObject(provider.models, modelIds);

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [providerId]: {
          ...provider,
          models: newModels,
        },
      },
    });
  };

  const handlePreviewConfig = async () => {
    if (!config) return;
    setPreviewDataLocal(config);
    setPreviewModalOpen(true);
  };

  const presetModelsVersion = React.useSyncExternalStore(
    subscribePresetModels,
    getPresetModelsVersion,
    getPresetModelsVersion,
  );
  const existingModelIds = React.useMemo(() => {
    if (!config || !config.provider || !currentModelProviderId) return [];
    const provider = config.provider[currentModelProviderId];
    return provider?.models ? Object.keys(provider.models) : [];
  }, [config, currentModelProviderId]);

  // OMO settings should keep all models available (including those from disabled providers),
  // but we still group options by provider for easier scanning/search.

  const selectedMainModel = config?.model;
  const selectedSmallModel = config?.small_model;

  const enabledUnifiedModels = React.useMemo(() => {
    return unifiedModels.filter((m) => {
      const isProviderDisabled = disabledProviderIds.has(m.providerId);
      if (!isProviderDisabled) return true;
      // Keep current selections visible even if their provider is disabled
      return m.id === selectedMainModel || m.id === selectedSmallModel;
    });
  }, [unifiedModels, disabledProviderIds, selectedMainModel, selectedSmallModel]);

  type ModelOption = { label: string; value: string; disabled?: boolean };
  type ModelGroup = { label: string; options: ModelOption[] };

  const omoModelGroupedOptions = React.useMemo((): ModelGroup[] => {
    const groups = new Map<string, { groupLabel: string; options: ModelOption[] }>();

    for (const m of unifiedModels) {
      const parts = m.displayName.split(' / ');
      const modelLabel = parts.slice(1).join(' / ') || m.modelId;

      const isProviderDisabled = disabledProviderIds.has(m.providerId);

      const entry = groups.get(m.providerId) || {
        groupLabel: m.providerId,
        options: [],
      };

      entry.options.push({
        label: `${m.providerId} / ${modelLabel}`,
        value: m.id,
        disabled: isProviderDisabled,
      });

      groups.set(m.providerId, entry);
    }

    const result: ModelGroup[] = [];
    for (const [providerId, entry] of groups.entries()) {
      const groupLabel = entry.groupLabel || providerId;
      entry.options.sort((a, b) => a.label.localeCompare(b.label));
      result.push({ label: groupLabel, options: entry.options });
    }

    result.sort((a, b) => a.label.localeCompare(b.label));
    return result;
  }, [unifiedModels, disabledProviderIds]);

  const groupedModelOptionsBase = React.useMemo((): ModelGroup[] => {
    const groups = new Map<string, { groupLabel: string; options: ModelOption[] }>();

    for (const m of enabledUnifiedModels) {
      const parts = m.displayName.split(' / ');
      const providerLabel = parts[0] || m.providerId;
      const modelLabel = parts.slice(1).join(' / ') || m.modelId;

      const entry = groups.get(m.providerId) || { groupLabel: providerLabel, options: [] };
      // Keep provider prefix for each option to avoid same model name confusion.
      entry.options.push({ label: `${providerLabel} / ${modelLabel}`, value: m.id });
      groups.set(m.providerId, entry);
    }

    const result: ModelGroup[] = [];
    for (const [providerId, entry] of groups.entries()) {
      const groupLabel = entry.groupLabel || providerId;
      entry.options.sort((a, b) => a.label.localeCompare(b.label));
      result.push({ label: groupLabel, options: entry.options });
    }

    result.sort((a, b) => a.label.localeCompare(b.label));
    return result;
  }, [enabledUnifiedModels]);

  const mainModelGroupedOptions = React.useMemo((): ModelGroup[] => {
    return groupedModelOptionsBase.map((g) => ({
      ...g,
      options: g.options.map((opt) => ({
        ...opt,
        label: opt.value === selectedMainModel ? `${opt.label} ✓` : opt.label,
      })),
    }));
  }, [groupedModelOptionsBase, selectedMainModel]);

  const smallModelGroupedOptions = React.useMemo((): ModelGroup[] => {
    return groupedModelOptionsBase.map((g) => ({
      ...g,
      options: g.options.map((opt) => ({
        ...opt,
        label: opt.value === selectedSmallModel ? `${opt.label} ✓` : opt.label,
      })),
    }));
  }, [groupedModelOptionsBase, selectedSmallModel]);

  // Build model variants map from config and preset models
  const modelVariantsMap = React.useMemo(
    () => {
      void presetModelsVersion; // Biome: dependency marker (PRESET_MODELS content can update without reference changes)
      return buildModelVariantsMap(config, unifiedModels, PRESET_MODELS);
    },
    [config, unifiedModels, presetModelsVersion]
  );

  const handleModelChange = async (field: 'model' | 'small_model', value: string | undefined) => {
    if (!config) return;

    await doSaveConfig({
      ...config,
      [field]: value || undefined,
    });
  };

  const handlePluginChange = async (plugins: OpenCodePluginEntry[]) => {
    if (!config) return;

    const sanitizedPlugins = sanitizeOpenCodePluginList(plugins);
    await doSaveConfig({
      ...config,
      plugin: sanitizedPlugins.length > 0 ? sanitizedPlugins : undefined,
    });
  };

  // Extract other config fields (unknown fields)
  const otherConfigFields = React.useMemo(() => {
    if (!config) return undefined;
    const knownFields = ['$schema', 'provider', 'model', 'small_model', 'plugin', 'mcp', 'disabled_providers'];
    const other: Record<string, unknown> = {};
    Object.keys(config).forEach((key) => {
      if (!knownFields.includes(key)) {
        other[key] = config[key];
      }
    });
    // 如果没有其他字段，返回 undefined 而不是空对象，这样 JsonEditor 会显示 placeholder
    return Object.keys(other).length > 0 ? other : undefined;
  }, [config]);

  const handleOtherConfigChange = (_value: unknown, isValid: boolean) => {
    otherConfigJsonValidRef.current = isValid;
    // 只验证，不保存
  };

  // 保存其他配置（用于 onBlur 回调）
  const handleOtherConfigBlur = async (value: unknown) => {
    if (!config || !otherConfigJsonValidRef.current) {
      return;
    }

    // Remove old unknown fields
    const newConfig: OpenCodeConfig = {
      $schema: config.$schema,
      provider: config.provider,
      disabled_providers: config.disabled_providers,
      model: config.model,
      small_model: config.small_model,
      plugin: config.plugin,
      mcp: config.mcp,
    };

    // Add new other fields
    if (typeof value === 'object' && value !== null) {
      Object.assign(newConfig, value);
    }

    await doSaveConfig(newConfig);
  };

  return (
    <div>
      {/* If parse error exists, only show the error alert */}
      {parseError ? (
        <div style={{
          display: 'flex',
          justifyContent: 'center',
          alignItems: 'center',
          minHeight: '60vh',
          padding: '24px'
        }}>
          <div style={{ width: '100%', maxWidth: '800px' }}>
            <ConfigParseErrorAlert
              path={parseError.path}
              error={parseError.error}
              contentPreview={parseError.contentPreview}
              onBackedUp={handleParseErrorBackedUp}
            />
          </div>
        </div>
      ) : (
        <SectionSidebarLayout
          sidebarTitle={t('opencode.title')}
          sidebarHidden={sidebarHidden}
          sections={sidebarSections}
          markerAttr="data-opencode-sidebar-section"
          getIcon={(id) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
          onSectionSelect={handleSidebarSelect}
        >
          <div className={styles.opencodePageContent}>
            <div style={{ marginBottom: 16, order: 0 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
                <div>
                  <div style={{ marginBottom: 8 }}>
                    <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                      {t('opencode.title')}
                    </Title>
                    <Link
                      type="secondary"
                      style={{ fontSize: 12 }}
                      onClick={(e) => {
                        e.stopPropagation();
                        openUrl('https://opencode.ai/docs/config/#format');
                      }}
                    >
                      <LinkOutlined /> {t('opencode.viewDocs')}
                    </Link>
                    <Link
                      type="secondary"
                      style={{ fontSize: 12, marginLeft: 16 }}
                      onClick={(e) => {
                        e.stopPropagation();
                        handlePreviewConfig();
                      }}
                    >
                      <EyeOutlined /> {t('common.previewConfig')}
                    </Link>
                  </div>
                  <Space>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('opencode.configPath')}:
                    </Text>
                    <Text code style={{ fontSize: 12 }}>
                      {configPathInfo?.path}
                    </Text>
                    <Button
                      type="text"
                      size="small"
                      icon={<EditOutlined />}
                      onClick={() => setPathModalOpen(true)}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('opencode.configPathSource.customize')}
                    </Button>
                    <Button
                      type="text"
                      size="small"
                      icon={<FolderOpenOutlined />}
                      onClick={handleOpenConfigFolder}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('opencode.openFolder')}
                    </Button>
                    <Button
                      type="text"
                      size="small"
                      icon={<ReloadOutlined />}
                      onClick={() => {
                        loadConfig(true);
                        incrementOpenCodeConfigRefresh();
                        incrementOmoConfigRefresh();
                        incrementOmosConfigRefresh();
                        refreshTrayMenu();
                      }}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('opencode.refreshConfig')}
                    </Button>
                    <Button
                      type="text"
                      size="small"
                      icon={<CloudSyncOutlined />}
                      onClick={handleRefreshModelsCache}
                      loading={refreshingModels}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('opencode.syncModels')}
                    </Button>
                  </Space>
                </div>
                <Space>
                  <Button type="text" icon={<EllipsisOutlined />} onClick={() => setSettingsModalOpen(true)}>
                    {t('common.moreOptions')}
                  </Button>
                </Space>
              </div>
              <div style={{ fontSize: 12, color: 'rgba(0,0,0,0.45)', borderLeft: '2px solid rgba(0,0,0,0.12)', paddingLeft: 8, marginTop: 4 }}>
                {t('opencode.pageHint')}
              </div>
            </div>

            <div
              id="opencode-model-settings"
              className={styles.opencodeSection}
              data-opencode-sidebar-section="true"
              data-sidebar-title={t('opencode.modelSettings.title')}
              data-sidebar-order={1}
              style={{ order: 1 }}
            >
              <div className={styles.modelCard}>
                <Title level={5} className={styles.modelCardTitle}>
                  <RobotOutlined style={{ marginRight: 8 }} />
                  {t('opencode.modelSettings.title')}
                </Title>
                <div className={styles.modelCardContent}>
                  <Space orientation="vertical" style={{ width: '100%' }} size={12}>
                    <div>
                      <div style={{ marginBottom: 4 }}>
                        <Text strong>{t('opencode.modelSettings.modelLabel')}</Text>
                      </div>
                      <Select
                        value={config?.model}
                        onChange={(value) => handleModelChange('model', value)}
                        placeholder={t('opencode.modelSettings.modelPlaceholder')}
                        allowClear
                        showSearch
                        optionFilterProp="label"
                        options={mainModelGroupedOptions}
                        optionLabelProp="label"
                        style={{ width: '100%' }}
                        notFoundContent={t('opencode.modelSettings.noModels')}
                      />
                    </div>

                    <div>
                      <div style={{ marginBottom: 4 }}>
                        <Text strong>{t('opencode.modelSettings.smallModelLabel')}</Text>
                        <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                          {t('opencode.modelSettings.smallModelHint')}
                        </Text>
                      </div>
                      <Select
                        value={config?.small_model}
                        onChange={(value) => handleModelChange('small_model', value)}
                        placeholder={t('opencode.modelSettings.smallModelPlaceholder')}
                        allowClear
                        showSearch
                        optionFilterProp="label"
                        options={smallModelGroupedOptions}
                        optionLabelProp="label"
                        style={{ width: '100%' }}
                        notFoundContent={t('opencode.modelSettings.noModels')}
                      />
                    </div>

                    {/* Oh My OpenAgent Config Selector - show only if plugin is enabled */}
                    {omoPluginEnabled && (
                      <div>
                        <div style={{ marginBottom: 4 }}>
                          <Text strong>{t('opencode.ohMyOpenCode.configLabel')}</Text>
                          <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                            {t('opencode.ohMyOpenCode.configHint')}
                          </Text>
                        </div>
                        <OhMyOpenAgentConfigSelector
                          key={ohMyOpenAgentRefreshKey} // 当 key 改变时，组件会重新挂载并刷新
                          disabled={false}
                          onConfigSelected={() => {
                            message.success(t('opencode.ohMyOpenCode.configSelected'));
                            // 当在快速切换框中选择配置时，触发设置列表刷新
                            setOhMyOpenAgentSettingsRefreshKey((prev) => prev + 1);
                          }}
                        />
                      </div>
                    )}

                    {/* Oh My OpenCode Slim Config Selector - show only if plugin is enabled */}
                    {omoSlimPluginEnabled && (
                      <div>
                        <div style={{ marginBottom: 4 }}>
                          <Text strong>{t('opencode.ohMyOpenCode.slimConfigLabel')}</Text>
                          <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                            {t('opencode.ohMyOpenCode.slimConfigHint')}
                          </Text>
                        </div>
                        <OhMyOpenCodeSlimConfigSelector
                          disabled={false}
                          onConfigSelected={() => {
                            message.success(t('opencode.ohMyOpenCode.configSelected'));
                          }}
                        />
                      </div>
                    )}
                  </Space>
                </div>
              </div>
            </div>

            <div
              id="opencode-plugin-configuration"
              className={styles.opencodeSection}
              data-opencode-sidebar-section="true"
              data-sidebar-title={t('opencode.plugin.title')}
              data-sidebar-order={2}
              style={{ order: 2 }}
            >
              <PluginSettings
                key={`opencode-plugin-settings-${pluginExpandNonce}`}
                plugins={config?.plugin || []}
                onChange={handlePluginChange}
                defaultCollapsed={pluginExpandNonce === 0}
              />
            </div>

            {(omoPluginEnabled || omoConfigs.length > 0) && (
              <div
                id="opencode-omo-configuration"
                className={styles.opencodeSection}
                data-opencode-sidebar-section="true"
                data-sidebar-title={t('opencode.ohMyOpenCode.title')}
                data-sidebar-order={3}
                style={{ order: 3 }}
              >
	                <OhMyOpenAgentSettings
	                  key={`opencode-omo-settings-${ohMyOpenAgentSettingsRefreshKey}-${omoSettingsExpandNonce}`}
	                  modelOptions={omoModelGroupedOptions}
	                  modelVariantsMap={modelVariantsMap}
	                  disabled={!omoPluginEnabled}
	                  allowClearAppliedConfig={opencodeAllowClearAppliedOhMyConfig}
	                  onConfigApplied={() => {
                    // 当配置被应用时，触发 Selector 刷新以更新选中状态
                    setOhMyOpenAgentRefreshKey((prev) => prev + 1);
                  }}
                  onConfigUpdated={() => {
                    // 当配置被创建/更新/删除时，触发 Selector 刷新
                    setOhMyOpenAgentRefreshKey((prev) => prev + 1);
                  }}
                  onLegacyUpgraded={() => {
                    loadConfig();
                    incrementOpenCodeConfigRefresh();
                    setOhMyOpenAgentRefreshKey((prev) => prev + 1);
                    setOhMyOpenAgentSettingsRefreshKey((prev) => prev + 1);
                  }}
                />
              </div>
            )}

            {(omoSlimPluginEnabled || omoSlimConfigs.length > 0) && (
              <div
                id="opencode-omo-slim-configuration"
                className={styles.opencodeSection}
                data-opencode-sidebar-section="true"
                data-sidebar-title={t('opencode.ohMyOpenCodeSlim.title')}
                data-sidebar-order={4}
                style={{ order: 4 }}
              >
	                <OhMyOpenCodeSlimSettings
	                  key={`opencode-omo-slim-settings-${ohMyOpenAgentSettingsRefreshKey}-${omoSlimSettingsExpandNonce}`}
	                  modelOptions={omoModelGroupedOptions}
	                  modelVariantsMap={modelVariantsMap}
	                  disabled={!omoSlimPluginEnabled}
	                  allowClearAppliedConfig={opencodeAllowClearAppliedOhMyConfig}
	                  onConfigApplied={() => {
                    message.success(t('opencode.ohMyOpenCode.configSelected'));
                  }}
                  onConfigUpdated={() => {
                    // 配置更新后刷新
                    loadConfig();
                  }}
                />
              </div>
            )}


            <div
              id="opencode-providers"
              className={styles.opencodeSection}
              data-opencode-sidebar-section="true"
              data-sidebar-title={t('opencode.provider.title')}
              data-sidebar-order={5}
              style={{ order: 5 }}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={providerListCollapsed ? [] : ['providers']}
                onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
                items={[
                  {
                    key: 'providers',
                    label: (
                      <Text strong><DatabaseOutlined style={{ marginRight: 8 }} />{t('opencode.provider.title')}</Text>
                    ),
                    extra: (
                      <Space size={4}>
                        <Button
                          type="link"
                          size="small"
                          style={{ fontSize: 12 }}
                          icon={<ThunderboltOutlined />}
                          loading={batchTestingProviders}
                          onClick={(e) => {
                            e.stopPropagation();
                            handleBatchTestProviders();
                          }}
                        >
                          {t('common.batchTest')}
                        </Button>
                        <Button
                          type="link"
                          size="small"
                          style={{ fontSize: 12 }}
                          icon={<PlusOutlined />}
                          onClick={(e) => {
                            e.stopPropagation();
                            handleAddProvider();
                          }}
                        >
                          {t('opencode.addProvider')}
                        </Button>
                      </Space>
                    ),
                    children: (
                      <Spin spinning={loading}>
                        {providerEntries.length === 0 ? (
                          <Empty description={t('opencode.emptyText')} style={{ marginTop: 40 }} />
                        ) : (
                          <DndContext
                            sensors={sensors}
                            collisionDetection={closestCenter}
                            modifiers={[restrictToVerticalAxis]}
                            onDragEnd={handleProviderDragEnd}
                          >
                            <SortableContext
                              items={providerEntries.map(([id]) => id)}
                              strategy={verticalListSortingStrategy}
                            >
                              {providerEntries.map(([providerId, provider]) => {
                                const providerNpm = provider.npm || '@ai-sdk/openai-compatible';
                                const isBatchDeleteMode = batchDeleteProviderId === providerId;
                                const selectedModelIds = selectedModelIdsByProvider[providerId] ?? [];
                                const selectedModelCount = selectedModelIds.length;
                                const isConnectivitySupported = SUPPORTED_PROVIDER_NPMS.has(providerNpm);
                                const providerBaseUrl = provider.options?.baseURL?.trim() || '';
                                const providerApiKey = provider.options?.apiKey?.trim() || '';
                                const hasOfficialAuthFallback = resolvedAuthProviderIds.has(providerId);
                                const isProviderAuthReady = Boolean(
                                  hasOfficialAuthFallback || (providerBaseUrl && providerApiKey),
                                );
                                const connectivityTooltip = !isConnectivitySupported
                                  ? t('opencode.connectivity.unsupportedNpm', { npm: providerNpm })
                                  : !isProviderAuthReady
                                    ? t('opencode.provider.completeUrlAndKey')
                                    : '';
                                const fetchModelsTooltip = !isProviderAuthReady
                                  ? t('opencode.provider.completeUrlAndKey')
                                  : '';
                                const providerModels = provider.models ? Object.entries(provider.models).map(([modelId, model]) => ({
                                  ...toModelDisplayData(modelId, model),
                                  isPrimary: buildUnifiedModelId(providerId, modelId) === config?.model,
                                })) : [];
                                return (
                                  <ProviderCard
                                    key={providerId}
                                    provider={toProviderDisplayData(providerId, provider)}
                                    models={providerModels}
                                    officialModels={authProvidersData?.mergedModels?.[providerId]?.map((m): OfficialModelDisplayData => ({
                                      id: m.id,
                                      name: m.name,
                                      isFree: m.isFree,
                                      context: m.context,
                                      output: m.output,
                                      status: m.status,
                                    }))}
                                    draggable
                                    sortableId={providerId}
                                    onEdit={() => handleEditProvider(providerId)}
                                    onCopy={() => handleCopyProvider(providerId)}
                                    onDelete={() => handleDeleteProvider(providerId)}
                                    isDisabled={disabledProviderIds.has(providerId)}
                                    onToggleDisabled={() => handleToggleProviderDisabled(providerId)}
                                    connectivityStatus={connectivityStatuses[providerId]}
                                    extraActions={
                                      <Space size={0}>
                                        <Button
                                          size="small"
                                          type="text"
                                          icon={<DeleteOutlined />}
                                          style={{ fontSize: 12 }}
                                          onClick={() => handleToggleBatchDeleteMode(providerId)}
                                        >
                                          {isBatchDeleteMode
                                            ? t('opencode.model.cancelBatchDelete')
                                            : t('opencode.model.batchDelete')}
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
                                                title: t('opencode.model.batchDeleteConfirmTitle'),
                                                content: t('opencode.model.batchDeleteConfirmContent', { count: selectedModelCount }),
                                                okText: t('common.confirm'),
                                                cancelText: t('common.cancel'),
                                                onOk: async () => {
                                                  await handleBatchDeleteModels(providerId);
                                                },
                                              });
                                            }}
                                          >
                                            {t('opencode.model.deleteSelected', { count: selectedModelCount })}
                                          </Button>
                                        )}
                                        <Tooltip title={connectivityTooltip}>
                                          <span>
                                            <Button
                                              size="small"
                                              type="text"
                                              style={{ fontSize: 12 }}
                                              onClick={() => handleOpenConnectivityTest(providerId)}
                                              disabled={!isConnectivitySupported || !isProviderAuthReady}
                                            >
                                              <ApiOutlined style={{ marginRight: 4 }} />
                                              {t('opencode.connectivity.button')}
                                            </Button>
                                          </span>
                                        </Tooltip>
                                        <Tooltip title={fetchModelsTooltip}>
                                          <span>
                                            <Button
                                              size="small"
                                              type="text"
                                              style={{ fontSize: 12 }}
                                              onClick={() => handleOpenFetchModels(providerId)}
                                              disabled={!isProviderAuthReady}
                                            >
                                              <CloudDownloadOutlined style={{ marginRight: 4 }} />
                                              {t('opencode.fetchModels.button')}
                                            </Button>
                                          </span>
                                        </Tooltip>
                                      </Space>
                                    }

                                    onAddModel={() => handleAddModel(providerId)}
                                    onEditModel={(modelId) => handleEditModel(providerId, modelId)}
                                    onCopyModel={(modelId) => handleCopyModel(providerId, modelId)}
                                    onDeleteModel={(modelId) => handleDeleteModel(providerId, modelId)}
                                    onSetPrimaryModel={(modelId) => handleSetPrimaryModel(providerId, modelId)}
                                    modelSelectionMode={isBatchDeleteMode}
                                    selectedModelIds={selectedModelIds}
                                    onToggleModelSelection={(modelId, selected) => handleToggleModelSelection(providerId, modelId, selected)}
                                    modelsDraggable={!isBatchDeleteMode}
                                    onReorderModels={(modelIds) => handleReorderModels(providerId, modelIds)}
                                    i18nPrefix="opencode"
                                  />
                                );
                              })}
                            </SortableContext>
                          </DndContext>
                        )}
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
                                {t('opencode.provider.importAllApiHub')}
                              </Button>
                            )}
                          </Space>
                        </div>
                      </Spin>
                    ),
                  },
                ]}
              />
            </div>

            <div
              id="opencode-global-prompt"
              className={styles.opencodeSection}
              data-opencode-sidebar-section="true"
              data-sidebar-title={t('opencode.prompt.title')}
              data-sidebar-order={7}
              style={{ order: 7 }}
            >
              <GlobalPromptSettings
                key={`opencode-global-prompt-${globalPromptExpandNonce}`}
                translationKeyPrefix="opencode.prompt"
                service={openCodePromptApi}
                collapseKey="opencode-prompt"
                refreshKey={openCodeConfigRefreshKey}
                defaultExpanded={globalPromptExpandNonce > 0}
                onUpdated={() => {
                  loadConfig();
                  incrementOpenCodeConfigRefresh();
                }}
              />
            </div>

            <div
              id="opencode-official-auth-channels"
              className={styles.opencodeSection}
              data-opencode-sidebar-section="true"
              data-sidebar-title={t('opencode.official.title')}
              data-sidebar-order={6}
              style={{ order: 6 }}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={officialProvidersCollapsed ? [] : ['official-providers']}
                onChange={(keys) => setOfficialProvidersCollapsed(!keys.includes('official-providers'))}
                items={[
                  {
                    key: 'official-providers',
                    label: (
                      <Space size={8}>
                        <Text strong>
                          <SafetyCertificateOutlined style={{ marginRight: 8 }} />
                          {t('opencode.official.title')}
                        </Text>
                        <Tooltip title={t('opencode.official.openConfigHint')}>
                          <Button
                            type="link"
                            size="small"
                            icon={<FileOutlined />}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleOpenAuthConfig();
                            }}
                            style={{ padding: 0, height: 'auto' }}
                          >
                            auth.json
                          </Button>
                        </Tooltip>
                      </Space>
                    ),
                    children: (
                      <div>
                        <div style={{ marginBottom: 12 }}>
                          <Text type="secondary" style={{ fontSize: 12 }}>
                            {t('opencode.official.description')}
                          </Text>
                        </div>
                        {authProvidersData ? (
                          authProvidersData.standaloneProviders.length > 0 ? (
                            authProvidersData.standaloneProviders.map((provider) => (
                              <OfficialProviderCard
                                key={provider.id}
                                id={provider.id}
                                name={provider.name}
                                models={provider.models}
                                i18nPrefix="opencode"
                                isDisabled={disabledProviderIds.has(provider.id)}
                                onToggleDisabled={() => handleToggleProviderDisabled(provider.id)}
                              />
                            ))
                          ) : (
                            <Empty description={t('opencode.official.noModels')} style={{ marginTop: 40 }} />
                          )
                        ) : (
                          <Spin spinning />
                        )}
                      </div>
                    ),
                  },
                ]}
              />
            </div>

            <div
              id="opencode-other-configuration"
              className={styles.opencodeSection}
              data-opencode-sidebar-section="true"
              data-sidebar-title={t('opencode.otherConfig.title')}
              data-sidebar-order={8}
              style={{ order: 8 }}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={otherConfigCollapsed ? [] : ['other']}
                onChange={(keys) => setOtherConfigCollapsed(!keys.includes('other'))}
                items={[
                  {
                    key: 'other',
                    label: <Text strong><ToolOutlined style={{ marginRight: 8 }} />{t('opencode.otherConfig.title')}</Text>,
                    children: (
                      <div>
                        <Form.Item
                          help={
                            <span>
                              <Text type="secondary">{t('opencode.otherConfig.hint')}，</Text>
                              <span style={{ color: '#1677ff' }}>
                                {t('opencode.otherConfig.autoSaveHint')}
                              </span>
                            </span>
                          }
                          style={{ marginBottom: 0 }}
                        >
                          <JsonEditor
                            value={otherConfigFields}
                            onChange={handleOtherConfigChange}
                            onBlur={handleOtherConfigBlur}
                            height={300}
                            minHeight={200}
                            maxHeight={500}
                            resizable
                            mode="text"
                            placeholder={`{
    "permission": "allow",
    "autoupdate": true
}`}
                          />
                        </Form.Item>
                      </div>
                    ),
                  },
                ]}
              />
            </div>

            <div
              id="opencode-session-manager"
              className={styles.opencodeSection}
              data-opencode-sidebar-section="true"
              data-sidebar-title={t('sessionManager.title')}
              data-sidebar-order={9}
              style={{ order: 9 }}
            >
              <SessionManagerPanel tool="opencode" expandNonce={sessionManagerExpandNonce} />
            </div>

            <ProviderFormModal
              open={providerModalOpen}
              isEdit={!!currentProviderId}
              initialValues={providerInitialValues}
              existingIds={currentProviderId ? [] : existingProviderIds}
              apiKeyRequired={false}
              onCancel={() => {
                setProviderModalOpen(false);
                setProviderInitialValues(undefined);
                setBaseProviderSnapshot(null);
              }}
              onSuccess={handleProviderSuccess}
              onDuplicateId={handleProviderDuplicateId}
              i18nPrefix="opencode"
              headersOutputFormat="object"
              showOpenCodeAdvanced={true}
              modelOptions={providerFilterOptions}
            />

            <ModelFormModal
              open={modelModalOpen}
              isEdit={!!currentModelId}
              initialValues={modelInitialValues}
              existingIds={currentModelId ? [] : existingModelIds}
              showOptions
              showVariants={true}
              showModalities={true}
              limitRequired={false}
              nameRequired={false}
              npmType={currentModelProviderId && config?.provider[currentModelProviderId]?.npm || '@ai-sdk/openai-compatible'}
              onCancel={() => {
                setModelModalOpen(false);
                setModelInitialValues(undefined);
                setBaseModelSnapshot(null);
              }}
              onSuccess={handleModelSuccess}
              onDuplicateId={handleModelDuplicateId}
              i18nPrefix="opencode"
            />

            <ConfigPathModal
              open={pathModalOpen}
              currentPathInfo={configPathInfo}
              onCancel={() => setPathModalOpen(false)}
              onSuccess={handlePathModalSuccess}
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
              providerFilter={(provider) => isFavoriteProviderForSource('opencode', provider)}
            />

            {allApiHubAvailable && (
              <ImportFromAllApiHubModal
                open={allApiHubImportModalOpen}
                onClose={() => setAllApiHubImportModalOpen(false)}
                onImport={handleImportAllApiHubProviders}
                existingProviderIds={existingProviderIds}
              />
            )}

            {connectivityProviderInfo && (
              <ConnectivityTestModal
                open={connectivityModalOpen}
                onCancel={() => setConnectivityModalOpen(false)}
                providerId={connectivityProviderId}
                providerName={connectivityProviderInfo.name}
                providerConfig={connectivityProviderInfo.config}
                modelIds={connectivityProviderInfo.modelIds}
                removableModelIds={connectivityProviderInfo.removableModelIds}
                diagnostics={connectivityProviderInfo.diagnostics}
                onSaveDiagnostics={handleSaveDiagnostics}
                onRemoveModels={handleRemoveModels}
              />
            )}

            {/* Preview Modal */}
            <JsonPreviewModal
              open={previewModalOpen}
              onClose={() => setPreviewModalOpen(false)}
              title={t('opencode.preview.title')}
              data={previewData}
            />

            <SidebarSettingsModal
              open={settingsModalOpen}
              onClose={() => setSettingsModalOpen(false)}
              sidebarVisible={!sidebarHidden}
              onSidebarVisibleChange={(visible) => setSidebarHidden('opencode', !visible)}
            >
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 16 }}>
                <div style={{ width: 180, paddingTop: 4, color: 'var(--color-text-primary)', fontWeight: 500 }}>
                  {t('opencode.ohMyOpenCode.clearAppliedEnable')}
                </div>
                <div style={{ flex: 1 }}>
                  <Switch
                    checked={opencodeAllowClearAppliedOhMyConfig}
                    onChange={(checked) => {
                      void setOpencodeAllowClearAppliedOhMyConfig(checked);
                    }}
                  />
                  <div style={{ marginTop: 6, color: 'var(--color-text-secondary)', fontSize: 12, lineHeight: 1.5 }}>
                    {t('opencode.ohMyOpenCode.clearAppliedEnableHint')}
                  </div>
                </div>
              </div>
            </SidebarSettingsModal>
          </div>
        </SectionSidebarLayout>
      )}
    </div>
  );
};

export default OpenCodePage;
