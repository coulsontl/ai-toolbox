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
import ModelsJsonProviderFormModal, {
  type ModelsJsonProviderFormValues,
} from '@/features/coding/shared/modelsJson/ModelsJsonProviderFormModal';
import ModelsJsonProviderSection from '@/features/coding/shared/modelsJson/ModelsJsonProviderSection';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import ProviderConnectivityTestModal from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import type { ProviderConnectivityInfo } from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
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

  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [providerSaving, setProviderSaving] = React.useState(false);
  const [connectivityOpen, setConnectivityOpen] = React.useState(false);
  const [connectivityChannelId, setConnectivityChannelId] = React.useState<string | null>(null);
  const [fetchModelsChannelId, setFetchModelsChannelId] = React.useState<string | null>(null);

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

  /**
   * Opens the model editor.
   *
   * `channel` is required: a channel is derived from `(endpoint, api key)` and
   * every model belongs to one, so there is no "model without a channel". The
   * only exception is the provider form, which writes the first entry itself.
   *
   * `copy` pre-fills a duplicate under a new id and deliberately clears
   * `previousId`, so saving appends a new entry instead of renaming the source
   * onto itself.
   */
  const openModelModal = (
    channel: WorkbuddyProviderView,
    modelId?: string,
    options?: { copy?: boolean },
  ) => {
    const copy = options?.copy === true;
    setEditingChannel(channel);
    setEditingModelId(copy ? null : (modelId ?? null));
    form.resetFields();
    const model = modelId
      ? channel.models.find((item) => item.id === modelId)
      : undefined;
    form.setFieldsValue({
      previousId: copy ? null : (model?.id ?? null),
      id: copy && model ? `${model.id}_copy` : (model?.id ?? ''),
      name: copy && model ? `${model.name || model.id} copy` : (model?.name ?? ''),
      vendor: channel.vendor ?? '',
      baseUrl: channel.baseUrl ?? '',
      apiKey: channel.apiKey ?? '',
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

  /** Saves the provider form: one connection plus its first model entry. */
  const handleSubmitProvider = async (values: ModelsJsonProviderFormValues) => {
    try {
      setRuntime(await saveWorkbuddyModel({
        id: values.modelId.trim(),
        name: values.modelName?.trim() || values.modelId.trim(),
        vendor: values.vendor?.trim() || undefined,
        baseUrl: values.baseUrl.trim(),
        apiKey: values.apiKey?.trim() || undefined,
        supportsToolCall: true,
        supportsImages: false,
        supportsReasoning: false,
        onlyReasoning: false,
        useCustomProtocol: false,
      }));
      setProviderModalOpen(false);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save WorkBuddy provider:', error);
      message.error(String(error));
    }
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

  /**
   * Batch-removes several models of one channel.
   *
   * Confirmed here, like the channel delete: the model list's batch button is
   * the only caller and it must not destroy entries without asking.
   */
  const handleDeleteModels = async (channel: WorkbuddyProviderView, modelIds: string[]) => {
    if (modelIds.length === 0) {
      return;
    }
    const confirmed = await new Promise<boolean>((resolve) => {
      Modal.confirm({
        title: t('common.model.batchDelete'),
        content: t('workbuddy.deleteModelsConfirm', { count: modelIds.length }),
        okText: t('common.confirm'),
        cancelText: t('common.cancel'),
        okButtonProps: { danger: true },
        onOk: () => resolve(true),
        onCancel: () => resolve(false),
      });
    });
    if (!confirmed) {
      return;
    }
    try {
      // Each delete returns the whole recomputed channel list, so the last
      // response is the state to keep.
      let next = runtime;
      for (const modelId of modelIds) {
        next = await deleteWorkbuddyModel(modelId);
      }
      if (next) {
        setRuntime(next);
      }
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete WorkBuddy models:', error);
      message.error(String(error));
      await reload();
    }
    void channel;
  };

  const channels = runtime?.providers ?? [];
  const availableModels = runtime?.availableModels;

  const connectivityChannel = channels.find((item) => item.id === connectivityChannelId) ?? null;
  const fetchModelsChannel = channels.find((item) => item.id === fetchModelsChannelId) ?? null;

  /**
   * The channel as an `OpenCodeProvider`, which is the shape the shared
   * connectivity probe speaks.
   *
   * `ChannelView.baseUrl` has already had `/chat/completions` stripped
   * (`models_json::normalize_channel_base_url`), and the probe appends it back,
   * so the two agree.
   */
  const connectivityInfo = React.useMemo<ProviderConnectivityInfo | null>(() => {
    if (!connectivityChannel) {
      return null;
    }
    return {
      providerId: connectivityChannel.id,
      providerName: connectivityChannel.name || connectivityChannel.baseUrl,
      providerConfig: {
        npm: '@ai-sdk/openai-compatible',
        name: connectivityChannel.name || connectivityChannel.baseUrl,
        options: {
          baseURL: connectivityChannel.baseUrl,
          ...(connectivityChannel.apiKey ? { apiKey: connectivityChannel.apiKey } : {}),
        },
        models: Object.fromEntries(connectivityChannel.models.map((model) => [model.id, {}])),
      },
      modelIds: connectivityChannel.models.map((model) => model.id),
    };
  }, [connectivityChannel]);

  /**
   * Applies the models picked in the shared fetch dialog.
   *
   * Every new id becomes a plain entry on the same channel, so it inherits the
   * endpoint and key by construction — `models.json` stores those per entry.
   */
  const handleFetchModelsSuccess = async (result: {
    selectedModels: Array<{ id: string; name?: string }>;
    removedModelIds: string[];
  }) => {
    setFetchModelsChannelId(null);
    if (!fetchModelsChannel) {
      return;
    }
    try {
      let next = runtime;
      for (const modelId of result.removedModelIds) {
        next = await deleteWorkbuddyModel(modelId);
      }
      for (const model of result.selectedModels) {
        next = await saveWorkbuddyModel({
          id: model.id,
          name: model.name || model.id,
          vendor: fetchModelsChannel.vendor || undefined,
          baseUrl: fetchModelsChannel.baseUrl,
          apiKey: fetchModelsChannel.apiKey || undefined,
          supportsToolCall: true,
          supportsImages: false,
          supportsReasoning: false,
          onlyReasoning: false,
          useCustomProtocol: false,
        });
      }
      if (next) {
        setRuntime(next);
      }
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to apply fetched WorkBuddy models:', error);
      message.error(String(error));
      await reload();
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
              onAddProvider={() => setProviderModalOpen(true)}
              onAddModel={openModelModal}
              onEditChannel={openModelModal}
              onDeleteChannels={requestDeleteChannels}
              onDeleteModel={handleDeleteModel}
              onDeleteModels={handleDeleteModels}
              onTestModels={(channel) => {
                setConnectivityChannelId(channel.id);
                setConnectivityOpen(true);
              }}
              onFetchModels={(channel) => setFetchModelsChannelId(channel.id)}
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

          <ModelsJsonProviderFormModal
            open={providerModalOpen}
            saving={providerSaving}
            onCancel={() => setProviderModalOpen(false)}
            onSubmit={async (values) => {
              setProviderSaving(true);
              try {
                await handleSubmitProvider(values);
              } finally {
                setProviderSaving(false);
              }
            }}
          />

          <ProviderConnectivityTestModal
            open={connectivityOpen}
            connectivityInfo={connectivityInfo}
            onCancel={() => {
              setConnectivityOpen(false);
              setConnectivityChannelId(null);
            }}
          />

          {fetchModelsChannel && (
            <FetchModelsModal
              open={Boolean(fetchModelsChannelId)}
              providerId={fetchModelsChannel.id}
              providerName={fetchModelsChannel.name || fetchModelsChannel.baseUrl}
              baseUrl={fetchModelsChannel.baseUrl}
              apiKey={fetchModelsChannel.apiKey}
              sdkType="@ai-sdk/openai-compatible"
              existingModelIds={fetchModelsChannel.models.map((model) => model.id)}
              onCancel={() => setFetchModelsChannelId(null)}
              onSuccess={(result) => {
                void handleFetchModelsSuccess(result);
              }}
            />
          )}

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
