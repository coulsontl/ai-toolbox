import React from 'react';
import { Typography, Card, Button, Space, Empty, message, Modal, Spin, Switch, Tooltip } from 'antd';
import { PlusOutlined, FolderOpenOutlined, SettingOutlined, SyncOutlined, ExclamationCircleOutlined, LinkOutlined, EyeOutlined, QuestionCircleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useNavigate, useLocation } from 'react-router-dom';
import type {
  ClaudeCodeProvider,
  ClaudeProviderFormValues,
  ImportConflictInfo,
  ImportConflictAction,
  ClaudePluginStatus,
} from '@/types/claudecode';
import {
  getClaudeConfigPath,
  listClaudeProviders,
  createClaudeProvider,
  updateClaudeProvider,
  deleteClaudeProvider,
  selectClaudeProvider,
  applyClaudeConfig,
  revealClaudeConfigFolder,
  getClaudeCommonConfig,
  readClaudeSettings,
  getClaudePluginStatus,
  applyClaudePluginConfig,
} from '@/services/claudeCodeApi';
import { usePreviewStore, useAppStore, useRefreshStore } from '@/stores';
import ClaudeProviderCard from '../components/ClaudeProviderCard';
import ClaudeProviderFormModal from '../components/ClaudeProviderFormModal';
import CommonConfigModal from '../components/CommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';

const { Title, Text, Link } = Typography;

interface SettingsConfig {
  env?: {
    ANTHROPIC_API_KEY?: string;
    ANTHROPIC_BASE_URL?: string;
    ANTHROPIC_AUTH_TOKEN?: string;
  };
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
  [key: string]: unknown;
}

function mergeClaudeConfig(commonConfig: Record<string, unknown>, providerConfig: SettingsConfig): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  const env: Record<string, unknown> = {};

  const commonEnv = commonConfig.env as Record<string, unknown> | undefined;

  if (providerConfig.env) {
    if (providerConfig.env.ANTHROPIC_API_KEY) {
      env.ANTHROPIC_API_KEY = providerConfig.env.ANTHROPIC_API_KEY;
    }
    if (providerConfig.env.ANTHROPIC_BASE_URL) {
      env.ANTHROPIC_BASE_URL = providerConfig.env.ANTHROPIC_BASE_URL;
    }
    if (providerConfig.env.ANTHROPIC_AUTH_TOKEN) {
      env.ANTHROPIC_AUTH_TOKEN = providerConfig.env.ANTHROPIC_AUTH_TOKEN;
    }
  }

  if (providerConfig.model) {
    env.ANTHROPIC_MODEL = providerConfig.model;
  }
  if (providerConfig.haikuModel) {
    env.ANTHROPIC_DEFAULT_HAIKU_MODEL = providerConfig.haikuModel;
  }
  if (providerConfig.sonnetModel) {
    env.ANTHROPIC_DEFAULT_SONNET_MODEL = providerConfig.sonnetModel;
  }
  if (providerConfig.opusModel) {
    env.ANTHROPIC_DEFAULT_OPUS_MODEL = providerConfig.opusModel;
  }

  if (commonEnv) {
    for (const [key, value] of Object.entries(commonEnv)) {
      if (!(key in env)) {
        env[key] = value;
      }
    }
  }

  result.env = env;

  for (const [key, value] of Object.entries(commonConfig)) {
    if (key !== 'env') {
      result[key] = value;
    }
  }

  return result;
}

const ClaudeCodePage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { setPreviewData } = usePreviewStore();
  const appStoreState = useAppStore.getState();
  const { claudeProviderRefreshKey } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [providers, setProviders] = React.useState<ClaudeCodeProvider[]>([]);
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');
  const [pluginStatus, setPluginStatus] = React.useState<ClaudePluginStatus>({
    enabled: false,
    hasConfigFile: false,
  });
  const [pluginLoading, setPluginLoading] = React.useState(false);

  // 模态框状态
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<ClaudeCodeProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [modalDefaultTab, setModalDefaultTab] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<ClaudeProviderFormValues | null>(null);

  // 加载配置（on mount and when refresh key changes）
  React.useEffect(() => {
    loadConfig();
    loadPluginStatus();
  }, [claudeProviderRefreshKey]);

  const loadConfig = async () => {
    setLoading(true);
    try {
      const [path, providerList] = await Promise.all([
        getClaudeConfigPath(),
        listClaudeProviders(),
      ]);

      setConfigPath(path);
      setProviders(providerList);

      const applied = providerList.find((p) => p.isApplied);
      setAppliedProviderId(applied?.id || '');
    } catch (error) {
      console.error('Failed to load config:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const handleOpenFolder = async () => {
    try {
      await revealClaudeConfigFolder();
    } catch (error) {
      console.error('Failed to open folder:', error);
      message.error(t('common.error'));
    }
  };

  const loadPluginStatus = async () => {
    try {
      const status = await getClaudePluginStatus();
      setPluginStatus(status);
    } catch (error) {
      console.error('Failed to load plugin status:', error);
    }
  };

  const handlePluginToggle = async (enabled: boolean) => {
    setPluginLoading(true);
    try {
      await applyClaudePluginConfig(enabled);
      setPluginStatus({ ...pluginStatus, enabled });
      message.success(enabled ? t('claudecode.plugin.enabled') : t('claudecode.plugin.disabled'));
    } catch (error) {
      console.error('Failed to toggle plugin config:', error);
      message.error(t('common.error'));
    } finally {
      setPluginLoading(false);
    }
  };

  const handleSelectProvider = async (provider: ClaudeCodeProvider) => {
    try {
      await selectClaudeProvider(provider.id);
      await applyClaudeConfig(provider.id);
      message.success(t('claudecode.apply.success'));
      await loadConfig();
    } catch (error) {
      console.error('Failed to select provider:', error);
      message.error(t('common.error'));
    }
  };

  const handleAddProvider = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setModalDefaultTab('manual');
    setProviderModalOpen(true);
  };

  const handleImportFromOpenCode = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setModalDefaultTab('import');
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: ClaudeCodeProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setModalDefaultTab('manual');
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: ClaudeCodeProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name}_copy`,
      isApplied: false,
    });
    setIsCopyMode(true);
    setModalDefaultTab('manual');
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = (provider: ClaudeCodeProvider) => {
    Modal.confirm({
      title: t('claudecode.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteClaudeProvider(provider.id);
          message.success(t('common.success'));
          await loadConfig();
        } catch (error) {
          console.error('Failed to delete provider:', error);
          message.error(t('common.error'));
        }
      },
    });
  };

  const handleProviderSubmit = async (values: ClaudeProviderFormValues) => {
    // 检查是否有冲突
    if (values.sourceProviderId && !editingProvider) {
      const existingProvider = providers.find(
        (p) => p.sourceProviderId === values.sourceProviderId
      );
      
      if (existingProvider) {
        // 显示冲突对话框
        setConflictInfo({
          existingProvider,
          newProviderName: values.name,
          sourceProviderId: values.sourceProviderId,
        });
        setPendingFormValues(values);
        setConflictDialogOpen(true);
        return;
      }
    }

    // 没有冲突，直接保存
    await doSaveProvider(values);
  };

  const handleConflictResolve = async (action: ImportConflictAction) => {
    if (!pendingFormValues || !conflictInfo) return;

    if (action === 'cancel') {
      setConflictDialogOpen(false);
      setConflictInfo(null);
      setPendingFormValues(null);
      return;
    }

    if (action === 'overwrite') {
      // 覆盖现有配置
      await doUpdateProvider(conflictInfo.existingProvider.id, pendingFormValues);
    } else {
      // 创建副本
      await doSaveProvider({
        ...pendingFormValues,
        sourceProviderId: undefined, // 移除 sourceProviderId 以避免再次冲突
      });
    }

    setConflictDialogOpen(false);
    setConflictInfo(null);
    setPendingFormValues(null);
  };

  const doSaveProvider = async (values: ClaudeProviderFormValues) => {
    try {
      // 生成唯一的 provider ID
      const generateId = (name: string): string => {
        const timestamp = Date.now().toString(36);
        const random = Math.random().toString(36).substring(2, 8);
        const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-');
        return `${slug}-${timestamp}-${random}`;
      };

      const settingsConfigObj: Record<string, unknown> = {
        env: {
          ANTHROPIC_BASE_URL: values.baseUrl,
          ANTHROPIC_API_KEY: values.apiKey,
        },
      };

      if (values.model) settingsConfigObj.model = values.model;
      if (values.haikuModel) settingsConfigObj.haikuModel = values.haikuModel;
      if (values.sonnetModel) settingsConfigObj.sonnetModel = values.sonnetModel;
      if (values.opusModel) settingsConfigObj.opusModel = values.opusModel;

      // 复制模式下创建新供应商，编辑模式下更新
      if (editingProvider && !isCopyMode) {
        await updateClaudeProvider({
          id: editingProvider.id,
          name: values.name,
          category: values.category,
          settingsConfig: JSON.stringify(settingsConfigObj),
          sourceProviderId: values.sourceProviderId,
          notes: values.notes,
          isApplied: editingProvider.isApplied,
          createdAt: editingProvider.createdAt,
          updatedAt: editingProvider.updatedAt,
        });
      } else {
        await createClaudeProvider({
          id: generateId(values.name),
          name: values.name,
          category: values.category,
          settingsConfig: JSON.stringify(settingsConfigObj),
          sourceProviderId: values.sourceProviderId,
          notes: values.notes,
          isApplied: false,
        });
      }

      message.success(t('common.success'));
      setProviderModalOpen(false);
      setIsCopyMode(false);
      await loadConfig();
    } catch (error) {
      console.error('Failed to save provider:', error);
      message.error(t('common.error'));
      throw error;
    }
  };

  const doUpdateProvider = async (id: string, values: ClaudeProviderFormValues) => {
    try {
      const existingProvider = providers.find((p) => p.id === id);
      if (!existingProvider) return;

      const settingsConfigObj: Record<string, unknown> = {
        env: {
          ANTHROPIC_BASE_URL: values.baseUrl,
          ANTHROPIC_API_KEY: values.apiKey,
        },
      };

      if (values.model) settingsConfigObj.model = values.model;
      if (values.haikuModel) settingsConfigObj.haikuModel = values.haikuModel;
      if (values.sonnetModel) settingsConfigObj.sonnetModel = values.sonnetModel;
      if (values.opusModel) settingsConfigObj.opusModel = values.opusModel;

      const providerData: ClaudeCodeProvider = {
        ...existingProvider,
        name: values.name,
        category: values.category,
        settingsConfig: JSON.stringify(settingsConfigObj),
        notes: values.notes,
        createdAt: existingProvider.createdAt,
        updatedAt: existingProvider.updatedAt,
      };

      await updateClaudeProvider(providerData);
      message.success(t('common.success'));
      setProviderModalOpen(false);
      await loadConfig();
    } catch (error) {
      console.error('Failed to update provider:', error);
      message.error(t('common.error'));
      throw error;
    }
  };

  const handlePreviewCurrentConfig = async () => {
    try {
      const settings = await readClaudeSettings();
      const finalConfig: Record<string, unknown> = { ...settings };

      appStoreState.setCurrentModule('coding');
      appStoreState.setCurrentSubTab('claudecode');
      setPreviewData(t('claudecode.preview.currentConfigTitle'), finalConfig, location.pathname);
      navigate('/preview/config');
    } catch (error) {
      console.error('Failed to preview config:', error);
      message.error(t('common.error'));
    }
  };

  const handlePreviewProvider = async (provider: ClaudeCodeProvider) => {
    try {
      if (provider.isApplied) {
        const settings = await readClaudeSettings();
        const finalConfig: Record<string, unknown> = { ...settings };

        appStoreState.setCurrentModule('coding');
        appStoreState.setCurrentSubTab('claudecode');
        setPreviewData(t('claudecode.preview.providerConfigTitle', { name: provider.name }), finalConfig, location.pathname);
        navigate('/preview/config');
      } else {
        const commonConfig = await getClaudeCommonConfig();
        let commonConfigObj: Record<string, unknown> = {};
        if (commonConfig?.config) {
          try {
            commonConfigObj = JSON.parse(commonConfig.config);
          } catch (e) {
            console.error('Failed to parse common config:', e);
          }
        }

        const providerConfig = JSON.parse(provider.settingsConfig) as SettingsConfig;
        const finalConfig = mergeClaudeConfig(commonConfigObj, providerConfig);

        appStoreState.setCurrentModule('coding');
        appStoreState.setCurrentSubTab('claudecode');
        setPreviewData(t('claudecode.preview.providerConfigTitle', { name: provider.name }), finalConfig, location.pathname);
        navigate('/preview/config');
      }
    } catch (error) {
      console.error('Failed to preview provider config:', error);
      message.error(t('common.error'));
    }
  };

  return (
    <div>
      {/* 页面头部 */}
      <div style={{ marginBottom: 16 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
          <div>
            <div style={{ marginBottom: 8 }}>
              <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                {t('claudecode.title')}
              </Title>
              <Link
                type="secondary"
                style={{ fontSize: 12 }}
                onClick={(e) => {
                  e.stopPropagation();
                  openUrl('https://code.claude.com/docs/en/settings#environment-variables');
                }}
              >
                <LinkOutlined /> {t('claudecode.viewDocs')}
              </Link>
              {appliedProviderId && (
                <Link
                  type="secondary"
                  style={{ fontSize: 12, marginLeft: 16 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    handlePreviewCurrentConfig();
                  }}
                >
                  <EyeOutlined /> {t('common.previewConfig')}
                </Link>
              )}
            </div>
            <Space>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t('claudecode.configPath')}:
              </Text>
              <Text code style={{ fontSize: 12 }}>
                {configPath || '~/.claude/settings.json'}
              </Text>
              <Button
                type="link"
                size="small"
                icon={<FolderOpenOutlined />}
                onClick={handleOpenFolder}
                style={{ padding: 0, fontSize: 12 }}
              >
                {t('claudecode.openFolder')}
              </Button>
            </Space>
          </div>

          <Space>
            <Tooltip title={t('claudecode.plugin.tooltip')}>
              <span>
                <Switch
                  checked={pluginStatus.enabled}
                  loading={pluginLoading}
                  onChange={handlePluginToggle}
                  checkedChildren="VSCode"
                  unCheckedChildren="VSCode"
                  disabled={!pluginStatus.hasConfigFile && !pluginStatus.enabled}
                />
              </span>
            </Tooltip>
            <Button icon={<SettingOutlined />} onClick={() => setCommonConfigModalOpen(true)}>
              {t('claudecode.commonConfigButton')}
            </Button>
          </Space>
        </div>
      </div>

      {/* 操作栏 */}
      <div style={{ marginBottom: 16 }}>
        <Space>
          <Button icon={<SyncOutlined />} onClick={handleImportFromOpenCode}>
            {t('claudecode.importFromOpenCode')}
          </Button>
          <Button type="primary" icon={<PlusOutlined />} onClick={handleAddProvider}>
            {t('claudecode.addProvider')}
          </Button>
        </Space>
      </div>

      {/* Provider 列表 */}
      <Spin spinning={loading}>
        {providers.length === 0 ? (
          <Card>
            <Empty description={t('claudecode.emptyText')} style={{ padding: '60px 0' }} />
          </Card>
        ) : (
          <div>
            {providers.map((provider) => (
              <ClaudeProviderCard
                key={provider.id}
                provider={provider}
                isApplied={provider.id === appliedProviderId}
                onEdit={handleEditProvider}
                onDelete={handleDeleteProvider}
                onCopy={handleCopyProvider}
                onSelect={handleSelectProvider}
                onPreview={handlePreviewProvider}
              />
            ))}
          </div>
        )}
      </Spin>

      {/* 模态框 */}
      <ClaudeProviderFormModal
        open={providerModalOpen}
        provider={editingProvider}
        isCopy={isCopyMode}
        defaultTab={modalDefaultTab}
        onCancel={() => {
          setProviderModalOpen(false);
          setEditingProvider(null);
          setIsCopyMode(false);
        }}
        onSubmit={handleProviderSubmit}
      />

      <CommonConfigModal
        open={commonConfigModalOpen}
        onCancel={() => setCommonConfigModalOpen(false)}
        onSuccess={() => {
          setCommonConfigModalOpen(false);
          message.success(t('common.success'));
        }}
      />

      <ImportConflictDialog
        open={conflictDialogOpen}
        conflictInfo={conflictInfo}
        onResolve={handleConflictResolve}
        onCancel={() => {
          setConflictDialogOpen(false);
          setConflictInfo(null);
          setPendingFormValues(null);
        }}
      />
    </div>
  );
};

export default ClaudeCodePage;
