import React from 'react';
import {
  Alert,
  Button,
  Collapse,
  Empty,
  Input,
  Modal,
  Popconfirm,
  Spin,
  Tabs,
  Tag,
  Typography,
  message,
} from 'antd';
import {
  CheckCircleOutlined,
  CloudDownloadOutlined,
  CodeSandboxOutlined,
  DeleteOutlined,
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
  StopOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  addCodexPluginWorkspaceRoot,
  disableCodexPlugin,
  enableCodexPlugin,
  enableCodexPluginsFeature,
  getCodexPluginRuntimeStatus,
  installCodexPlugin,
  listCodexInstalledPlugins,
  listCodexMarketplacePlugins,
  listCodexMarketplaces,
  listCodexPluginWorkspaceRoots,
  removeCodexPluginWorkspaceRoot,
  uninstallCodexPlugin,
} from '@/services/codexApi';
import type {
  CodexInstalledPlugin,
  CodexMarketplacePlugin,
  CodexPluginMarketplace,
  CodexPluginRuntimeStatus,
  CodexPluginWorkspaceRoot,
} from '@/types/codex';
import styles from './CodexPluginsPanel.module.less';

const { Text } = Typography;

type CodexPluginActionKey =
  | `feature:enable`
  | `installed:${string}:enable`
  | `installed:${string}:disable`
  | `installed:${string}:uninstall`
  | `discover:${string}:install`
  | 'workspace:add'
  | `workspace:${string}:remove`;

interface CodexPluginsPanelProps {
  refreshToken?: number;
}

function matchesPlugin(plugin: CodexMarketplacePlugin, normalizedKeyword: string): boolean {
  if (!normalizedKeyword) {
    return true;
  }

  const searchableText = [
    plugin.pluginId,
    plugin.name,
    plugin.displayName,
    plugin.marketplaceName,
    plugin.description,
    plugin.category,
    ...plugin.capabilities,
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();

  return searchableText.includes(normalizedKeyword);
}

const CodexPluginsPanel: React.FC<CodexPluginsPanelProps> = ({ refreshToken = 0 }) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [activeActionKey, setActiveActionKey] = React.useState<CodexPluginActionKey | null>(null);
  const [activeTabKey, setActiveTabKey] = React.useState('installed');
  const [runtimeCollapsed, setRuntimeCollapsed] = React.useState(true);
  const [workspaceCollapsed, setWorkspaceCollapsed] = React.useState(true);
  const [runtimeStatus, setRuntimeStatus] = React.useState<CodexPluginRuntimeStatus | null>(null);
  const [installedPlugins, setInstalledPlugins] = React.useState<CodexInstalledPlugin[]>([]);
  const [marketplaces, setMarketplaces] = React.useState<CodexPluginMarketplace[]>([]);
  const [workspaceRoots, setWorkspaceRoots] = React.useState<CodexPluginWorkspaceRoot[]>([]);
  const [marketplacePlugins, setMarketplacePlugins] = React.useState<CodexMarketplacePlugin[]>([]);
  const [discoverSearchKeyword, setDiscoverSearchKeyword] = React.useState('');

  const deferredDiscoverSearchKeyword = React.useDeferredValue(
    discoverSearchKeyword.trim().toLowerCase(),
  );

  const loadData = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [runtime, installed, marketplaceList, workspaceRootList, discoverPlugins] = await Promise.all([
        getCodexPluginRuntimeStatus(),
        listCodexInstalledPlugins(),
        listCodexMarketplaces(),
        listCodexPluginWorkspaceRoots(),
        listCodexMarketplacePlugins(),
      ]);
      setRuntimeStatus(runtime);
      setInstalledPlugins(installed);
      setMarketplaces(marketplaceList);
      setWorkspaceRoots(workspaceRootList);
      setMarketplacePlugins(discoverPlugins);
    } catch (error) {
      console.error('Failed to load Codex plugins panel data:', error);
      if (!silent) {
        message.error(t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    loadData(true);
  }, [loadData, refreshToken]);

  const runAction = React.useCallback(async (
    actionKey: CodexPluginActionKey,
    action: () => Promise<void>,
    successMessage: string,
  ): Promise<boolean> => {
    setActiveActionKey(actionKey);
    try {
      await action();
      message.success(successMessage);
      await loadData(true);
      return true;
    } catch (error) {
      console.error('Codex plugin action failed:', error);
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
      return false;
    } finally {
      setActiveActionKey(null);
    }
  }, [loadData, t]);

  const filteredMarketplacePlugins = React.useMemo(
    () => marketplacePlugins.filter((plugin) => matchesPlugin(plugin, deferredDiscoverSearchKeyword)),
    [marketplacePlugins, deferredDiscoverSearchKeyword],
  );

  const showManualRestartNotice = React.useCallback(() => {
    Modal.info({
      title: t('codex.plugins.restartRequiredTitle'),
      content: t('codex.plugins.restartRequiredDescription'),
      okText: t('common.confirm'),
    });
  }, [t]);

  const handleInstallPlugin = React.useCallback(async (pluginId: string) => {
    const succeeded = await runAction(
      `discover:${pluginId}:install`,
      () => installCodexPlugin({ pluginId }),
      t('codex.plugins.marketplaces.installSuccess'),
    );
    if (succeeded) {
      showManualRestartNotice();
    }
  }, [runAction, showManualRestartNotice, t]);

  const handleUninstallPlugin = React.useCallback(async (
    pluginId: string,
  ) => {
    const succeeded = await runAction(
      `installed:${pluginId}:uninstall`,
      () => uninstallCodexPlugin({ pluginId }),
      t('codex.plugins.installed.uninstallSuccess'),
    );
    if (succeeded) {
      showManualRestartNotice();
    }
  }, [runAction, showManualRestartNotice, t]);

  const handleTogglePluginEnabled = React.useCallback(async (
    pluginId: string,
    enabled: boolean,
    successMessage: string,
  ) => {
    const succeeded = await runAction(
      `installed:${pluginId}:${enabled ? 'enable' : 'disable'}`,
      () => (
        enabled
          ? enableCodexPlugin({ pluginId })
          : disableCodexPlugin({ pluginId })
      ),
      successMessage,
    );
    if (succeeded) {
      showManualRestartNotice();
    }
  }, [runAction, showManualRestartNotice]);

  const handleEnablePluginsFeature = React.useCallback(async () => {
    const succeeded = await runAction(
      'feature:enable',
      () => enableCodexPluginsFeature(),
      t('codex.plugins.featureEnableSuccess'),
    );
    if (succeeded) {
      showManualRestartNotice();
    }
  }, [runAction, showManualRestartNotice, t]);

  const handlePickWorkspace = React.useCallback(async () => {
    try {
      const selected = await open({
        title: t('codex.plugins.marketplaces.pickDirectoryTitle'),
        multiple: false,
        directory: true,
      });

      if (!selected || typeof selected !== 'string') {
        return;
      }

      const succeeded = await runAction(
        'workspace:add',
        () => addCodexPluginWorkspaceRoot({ path: selected }),
        t('codex.plugins.marketplaces.addSuccess'),
      );

      if (succeeded) {
        setActiveTabKey('marketplaces');
      }
    } catch (error) {
      console.error('Failed to pick Codex workspace directory:', error);
      message.error(t('codex.plugins.marketplaces.pickDirectoryError'));
    }
  }, [runAction, t]);

  const handleRemoveWorkspace = React.useCallback(async (path: string) => {
    await runAction(
      `workspace:${path}:remove`,
      () => removeCodexPluginWorkspaceRoot({ path }),
      t('codex.plugins.marketplaces.removeSuccess'),
    );
  }, [runAction, t]);

  const pluginsFeatureEnabled = runtimeStatus?.pluginsFeatureEnabled ?? false;

  const installedItems = installedPlugins.length === 0 ? (
    <div className={styles.emptyWrap}>
      <Empty description={t('codex.plugins.installed.empty')} />
    </div>
  ) : (
    <div className={styles.list}>
      {installedPlugins.map((plugin) => (
        <div key={plugin.pluginId} className={styles.pluginCard}>
          <div className={styles.pluginHeader}>
            <div className={styles.pluginTitleWrap}>
              <div className={styles.pluginTitleRow}>
                <Text className={styles.pluginTitle}>{plugin.displayName || plugin.name}</Text>
                <Tag color={plugin.enabled ? 'green' : 'default'}>
                  {plugin.enabled
                    ? t('codex.plugins.installed.enabled')
                    : t('codex.plugins.installed.disabled')}
                </Tag>
                <Tag>{plugin.marketplaceName}</Tag>
                <Text code className={styles.pluginId}>{plugin.pluginId}</Text>
                {plugin.activeVersion ? <Tag>{plugin.activeVersion}</Tag> : null}
              </div>
              {plugin.description ? (
                <div className={styles.pluginDescription}>{plugin.description}</div>
              ) : null}
            </div>

            <div className={styles.pluginActions}>
              <Button
                type="text"
                className={styles.ghostActionButton}
                size="small"
                icon={plugin.enabled ? <StopOutlined /> : <CheckCircleOutlined />}
                loading={activeActionKey === `installed:${plugin.pluginId}:${plugin.enabled ? 'disable' : 'enable'}`}
                disabled={Boolean(activeActionKey)}
                onClick={() => void handleTogglePluginEnabled(
                  plugin.pluginId,
                  !plugin.enabled,
                  plugin.enabled
                    ? t('codex.plugins.installed.disableSuccess')
                    : t('codex.plugins.installed.enableSuccess'),
                )}
              >
                {plugin.enabled
                  ? t('codex.plugins.installed.disable')
                  : t('codex.plugins.installed.enable')}
              </Button>
              <Popconfirm
                title={t('codex.plugins.installed.uninstallConfirm', { name: plugin.displayName || plugin.name })}
                onConfirm={() => handleUninstallPlugin(plugin.pluginId)}
                okText={t('common.confirm')}
                cancelText={t('common.cancel')}
              >
                <Button
                  type="text"
                  className={styles.ghostActionButton}
                  size="small"
                  danger
                  icon={<DeleteOutlined />}
                  loading={activeActionKey === `installed:${plugin.pluginId}:uninstall`}
                  disabled={Boolean(activeActionKey)}
                >
                  {t('codex.plugins.installed.uninstall')}
                </Button>
              </Popconfirm>
            </div>
          </div>

          <div className={styles.pluginMeta}>
            {plugin.installedPath ? (
              <div className={styles.pluginMetaItem}>
                <Text className={styles.pluginMetaLabel}>
                  {t('codex.plugins.installed.installPath')}:
                </Text>{' '}
                <Text code>{plugin.installedPath}</Text>
              </div>
            ) : null}
          </div>

          <div className={styles.tagList}>
            {plugin.category ? <Tag color="blue">{plugin.category}</Tag> : null}
            {plugin.hasSkills ? <Tag color="blue">skills</Tag> : null}
            {plugin.hasMcpServers ? <Tag color="purple">MCP</Tag> : null}
            {plugin.hasApps ? <Tag color="cyan">apps</Tag> : null}
            {plugin.capabilities.map((capability) => (
              <Tag key={`${plugin.pluginId}-${capability}`}>{capability}</Tag>
            ))}
          </div>
        </div>
      ))}
    </div>
  );

  const marketplaceItems = (
    <>
      <section className={styles.workspaceSection}>
        <Collapse
          bordered={false}
          className={styles.workspaceCollapse}
          activeKey={workspaceCollapsed ? [] : ['workspace']}
          onChange={(keys) => setWorkspaceCollapsed(!keys.includes('workspace'))}
          items={[
            {
              key: 'workspace',
              label: (
                <div className={styles.workspaceCollapseHeader}>
                  <div className={styles.workspaceHeaderText}>
                    <div className={styles.workspaceTitleRow}>
                      <div className={styles.workspaceTitle}>
                        {t('codex.plugins.marketplaces.workspaceTitle')}
                      </div>
                      <Tag>{workspaceRoots.length}</Tag>
                    </div>
                    <div className={styles.workspaceHint}>
                      {t('codex.plugins.marketplaces.sectionHint')}
                    </div>
                  </div>

                  <Button
                    type="text"
                    className={styles.ghostActionButton}
                    size="small"
                    icon={<PlusOutlined />}
                    disabled={Boolean(activeActionKey)}
                    onClick={(event) => {
                      event.stopPropagation();
                      void handlePickWorkspace();
                    }}
                  >
                    {t('codex.plugins.marketplaces.addWorkspace')}
                  </Button>
                </div>
              ),
              children: workspaceRoots.length === 0 ? (
                <div className={styles.emptyWrap}>
                  <Empty description={t('codex.plugins.marketplaces.workspaceEmpty')} />
                </div>
              ) : (
                <div className={styles.list}>
                  {workspaceRoots.map((workspaceRoot) => (
                    <div key={workspaceRoot.path} className={styles.pluginCard}>
                      <div className={styles.pluginHeader}>
                        <div className={styles.pluginTitleWrap}>
                          <div className={styles.pluginTitleRow}>
                            <Text className={styles.pluginTitle}>{t('codex.plugins.marketplaces.workspacePath')}</Text>
                            <Tag color={workspaceRoot.status === 'ready' ? 'green' : 'default'}>
                              {workspaceRoot.status === 'ready'
                                ? t('codex.plugins.marketplaces.statusReady')
                                : t('codex.plugins.marketplaces.statusMissing')}
                            </Tag>
                            {workspaceRoot.resolutionSource ? (
                              <Tag>
                                {workspaceRoot.resolutionSource === 'direct'
                                  ? t('codex.plugins.marketplaces.resolutionSourceDirect')
                                  : t('codex.plugins.marketplaces.resolutionSourceGitRepo')}
                              </Tag>
                            ) : null}
                          </div>
                        </div>

                        <div className={styles.pluginActions}>
                          <Popconfirm
                            title={t('codex.plugins.marketplaces.removeConfirm')}
                            onConfirm={() => handleRemoveWorkspace(workspaceRoot.path)}
                            okText={t('common.confirm')}
                            cancelText={t('common.cancel')}
                          >
                            <Button
                              type="text"
                              className={styles.ghostActionButton}
                              size="small"
                              danger
                              icon={<DeleteOutlined />}
                              loading={activeActionKey === `workspace:${workspaceRoot.path}:remove`}
                              disabled={Boolean(activeActionKey)}
                            >
                              {t('codex.plugins.marketplaces.remove')}
                            </Button>
                          </Popconfirm>
                        </div>
                      </div>

                      <div className={styles.pluginMeta}>
                        <div className={styles.pluginMetaItem}>
                          <Text className={styles.pluginMetaLabel}>
                            {t('codex.plugins.marketplaces.workspacePath')}:
                          </Text>{' '}
                          <Text code>{workspaceRoot.path}</Text>
                        </div>
                        {workspaceRoot.resolvedMarketplacePath ? (
                          <div className={styles.pluginMetaItem}>
                            <Text className={styles.pluginMetaLabel}>
                              {t('codex.plugins.marketplaces.resolvedMarketplacePath')}:
                            </Text>{' '}
                            <Text code>{workspaceRoot.resolvedMarketplacePath}</Text>
                          </div>
                        ) : null}
                        {workspaceRoot.resolvedRepoRoot ? (
                          <div className={styles.pluginMetaItem}>
                            <Text className={styles.pluginMetaLabel}>
                              {t('codex.plugins.marketplaces.resolvedRepoRoot')}:
                            </Text>{' '}
                            <Text code>{workspaceRoot.resolvedRepoRoot}</Text>
                          </div>
                        ) : null}
                      </div>

                      {workspaceRoot.error ? (
                        <div className={styles.workspaceError}>
                          {workspaceRoot.error}
                        </div>
                      ) : null}
                    </div>
                  ))}
                </div>
              ),
            },
          ]}
        />
      </section>

      {marketplaces.length === 0 ? (
        <div className={styles.emptyWrap}>
          <Empty description={t('codex.plugins.marketplaces.empty')} />
        </div>
      ) : (
        <div className={styles.list}>
          {marketplaces.map((marketplace) => (
            <div key={marketplace.path} className={styles.pluginCard}>
              <div className={styles.pluginHeader}>
                <div className={styles.pluginTitleWrap}>
                  <div className={styles.pluginTitleRow}>
                    <Text className={styles.pluginTitle}>
                      {marketplace.displayName || marketplace.name}
                    </Text>
                    {marketplace.isCurated ? (
                      <Tag color="gold">{t('codex.plugins.marketplaces.curated')}</Tag>
                    ) : null}
                    <Tag>{t('codex.plugins.marketplaces.pluginCount', { count: marketplace.pluginCount })}</Tag>
                    {marketplace.isCurated ? (
                      <span className={styles.marketplaceInlineHint}>
                      {t('codex.plugins.marketplaces.updateTimingHint')}
                      </span>
                    ) : null}
                  </div>
                  {marketplace.description ? (
                    <div className={styles.pluginDescription}>{marketplace.description}</div>
                  ) : null}
                </div>
              </div>

              <div className={styles.pluginMeta}>
                <div className={styles.pluginMetaItem}>
                  <Text className={styles.pluginMetaLabel}>
                    {t('codex.plugins.marketplaces.marketplacePath')}:
                  </Text>{' '}
                  <Text code>{marketplace.path}</Text>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {marketplacePlugins.length > 0 ? (
        <div className={styles.discoverSection}>
          <div className={styles.discoverToolbar}>
            <Input
              allowClear
              value={discoverSearchKeyword}
              onChange={(event) => setDiscoverSearchKeyword(event.target.value)}
              placeholder={t('codex.plugins.marketplaces.searchPlaceholder')}
              prefix={<SearchOutlined />}
            />
          </div>

          {filteredMarketplacePlugins.length === 0 ? (
            <div className={styles.emptyWrap}>
              <Empty description={t('codex.plugins.marketplaces.searchEmpty')} />
            </div>
          ) : (
            <div className={styles.list}>
              {filteredMarketplacePlugins.map((plugin) => (
                <div key={plugin.pluginId} className={styles.pluginCard}>
                  <div className={styles.pluginHeader}>
                    <div className={styles.pluginTitleWrap}>
                      <div className={styles.pluginTitleRow}>
                        <Text className={styles.pluginTitle}>{plugin.displayName || plugin.name}</Text>
                        <Tag>{plugin.marketplaceName}</Tag>
                        <Text code className={styles.pluginId}>{plugin.pluginId}</Text>
                        {plugin.installed ? (
                          <Tag color={plugin.enabled ? 'green' : 'default'}>
                            {plugin.enabled
                              ? t('codex.plugins.marketplaces.enabled')
                              : t('codex.plugins.marketplaces.installed')}
                          </Tag>
                        ) : null}
                        {!plugin.installAvailable ? (
                          <Tag color="red">{t('codex.plugins.marketplaces.notAvailable')}</Tag>
                        ) : null}
                      </div>
                      {plugin.description ? (
                        <div className={styles.pluginDescription}>{plugin.description}</div>
                      ) : null}
                    </div>

                    <div className={styles.pluginActions}>
                      <Button
                        type="text"
                        className={styles.ghostActionButton}
                        size="small"
                        icon={<CloudDownloadOutlined />}
                        loading={activeActionKey === `discover:${plugin.pluginId}:install`}
                        disabled={Boolean(activeActionKey) || plugin.installed || !plugin.installAvailable}
                        onClick={() => void handleInstallPlugin(plugin.pluginId)}
                      >
                        {plugin.installed
                          ? t('codex.plugins.marketplaces.alreadyInstalled')
                          : t('codex.plugins.marketplaces.install')}
                      </Button>
                    </div>
                  </div>

                  <div className={styles.tagList}>
                    {plugin.category ? <Tag color="blue">{plugin.category}</Tag> : null}
                    {plugin.capabilities.map((capability) => (
                      <Tag key={`${plugin.pluginId}-${capability}`}>{capability}</Tag>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : null}
    </>
  );

  return (
    <Spin spinning={loading}>
      <div className={styles.panel}>
        <div className={styles.hintBlock}>
          <div>{t('codex.plugins.sectionHint')}</div>
        </div>

        {!pluginsFeatureEnabled ? (
          <Alert
            className={styles.featureAlert}
            type="warning"
            showIcon
            message={t('codex.plugins.featureDisabled')}
            action={(
              <Button
                type="text"
                className={styles.ghostActionButton}
                size="small"
                loading={activeActionKey === 'feature:enable'}
                disabled={Boolean(activeActionKey)}
                onClick={() => void handleEnablePluginsFeature()}
              >
                {t('codex.plugins.enableFeature')}
              </Button>
            )}
          />
        ) : null}

        {runtimeStatus ? (
          <Collapse
            bordered={false}
            className={styles.runtimeCollapse}
            activeKey={runtimeCollapsed ? [] : ['runtime']}
            onChange={(keys) => setRuntimeCollapsed(!keys.includes('runtime'))}
            items={[
              {
                key: 'runtime',
                label: (
                  <div className={styles.runtimeCollapseHeader}>
                    <div>
                      <div className={styles.runtimeTitle}>{t('codex.plugins.runtime.title')}</div>
                      <span className={styles.runtimeHint}>
                        {t('codex.plugins.runtime.description')}
                      </span>
                    </div>
                    <div className={styles.runtimeTags}>
                      <Tag color={runtimeStatus.mode === 'wslDirect' ? 'cyan' : 'blue'}>
                        {runtimeStatus.mode === 'wslDirect'
                          ? t('codex.plugins.runtime.wslDirect', {
                              distro: runtimeStatus.distro || '-',
                            })
                          : t('codex.plugins.runtime.local')}
                      </Tag>
                      <Tag>
                        {t(`codex.rootPathSource.modal.source${runtimeStatus.source.charAt(0).toUpperCase()}${runtimeStatus.source.slice(1)}`)}
                      </Tag>
                    </div>
                  </div>
                ),
                children: (
                  <div className={styles.runtimeGrid}>
                    <div className={styles.runtimeItem}>
                      <span className={styles.runtimeLabel}>{t('codex.plugins.runtime.rootDir')}</span>
                      <Text code className={styles.runtimeValue}>{runtimeStatus.rootDir}</Text>
                    </div>
                    <div className={styles.runtimeItem}>
                      <span className={styles.runtimeLabel}>{t('codex.plugins.runtime.pluginsDir')}</span>
                      <Text code className={styles.runtimeValue}>{runtimeStatus.pluginsDir}</Text>
                    </div>
                    <div className={styles.runtimeItem}>
                      <span className={styles.runtimeLabel}>{t('codex.plugins.runtime.configPath')}</span>
                      <Text code className={styles.runtimeValue}>{runtimeStatus.configPath}</Text>
                    </div>
                    {runtimeStatus.curatedMarketplacePath ? (
                      <div className={styles.runtimeItem}>
                        <span className={styles.runtimeLabel}>{t('codex.plugins.runtime.curatedMarketplacePath')}</span>
                        <Text code className={styles.runtimeValue}>{runtimeStatus.curatedMarketplacePath}</Text>
                      </div>
                    ) : null}
                    {runtimeStatus.linuxRootDir ? (
                      <div className={styles.runtimeItem}>
                        <span className={styles.runtimeLabel}>{t('codex.plugins.runtime.linuxRootDir')}</span>
                        <Text code className={styles.runtimeValue}>{runtimeStatus.linuxRootDir}</Text>
                      </div>
                    ) : null}
                  </div>
                ),
              },
            ]}
          />
        ) : null}

        <section className={styles.tabsCard}>
          <Tabs
            activeKey={activeTabKey}
            destroyInactiveTabPane={false}
            onChange={setActiveTabKey}
            tabBarExtraContent={{
              right: (
                <div className={styles.tabExtra}>
                  <Button
                    type="text"
                    className={styles.ghostActionButton}
                    size="small"
                    icon={<ReloadOutlined />}
                    disabled={Boolean(activeActionKey)}
                    onClick={() => loadData()}
                  >
                    {t('common.refresh')}
                  </Button>
                  <Button
                    type="text"
                    className={styles.ghostActionButton}
                    size="small"
                    icon={<CodeSandboxOutlined />}
                    onClick={() => openUrl('https://github.com/openai/codex')}
                  >
                    {t('codex.plugins.viewDocs')}
                  </Button>
                </div>
              ),
            }}
            items={[
              {
                key: 'installed',
                label: `${t('codex.plugins.installed.title')} (${installedPlugins.length})`,
                children: installedItems,
              },
              {
                key: 'marketplaces',
                label: `${t('codex.plugins.marketplaces.title')} (${marketplaces.length})`,
                children: marketplaceItems,
              },
            ]}
          />
        </section>
      </div>
    </Spin>
  );
};

export default CodexPluginsPanel;
