import React from 'react';
import { App, Form, Modal, Spin } from 'antd';
import { DatabaseOutlined, FileTextOutlined, HistoryOutlined } from '@ant-design/icons';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';

import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import { useSettingsStore } from '@/stores';
import {
  deleteWorkbuddyModel,
  deleteWorkbuddyProvider,
  getWorkbuddyPathInfo,
  getWorkbuddySettingsConfig,
  readWorkbuddyModelsFile,
  readWorkbuddyRuntimeConfig,
  saveWorkbuddyModel,
  saveWorkbuddySettingsConfig,
} from '@/services/workbuddyApi';
import type {
  WorkbuddyModelInput,
  WorkbuddyProviderView,
  WorkbuddyRuntimeConfig,
} from '@/types/workbuddy';
import ModelsJsonModelModal from '@/features/coding/shared/modelsJson/ModelsJsonModelModal';
import ModelsJsonProviderSection from '@/features/coding/shared/modelsJson/ModelsJsonProviderSection';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import WorkbuddyIdentityPanel from '../components/WorkbuddyIdentityPanel';
import styles from './WorkbuddyPage.module.less';

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'workbuddy-providers': <DatabaseOutlined />,
  'workbuddy-identity': <FileTextOutlined />,
  'workbuddy-session-manager': <HistoryOutlined />,
};

/**
 * WorkBuddy desktop: custom model channels plus the per-account identity files.
 *
 * `models.json` is the only store of channels — there is no SQLite provider
 * table, so the page always renders what the file says and never keeps its own
 * copy of the list.
 */
const WorkbuddyPage: React.FC = () => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const sidebarHidden = sidebarHiddenByPage.workbuddy;
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);

  const [runtime, setRuntime] = React.useState<WorkbuddyRuntimeConfig | null>(null);
  const [pathInfo, setPathInfo] = React.useState<{ path: string; source: 'custom' | 'env' | 'shell' | 'default' } | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [previewOpen, setPreviewOpen] = React.useState(false);
  const [previewText, setPreviewText] = React.useState<string | null>(null);

  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [editingChannel, setEditingChannel] = React.useState<WorkbuddyProviderView | null>(null);
  const [editingModelId, setEditingModelId] = React.useState<string | null>(null);
  const [form] = Form.useForm<WorkbuddyModelInput>();

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'workbuddy-providers',
      title: t('workbuddy.provider.title'),
      order: 1,
    },
    {
      id: 'workbuddy-identity',
      title: t('workbuddy.identity.title'),
      order: 2,
    },
    {
      id: 'workbuddy-session-manager',
      title: t('sessionManager.title'),
      order: 3,
    },
  ], [t]);

  const reload = React.useCallback(async () => {
    setLoading(true);
    try {
      const [nextRuntime, nextPathInfo] = await Promise.all([
        readWorkbuddyRuntimeConfig(),
        getWorkbuddyPathInfo(),
      ]);
      setRuntime(nextRuntime);
      setPathInfo(nextPathInfo);
    } catch (error) {
      console.error('Failed to load WorkBuddy runtime config:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [message, t]);

  React.useEffect(() => {
    void reload();
  }, [reload]);

  /** The only field AI Toolbox stores for this module is the config root. */
  const rootDirectory = useRootDirectoryConfig<{ config?: string; rootDir?: string | null }>({
    t,
    translationKeyPrefix: 'workbuddy',
    defaultConfig: '',
    loadConfig: async () => {
      await reload();
    },
    getCommonConfig: async () => {
      const stored = await getWorkbuddySettingsConfig();
      return { config: '', rootDir: stored?.configDir ?? null };
    },
    saveCommonConfig: async (input) => {
      await saveWorkbuddySettingsConfig({
        configDir: input.rootDir ?? null,
        clearConfigDir: input.clearRootDir,
      });
    },
  });

  const openModelModal = (channel: WorkbuddyProviderView | null, modelId?: string) => {
    setEditingChannel(channel);
    setEditingModelId(modelId ?? null);
    form.resetFields();
    const model = channel && modelId
      ? channel.models.find((item) => item.id === modelId)
      : undefined;
    form.setFieldsValue({
      previousId: model?.id ?? null,
      id: model?.id ?? '',
      name: model?.name ?? '',
      vendor: channel?.vendor ?? '',
      baseUrl: channel?.baseUrl ?? '',
      apiKey: channel?.apiKey ?? '',
      maxInputTokens: model?.maxInputTokens,
      maxOutputTokens: model?.maxOutputTokens,
      temperature: model?.temperature,
      supportsToolCall: model?.supportsToolCall ?? true,
      supportsImages: model?.supportsImages ?? false,
      supportsReasoning: model?.supportsReasoning ?? false,
      onlyReasoning: model?.onlyReasoning ?? false,
      useCustomProtocol: model?.useCustomProtocol ?? false,
    });
    setModelModalOpen(true);
  };

  const handleSubmitModel = async (values: WorkbuddyModelInput) => {
    try {
      setRuntime(await saveWorkbuddyModel(values));
      setModelModalOpen(false);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save WorkBuddy model:', error);
      message.error(String(error));
    }
  };

  /**
   * Deletes one or more channels.
   *
   * The card passes `deleteConfirm: false`, so the confirmation lives here —
   * that is the one place the list card, the grid menu and the detail sidebar
   * all pass through (same pattern as OpenClaw). Batch deletion skips it: the
   * user already confirmed through the batch bar.
   *
   * A partial failure must not report the whole batch as failed while leaving
   * the page showing channels that are already gone: re-read the real state
   * first, then return `false` so the surviving selection stays selected.
   */
  const handleDeleteChannels = async (channels: WorkbuddyProviderView[]) => {
    try {
      let next = runtime;
      for (const channel of channels) {
        next = await deleteWorkbuddyProvider(channel.baseUrl, channel.apiKey);
      }
      if (next) {
        setRuntime(next);
      }
      message.success(t('common.success'));
      return true;
    } catch (error) {
      console.error('Failed to delete WorkBuddy channel:', error);
      message.error(String(error));
      // Earlier channels of the batch may already be gone; without this the
      // list keeps rendering them and every retry fails on the first one.
      await reload();
      return false;
    }
  };

  /**
   * The entry point the list uses. A single card's bin icon arrives without any
   * confirmation (`deleteConfirm: false`), so ask here; the batch bar already
   * confirmed, so it goes straight through. Resolving `false` when the user
   * cancels keeps the batch selection untouched.
   */
  const requestDeleteChannels = async (
    channels: WorkbuddyProviderView[],
  ): Promise<boolean> => {
    if (channels.length > 1) {
      return handleDeleteChannels(channels);
    }
    const confirmed = await new Promise<boolean>((resolve) => {
      Modal.confirm({
        title: t('common.provider.deleteProvider'),
        content: t('workbuddy.deleteProviderConfirm'),
        okText: t('common.confirm'),
        cancelText: t('common.cancel'),
        okButtonProps: { danger: true },
        onOk: () => resolve(true),
        onCancel: () => resolve(false),
      });
    });
    return confirmed ? handleDeleteChannels(channels) : false;
  };

  const handleDeleteModel = async (channel: WorkbuddyProviderView, modelId: string) => {
    // A model is one entry of a channel; the backend deletes by model id and
    // returns the recomputed channel list.
    void channel;
    try {
      setRuntime(await deleteWorkbuddyModel(modelId));
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete WorkBuddy model:', error);
      message.error(String(error));
    }
  };

  const handleOpenConfigFolder = () => {
    if (pathInfo?.path) {
      void revealItemInDir(pathInfo.path);
    }
  };

  // Preview reads the file verbatim: the channel view drops URL-less entries
  // and reorders fields, so a rebuilt preview would show a file that is not
  // on disk.
  const handleOpenPreview = async () => {
    try {
      setPreviewText(await readWorkbuddyModelsFile());
      setPreviewOpen(true);
    } catch (error) {
      console.error('Failed to read WorkBuddy models.json:', error);
      message.error(String(error));
    }
  };

  const channels = runtime?.providers ?? [];
  const availableModels = runtime?.availableModels;

  return (
    <Spin spinning={loading}>
      <SectionSidebarLayout
        sidebarTitle={t('workbuddy.title')}
        sidebarHidden={sidebarHidden}
        sections={sidebarSections}
        markerAttr="data-workbuddy-sidebar-section"
        getIcon={(id) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
      >
        <div className={styles.pageContent}>
          <CodingPageHeader
            title={t('workbuddy.title')}
            onPreviewConfig={runtime ? () => { void handleOpenPreview(); } : undefined}
            configPath={pathInfo?.path ?? ''}
            configPathKind="directory"
            onCustomizeConfig={() => rootDirectory.setRootDirectoryModalOpen(true)}
            onOpenFolder={handleOpenConfigFolder}
            onRefresh={() => {
              void reload();
            }}
            onMoreOptions={() => setSettingsModalOpen(true)}
          />
          <div className={styles.pageHint}>{t('workbuddy.pageHint')}</div>

          <div
            id="workbuddy-providers"
            className={styles.workbuddySection}
            data-workbuddy-sidebar-section="true"
            data-sidebar-title={t('workbuddy.provider.title')}
          >
            <ModelsJsonProviderSection
              moduleKey="workbuddy"
              providers={channels}
              loading={loading}
              hint={(
                <div>
                  {availableModels && availableModels.length > 0
                    ? t('common.availableModelsRestricted', { count: availableModels.length })
                    : t('common.availableModelsFree')}
                </div>
              )}
              onAddModel={openModelModal}
              onEditChannel={openModelModal}
              onDeleteChannels={requestDeleteChannels}
              onDeleteModel={handleDeleteModel}
            />
          </div>

          <div
            id="workbuddy-identity"
            className={styles.workbuddySection}
            data-workbuddy-sidebar-section="true"
            data-sidebar-title={t('workbuddy.identity.title')}
          >
            {/* One section per block, matching Hermes / DSH: the panel owns its
                own card chrome, so wrapping it in a Collapse would nest two
                borders around the same content. */}
            {/*
              The panel reads the signed-in account of the *current* root, so the
              root's path doubles as its refresh signal: after "自定义配置目录"
              changes it, the panel must drop the previous root's files.
            */}
            <WorkbuddyIdentityPanel onSaved={reload} rootPath={pathInfo?.path} />
          </div>

          <div
            id="workbuddy-session-manager"
            className={styles.workbuddySection}
            data-workbuddy-sidebar-section="true"
            data-sidebar-title={t('sessionManager.title')}
          >
            <SessionManagerPanel tool="workbuddy" />
          </div>

          <ModelsJsonModelModal
            open={modelModalOpen}
            form={form}
            editing={Boolean(editingModelId)}
            channel={editingChannel}
            onCancel={() => setModelModalOpen(false)}
            onSubmit={handleSubmitModel}
          />

          <RootDirectoryModal
            open={rootDirectory.rootDirectoryModalOpen}
            {...rootDirectory.getRootDirectoryModalProps(pathInfo)}
            onCancel={() => rootDirectory.setRootDirectoryModalOpen(false)}
            onSubmit={rootDirectory.handleSaveRootDirectory}
            onReset={rootDirectory.handleResetRootDirectory}
          />

          <FileConfigPreviewModal
            open={previewOpen}
            onClose={() => setPreviewOpen(false)}
            title={t('common.previewConfig')}
            files={[
              {
                key: 'models',
                label: 'models.json',
                content: previewText ?? '',
              },
            ]}
          />

          {/* The section sidebar has no other way back once hidden, so the
              toggle lives here like every other coding page. */}
          <SidebarSettingsModal
            open={settingsModalOpen}
            onClose={() => setSettingsModalOpen(false)}
            sidebarVisible={!sidebarHidden}
            onSidebarVisibleChange={async (visible) => {
              await setSidebarHidden('workbuddy', !visible);
            }}
          />
        </div>
      </SectionSidebarLayout>
    </Spin>
  );
};

export default WorkbuddyPage;
