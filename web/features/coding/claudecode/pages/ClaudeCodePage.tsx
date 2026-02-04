import React from 'react';
import { Typography, Card, Button, Space, Empty, message, Modal, Spin } from 'antd';
import { PlusOutlined, FolderOpenOutlined, AppstoreOutlined, SyncOutlined, ExclamationCircleOutlined, LinkOutlined, EyeOutlined, EllipsisOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
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
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
  arrayMove,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import type {
  ClaudeCodeProvider,
  ClaudeProviderFormValues,
  ClaudeProviderInput,
  ImportConflictInfo,
  ImportConflictAction,
} from '@/types/claudecode';
import {
  getClaudeConfigPath,
  listClaudeProviders,
  createClaudeProvider,
  updateClaudeProvider,
  saveClaudeLocalConfig,
  deleteClaudeProvider,
  selectClaudeProvider,
  applyClaudeConfig,
  readClaudeSettings,
  toggleClaudeCodeProviderDisabled,
  reorderClaudeProviders,
} from '@/services/claudeCodeApi';
import { useRefreshStore } from '@/stores';
import { refreshTrayMenu } from '@/services/appApi';
import ClaudeProviderCard from '../components/ClaudeProviderCard';
import ClaudeProviderFormModal from '../components/ClaudeProviderFormModal';
import CommonConfigModal from '../components/CommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';
import ClaudeCodeSettingsModal from '../components/ClaudeCodeSettingsModal';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';

const { Title, Text, Link } = Typography;



const ClaudeCodePage: React.FC = () => {
  const { t } = useTranslation();
  const { claudeProviderRefreshKey } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [providers, setProviders] = React.useState<ClaudeCodeProvider[]>([]);
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');

  // 模态框状态
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<ClaudeCodeProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [modalDefaultTab, setModalDefaultTab] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<ClaudeProviderFormValues | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<unknown>(null);

  // 配置拖拽传感器
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8, // 防止点击误触
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  // 加载配置（on mount and when refresh key changes）
  React.useEffect(() => {
    loadConfig();
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
    if (!configPath) return;

    try {
      // Try to reveal the file in explorer
      await revealItemInDir(configPath);
    } catch {
      // If file doesn't exist, fallback to opening parent directory
      try {
        const parentDir = configPath.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open folder:', error);
        message.error(t('common.error'));
      }
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

  const handleToggleDisabled = async (provider: ClaudeCodeProvider, isDisabled: boolean) => {
    try {
      await toggleClaudeCodeProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('claudecode.providerDisabled') : t('claudecode.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle provider disabled status:', error);
      message.error(t('common.error'));
    }
  };

  // 拖拽结束
  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;

    if (!over || active.id === over.id) {
      return;
    }

    const oldIndex = providers.findIndex((p) => p.id === active.id);
    const newIndex = providers.findIndex((p) => p.id === over.id);

    if (oldIndex === -1 || newIndex === -1) {
      return;
    }

    // 乐观更新
    const oldProviders = [...providers];
    const newProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(newProviders);

    try {
      await reorderClaudeProviders(newProviders.map((p) => p.id));
      await refreshTrayMenu();
    } catch (error) {
      // 失败回滚
      console.error('Failed to reorder providers:', error);
      setProviders(oldProviders);
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
      const settingsConfigObj: Record<string, unknown> = {
        env: {
          ANTHROPIC_BASE_URL: values.baseUrl,
          ANTHROPIC_AUTH_TOKEN: values.apiKey,
        },
      };

      if (values.model) settingsConfigObj.model = values.model;
      if (values.haikuModel) settingsConfigObj.haikuModel = values.haikuModel;
      if (values.sonnetModel) settingsConfigObj.sonnetModel = values.sonnetModel;
      if (values.opusModel) settingsConfigObj.opusModel = values.opusModel;

      // Check if this is a temporary provider from local file
      const isLocalTemp = editingProvider?.id === "__local__";

      const providerInput: ClaudeProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig: JSON.stringify(settingsConfigObj),
        sourceProviderId: values.sourceProviderId,
        notes: values.notes,
      };

      if (isLocalTemp) {
        await saveClaudeLocalConfig({ provider: providerInput });
      } else if (editingProvider && !isCopyMode) {
        // Update existing provider
        await updateClaudeProvider({
          id: editingProvider.id,
          name: values.name,
          category: values.category,
          settingsConfig: providerInput.settingsConfig,
          sourceProviderId: values.sourceProviderId,
          notes: values.notes,
          sortIndex: editingProvider.sortIndex,
          isApplied: editingProvider.isApplied,
          isDisabled: editingProvider.isDisabled,
          createdAt: editingProvider.createdAt,
          updatedAt: editingProvider.updatedAt,
        });
      } else {
        // 让服务端生成 ID
        await createClaudeProvider(providerInput);
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
          ANTHROPIC_AUTH_TOKEN: values.apiKey,
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
      setPreviewDataLocal(finalConfig);
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview config:', error);
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
                type="text"
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
            <Button type="text" icon={<AppstoreOutlined />} onClick={() => setCommonConfigModalOpen(true)}>
              {t('claudecode.commonConfigButton')}
            </Button>
            <Button type="text" icon={<EllipsisOutlined />} onClick={() => setSettingsModalOpen(true)}>
              {t('claudecode.moreOptions')}
            </Button>
          </Space>
        </div>
      </div>

      {/* 操作栏 */}
      <div style={{ marginBottom: 16 }}>
        <Space size={4}>
          <Button type="text" icon={<SyncOutlined />} onClick={handleImportFromOpenCode}>
            {t('claudecode.importFromOpenCode')}
          </Button>
          <Button type="link" icon={<PlusOutlined />} onClick={handleAddProvider}>
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
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            modifiers={[restrictToVerticalAxis]}
            onDragEnd={handleDragEnd}
          >
            <SortableContext
              items={providers.map((p) => p.id)}
              strategy={verticalListSortingStrategy}
            >
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
                    onToggleDisabled={handleToggleDisabled}
                  />
                ))}
              </div>
            </SortableContext>
          </DndContext>
        )}
      </Spin>

      {/* 模态框 */}
      {providerModalOpen && (
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
      )}

      <CommonConfigModal
        open={commonConfigModalOpen}
        onCancel={() => setCommonConfigModalOpen(false)}
        onSuccess={() => {
          setCommonConfigModalOpen(false);
          message.success(t('common.success'));
        }}
        isLocalProvider={providers.some((provider) => provider.id === '__local__')}
      />

      {settingsModalOpen && (
        <ClaudeCodeSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
        />
      )}

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

      {/* Preview Modal */}
      <JsonPreviewModal
        open={previewModalOpen}
        onClose={() => setPreviewModalOpen(false)}
        title={t('claudecode.preview.currentConfigTitle')}
        data={previewData}
      />
    </div>
  );
};

export default ClaudeCodePage;
