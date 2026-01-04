import React from 'react';
import { Typography, Card, Button, Space, Empty, message, Modal, Spin } from 'antd';
import { PlusOutlined, FolderOpenOutlined, SettingOutlined, SyncOutlined, ExclamationCircleOutlined, QuestionCircleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import type {
  ClaudeCodeProvider,
  ClaudeProviderFormValues,
  ImportConflictInfo,
  ImportConflictAction,
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
} from '@/services/claudeCodeApi';
import ClaudeProviderCard from '../components/ClaudeProviderCard';
import ClaudeProviderFormModal from '../components/ClaudeProviderFormModal';
import CommonConfigModal from '../components/CommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';

const { Title, Text } = Typography;

const ClaudeCodePage: React.FC = () => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [providers, setProviders] = React.useState<ClaudeCodeProvider[]>([]);
  const [currentProvider, setCurrentProvider] = React.useState<ClaudeCodeProvider | null>(null);
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');

  // 模态框状态
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<ClaudeCodeProvider | null>(null);
  const [modalDefaultTab, setModalDefaultTab] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<ClaudeProviderFormValues | null>(null);

  // 加载配置
  React.useEffect(() => {
    loadConfig();
  }, []);

  const loadConfig = async () => {
    setLoading(true);
    try {
      const [path, providerList] = await Promise.all([
        getClaudeConfigPath(),
        listClaudeProviders(),
      ]);
      
      setConfigPath(path);
      setProviders(providerList);
      
      const current = providerList.find((p) => p.isCurrent);
      setCurrentProvider(current || null);
      
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
    setModalDefaultTab('manual');
    setProviderModalOpen(true);
  };

  const handleImportFromSettings = () => {
    setEditingProvider(null);
    setModalDefaultTab('import');
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: ClaudeCodeProvider) => {
    setEditingProvider(provider);
    setModalDefaultTab('manual');
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: ClaudeCodeProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name} (Copy)`,
      isCurrent: false,
      isApplied: false,
    });
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

      if (editingProvider) {
        await updateClaudeProvider({
          id: editingProvider.id,
          name: values.name,
          category: values.category,
          settingsConfig: JSON.stringify(settingsConfigObj),
          sourceProviderId: values.sourceProviderId,
          notes: values.notes,
          isCurrent: editingProvider.isCurrent,
          isApplied: editingProvider.isApplied,
        });
      } else {
        await createClaudeProvider({
          id: generateId(values.name),
          name: values.name,
          category: values.category,
          settingsConfig: JSON.stringify(settingsConfigObj),
          sourceProviderId: values.sourceProviderId,
          notes: values.notes,
          isCurrent: false,
          isApplied: false,
        });
      }

      message.success(t('common.success'));
      setProviderModalOpen(false);
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

  return (
    <div>
      {/* 页面头部 */}
      <div style={{ marginBottom: 16 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
          <div>
            <Title level={4} style={{ margin: 0, marginBottom: 8 }}>
              {t('claudecode.title')}
            </Title>
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
                style={{ padding: 0 }}
              >
                {t('claudecode.openFolder')}
              </Button>
              <Button
                type="link"
                size="small"
                icon={<QuestionCircleOutlined />}
                onClick={() => openUrl('https://code.claude.com/docs/en/settings#environment-variables')}
                style={{ padding: 0 }}
              >
                {t('claudecode.viewDocs')}
              </Button>
            </Space>
          </div>

          <Space>
            <Button icon={<SettingOutlined />} onClick={() => setCommonConfigModalOpen(true)}>
              {t('claudecode.commonConfigButton')}
            </Button>
          </Space>
        </div>
      </div>

      {/* 操作栏 */}
      <div style={{ marginBottom: 16 }}>
        <Space>
          <Button icon={<SyncOutlined />} onClick={handleImportFromSettings}>
            {t('claudecode.importFromSettings')}
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
                isCurrent={provider.id === currentProvider?.id}
                isApplied={provider.id === appliedProviderId}
                onEdit={handleEditProvider}
                onDelete={handleDeleteProvider}
                onCopy={handleCopyProvider}
                onSelect={handleSelectProvider}
              />
            ))}
          </div>
        )}
      </Spin>

      {/* 模态框 */}
      <ClaudeProviderFormModal
        open={providerModalOpen}
        provider={editingProvider}
        defaultTab={modalDefaultTab}
        onCancel={() => {
          setProviderModalOpen(false);
          setEditingProvider(null);
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
