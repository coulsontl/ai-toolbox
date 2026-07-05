import React from 'react';
import { Button, Collapse, Descriptions, Empty, Modal, Space, Spin, Typography, message } from 'antd';
import {
  AppstoreOutlined,
  CopyOutlined,
  DatabaseOutlined,
  EditOutlined,
  EllipsisOutlined,
  ExclamationCircleOutlined,
  EyeOutlined,
  FileTextOutlined,
  FolderOpenOutlined,
  MessageOutlined,
  PlusOutlined,
  SyncOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import {
  firstGatewayApiFormat,
  GatewayFailoverButton,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  providerNeedsGatewayProxy,
  saveProviderWithGatewayReengage,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import { useSettingsStore } from '@/stores';
import { refreshTrayMenu } from '@/services/appApi';
import {
  createGeminiCliProvider,
  deleteGeminiCliProvider,
  applyGeminiCliOfficialAccount,
  copyGeminiCliOfficialAccountToken,
  deleteGeminiCliOfficialAccount,
  getGeminiCliCommonConfig,
  getGeminiCliConfigPath,
  getGeminiCliRootPathInfo,
  listGeminiCliOfficialAccounts,
  listGeminiCliProviders,
  readGeminiCliSettings,
  refreshGeminiCliOfficialAccountLimits,
  reorderGeminiCliProviders,
  saveGeminiCliCommonConfig,
  saveGeminiCliOfficialLocalAccount,
  saveGeminiCliLocalConfig,
  selectGeminiCliProvider,
  startGeminiCliOfficialAccountOauth,
  toggleGeminiCliProviderDisabled,
  updateGeminiCliProvider,
} from '@/services/geminiCliApi';
import { geminiCliPromptApi } from '@/services/geminiCliPromptApi';
import type {
  ConfigPathInfo,
  GeminiCliOfficialAccount,
  GeminiCliProvider,
  GeminiCliProviderFormValues,
  GeminiCliProviderInput,
  GeminiCliSettings,
} from '@/types/geminicli';
import {
  engageProxyGatewayFailover,
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  type GatewayCliTakeoverStatus,
} from '@/services';
import GeminiCliProviderCard from '../components/GeminiCliProviderCard';
import GeminiCliProviderFormModal from '../components/GeminiCliProviderFormModal';
import GeminiCliCommonConfigModal from '../components/GeminiCliCommonConfigModal';
import {
  GEMINI_CLI_LOCAL_PROVIDER_ID,
  shouldLoadGeminiCliOfficialAccounts,
} from '../utils/localProvider';

const { Title, Text, Link } = Typography;
const ACCOUNT_DETAILS_EMPTY_VALUE = '-';

function maskTokenPreview(tokenKind: 'access' | 'refresh', account: GeminiCliOfficialAccount): string {
  const preview = tokenKind === 'access'
    ? account.accessTokenPreview
    : account.refreshTokenPreview;
  if (preview?.trim()) {
    return preview.trim();
  }
  return ACCOUNT_DETAILS_EMPTY_VALUE;
}

function renderTokenPreview(
  previewValue: string,
  onCopy: () => Promise<void>,
): React.ReactNode {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8, maxWidth: '100%' }}>
      <div
        style={{
          flex: 1,
          minWidth: 0,
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          fontFamily: 'ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, monospace',
          fontSize: 12,
        }}
      >
        {previewValue}
      </div>
      <Button
        type="text"
        size="small"
        icon={<CopyOutlined />}
        onClick={() => {
          void onCopy();
        }}
        style={{ height: 'auto', paddingInline: 4, flexShrink: 0 }}
      />
    </div>
  );
}

const GeminiCliPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [providers, setProviders] = React.useState<GeminiCliProvider[]>([]);
  const [officialAccountsByProviderId, setOfficialAccountsByProviderId] = React.useState<
    Record<string, GeminiCliOfficialAccount[]>
  >({});
  const [appliedProviderId, setAppliedProviderId] = React.useState('');
  const [gatewayCliStatus, setGatewayCliStatus] = React.useState<GatewayCliTakeoverStatus | null>(null);
  const gatewayTakeoverActive = Boolean(gatewayCliStatus?.can_restore_direct);
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const primaryGatewayProviderNeedsProxy = React.useMemo(() => {
    const primaryProvider = providers.find(
      (provider) => provider.id === gatewayCliStatus?.primary_provider_id,
    );
    if (!primaryProvider || primaryProvider.category === 'official' || primaryProvider.id === GEMINI_CLI_LOCAL_PROVIDER_ID) {
      return false;
    }
    const providerApiFormat = firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(primaryProvider.meta, 'gemini'),
      primaryProvider.meta?.apiFormat,
    );
    return providerNeedsGatewayProxy(providerApiFormat, 'gemini_native');
  }, [gatewayCliStatus?.primary_provider_id, gatewayProviderProfilesVersion, providers]);
  const [refreshingOfficialAccountId, setRefreshingOfficialAccountId] = React.useState<string | null>(null);
  const [savingOfficialAccountId, setSavingOfficialAccountId] = React.useState<string | null>(null);
  const [officialAccountDetails, setOfficialAccountDetails] = React.useState<{
    provider: GeminiCliProvider;
    account: GeminiCliOfficialAccount;
  } | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewData] = React.useState<GeminiCliSettings | null>(null);
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<GeminiCliProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const sidebarHidden = sidebarHiddenByPage.geminicli;

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    { id: 'geminicli-providers', title: t('geminicli.provider.title'), order: 1 },
    { id: 'geminicli-global-prompt', title: t('geminicli.prompt.title'), order: 2 },
    { id: 'geminicli-session-manager', title: t('sessionManager.title'), order: 3 },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [path, nextRootPathInfo, providerList] = await Promise.all([
        getGeminiCliConfigPath(),
        getGeminiCliRootPathInfo(),
        listGeminiCliProviders(),
      ]);
      setConfigPath(path);
      setRootPathInfo(nextRootPathInfo);
      setProviders(providerList);
      const officialAccountEntries = await Promise.all(
        providerList.map(async (provider) => [
          provider.id,
          shouldLoadGeminiCliOfficialAccounts(provider)
            ? await listGeminiCliOfficialAccounts(provider.id)
            : [],
        ] as const),
      );
      setOfficialAccountsByProviderId(Object.fromEntries(officialAccountEntries));
      setAppliedProviderId(providerList.find((provider) => provider.isApplied)?.id || '');
    } catch (error) {
      console.error('Failed to load Gemini CLI config:', error);
      if (!silent) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    void loadConfig();
  }, [loadConfig]);

  const hasInitializedRef = React.useRef(false);
  React.useEffect(() => {
    if (!isActive) {
      hasInitializedRef.current = true;
      return;
    }
    if (hasInitializedRef.current) {
      void loadConfig(true);
    }
  }, [isActive, loadConfig]);

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
    translationKeyPrefix: 'geminicli',
    defaultConfig: '{}',
    rootDirectoryChangeLocked: gatewayTakeoverActive,
    rootDirectoryChangeLockedText: t('gateway.proxy.rootDirectorySaveLockedTooltip'),
    loadConfig,
    getCommonConfig: getGeminiCliCommonConfig,
    saveCommonConfig: saveGeminiCliCommonConfig,
  });

  const handleOpenFolder = async () => {
    if (!configPath) return;

    try {
      await revealItemInDir(configPath);
    } catch {
      try {
        const parentDir = configPath.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open Gemini CLI folder:', error);
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    }
  };

  const handlePreviewCurrentConfig = async () => {
    try {
      const settings = await readGeminiCliSettings();
      setPreviewData(settings);
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview Gemini CLI config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const oldIndex = providers.findIndex((provider) => provider.id === active.id);
    const newIndex = providers.findIndex((provider) => provider.id === over.id);
    if (oldIndex < 0 || newIndex < 0) return;

    const oldProviders = [...providers];
    const newProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(newProviders);

    try {
      await reorderGeminiCliProviders(newProviders.map((provider) => provider.id));
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to reorder Gemini CLI providers:', error);
      setProviders(oldProviders);
      message.error(t('common.error'));
    }
  };

  const handleAddProvider = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: GeminiCliProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: GeminiCliProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name}_copy`,
      isApplied: false,
    });
    setIsCopyMode(true);
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = (provider: GeminiCliProvider) => {
    Modal.confirm({
      title: t('geminicli.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteGeminiCliProvider(provider.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete Gemini CLI provider:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        }
      },
    });
  };

  const handleSelectProvider = async (provider: GeminiCliProvider) => {
    try {
      await selectGeminiCliProvider(provider.id);
      message.success(t('geminicli.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to select Gemini CLI provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleStartOfficialAccountOauth = async (provider: GeminiCliProvider) => {
    try {
      await startGeminiCliOfficialAccountOauth(provider.id);
      message.success(t('geminicli.provider.officialAccountOauthSuccess'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to start Gemini official account OAuth:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleApplyOfficialAccount = async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    try {
      await applyGeminiCliOfficialAccount(provider.id, account.id);
      message.success(t('geminicli.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to apply Gemini official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleSaveOfficialLocalAccount = async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    try {
      setSavingOfficialAccountId(account.id);
      await saveGeminiCliOfficialLocalAccount(provider.id);
      if (officialAccountDetails?.account.id === account.id) {
        setOfficialAccountDetails(null);
      }
      message.success(t('geminicli.provider.officialAccountSaveSuccess'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Gemini local official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setSavingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleDeleteOfficialAccount = (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    Modal.confirm({
      title: t('geminicli.provider.officialAccountDeleteConfirm', {
        name: account.email || account.projectId || account.name,
      }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteGeminiCliOfficialAccount(provider.id, account.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete Gemini official account:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        }
      },
    });
  };

  const handleRefreshOfficialAccount = async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    try {
      setRefreshingOfficialAccountId(account.id);
      const refreshedAccount = await refreshGeminiCliOfficialAccountLimits(provider.id, account.id);
      setOfficialAccountsByProviderId((previous) => ({
        ...previous,
        [provider.id]: (previous[provider.id] || []).map((currentAccount) =>
          currentAccount.id === refreshedAccount.id ? refreshedAccount : currentAccount,
        ),
      }));
      setOfficialAccountDetails((current) => {
        if (!current || current.account.id !== refreshedAccount.id) {
          return current;
        }
        return {
          provider: current.provider,
          account: refreshedAccount,
        };
      });
      message.success(t('geminicli.provider.officialAccountRefreshSuccess'));
    } catch (error) {
      console.error('Failed to refresh Gemini official account quota:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setRefreshingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleViewOfficialAccountDetails = (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    setOfficialAccountDetails({ provider, account });
  };

  const handleCopyOfficialAccountToken = React.useCallback(async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
    tokenKind: 'access' | 'refresh',
  ) => {
    try {
      await copyGeminiCliOfficialAccountToken(provider.id, account.id, tokenKind);
      message.success(t('common.copied'));
    } catch (error) {
      console.error('Failed to copy Gemini official account token:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  }, [t]);

  const formatDateTime = React.useCallback((value?: string | null) => {
    if (!value) {
      return ACCOUNT_DETAILS_EMPTY_VALUE;
    }
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }
    return date.toLocaleString();
  }, []);

  const formatUnixTimestamp = React.useCallback((value?: number | null) => {
    if (value == null) {
      return ACCOUNT_DETAILS_EMPTY_VALUE;
    }
    return new Date(value * 1000).toLocaleString();
  }, []);

  const handleToggleDisabled = async (provider: GeminiCliProvider, isDisabled: boolean) => {
    try {
      await toggleGeminiCliProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('geminicli.providerDisabled') : t('geminicli.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle Gemini CLI provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleProviderSubmit = async (values: GeminiCliProviderFormValues) => {
    try {
      const providerInput: GeminiCliProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig: values.settingsConfig,
        meta: values.meta,
        notes: values.notes,
      };

      const isLocalTemp = editingProvider?.id === GEMINI_CLI_LOCAL_PROVIDER_ID;
      const gatewayModeBeforeSave = gatewayCliStatus?.mode;
      const shouldReengageGatewayProxy =
        Boolean(editingProvider && !isCopyMode && !isLocalTemp && editingProvider.isApplied) &&
        (gatewayModeBeforeSave === 'single' || gatewayModeBeforeSave === 'failover');

      await saveProviderWithGatewayReengage({
        gatewayMode: shouldReengageGatewayProxy ? gatewayModeBeforeSave : null,
        restoreDirect: () => restoreProxyGatewayCliDirect('gemini'),
        engageSingle: () => engageProxyGatewaySingle('gemini', editingProvider?.id || ''),
        engageFailover: () => engageProxyGatewayFailover('gemini'),
        onGatewayStatusChange: setGatewayCliStatus,
        saveProvider: async () => {
          if (isLocalTemp) {
            await saveGeminiCliLocalConfig({ provider: providerInput });
          } else if (editingProvider && !isCopyMode) {
            await updateGeminiCliProvider({
              ...editingProvider,
              name: values.name,
              category: values.category,
              settingsConfig: values.settingsConfig,
              meta: values.meta,
              notes: values.notes,
            });
          } else {
            await createGeminiCliProvider(providerInput);
          }
        },
      });

      message.success(t('common.success'));
      setProviderModalOpen(false);
      setIsCopyMode(false);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Gemini CLI provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
      throw error;
    }
  };

  return (
    <SectionSidebarLayout
      sidebarTitle={t('geminicli.title')}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'geminicli-providers':
            return <DatabaseOutlined />;
          case 'geminicli-global-prompt':
            return <FileTextOutlined />;
          case 'geminicli-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'geminicli-providers':
            setProviderListCollapsed(false);
            break;
          case 'geminicli-global-prompt':
            setPromptExpandNonce((value) => value + 1);
            break;
          case 'geminicli-session-manager':
            setSessionManagerExpandNonce((value) => value + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        <div style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
            <div>
              <div style={{ marginBottom: 8 }}>
                <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                  {t('geminicli.title')}
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(event) => {
                    event.stopPropagation();
                    void openUrl('https://github.com/google-gemini/gemini-cli');
                  }}
                >
                  {t('geminicli.viewDocs')}
                </Link>
                {appliedProviderId && (
                  <Link
                    type="secondary"
                    style={{ fontSize: 12, marginLeft: 16 }}
                    onClick={(event) => {
                      event.stopPropagation();
                      void handlePreviewCurrentConfig();
                    }}
                  >
                    <EyeOutlined /> {t('common.previewConfig')}
                  </Link>
                )}
              </div>
              <Space size="small" wrap>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('geminicli.configPath')}:
                </Text>
                <Text code style={{ fontSize: 12 }}>
                  {configPath || '~/.gemini/settings.json'}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => setRootDirectoryModalOpen(true)}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('geminicli.rootPathSource.customize')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={() => void handleOpenFolder()}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('geminicli.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<SyncOutlined />}
                  onClick={() => void loadConfig()}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('geminicli.refreshConfig')}
                </Button>
              </Space>
            </div>

            <Space>
              <Button type="text" icon={<EllipsisOutlined />} onClick={() => setSettingsModalOpen(true)}>
                {t('common.moreOptions')}
              </Button>
            </Space>
          </div>
        </div>

        <div
          id="geminicli-providers"
          data-sidebar-section="true"
          data-sidebar-title={t('geminicli.provider.title')}
        >
          <Collapse
            style={{ marginBottom: 16 }}
            activeKey={providerListCollapsed ? [] : ['providers']}
            onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
            items={[
              {
                key: 'providers',
                label: (
                  <Space size={8} wrap>
                    <Text strong>
                      <DatabaseOutlined style={{ marginRight: 8 }} />
                      {t('geminicli.provider.title')}
                    </Text>
                    <GatewayFailoverButton
                      cliKey="gemini"
                      status={gatewayCliStatus}
                      primaryProviderNeedsGatewayProxy={primaryGatewayProviderNeedsProxy}
                      onStatusChange={setGatewayCliStatus}
                    />
                  </Space>
                ),
                extra: (
                  <Space size={4}>
                    <Button
                      type="link"
                      size="small"
                      style={{ fontSize: 12 }}
                      icon={<AppstoreOutlined />}
                      onClick={(event) => {
                        event.stopPropagation();
                        setCommonConfigModalOpen(true);
                      }}
                    >
                      {t('geminicli.commonConfigButton')}
                    </Button>
                    <Button
                      type="link"
                      size="small"
                      style={{ fontSize: 12 }}
                      icon={<PlusOutlined />}
                      onClick={(event) => {
                        event.stopPropagation();
                        handleAddProvider();
                      }}
                    >
                      {t('geminicli.addProvider')}
                    </Button>
                  </Space>
                ),
                children: (
                  <Spin spinning={loading}>
                    <div
                      style={{
                        fontSize: 12,
                        color: 'var(--color-text-secondary)',
                        borderLeft: '2px solid var(--color-border)',
                        paddingLeft: 8,
                        marginBottom: 12,
                      }}
                    >
                      <div>{t('geminicli.pageHint')}</div>
                      <div>{t('geminicli.pageWarning')}</div>
                    </div>

                    {providers.length === 0 ? (
                      <Empty description={t('geminicli.emptyText')} style={{ marginTop: 40 }} />
                    ) : (
                      <DndContext
                        sensors={sensors}
                        collisionDetection={closestCenter}
                        onDragEnd={handleDragEnd}
                        modifiers={[restrictToVerticalAxis]}
                      >
                        <SortableContext
                          items={providers.map((provider) => provider.id)}
                          strategy={verticalListSortingStrategy}
                        >
                          <div>
                            {providers.map((provider) => (
                              <GeminiCliProviderCard
                                key={provider.id}
                                provider={provider}
                                isApplied={provider.id === appliedProviderId}
                                onEdit={handleEditProvider}
                                onDelete={handleDeleteProvider}
                                onCopy={handleCopyProvider}
                                onSelect={handleSelectProvider}
                                onToggleDisabled={handleToggleDisabled}
                                officialAccounts={officialAccountsByProviderId[provider.id] || []}
                                onOfficialAccountLogin={handleStartOfficialAccountOauth}
                                onOfficialLocalAccountSave={handleSaveOfficialLocalAccount}
                                onOfficialAccountApply={handleApplyOfficialAccount}
                                onOfficialAccountDelete={handleDeleteOfficialAccount}
                                onOfficialAccountRefresh={handleRefreshOfficialAccount}
                                onOfficialAccountViewDetails={handleViewOfficialAccountDetails}
                                refreshingOfficialAccountId={refreshingOfficialAccountId}
                                savingOfficialAccountId={savingOfficialAccountId}
                                gatewayTakeoverActive={gatewayTakeoverActive}
                                gatewayStatus={gatewayCliStatus}
                                onGatewayStatusChange={async (status) => {
                                  setGatewayCliStatus(status);
                                  await loadConfig();
                                }}
                              />
                            ))}
                          </div>
                        </SortableContext>
                      </DndContext>
                    )}
                  </Spin>
                ),
              },
            ]}
          />
        </div>

        <div
          id="geminicli-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('geminicli.prompt.title')}
        >
          <GlobalPromptSettings
            key={`geminicli-prompt-${promptExpandNonce}`}
            translationKeyPrefix="geminicli.prompt"
            service={geminiCliPromptApi}
            collapseKey="geminicli-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <div
          id="geminicli-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title')}
        >
          <SessionManagerPanel tool="geminicli" expandNonce={sessionManagerExpandNonce} />
        </div>

        <GeminiCliProviderFormModal
          open={providerModalOpen}
          provider={editingProvider}
          isCopy={isCopyMode}
          onCancel={() => {
            setProviderModalOpen(false);
            setEditingProvider(null);
            setIsCopyMode(false);
          }}
          onSubmit={handleProviderSubmit}
        />

        <GeminiCliCommonConfigModal
          open={commonConfigModalOpen}
          onCancel={() => setCommonConfigModalOpen(false)}
          onSuccess={() => {
            void loadConfig();
            void refreshTrayMenu();
          }}
          gatewaySaveLocked={gatewayTakeoverActive}
        />

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(rootPathInfo)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        <JsonPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('geminicli.preview.currentConfigTitle')}
          data={previewData}
        />

        <Modal
          open={Boolean(officialAccountDetails)}
          title={t('geminicli.provider.officialAccountDetailsTitle')}
          onCancel={() => setOfficialAccountDetails(null)}
          footer={null}
          width={720}
        >
          {officialAccountDetails && (
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label={t('geminicli.provider.officialAccountLabel')}>
                {officialAccountDetails.account.email
                  || officialAccountDetails.account.projectId
                  || officialAccountDetails.account.name}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountProjectId')}>
                {officialAccountDetails.account.projectId || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountPlanType')}>
                {officialAccountDetails.account.planType || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountWeeklyLimit')}>
                {officialAccountDetails.account.limitWeeklyText || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountWeeklyResetAt')}>
                {formatUnixTimestamp(officialAccountDetails.account.limitWeeklyResetAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountLastLimitRefreshAt')}>
                {formatDateTime(officialAccountDetails.account.lastLimitsFetchedAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountTokenExpiresAt')}>
                {formatUnixTimestamp(officialAccountDetails.account.tokenExpiresAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountAccessToken')}>
                {renderTokenPreview(
                  maskTokenPreview('access', officialAccountDetails.account),
                  () => handleCopyOfficialAccountToken(
                    officialAccountDetails.provider,
                    officialAccountDetails.account,
                    'access',
                  ),
                )}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountRefreshToken')}>
                {renderTokenPreview(
                  maskTokenPreview('refresh', officialAccountDetails.account),
                  () => handleCopyOfficialAccountToken(
                    officialAccountDetails.provider,
                    officialAccountDetails.account,
                    'refresh',
                  ),
                )}
              </Descriptions.Item>
              {officialAccountDetails.account.lastError && (
                <Descriptions.Item label={t('geminicli.provider.officialAccountLastErrorLabel')}>
                  <Text type="danger">{officialAccountDetails.account.lastError}</Text>
                </Descriptions.Item>
              )}
            </Descriptions>
          )}
        </Modal>

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={(visible) => setSidebarHidden('geminicli', !visible)}
        />
      </div>
    </SectionSidebarLayout>
  );
};

export default GeminiCliPage;
