import React from 'react';
import {
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
import EnabledTag from '@/components/common/EnabledTag';
import {
  addGrokPluginWorkspaceRoot,
  disableGrokPlugin,
  enableGrokPlugin,
  getGrokPluginDetails,
  getGrokPluginRuntimeStatus,
  installGrokPlugin,
  listGrokInstalledPlugins,
  listGrokMarketplacePlugins,
  listGrokMarketplaces,
  listGrokPluginWorkspaceRoots,
  removeGrokPluginWorkspaceRoot,
  setGrokInstalledPluginsEnabled,
  uninstallGrokPlugin,
  updateGrokPlugin,
  updateGrokPluginMarketplace,
  validateGrokPlugin,
} from '@/services/grokApi';
import type {
  GrokInstalledPlugin,
  GrokMarketplacePlugin,
  GrokPluginMarketplace,
  GrokPluginRuntimeStatus,
  GrokPluginWorkspaceRoot,
} from '@/types/grok';
import styles from './GrokPluginsPanel.module.less';

const { Text } = Typography;

type GrokPluginActionKey =
  | `installed:${string}:enable`
  | `installed:${string}:disable`
  | `installed:${string}:uninstall`
  | `installed:${string}:update`
  | `installed:${string}:details`
  | `installed:${string}:validate`
  | `discover:${string}:install`
  | 'workspace:add'
  | `workspace:${string}:remove`
  | `marketplace:${string}:update`;

interface GrokPluginsPanelProps {
  refreshToken?: number;
}

function matchesPlugin(plugin: GrokMarketplacePlugin, normalizedKeyword: string): boolean {
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

const GrokPluginsPanel: React.FC<GrokPluginsPanelProps> = ({ refreshToken = 0 }) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [activeActionKey, setActiveActionKey] = React.useState<GrokPluginActionKey | null>(null);
  const [activeTabKey, setActiveTabKey] = React.useState('installed');
  const [runtimeCollapsed, setRuntimeCollapsed] = React.useState(true);
  const [workspaceCollapsed, setWorkspaceCollapsed] = React.useState(true);
  const [runtimeStatus, setRuntimeStatus] = React.useState<GrokPluginRuntimeStatus | null>(null);
  const [installedPlugins, setInstalledPlugins] = React.useState<GrokInstalledPlugin[]>([]);
  const [marketplaces, setMarketplaces] = React.useState<GrokPluginMarketplace[]>([]);
  const [workspaceRoots, setWorkspaceRoots] = React.useState<GrokPluginWorkspaceRoot[]>([]);
  const [marketplacePlugins, setMarketplacePlugins] = React.useState<GrokMarketplacePlugin[]>([]);
  const [discoverSearchKeyword, setDiscoverSearchKeyword] = React.useState('');
  const [pluginDetails, setPluginDetails] = React.useState<{ name: string; content: string } | null>(null);

  const deferredDiscoverSearchKeyword = React.useDeferredValue(
    discoverSearchKeyword.trim().toLowerCase(),
  );

  const loadData = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [runtime, installed, marketplaceList, workspaceRootList, discoverPlugins] = await Promise.all([
        getGrokPluginRuntimeStatus(),
        listGrokInstalledPlugins(),
        listGrokMarketplaces(),
        listGrokPluginWorkspaceRoots(),
        listGrokMarketplacePlugins(),
      ]);
      setRuntimeStatus(runtime);
      setInstalledPlugins(installed);
      setMarketplaces(marketplaceList);
      setWorkspaceRoots(workspaceRootList);
      setMarketplacePlugins(discoverPlugins);
    } catch (error) {
      console.error('Failed to load Grok plugins panel data:', error);
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
    actionKey: GrokPluginActionKey,
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
      console.error('Grok plugin action failed:', error);
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
      title: t('grok.plugins.restartRequiredTitle'),
      content: t('grok.plugins.restartRequiredDescription'),
      okText: t('common.confirm'),
    });
  }, [t]);

  const handleInstallPlugin = React.useCallback(async (
    pluginId: string,
    installSource?: string,
  ) => {
    const succeeded = await runAction(
      `discover:${pluginId}:install`,
      () => installGrokPlugin({ pluginId, source: installSource }),
      t('grok.plugins.marketplaces.installSuccess'),
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
      () => uninstallGrokPlugin({ pluginId }),
      t('grok.plugins.installed.uninstallSuccess'),
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
          ? enableGrokPlugin({ pluginId })
          : disableGrokPlugin({ pluginId })
      ),
      successMessage,
    );
    if (succeeded) {
      showManualRestartNotice();
    }
  }, [runAction, showManualRestartNotice]);

  const handleUpdatePlugin = React.useCallback(async (pluginId: string) => {
    const succeeded = await runAction(
      `installed:${pluginId}:update`,
      () => updateGrokPlugin({ pluginId }),
      t('grok.plugins.installed.updateSuccess'),
    );
    if (succeeded) showManualRestartNotice();
  }, [runAction, showManualRestartNotice, t]);

  const handleShowPluginDetails = React.useCallback(async (plugin: GrokInstalledPlugin) => {
    setActiveActionKey(`installed:${plugin.pluginId}:details`);
    try {
      const content = await getGrokPluginDetails({ pluginId: plugin.pluginId });
      setPluginDetails({ name: plugin.displayName || plugin.name, content });
    } catch (error) {
      console.error('Failed to load Grok plugin details:', error);
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    } finally {
      setActiveActionKey(null);
    }
  }, [t]);

  const handleValidatePlugin = React.useCallback(async (plugin: GrokInstalledPlugin) => {
    if (!plugin.installedPath) return;
    await runAction(
      `installed:${plugin.pluginId}:validate`,
      async () => {
        const result = await validateGrokPlugin({
          pluginId: plugin.pluginId,
          source: plugin.installedPath,
        });
        if (result.trim()) message.info(result);
      },
      t('grok.plugins.installed.validateSuccess'),
    );
  }, [runAction, t]);

  const handleUpdateMarketplace = React.useCallback(async (marketplaceName: string) => {
    await runAction(
      `marketplace:${marketplaceName}:update`,
      () => updateGrokPluginMarketplace({ path: marketplaceName }),
      t('grok.plugins.marketplaces.updateSuccess'),
    );
  }, [runAction, t]);

  const handleSetAllInstalledPluginsEnabled = React.useCallback((enabled: boolean) => {
    if (installedPlugins.length === 0) {
      return;
    }

    Modal.confirm({
      title: enabled
        ? t('grok.plugins.installed.enableAllConfirmTitle')
        : t('grok.plugins.installed.disableAllConfirmTitle'),
      content: enabled
        ? t('grok.plugins.installed.enableAllConfirmContent', {
            count: installedPlugins.length,
          })
        : t('grok.plugins.installed.disableAllConfirmContent', {
            count: installedPlugins.length,
          }),
      okText: enabled
        ? t('grok.plugins.installed.enableAll')
        : t('grok.plugins.installed.disableAll'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        const actionKey = `installed:bulk:${enabled ? 'enable' : 'disable'}` as GrokPluginActionKey;
        setActiveActionKey(actionKey);
        try {
          const result = await setGrokInstalledPluginsEnabled({ enabled });
          if (result.failures.length > 0) {
            message.warning(t('grok.plugins.installed.bulkPartialFailure', {
              success: result.updatedCount,
              failed: result.failures.length,
              errors: result.failures.join('; '),
            }));
          } else {
            message.success(
              enabled
                ? t('grok.plugins.installed.enableAllSuccess', {
                    count: result.updatedCount,
                  })
                : t('grok.plugins.installed.disableAllSuccess', {
                    count: result.updatedCount,
                  }),
            );
          }
          await loadData(true);
          showManualRestartNotice();
        } catch (error) {
          console.error('Grok plugin bulk action failed:', error);
          const errorMessage = error instanceof Error ? error.message : String(error);
          message.error(errorMessage || t('common.error'));
        } finally {
          setActiveActionKey(null);
        }
      },
    });
  }, [installedPlugins.length, loadData, showManualRestartNotice, t]);

  const handlePickWorkspace = React.useCallback(async () => {
    try {
      const selected = await open({
        title: t('grok.plugins.marketplaces.pickDirectoryTitle'),
        multiple: false,
        directory: true,
      });

      if (!selected || typeof selected !== 'string') {
        return;
      }

      const succeeded = await runAction(
        'workspace:add',
        () => addGrokPluginWorkspaceRoot({ path: selected }),
        t('grok.plugins.marketplaces.addSuccess'),
      );

      if (succeeded) {
        setActiveTabKey('marketplaces');
      }
    } catch (error) {
      console.error('Failed to pick Grok workspace directory:', error);
      message.error(t('grok.plugins.marketplaces.pickDirectoryError'));
    }
  }, [runAction, t]);

  const handleRemoveWorkspace = React.useCallback(async (path: string) => {
    await runAction(
      `workspace:${path}:remove`,
      () => removeGrokPluginWorkspaceRoot({ path }),
      t('grok.plugins.marketplaces.removeSuccess'),
    );
  }, [runAction, t]);

  const installedPluginCount = installedPlugins.length;
  const enabledInstalledPluginCount = installedPlugins.filter((plugin) => plugin.enabled).length;
  const canEnableAllInstalledPlugins = installedPluginCount > 0
    && enabledInstalledPluginCount < installedPluginCount;
  const canDisableAllInstalledPlugins = enabledInstalledPluginCount > 0;

  const installedItems = installedPlugins.length === 0 ? (
    <div className={styles.emptyWrap}>
      <Empty description={t('grok.plugins.installed.empty')} />
    </div>
  ) : (
    <div className={styles.list}>
      {installedPlugins.map((plugin) => (
        <div key={plugin.pluginId} className={styles.pluginCard}>
          <div className={styles.pluginHeader}>
            <div className={styles.pluginTitleWrap}>
              <div className={styles.pluginTitleRow}>
                <Text className={styles.pluginTitle}>{plugin.displayName || plugin.name}</Text>
                {plugin.enabled ? (
                  <EnabledTag>{t('grok.plugins.installed.enabled')}</EnabledTag>
                ) : (
                  <Tag color="default">{t('grok.plugins.installed.disabled')}</Tag>
                )}
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
                loading={activeActionKey === `installed:${plugin.pluginId}:details`}
                disabled={Boolean(activeActionKey)}
                onClick={() => void handleShowPluginDetails(plugin)}
              >
                {t('grok.plugins.installed.details')}
              </Button>
              <Button
                type="text"
                className={styles.ghostActionButton}
                size="small"
                icon={<ReloadOutlined />}
                loading={activeActionKey === `installed:${plugin.pluginId}:update`}
                disabled={Boolean(activeActionKey)}
                onClick={() => void handleUpdatePlugin(plugin.pluginId)}
              >
                {t('grok.plugins.installed.update')}
              </Button>
              <Button
                type="text"
                className={styles.ghostActionButton}
                size="small"
                loading={activeActionKey === `installed:${plugin.pluginId}:validate`}
                disabled={Boolean(activeActionKey) || !plugin.installedPath}
                onClick={() => void handleValidatePlugin(plugin)}
              >
                {t('grok.plugins.installed.validate')}
              </Button>
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
                    ? t('grok.plugins.installed.disableSuccess')
                    : t('grok.plugins.installed.enableSuccess'),
                )}
              >
                {plugin.enabled
                  ? t('grok.plugins.installed.disable')
                  : t('grok.plugins.installed.enable')}
              </Button>
              <Popconfirm
                title={t('grok.plugins.installed.uninstallConfirm', { name: plugin.displayName || plugin.name })}
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
                  {t('grok.plugins.installed.uninstall')}
                </Button>
              </Popconfirm>
            </div>
          </div>

          <div className={styles.pluginMeta}>
            {plugin.installedPath ? (
              <div className={styles.pluginMetaItem}>
                <Text className={styles.pluginMetaLabel}>
                  {t('grok.plugins.installed.installPath')}:
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
                        {t('grok.plugins.marketplaces.workspaceTitle')}
                      </div>
                      <Tag>{workspaceRoots.length}</Tag>
                    </div>
                    <div className={styles.workspaceHint}>
                      {t('grok.plugins.marketplaces.sectionHint')}
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
                    {t('grok.plugins.marketplaces.addWorkspace')}
                  </Button>
                </div>
              ),
              children: workspaceRoots.length === 0 ? (
                <div className={styles.emptyWrap}>
                  <Empty description={t('grok.plugins.marketplaces.workspaceEmpty')} />
                </div>
              ) : (
                <div className={styles.list}>
                  {workspaceRoots.map((workspaceRoot) => (
                    <div key={workspaceRoot.path} className={styles.pluginCard}>
                      <div className={styles.pluginHeader}>
                        <div className={styles.pluginTitleWrap}>
                          <div className={styles.pluginTitleRow}>
                            <Text className={styles.pluginTitle}>{t('grok.plugins.marketplaces.workspacePath')}</Text>
                            <Tag color={workspaceRoot.status === 'ready' ? 'green' : 'default'}>
                              {workspaceRoot.status === 'ready'
                                ? t('grok.plugins.marketplaces.statusReady')
                                : t('grok.plugins.marketplaces.statusMissing')}
                            </Tag>
                            {workspaceRoot.resolutionSource ? (
                              <Tag>
                                {workspaceRoot.resolutionSource === 'direct'
                                  ? t('grok.plugins.marketplaces.resolutionSourceDirect')
                                  : t('grok.plugins.marketplaces.resolutionSourceGitRepo')}
                              </Tag>
                            ) : null}
                          </div>
                        </div>

                        <div className={styles.pluginActions}>
                          <Popconfirm
                            title={t('grok.plugins.marketplaces.removeConfirm')}
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
                              {t('grok.plugins.marketplaces.remove')}
                            </Button>
                          </Popconfirm>
                        </div>
                      </div>

                      <div className={styles.pluginMeta}>
                        <div className={styles.pluginMetaItem}>
                          <Text className={styles.pluginMetaLabel}>
                            {t('grok.plugins.marketplaces.workspacePath')}:
                          </Text>{' '}
                          <Text code>{workspaceRoot.path}</Text>
                        </div>
                        {workspaceRoot.resolvedMarketplacePath ? (
                          <div className={styles.pluginMetaItem}>
                            <Text className={styles.pluginMetaLabel}>
                              {t('grok.plugins.marketplaces.resolvedMarketplacePath')}:
                            </Text>{' '}
                            <Text code>{workspaceRoot.resolvedMarketplacePath}</Text>
                          </div>
                        ) : null}
                        {workspaceRoot.resolvedRepoRoot ? (
                          <div className={styles.pluginMetaItem}>
                            <Text className={styles.pluginMetaLabel}>
                              {t('grok.plugins.marketplaces.resolvedRepoRoot')}:
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
          <Empty description={t('grok.plugins.marketplaces.empty')} />
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
                      <Tag color="gold">{t('grok.plugins.marketplaces.curated')}</Tag>
                    ) : null}
                    <Tag>{t('grok.plugins.marketplaces.pluginCount', { count: marketplace.pluginCount })}</Tag>
                    {marketplace.isCurated ? (
                      <span className={styles.marketplaceInlineHint}>
                      {t('grok.plugins.marketplaces.updateTimingHint')}
                      </span>
                    ) : null}
                  </div>
                  {marketplace.description ? (
                    <div className={styles.pluginDescription}>{marketplace.description}</div>
                  ) : null}
                </div>
                <div className={styles.pluginActions}>
                  <Button
                    type="text"
                    className={styles.ghostActionButton}
                    size="small"
                    icon={<ReloadOutlined />}
                    loading={activeActionKey === `marketplace:${marketplace.name}:update`}
                    disabled={Boolean(activeActionKey)}
                    onClick={() => void handleUpdateMarketplace(marketplace.name)}
                  >
                    {t('grok.plugins.marketplaces.update')}
                  </Button>
                </div>
              </div>

              <div className={styles.pluginMeta}>
                <div className={styles.pluginMetaItem}>
                  <Text className={styles.pluginMetaLabel}>
                    {t('grok.plugins.marketplaces.marketplacePath')}:
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
              placeholder={t('grok.plugins.marketplaces.searchPlaceholder')}
              prefix={<SearchOutlined />}
            />
          </div>

          {filteredMarketplacePlugins.length === 0 ? (
            <div className={styles.emptyWrap}>
              <Empty description={t('grok.plugins.marketplaces.searchEmpty')} />
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
                          plugin.enabled ? (
                            <EnabledTag>{t('grok.plugins.marketplaces.enabled')}</EnabledTag>
                          ) : (
                            <Tag color="default">{t('grok.plugins.marketplaces.installed')}</Tag>
                          )
                        ) : null}
                        {!plugin.installAvailable ? (
                          <Tag color="red">{t('grok.plugins.marketplaces.notAvailable')}</Tag>
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
                        onClick={() => void handleInstallPlugin(
                          plugin.pluginId,
                          plugin.installSource,
                        )}
                      >
                        {plugin.installed
                          ? t('grok.plugins.marketplaces.alreadyInstalled')
                          : t('grok.plugins.marketplaces.install')}
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
    <>
      <Spin spinning={loading}>
      <div className={styles.panel}>
        <div className={styles.hintBlock}>
          <div>{t('grok.plugins.sectionHint')}</div>
        </div>

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
                      <div className={styles.runtimeTitle}>{t('grok.plugins.runtime.title')}</div>
                      <span className={styles.runtimeHint}>
                        {t('grok.plugins.runtime.description')}
                      </span>
                    </div>
                    <div className={styles.runtimeTags}>
                      <Tag color={runtimeStatus.mode === 'wslDirect' ? 'cyan' : 'blue'}>
                        {runtimeStatus.mode === 'wslDirect'
                          ? t('grok.plugins.runtime.wslDirect', {
                              distro: runtimeStatus.distro || '-',
                            })
                          : t('grok.plugins.runtime.local')}
                      </Tag>
                      <Tag>
                        {t(`grok.rootPathSource.modal.source${runtimeStatus.source.charAt(0).toUpperCase()}${runtimeStatus.source.slice(1)}`)}
                      </Tag>
                    </div>
                  </div>
                ),
                children: (
                  <div className={styles.runtimeGrid}>
                    <div className={styles.runtimeItem}>
                      <span className={styles.runtimeLabel}>{t('grok.plugins.runtime.rootDir')}</span>
                      <Text code className={styles.runtimeValue}>{runtimeStatus.rootDir}</Text>
                    </div>
                    <div className={styles.runtimeItem}>
                      <span className={styles.runtimeLabel}>{t('grok.plugins.runtime.pluginsDir')}</span>
                      <Text code className={styles.runtimeValue}>{runtimeStatus.pluginsDir}</Text>
                    </div>
                    <div className={styles.runtimeItem}>
                      <span className={styles.runtimeLabel}>{t('grok.plugins.runtime.configPath')}</span>
                      <Text code className={styles.runtimeValue}>{runtimeStatus.configPath}</Text>
                    </div>
                    {runtimeStatus.curatedMarketplacePath ? (
                      <div className={styles.runtimeItem}>
                        <span className={styles.runtimeLabel}>{t('grok.plugins.runtime.curatedMarketplacePath')}</span>
                        <Text code className={styles.runtimeValue}>{runtimeStatus.curatedMarketplacePath}</Text>
                      </div>
                    ) : null}
                    {runtimeStatus.linuxRootDir ? (
                      <div className={styles.runtimeItem}>
                        <span className={styles.runtimeLabel}>{t('grok.plugins.runtime.linuxRootDir')}</span>
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
            destroyOnHidden={false}
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
                  {activeTabKey === 'installed' ? (
                    <>
                      {canEnableAllInstalledPlugins ? (
                        <Button
                          type="text"
                          className={styles.ghostActionButton}
                          size="small"
                          icon={<CheckCircleOutlined />}
                          loading={activeActionKey === 'installed:bulk:enable'}
                          disabled={Boolean(activeActionKey)}
                          onClick={() => handleSetAllInstalledPluginsEnabled(true)}
                        >
                          {t('grok.plugins.installed.enableAll')}
                        </Button>
                      ) : null}
                      {canDisableAllInstalledPlugins ? (
                        <Button
                          type="text"
                          className={styles.ghostActionButton}
                          size="small"
                          icon={<StopOutlined />}
                          loading={activeActionKey === 'installed:bulk:disable'}
                          disabled={Boolean(activeActionKey)}
                          onClick={() => handleSetAllInstalledPluginsEnabled(false)}
                        >
                          {t('grok.plugins.installed.disableAll')}
                        </Button>
                      ) : null}
                    </>
                  ) : null}
                  <Button
                    type="text"
                    className={styles.ghostActionButton}
                    size="small"
                    icon={<CodeSandboxOutlined />}
                    onClick={() => openUrl('https://docs.x.ai/build/overview')}
                  >
                    {t('grok.plugins.viewDocs')}
                  </Button>
                </div>
              ),
            }}
            items={[
              {
                key: 'installed',
                label: `${t('grok.plugins.installed.title')} (${installedPlugins.length})`,
                children: installedItems,
              },
              {
                key: 'marketplaces',
                label: `${t('grok.plugins.marketplaces.title')} (${marketplaces.length})`,
                children: marketplaceItems,
              },
            ]}
          />
        </section>
      </div>
      </Spin>
      <Modal
        open={Boolean(pluginDetails)}
        title={pluginDetails?.name}
        footer={null}
        width={720}
        onCancel={() => setPluginDetails(null)}
      >
        <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', margin: 0 }}>
          {pluginDetails?.content}
        </pre>
      </Modal>
    </>
  );
};

export default GrokPluginsPanel;
