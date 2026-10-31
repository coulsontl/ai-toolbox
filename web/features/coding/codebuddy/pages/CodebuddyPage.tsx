import React from 'react';
import { App, Form, Modal, Spin, Typography } from 'antd';
import { DatabaseOutlined, FileTextOutlined, HistoryOutlined, SettingOutlined } from '@ant-design/icons';
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
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import { useSettingsStore } from '@/stores';
import { codebuddyPromptApi } from '@/services/codebuddyPromptApi';
import {
  deleteCodebuddyModel,
  deleteCodebuddyProvider,
  getCodebuddyPathInfo,
  getCodebuddySettingsConfig,
  readCodebuddyModelsFile,
  readCodebuddyRuntimeConfig,
  readCodebuddySettingsFile,
  saveCodebuddyModel,
  saveCodebuddySettingsConfig,
} from '@/services/codebuddyApi';
import type {
  CodebuddyModelInput,
  CodebuddyProviderView,
  CodebuddyRuntimeConfig,
} from '@/types/codebuddy';
import ModelsJsonModelModal from '@/features/coding/shared/modelsJson/ModelsJsonModelModal';
import ModelsJsonProviderFormModal, {
  type ModelsJsonProviderFormValues,
} from '@/features/coding/shared/modelsJson/ModelsJsonProviderFormModal';
import ModelsJsonProviderSection from '@/features/coding/shared/modelsJson/ModelsJsonProviderSection';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import ProviderConnectivityTestModal, {
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import styles from './CodebuddyPage.module.less';

const { Paragraph, Text } = Typography;

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'codebuddy-providers': <DatabaseOutlined />,
  'codebuddy-global-prompt': <FileTextOutlined />,
  'codebuddy-settings': <SettingOutlined />,
  'codebuddy-session-manager': <HistoryOutlined />,
};

/**
 * CodeBuddy Code CLI: the same `models.json` channel model as WorkBuddy
 * desktop, plus the CLI's own `CODEBUDDY.md` presets and a read-only view of
 * `settings.json`.
 *
 * The provider section is shared with the desktop page because both products
 * run one engine and read one file shape; re-deriving it here would be the
 * second implementation of the same contract.
 */
const CodebuddyPage: React.FC = () => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const sidebarHidden = sidebarHiddenByPage.codebuddy;
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);

  const [runtime, setRuntime] = React.useState<CodebuddyRuntimeConfig | null>(null);
  const [pathInfo, setPathInfo] = React.useState<{ path: string; source: 'custom' | 'env' | 'shell' | 'default' } | null>(null);
  const [settingsText, setSettingsText] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(true);

  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [editingChannel, setEditingChannel] = React.useState<CodebuddyProviderView | null>(null);
  const [editingModelId, setEditingModelId] = React.useState<string | null>(null);
  const [previewOpen, setPreviewOpen] = React.useState(false);
  const [modelsText, setModelsText] = React.useState<string | null>(null);
  const [form] = Form.useForm<CodebuddyModelInput>();

  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [providerSaving, setProviderSaving] = React.useState(false);
  /** The channel whose connection the provider form is rewriting; `null` = add. */
  const [editingConnection, setEditingConnection] = React.useState<CodebuddyProviderView | null>(null);
  const [connectivityOpen, setConnectivityOpen] = React.useState(false);
  const [connectivityChannelId, setConnectivityChannelId] = React.useState<string | null>(null);
  const [fetchModelsChannelId, setFetchModelsChannelId] = React.useState<string | null>(null);

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'codebuddy-providers',
      title: t('codebuddy.provider.title'),
      order: 1,
    },
    {
      id: 'codebuddy-global-prompt',
      title: t('common.prompt.title'),
      order: 2,
    },
    {
      id: 'codebuddy-settings',
      title: t('codebuddy.settingsPath'),
      order: 3,
    },
    {
      id: 'codebuddy-session-manager',
      title: t('sessionManager.title'),
      order: 4,
    },
  ], [t]);

  const reload = React.useCallback(async () => {
    setLoading(true);
    try {
      const [nextRuntime, nextPathInfo, nextSettings] = await Promise.all([
        readCodebuddyRuntimeConfig(),
        getCodebuddyPathInfo(),
        readCodebuddySettingsFile(),
      ]);
      setRuntime(nextRuntime);
      setPathInfo(nextPathInfo);
      setSettingsText(nextSettings);
    } catch (error) {
      console.error('Failed to load CodeBuddy runtime config:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [message, t]);

  React.useEffect(() => {
    void reload();
  }, [reload]);

  const rootDirectory = useRootDirectoryConfig<{ config?: string; rootDir?: string | null }>({
    t,
    translationKeyPrefix: 'codebuddy',
    defaultConfig: '',
    loadConfig: async () => {
      await reload();
    },
    getCommonConfig: async () => {
      const stored = await getCodebuddySettingsConfig();
      return { config: '', rootDir: stored?.configDir ?? null };
    },
    saveCommonConfig: async (input) => {
      await saveCodebuddySettingsConfig({
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
    channel: CodebuddyProviderView,
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

  /**
   * Saves the provider form.
   *
   * Add mode writes one connection plus its first model entry. Edit mode writes
   * that connection onto **every** entry of the channel — there is no channel
   * record on disk, so rewriting the group is what "edit this provider" means,
   * and each entry keeps its own id, name and capability switches because
   * `upsert_model_renaming` merges by id.
   */
  const handleSubmitProvider = async (values: ModelsJsonProviderFormValues) => {
    try {
      if (editingConnection) {
        let next = runtime;
        for (const model of editingConnection.models) {
          next = await saveCodebuddyModel({
            id: model.id,
            name: model.name,
            vendor: values.vendor?.trim() ?? '',
            baseUrl: values.baseUrl.trim(),
            apiKey: values.apiKey?.trim() ?? '',
            maxInputTokens: model.maxInputTokens,
            maxOutputTokens: model.maxOutputTokens,
            temperature: model.temperature,
            supportsToolCall: model.supportsToolCall,
            supportsImages: model.supportsImages,
            supportsReasoning: model.supportsReasoning,
            onlyReasoning: model.onlyReasoning,
            useCustomProtocol: model.useCustomProtocol,
          });
        }
        if (next) {
          setRuntime(next);
        }
      } else {
        setRuntime(await saveCodebuddyModel({
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
      }
      setProviderModalOpen(false);
      setEditingConnection(null);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save CodeBuddy provider:', error);
      message.error(String(error));
    }
  };

  const handleSubmitModel = async (values: CodebuddyModelInput) => {
    try {
      setRuntime(await saveCodebuddyModel(values));
      setModelModalOpen(false);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save CodeBuddy model:', error);
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
  const handleDeleteChannels = async (channels: CodebuddyProviderView[]) => {
    try {
      let next = runtime;
      for (const channel of channels) {
        next = await deleteCodebuddyProvider(channel.baseUrl, channel.apiKey);
      }
      if (next) {
        setRuntime(next);
      }
      message.success(t('common.success'));
      return true;
    } catch (error) {
      console.error('Failed to delete CodeBuddy channel:', error);
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
    channels: CodebuddyProviderView[],
  ): Promise<boolean> => {
    if (channels.length > 1) {
      return handleDeleteChannels(channels);
    }
    const confirmed = await new Promise<boolean>((resolve) => {
      Modal.confirm({
        title: t('common.provider.deleteProvider'),
        content: t('codebuddy.deleteProviderConfirm'),
        okText: t('common.confirm'),
        cancelText: t('common.cancel'),
        okButtonProps: { danger: true },
        onOk: () => resolve(true),
        onCancel: () => resolve(false),
      });
    });
    return confirmed ? handleDeleteChannels(channels) : false;
  };

  const handleDeleteModel = async (channel: CodebuddyProviderView, modelId: string) => {
    // A model is one entry of a channel; the backend deletes by model id and
    // returns the recomputed channel list.
    void channel;
    try {
      setRuntime(await deleteCodebuddyModel(modelId));
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete CodeBuddy model:', error);
      message.error(String(error));
    }
  };

  /**
   * Batch-removes several models of one channel.
   *
   * Confirmed here, like the channel delete: the model list's batch button is
   * the only caller and it must not destroy entries without asking.
   */
  const handleDeleteModels = async (channel: CodebuddyProviderView, modelIds: string[]) => {
    if (modelIds.length === 0) {
      return;
    }
    const confirmed = await new Promise<boolean>((resolve) => {
      Modal.confirm({
        title: t('common.model.batchDelete'),
        content: t('codebuddy.deleteModelsConfirm', { count: modelIds.length }),
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
        next = await deleteCodebuddyModel(modelId);
      }
      if (next) {
        setRuntime(next);
      }
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete CodeBuddy models:', error);
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
        next = await deleteCodebuddyModel(modelId);
      }
      for (const model of result.selectedModels) {
        next = await saveCodebuddyModel({
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
      console.error('Failed to apply fetched CodeBuddy models:', error);
      message.error(String(error));
      await reload();
    }
  };

  const handleOpenConfigFolder = () => {
    if (pathInfo?.path) {
      void revealItemInDir(pathInfo.path);
    }
  };

  // Preview shows both files as they are on disk. Rebuilding them from the
  // runtime view / parsed settings would change field order and drop keys.
  const handleOpenPreview = async () => {
    try {
      const [nextModels, nextSettings] = await Promise.all([
        readCodebuddyModelsFile(),
        readCodebuddySettingsFile(),
      ]);
      setModelsText(nextModels);
      setSettingsText(nextSettings);
      setPreviewOpen(true);
    } catch (error) {
      console.error('Failed to read CodeBuddy config files:', error);
      message.error(String(error));
    }
  };

  return (
    <Spin spinning={loading}>
      <SectionSidebarLayout
        sidebarTitle={t('codebuddy.title')}
        sidebarHidden={sidebarHidden}
        sections={sidebarSections}
        markerAttr="data-codebuddy-sidebar-section"
        getIcon={(id) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
      >
        <div className={styles.pageContent}>
          <CodingPageHeader
            title={t('codebuddy.title')}
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
          <div className={styles.pageHint}>{t('codebuddy.pageHint')}</div>

          <div
            id="codebuddy-providers"
            className={styles.codebuddySection}
            data-codebuddy-sidebar-section="true"
            data-sidebar-title={t('codebuddy.provider.title')}
          >
            <ModelsJsonProviderSection
              moduleKey="codebuddy"
              providers={channels}
              loading={loading}
              hint={(
                <div>
                  {availableModels && availableModels.length > 0
                    ? t('common.availableModelsRestricted', { count: availableModels.length })
                    : t('common.availableModelsFree')}
                </div>
              )}
              onAddProvider={() => {
                setEditingConnection(null);
                setProviderModalOpen(true);
              }}
              onAddModel={openModelModal}
              onEditChannel={openModelModal}
              onEditConnection={(channel) => {
                setEditingConnection(channel);
                setProviderModalOpen(true);
              }}
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
            id="codebuddy-global-prompt"
            className={styles.codebuddySection}
            data-codebuddy-sidebar-section="true"
            data-sidebar-title={t('common.prompt.title')}
          >
            <GlobalPromptSettings
              service={codebuddyPromptApi}
              collapseKey="codebuddy-prompt"
              toolName="CodeBuddy"
              promptFileName={t('codebuddy.promptFile')}
              onUpdated={reload}
            />
          </div>

          <div
            id="codebuddy-settings"
            className={styles.codebuddySection}
            data-codebuddy-sidebar-section="true"
            data-sidebar-title={t('codebuddy.settingsPath')}
          >
            {/* One section per block (Hermes / DSH shape): the card is the
                section itself, not a Collapse inside a section. */}
            <div className={styles.settingsPanel}>
              <div className={styles.settingsPanelHeader}>{t('codebuddy.settingsPath')}</div>
              {settingsText === null ? (
                <Text type="secondary">{t('codebuddy.settingsEmpty')}</Text>
              ) : (
                <>
                  {/* Read-only: the CLI owns this file, and re-serializing a
                      parsed copy would lose key order. */}
                  <pre className={styles.settingsPreview}>{settingsText}</pre>
                  <Paragraph type="secondary" style={{ marginTop: 8, marginBottom: 0 }}>
                    {t('codebuddy.settingsHint')}
                  </Paragraph>
                </>
              )}
            </div>
          </div>

          <div
            id="codebuddy-session-manager"
            className={styles.codebuddySection}
            data-codebuddy-sidebar-section="true"
            data-sidebar-title={t('sessionManager.title')}
          >
            <SessionManagerPanel tool="codebuddy" />
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
            channel={editingConnection}
            onCancel={() => {
              setProviderModalOpen(false);
              setEditingConnection(null);
            }}
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
                content: modelsText ?? '',
              },
              {
                key: 'settings',
                label: 'settings.json',
                content: settingsText,
              },
            ]}
          />

          {/* The section sidebar has no other way back once hidden. */}
          <SidebarSettingsModal
            open={settingsModalOpen}
            onClose={() => setSettingsModalOpen(false)}
            sidebarVisible={!sidebarHidden}
            onSidebarVisibleChange={async (visible) => {
              await setSidebarHidden('codebuddy', !visible);
            }}
          />
        </div>
      </SectionSidebarLayout>
    </Spin>
  );
};

export default CodebuddyPage;
