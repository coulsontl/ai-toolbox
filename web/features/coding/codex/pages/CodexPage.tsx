import React from 'react';
import { Typography, Button, Space, Empty, message, Modal, Spin, Collapse } from 'antd';
import { PlusOutlined, FolderOpenOutlined, AppstoreOutlined, SyncOutlined, EyeOutlined, ExclamationCircleOutlined, LinkOutlined, DatabaseOutlined, ImportOutlined, FileTextOutlined } from '@ant-design/icons';
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
  CodexProvider,
  CodexProviderFormValues,
  CodexProviderInput,
  CodexSettingsConfig,
  ImportConflictInfo,
  ImportConflictAction,
} from '@/types/codex';
import {
  getCodexConfigFilePath,
  listCodexProviders,
  selectCodexProvider,
  applyCodexConfig,
  readCodexSettings,
  createCodexProvider,
  updateCodexProvider,
  saveCodexLocalConfig,
  deleteCodexProvider,
  toggleCodexProviderDisabled,
  reorderCodexProviders,
} from '@/services/codexApi';
import { listen } from '@tauri-apps/api/event';
import { codexPromptApi } from '@/services/codexPromptApi';
import { refreshTrayMenu, hasAllApiHubExtension } from '@/services/appApi';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import CodexProviderCard from '../components/CodexProviderCard';
import CodexProviderFormModal from '../components/CodexProviderFormModal';
import CodexCommonConfigModal from '../components/CodexCommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import SectionSidebarLayout from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';

const { Title, Text, Link } = Typography;

const CodexPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [providers, setProviders] = React.useState<CodexProvider[]>([]);
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');

  // Modal states
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<CodexProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [providerModalMode, setProviderModalMode] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<CodexProviderFormValues | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<unknown>(null);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);

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

  const loadConfig = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [path, providerList] = await Promise.all([
        getCodexConfigFilePath(),
        listCodexProviders(),
      ]);
      setConfigPath(path);
      setProviders(providerList);
      const applied = providerList.find((p) => p.isApplied);
      setAppliedProviderId(applied?.id || '');
    } catch (error) {
      console.error('Failed to load config:', error);
      if (!silent) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  // 从其他 Tab 切回时刷新数据
  const hasInitializedRef = React.useRef(false);
  React.useEffect(() => {
    if (!isActive) {
      hasInitializedRef.current = true;
      return;
    }
    if (hasInitializedRef.current) {
      loadConfig(true);
    }
  }, [isActive, loadConfig]);

  // 监听 tray 或其他页面触发的配置变更
  React.useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      unlisten = await listen('config-changed', () => {
        loadConfig(true);
      });
    };
    setup();
    return () => { unlisten?.(); };
  }, [loadConfig]);

  React.useEffect(() => {
    const checkAllApiHubAvailability = async () => {
      try {
        const available = await hasAllApiHubExtension();
        setAllApiHubAvailable(available);
      } catch {
        setAllApiHubAvailable(false);
      }
    };

    checkAllApiHubAvailability();
  }, []);

  const handleOpenFolder = async () => {
    if (!configPath) return;

    try {
      await revealItemInDir(configPath);
    } catch {
      try {
        const parentDir = configPath.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open folder:', error);
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    }
  };

  const handleRefreshPage = () => {
    loadConfig();
  };

  const handleSelectProvider = async (provider: CodexProvider) => {
    try {
      await selectCodexProvider(provider.id);
      await applyCodexConfig(provider.id);
      message.success(t('codex.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to select provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleToggleDisabled = async (provider: CodexProvider, isDisabled: boolean) => {
    try {
      await toggleCodexProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('codex.providerDisabled') : t('codex.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle provider disabled status:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  // 拖拽排序处理
  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const oldIndex = providers.findIndex((p) => p.id === active.id);
    const newIndex = providers.findIndex((p) => p.id === over.id);
    const oldProviders = [...providers];
    const newProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(newProviders);

    try {
      await reorderCodexProviders(newProviders.map((p) => p.id));
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to reorder providers:', error);
      setProviders(oldProviders);
      message.error(t('common.error'));
    }
  };

  const handleAddProvider = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  const handleImportFromOpenCode = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalMode('import');
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: CodexProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: CodexProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name}_copy`,
      isApplied: false,
    });
    setIsCopyMode(true);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = (provider: CodexProvider) => {
    Modal.confirm({
      title: t('codex.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteCodexProvider(provider.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete provider:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        }
      },
    });
  };

  const handleProviderSubmit = async (values: CodexProviderFormValues) => {
    // Check for conflicts
    if (values.sourceProviderId && !editingProvider) {
      const existingProvider = providers.find(
        (p) => p.sourceProviderId === values.sourceProviderId
      );

      if (existingProvider) {
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
      await doUpdateProvider(conflictInfo.existingProvider.id, pendingFormValues);
    } else {
      await doSaveProvider({
        ...pendingFormValues,
        sourceProviderId: undefined,
      });
    }

    setConflictDialogOpen(false);
    setConflictInfo(null);
    setPendingFormValues(null);
  };

  const handleImportFromAllApiHub = async (imported: OpenCodeAllApiHubProvider[]) => {
    try {
      for (const item of imported) {
        const baseUrl = item.providerConfig.options?.baseURL || '';
        const apiKey = item.providerConfig.options?.apiKey || '';
        const configLines = [
          'model_provider = "custom"',
          'model_reasoning_effort = "high"',
          '',
          '[model_providers.custom]',
          'name = "OpenAI"',
          'wire_api = "responses"',
          'requires_openai_auth = true',
        ];

        if (baseUrl) {
          configLines.push(`base_url = "${baseUrl}"`);
        }

        const providerInput: CodexProviderInput = {
          name: item.name,
          category: 'custom',
          settingsConfig: JSON.stringify({
            auth: apiKey ? { OPENAI_API_KEY: apiKey } : {},
            config: configLines.join('\n'),
          }),
          sourceProviderId: item.providerId,
          notes: undefined,
        };

        await createCodexProvider(providerInput);
      }

      message.success(t('common.allApiHub.importSuccess', { count: imported.length }));
      setAllApiHubImportModalOpen(false);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from All API Hub:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const doSaveProvider = async (values: CodexProviderFormValues) => {
    try {
      // 新架构：直接使用 settingsConfig（由 Hook 构建）
      // 旧架构：手动构建（向后兼容）
      let settingsConfig: string;
      if (values.settingsConfig) {
        settingsConfig = values.settingsConfig;
      } else {
        // 向后兼容旧逻辑
        const settingsConfigObj: CodexSettingsConfig = {
          auth: {
            OPENAI_API_KEY: values.apiKey || '',
          },
        };

        const configParts: string[] = [];
        if (values.baseUrl) {
          configParts.push(`base_url = "${values.baseUrl}"`);
        }
        if (values.model) {
          configParts.push(`[chat]\nmodel = "${values.model}"`);
        }
        if (configParts.length > 0) {
          settingsConfigObj.config = configParts.join('\n');
        }
        if (values.configToml) {
          settingsConfigObj.config = (settingsConfigObj.config || '') + '\n' + values.configToml;
        }

        settingsConfig = JSON.stringify(settingsConfigObj);
      }

      // Check if this is a temporary provider from local files
      const isLocalTemp = editingProvider?.id === "__local__";

      const providerInput: CodexProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig,
        sourceProviderId: values.sourceProviderId,
        notes: values.notes,
      };

      if (isLocalTemp) {
        await saveCodexLocalConfig({ provider: providerInput });
      } else if (editingProvider && !isCopyMode) {
        await updateCodexProvider({
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
        await createCodexProvider(providerInput);
      }

      message.success(t('common.success'));
      setProviderModalOpen(false);
      setIsCopyMode(false);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
      throw error;
    }
  };

  const doUpdateProvider = async (id: string, values: CodexProviderFormValues) => {
    try {
      const existingProvider = providers.find((p) => p.id === id);
      if (!existingProvider) return;

      // 新架构：直接使用 settingsConfig（由 Hook 构建）
      // 旧架构：手动构建（向后兼容）
      let settingsConfig: string;
      if (values.settingsConfig) {
        settingsConfig = values.settingsConfig;
      } else {
        // 向后兼容旧逻辑
        const settingsConfigObj: CodexSettingsConfig = {
          auth: {
            OPENAI_API_KEY: values.apiKey || '',
          },
        };

        const configParts: string[] = [];
        if (values.baseUrl) {
          configParts.push(`base_url = "${values.baseUrl}"`);
        }
        if (values.model) {
          configParts.push(`[chat]\nmodel = "${values.model}"`);
        }
        if (configParts.length > 0) {
          settingsConfigObj.config = configParts.join('\n');
        }
        if (values.configToml) {
          settingsConfigObj.config = (settingsConfigObj.config || '') + '\n' + values.configToml;
        }

        settingsConfig = JSON.stringify(settingsConfigObj);
      }

      const providerData: CodexProvider = {
        ...existingProvider,
        name: values.name,
        category: values.category,
        settingsConfig,
        notes: values.notes,
        isDisabled: existingProvider.isDisabled,
        createdAt: existingProvider.createdAt,
        updatedAt: existingProvider.updatedAt,
      };

      await updateCodexProvider(providerData);
      message.success(t('common.success'));
      setProviderModalOpen(false);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to update provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
      throw error;
    }
  };

  const handlePreviewCurrentConfig = async () => {
    try {
      const settings = await readCodexSettings();
      setPreviewDataLocal(settings);
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  return (
    <SectionSidebarLayout
      sidebarTitle={t('codex.title')}
      getIcon={(id) => {
        switch (id) {
          case 'codex-providers':
            return <DatabaseOutlined />;
          case 'codex-global-prompt':
            return <FileTextOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'codex-providers':
            setProviderListCollapsed(false);
            break;
          case 'codex-global-prompt':
            setPromptExpandNonce((v) => v + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        {/* Page Header */}
        <div style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
            <div>
              <div style={{ marginBottom: 8 }}>
                <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                  {t('codex.title')}
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openUrl('https://developers.openai.com/codex/config-basic');
                  }}
                >
                  <LinkOutlined /> {t('codex.viewDocs')}
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
              <Space size="small">
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('codex.configPath')}:
                </Text>
                <Text code style={{ fontSize: 12 }}>
                  {configPath || '~/.codex/config.toml'}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenFolder}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('codex.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<SyncOutlined />}
                  onClick={handleRefreshPage}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('codex.refreshConfig')}
                </Button>
              </Space>
            </div>
          </div>
        </div>

        {/* Provider List */}
        <div
          id="codex-providers"
          data-sidebar-section="true"
          data-sidebar-title={t('codex.provider.title')}
        >
          <Collapse
            style={{ marginBottom: 16 }}
            activeKey={providerListCollapsed ? [] : ['providers']}
            onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
            items={[
              {
                key: 'providers',
                label: (
                  <Text strong>
                    <DatabaseOutlined style={{ marginRight: 8 }} />
                    {t('codex.provider.title')}
                  </Text>
                ),
                extra: (
                  <Space size={4}>
                    <Button
                      type="link"
                      size="small"
                      style={{ fontSize: 12 }}
                      icon={<AppstoreOutlined />}
                      onClick={(e) => {
                        e.stopPropagation();
                        setCommonConfigModalOpen(true);
                      }}
                    >
                      {t('codex.commonConfigButton')}
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
                      {t('codex.addProvider')}
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
                      <div>{t('codex.pageHint')}</div>
                      <div>{t('codex.pageWarning')}</div>
                    </div>

                    {providers.length === 0 ? (
                      <Empty description={t('codex.emptyText')} style={{ marginTop: 40 }} />
                    ) : (
                      <DndContext
                        sensors={sensors}
                        collisionDetection={closestCenter}
                        onDragEnd={handleDragEnd}
                        modifiers={[restrictToVerticalAxis]}
                      >
                        <SortableContext
                          items={providers.map((p) => p.id)}
                          strategy={verticalListSortingStrategy}
                        >
                          <div>
                            {providers.map((provider) => (
                              <CodexProviderCard
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

                    <div style={{ marginTop: 12 }}>
                      <Space wrap>
                        <Button
                          type="dashed"
                          icon={<ImportOutlined />}
                          onClick={handleImportFromOpenCode}
                        >
                          {t('codex.importFromOpenCode')}
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
                      </Space>
                    </div>
                  </Spin>
                ),
              },
            ]}
          />
        </div>

        <div
          id="codex-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('codex.prompt.title')}
        >
          <GlobalPromptSettings
            key={`codex-prompt-${promptExpandNonce}`}
            translationKeyPrefix="codex.prompt"
            service={codexPromptApi}
            collapseKey="codex-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        {/* Modals */}
        {providerModalOpen && (
          <CodexProviderFormModal
            open={providerModalOpen}
            provider={editingProvider}
            isCopy={isCopyMode}
            mode={providerModalMode}
            onCancel={() => {
              setProviderModalOpen(false);
              setEditingProvider(null);
              setIsCopyMode(false);
            }}
            onSubmit={handleProviderSubmit}
          />
        )}

        <CodexCommonConfigModal
          open={commonConfigModalOpen}
          onCancel={() => setCommonConfigModalOpen(false)}
          onSuccess={() => {
            setCommonConfigModalOpen(false);
          }}
          isLocalProvider={providers.some((provider) => provider.id === '__local__')}
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

        {allApiHubAvailable && (
          <ImportFromAllApiHubModal
            open={allApiHubImportModalOpen}
            existingProviderIds={providers.map((provider) => provider.sourceProviderId || provider.id)}
            onCancel={() => setAllApiHubImportModalOpen(false)}
            onImport={handleImportFromAllApiHub}
          />
        )}

        {/* Preview Modal */}
        <JsonPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('codex.preview.currentConfigTitle')}
          data={previewData}
        />
      </div>
    </SectionSidebarLayout>
  );
};

export default CodexPage;
