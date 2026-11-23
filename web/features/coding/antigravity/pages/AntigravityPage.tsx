import React from 'react';
import {
  Button,
  Collapse,
  Descriptions,
  Empty,
  Modal,
  Space,
  Spin,
  Typography,
  message,
} from 'antd';
import {
  CopyOutlined,
  DatabaseOutlined,
  EditOutlined,
  EllipsisOutlined,
  ExclamationCircleOutlined,
  EyeOutlined,
  FileTextOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  MessageOutlined,
  SyncOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import { TRAY_CONFIG_REFRESH_EVENT, DEEP_LINK_IMPORT_COMPLETED } from '@/constants/configEvents';
import { useSettingsStore } from '@/stores';
import { refreshTrayMenu } from '@/services/appApi';
import {
  applyAntigravityOfficialAccount,
  copyAntigravityOfficialAccountToken,
  deleteAntigravityOfficialAccount,
  getAntigravityCommonConfig,
  getAntigravityConfigPath,
  getAntigravityRootPathInfo,
  listAntigravityLocalRuleFiles,
  listAntigravityOfficialAccounts,
  getAntigravityOfficialProvider,
  readAntigravitySettings,
  refreshAntigravityOfficialAccountLimits,
  saveAntigravityCommonConfig,
  saveAntigravityOfficialLocalAccount,
  startAntigravityOfficialAccountOauth,
} from '@/services/antigravityApi';
import type { OfficialAccountPendingAction } from '@/features/coding/shared/officialAccounts';
import { antigravityPromptApi } from '@/services/antigravityPromptApi';
import type {
  ConfigPathInfo,
  AntigravityLocalRuleFile,
  AntigravityOfficialAccount,
  AntigravityProvider,
  AntigravitySettings,
} from '@/types/antigravity';
import type { GlobalPromptTargetFileOption } from '@/types/globalPrompt';
import AntigravityLocalRuleFiles from '../components/AntigravityLocalRuleFiles';
import AntigravityProviderCard from '../components/AntigravityProviderCard';
import {
  shouldLoadAntigravityOfficialAccounts,
} from '../utils/localProvider';

const { Title, Text, Link } = Typography;
const ACCOUNT_DETAILS_EMPTY_VALUE = '-';

function formatUnixTimestamp(timestamp?: number | null): string {
  if (!timestamp) return ACCOUNT_DETAILS_EMPTY_VALUE;
  const date = new Date(timestamp * 1000);
  return date.toLocaleString();
}

function formatDateTime(dateStr?: string | null): string {
  if (!dateStr) return ACCOUNT_DETAILS_EMPTY_VALUE;
  const date = new Date(dateStr);
  return Number.isNaN(date.getTime()) ? dateStr : date.toLocaleString();
}

function maskTokenPreview(tokenKind: 'access' | 'refresh', account: AntigravityOfficialAccount): string {
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

const AntigravityPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [officialProvider, setOfficialProvider] = React.useState<AntigravityProvider | null>(null);
  const [officialAccountsByProviderId, setOfficialAccountsByProviderId] = React.useState<
    Record<string, AntigravityOfficialAccount[]>
  >({});
  const [pendingAction, setPendingAction] = React.useState<OfficialAccountPendingAction | null>(null);
  const [oauthPending, setOauthPending] = React.useState(false);
  const oauthPendingRef = React.useRef(false);
  const [officialAccountDetails, setOfficialAccountDetails] = React.useState<{
    provider: AntigravityProvider;
    account: AntigravityOfficialAccount;
  } | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewData] = React.useState<AntigravitySettings | null>(null);
  const [localRuleFiles, setLocalRuleFiles] = React.useState<AntigravityLocalRuleFile[]>([]);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const sidebarHidden = sidebarHiddenByPage.antigravity ?? false;

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    { id: 'antigravity-official-account', title: t('antigravity.officialAccounts.title', { defaultValue: 'Antigravity CLI 账号' }), order: 1 },
    { id: 'antigravity-global-prompt', title: t('antigravity.prompts', { defaultValue: '全局提示词' }), order: 2 },
    { id: 'antigravity-session-manager', title: t('sessionManager.title', { defaultValue: '会话管理' }), order: 3 },
  ], [t]);

  const promptTargetFileOptions = React.useMemo<GlobalPromptTargetFileOption[]>(() => [
    {
      value: '~/.gemini/config/GEMINI.md',
      label: t('antigravity.promptRules.optionConfigGemini'),
      hint: t('antigravity.promptRules.optionConfigGeminiHint'),
    },
    {
      value: '~/.gemini/config/AGENTS.md',
      label: '~/.gemini/config/AGENTS.md',
    },
    {
      value: '~/.gemini/GEMINI.md',
      label: t('antigravity.promptRules.optionHomeGemini'),
      hint: t('antigravity.promptRules.optionHomeGeminiHint'),
    },
    {
      value: '~/.gemini/AGENTS.md',
      label: '~/.gemini/AGENTS.md',
    },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [path, nextRootPathInfo, nextOfficialProvider, nextLocalRuleFiles] = await Promise.all([
        getAntigravityConfigPath(),
        getAntigravityRootPathInfo(),
        getAntigravityOfficialProvider(),
        listAntigravityLocalRuleFiles().catch(() => [] as AntigravityLocalRuleFile[]),
      ]);
      setConfigPath(path);
      setRootPathInfo(nextRootPathInfo);
      setOfficialProvider(nextOfficialProvider);
      setLocalRuleFiles(nextLocalRuleFiles);
      const accounts = shouldLoadAntigravityOfficialAccounts(nextOfficialProvider)
        ? await listAntigravityOfficialAccounts(nextOfficialProvider.id)
        : [];
      setOfficialAccountsByProviderId({ [nextOfficialProvider.id]: accounts });
    } catch (error) {
      if (!silent) {
        console.error('Failed to load Antigravity config:', error);
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    if (isActive) {
      void loadConfig(true);
    }
  }, [isActive, loadConfig]);

  React.useEffect(() => {
    const handleTrayConfigRefresh = (event: Event) => {
      event.preventDefault();
      void loadConfig(true);
    };
    const handleDeepLinkImport = (event: Event) => {
      const detail = (event as CustomEvent<{ app?: string; id?: string }>).detail;
      if (detail?.app === 'antigravity') {
        void loadConfig(true);
      }
    };

    window.addEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    window.addEventListener(DEEP_LINK_IMPORT_COMPLETED, handleDeepLinkImport);
    return () => {
      window.removeEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
      window.removeEventListener(DEEP_LINK_IMPORT_COMPLETED, handleDeepLinkImport);
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
    translationKeyPrefix: 'antigravity',
    defaultConfig: '{}',
    rootDirectoryChangeLocked: false,
    loadConfig,
    getCommonConfig: getAntigravityCommonConfig,
    saveCommonConfig: saveAntigravityCommonConfig,
  });

  const handleStartOfficialAccountOauth = async (provider: AntigravityProvider) => {
    if (oauthPendingRef.current) return;
    oauthPendingRef.current = true;
    setOauthPending(true);
    try {
      message.loading({ content: t('antigravity.officialAccounts.loggingIn', { defaultValue: '正在启动 Antigravity CLI OAuth 登录...' }), key: 'antigravity-oauth', duration: 0 });
      await startAntigravityOfficialAccountOauth(provider.id);
      message.success({ content: t('antigravity.officialAccounts.loginSuccess', { defaultValue: 'Antigravity CLI 账号绑定成功！' }), key: 'antigravity-oauth' });
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to start official account oauth:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error({ content: errorMsg || t('common.error'), key: 'antigravity-oauth' });
    } finally {
      oauthPendingRef.current = false;
      setOauthPending(false);
    }
  };

  const handleSaveLocalOfficialAccount = async (provider: AntigravityProvider) => {
    const virtualAccount = (officialAccountsByProviderId[provider.id] || []).find(
      (account) => account.isVirtual,
    );
    setPendingAction(virtualAccount ? { accountId: virtualAccount.id, action: 'save' } : null);
    try {
      await saveAntigravityOfficialLocalAccount(provider.id);
      message.success(t('antigravity.officialAccounts.saveLocalSuccess', { defaultValue: '已保存当前本地登录' }));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save local official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setPendingAction(null);
    }
  };

  const handleApplyOfficialAccount = async (
    provider: AntigravityProvider,
    account: AntigravityOfficialAccount,
  ) => {
    setPendingAction({ accountId: account.id, action: 'apply' });
    try {
      await applyAntigravityOfficialAccount(provider.id, account.id);
      message.success(t('common.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to apply official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setPendingAction(null);
    }
  };

  const handleDeleteOfficialAccount = (
    provider: AntigravityProvider,
    account: AntigravityOfficialAccount,
  ) => {
    if (account.isApplied) {
      message.warning(t('antigravity.provider.appliedCannotDelete', { defaultValue: '已生效账号不能删除' }));
      return;
    }
    Modal.confirm({
      title: t('antigravity.provider.officialAccountDeleteConfirm', {
        name: account.email || account.projectId || account.name,
      }),
      icon: <ExclamationCircleOutlined />,
      okText: t('common.delete'),
      okType: 'danger',
      cancelText: t('common.cancel'),
      onOk: async () => {
        setPendingAction({ accountId: account.id, action: 'delete' });
        try {
          await deleteAntigravityOfficialAccount(provider.id, account.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete official account:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        } finally {
          setPendingAction(null);
        }
      },
    });
  };

  const handleRefreshOfficialAccount = async (
    provider: AntigravityProvider,
    account: AntigravityOfficialAccount,
  ) => {
    setPendingAction({ accountId: account.id, action: 'refresh' });
    try {
      const refreshed = await refreshAntigravityOfficialAccountLimits(provider.id, account.id);
      message.success(t('antigravity.officialAccounts.refreshSuccess', { defaultValue: '配额信息已刷新' }));
      if (account.isVirtual) {
        setOfficialAccountsByProviderId((previous) => ({
          ...previous,
          [provider.id]: (previous[provider.id] || []).map((item) =>
            item.id === account.id ? refreshed : item,
          ),
        }));
      } else {
        await loadConfig();
      }
    } catch (error) {
      console.error('Failed to refresh official account limits:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setPendingAction(null);
    }
  };

  const handleViewOfficialAccountDetails = (
    provider: AntigravityProvider,
    account: AntigravityOfficialAccount,
  ) => {
    setOfficialAccountDetails({ provider, account });
  };

  const handleCopyOfficialAccountToken = async (
    provider: AntigravityProvider,
    account: AntigravityOfficialAccount,
    tokenKind: 'access' | 'refresh',
  ) => {
    try {
      await copyAntigravityOfficialAccountToken(provider.id, account.id, tokenKind);
      message.success(t('common.copied', { defaultValue: '已复制到剪贴板' }));
    } catch (error) {
      console.error('Failed to copy token:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleOpenFolder = async () => {
    try {
      if (configPath) {
        await revealItemInDir(configPath);
      }
    } catch (error) {
      console.error('Failed to reveal Antigravity config path:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handlePreviewConfig = async () => {
    try {
      const data = await readAntigravitySettings();
      setPreviewData(data);
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview Antigravity config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  return (
    <SectionSidebarLayout
      sidebarTitle={t('antigravity.title', { defaultValue: 'Antigravity CLI 配置管理' })}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'antigravity-official-account':
            return <DatabaseOutlined />;
          case 'antigravity-global-prompt':
            return <FileTextOutlined />;
          case 'antigravity-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'antigravity-official-account':
            break;
          case 'antigravity-global-prompt':
            setPromptExpandNonce((value) => value + 1);
            break;
          case 'antigravity-session-manager':
            setSessionManagerExpandNonce((value) => value + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        {/* 页面头部 */}
        <div style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
            <div>
              <div style={{ marginBottom: 8 }}>
                <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                  {t('antigravity.title')}
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openUrl('https://antigravity.google/docs/cli/settings');
                  }}
                >
                  <LinkOutlined /> {t('antigravity.viewDocs')}
                </Link>
                <Link
                  type="secondary"
                  style={{ fontSize: 12, marginLeft: 16 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    void handlePreviewConfig();
                  }}
                >
                  <EyeOutlined /> {t('common.previewConfig')}
                </Link>
              </div>
              <Space size="small">
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('antigravity.configPath')}:
                </Text>
                <Text code style={{ fontSize: 12 }}>
                  {configPath || '~/.gemini/antigravity-cli/settings.json'}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => setRootDirectoryModalOpen(true)}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('antigravity.rootPathSource.customize')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={() => void handleOpenFolder()}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('antigravity.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<SyncOutlined />}
                  onClick={() => void loadConfig()}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('antigravity.refreshConfig')}
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
          id="antigravity-official-account"
          data-sidebar-section="true"
          data-sidebar-title={t('antigravity.officialAccounts.title', { defaultValue: 'Antigravity CLI 账号' })}
        >
          <Collapse
            style={{ marginBottom: 16 }}
            defaultActiveKey={['official-account']}
            items={[{
              key: 'official-account',
              label: (
                <Text strong>
                  <DatabaseOutlined style={{ marginRight: 8 }} />
                  {t('antigravity.officialAccounts.title', { defaultValue: 'Antigravity CLI 账号' })}
                </Text>
              ),
              children: officialProvider ? (
                <Spin spinning={loading}>
                  <AntigravityProviderCard
                    provider={officialProvider}
                    officialAccounts={officialAccountsByProviderId[officialProvider.id] || []}
                    onOfficialAccountLogin={handleStartOfficialAccountOauth}
                    onOfficialAccountSaveLocal={handleSaveLocalOfficialAccount}
                    onOfficialAccountApply={handleApplyOfficialAccount}
                    onOfficialAccountDelete={handleDeleteOfficialAccount}
                    onOfficialAccountRefresh={handleRefreshOfficialAccount}
                    onOfficialAccountViewDetails={handleViewOfficialAccountDetails}
                    pendingAction={pendingAction}
                    oauthPending={oauthPending}
                  />
                </Spin>
              ) : (
                <Empty description={t('antigravity.emptyText', { defaultValue: '正在初始化 Antigravity CLI 配置' })} />
              ),
            }]}
          />
        </div>

        <div
          id="antigravity-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('common.prompt.title')}
        >
          <GlobalPromptSettings
            key={`antigravity-prompt-${promptExpandNonce}`}
            toolName="Antigravity CLI"
            promptFileName="~/.gemini/config/GEMINI.md"
            service={antigravityPromptApi}
            collapseKey="antigravity-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
            targetFileOptions={promptTargetFileOptions}
            defaultTargetFile="~/.gemini/config/GEMINI.md"
            extraHint={
              <div>
                {t('antigravity.promptRules.layeringHint')}
              </div>
            }
            footer={<AntigravityLocalRuleFiles files={localRuleFiles} />}
          />
        </div>

        <div
          id="antigravity-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title', { defaultValue: '会话管理' })}
        >
          <SessionManagerPanel tool="antigravity" expandNonce={sessionManagerExpandNonce} />
        </div>

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
          title={t('antigravity.preview.currentConfigTitle', { defaultValue: '当前 Antigravity CLI 配置预览' })}
          data={previewData}
        />

        <Modal
          open={Boolean(officialAccountDetails)}
          title={t('antigravity.provider.officialAccountDetailsTitle', { defaultValue: '官方账号明细' })}
          onCancel={() => setOfficialAccountDetails(null)}
          footer={null}
          width={720}
        >
          {officialAccountDetails && (
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label={t('antigravity.provider.officialAccountLabel', { defaultValue: '账号' })}>
                {officialAccountDetails.account.email
                  || (officialAccountDetails.account.isVirtual
                    ? t('antigravity.provider.officialAccountLocal')
                    : officialAccountDetails.account.projectId || officialAccountDetails.account.name)}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccountProjectId', { defaultValue: 'Project ID' })}>
                {officialAccountDetails.account.projectId || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccountPlanType', { defaultValue: '订阅类型' })}>
                {officialAccountDetails.account.planType || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccount5hLimit', { defaultValue: '5h 额度' })}>
                {officialAccountDetails.account.limit5hText || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccount5hResetAt', { defaultValue: '5h 重置时间' })}>
                {formatUnixTimestamp(officialAccountDetails.account.limit5hResetAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccountWeeklyLimit', { defaultValue: '周额度' })}>
                {officialAccountDetails.account.limitWeeklyText || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccountWeeklyResetAt', { defaultValue: '周重置时间' })}>
                {formatUnixTimestamp(officialAccountDetails.account.limitWeeklyResetAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccountLastLimitRefreshAt', { defaultValue: '额度刷新时间' })}>
                {formatDateTime(officialAccountDetails.account.lastLimitsFetchedAt)}
              </Descriptions.Item>
              {officialAccountDetails.account.lastError && (
                <Descriptions.Item label={t('antigravity.provider.officialAccountLastError', { defaultValue: '最近错误' })}>
                  <Text type="danger" style={{ fontSize: 12 }}>
                    {officialAccountDetails.account.lastError}
                  </Text>
                </Descriptions.Item>
              )}
              <Descriptions.Item label={t('antigravity.provider.officialAccountTokenExpiresAt', { defaultValue: 'Token 过期时间' })}>
                {formatUnixTimestamp(officialAccountDetails.account.tokenExpiresAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccountAccessToken', { defaultValue: 'Access Token' })}>
                {renderTokenPreview(
                  maskTokenPreview('access', officialAccountDetails.account),
                  () => handleCopyOfficialAccountToken(
                    officialAccountDetails.provider,
                    officialAccountDetails.account,
                    'access',
                  ),
                )}
              </Descriptions.Item>
              <Descriptions.Item label={t('antigravity.provider.officialAccountRefreshToken', { defaultValue: 'Refresh Token' })}>
                {renderTokenPreview(
                  maskTokenPreview('refresh', officialAccountDetails.account),
                  () => handleCopyOfficialAccountToken(
                    officialAccountDetails.provider,
                    officialAccountDetails.account,
                    'refresh',
                  ),
                )}
              </Descriptions.Item>
            </Descriptions>
          )}
        </Modal>

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={(visible) => setSidebarHidden('antigravity', !visible)}
        />
      </div>
    </SectionSidebarLayout>
  );
};

export default AntigravityPage;
