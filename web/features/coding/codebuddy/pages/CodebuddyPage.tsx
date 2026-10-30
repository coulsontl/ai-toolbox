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
import ModelsJsonProviderSection from '@/features/coding/shared/modelsJson/ModelsJsonProviderSection';
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

  const openModelModal = (channel: CodebuddyProviderView | null, modelId?: string) => {
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

  const channels = runtime?.providers ?? [];
  const availableModels = runtime?.availableModels;

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
              onAddModel={openModelModal}
              onEditChannel={openModelModal}
              onDeleteChannels={requestDeleteChannels}
              onDeleteModel={handleDeleteModel}
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
