import React from 'react';
import {
  Alert,
  App,
  Button,
  Collapse,
  Empty,
  Input,
  Modal,
  Space,
  Tag,
  Tooltip,
  Typography,
} from 'antd';
import {
  AppstoreAddOutlined,
  DeleteOutlined,
  DownloadOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  PlusOutlined,
  ReloadOutlined,
  SyncOutlined,
} from '@ant-design/icons';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';

import { MagicContextSettings } from '@/features/coding/shared/magicContext';
import {
  installPiExtension,
  listPiExtensions,
  uninstallPiExtension,
  updatePiExtensions,
} from '@/services/piApi';
import type {
  PiExtensionCommandResult,
  PiExtensionKind,
  PiExtensionListResult,
  PiExtensionSummary,
} from '@/types/pi';

import styles from './PiExtensionsSection.module.less';

const { Text, Paragraph } = Typography;
const PI_PACKAGES_URL = 'https://pi.dev/packages';

interface RecommendedPiExtension {
  name: string;
  installSource: string;
  descriptionKey: string;
  detailUrl: string;
}

const RECOMMENDED_PI_EXTENSIONS: RecommendedPiExtension[] = [
  {
    name: 'context-mode',
    installSource: 'npm:context-mode',
    descriptionKey: 'pi.extensions.recommended.contextMode',
    detailUrl: 'https://pi.dev/packages/context-mode?name=context-mode',
  },
  {
    name: '@cortexkit/pi-magic-context',
    installSource: 'npm:@cortexkit/pi-magic-context',
    descriptionKey: 'pi.extensions.recommended.magicContext',
    detailUrl: 'https://github.com/cortexkit/magic-context',
  },
  {
    name: 'pi-web-access',
    installSource: 'npm:pi-web-access',
    descriptionKey: 'pi.extensions.recommended.webAccess',
    detailUrl: 'https://pi.dev/packages/pi-web-access?name=pi-web-access',
  },
  {
    name: 'pi-mcp-adapter',
    installSource: 'npm:pi-mcp-adapter',
    descriptionKey: 'pi.extensions.recommended.mcpAdapter',
    detailUrl: 'https://pi.dev/packages/pi-mcp-adapter?name=pi-mcp-adapter',
  },
  {
    name: '@samfp/pi-memory',
    installSource: 'npm:@samfp/pi-memory',
    descriptionKey: 'pi.extensions.recommended.memory',
    detailUrl: 'https://pi.dev/packages/@samfp/pi-memory?name=%40samfp%2Fpi-memory',
  },
  {
    name: 'pi-subagents',
    installSource: 'npm:pi-subagents',
    descriptionKey: 'pi.extensions.recommended.subagents',
    detailUrl: 'https://pi.dev/packages/pi-subagents?name=pi-subagents',
  },
];

const normalizeSource = (source: string): string => source.trim().toLowerCase();

const getSourceDisplayName = (source: string): string => (
  source.replace(/^(?:npm|file|github|git):/i, '')
);

const isRecommendedInstalled = (
  extensions: PiExtensionSummary[],
  installSource: string,
): boolean => {
  const normalizedInstallSource = normalizeSource(installSource);
  const normalizedPackageName = normalizedInstallSource.startsWith('npm:')
    ? normalizedInstallSource.slice(4)
    : normalizedInstallSource;

  return extensions.some((extension) => {
    const normalizedSource = normalizeSource(extension.source);
    return normalizedSource === normalizedInstallSource || normalizedSource === normalizedPackageName;
  });
};

const isMagicContextInstalled = (extensions: PiExtensionSummary[]): boolean => (
  isRecommendedInstalled(extensions, 'npm:@cortexkit/pi-magic-context')
);

const PiExtensionsSection: React.FC = () => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [data, setData] = React.useState<PiExtensionListResult | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [customSource, setCustomSource] = React.useState('');
  const [installingSources, setInstallingSources] = React.useState<Set<string>>(() => new Set());
  const [uninstallingSource, setUninstallingSource] = React.useState<string | null>(null);
  const [pendingUninstall, setPendingUninstall] = React.useState<PiExtensionSummary | null>(null);
  const [updating, setUpdating] = React.useState(false);
  const [commandResult, setCommandResult] = React.useState<PiExtensionCommandResult | null>(null);

  const loadExtensions = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await listPiExtensions();
      setData(result);
    } catch (loadError) {
      const messageText = loadError instanceof Error ? loadError.message : String(loadError);
      setError(messageText);
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void loadExtensions();
  }, [loadExtensions]);

  const extensions = data?.extensions ?? [];
  const magicContextInstalled = isMagicContextInstalled(extensions);

  const handleInstall = async (source: string) => {
    const normalizedSource = source.trim();
    if (!normalizedSource) {
      void message.warning(t('pi.extensions.sourceRequired'));
      return;
    }

    setInstallingSources((current) => new Set(current).add(normalizedSource));
    try {
      await installPiExtension({ source: normalizedSource });
      void message.success(t('pi.extensions.installSuccess'));
      setCustomSource('');
      await loadExtensions();
    } catch (installError) {
      void message.error(
        installError instanceof Error ? installError.message : String(installError),
      );
    } finally {
      setInstallingSources((current) => {
        const next = new Set(current);
        next.delete(normalizedSource);
        return next;
      });
    }
  };

  const handleConfirmUninstall = async () => {
    if (!pendingUninstall) {
      return;
    }
    const extension = pendingUninstall;
    setUninstallingSource(extension.source);
    try {
      await uninstallPiExtension({
        source: extension.source,
        scope: extension.scope,
        kind: extension.kind,
        path: extension.path,
      });
      void message.success(
        extension.kind === 'package'
          ? t('pi.extensions.uninstallSuccess')
          : t('pi.extensions.deleteSuccess'),
      );
      setPendingUninstall(null);
      await loadExtensions();
    } catch (uninstallError) {
      void message.error(
        uninstallError instanceof Error ? uninstallError.message : String(uninstallError),
      );
    } finally {
      setUninstallingSource(null);
    }
  };

  const handleUpdateAll = async () => {
    setUpdating(true);
    try {
      const result = await updatePiExtensions();
      setCommandResult(result);
      await loadExtensions();
    } catch (updateError) {
      void message.error(
        updateError instanceof Error ? updateError.message : String(updateError),
      );
    } finally {
      setUpdating(false);
    }
  };

  const handleOpenExtensionsFolder = async () => {
    if (!data?.extensionsPath) {
      return;
    }
    try {
      await invoke('open_folder', { path: data.extensionsPath });
    } catch (openError) {
      void message.error(openError instanceof Error ? openError.message : String(openError));
    }
  };

  const handleOpenPackagesFolder = async () => {
    if (!data?.packagesPath) {
      return;
    }
    try {
      await invoke('open_folder', { path: data.packagesPath });
    } catch (openError) {
      void message.error(openError instanceof Error ? openError.message : String(openError));
    }
  };

  const renderKindLabel = (kind: PiExtensionKind) => {
    switch (kind) {
      case 'local_file':
        return t('pi.extensions.kindLocalFile');
      case 'local_directory':
        return t('pi.extensions.kindLocalDirectory');
      case 'package':
      default:
        return t('pi.extensions.kindPackage');
    }
  };

  const renderScopeLabel = (scope: PiExtensionSummary['scope']) => {
    switch (scope) {
      case 'project':
        return t('pi.extensions.scopeProject');
      case 'user':
        return t('pi.extensions.scopeUser');
      case 'unknown':
      default:
        return t('pi.extensions.scopeUnknown');
    }
  };

  const renderRecommendedExtension = (extension: RecommendedPiExtension) => {
    const installed = isRecommendedInstalled(extensions, extension.installSource);
    const installing = installingSources.has(extension.installSource);

    return (
      <div key={extension.installSource} className={styles.recommendedItem}>
        <div className={styles.recommendedContent}>
          <div className={styles.recommendedTitleRow}>
            <Space size={6} wrap>
              <Text strong>{extension.name}</Text>
              <Text code className={styles.inlineSourceText}>
                {extension.installSource}
              </Text>
              {installed && <Tag color="success">{t('pi.extensions.installed')}</Tag>}
            </Space>
          </div>
          <Text type="secondary" className={styles.recommendedDescription}>
            {t(extension.descriptionKey)}
          </Text>
        </div>
        <Space size={6} className={styles.itemActions}>
          <Tooltip title={t('pi.extensions.openPackage')}>
            <Button
              type="text"
              size="small"
              icon={<LinkOutlined />}
              onClick={() => {
                void openUrl(extension.detailUrl);
              }}
            />
          </Tooltip>
          <Button
            size="small"
            icon={<DownloadOutlined />}
            disabled={installed}
            loading={installing}
            onClick={() => {
              void handleInstall(extension.installSource);
            }}
          >
            {installed ? t('pi.extensions.installed') : t('pi.extensions.install')}
          </Button>
        </Space>
      </div>
    );
  };

  const renderInstalledExtension = (extension: PiExtensionSummary) => {
    const isPackage = extension.kind === 'package';
    const actionText = isPackage ? t('pi.extensions.uninstall') : t('pi.extensions.deleteLocal');

    return (
      <div key={extension.id} className={styles.installedItem}>
        <div className={styles.installedContent}>
          <div className={styles.installedTitleRow}>
            <Text strong>{getSourceDisplayName(extension.source)}</Text>
            <Space size={4} wrap>
              {extension.builtIn && <Tag color="blue">{t('pi.extensions.builtIn')}</Tag>}
              <Tag>{renderKindLabel(extension.kind)}</Tag>
              <Tag>{renderScopeLabel(extension.scope)}</Tag>
              {extension.currentVersion && (
                <Tag>{t('pi.extensions.currentVersion', { version: extension.currentVersion })}</Tag>
              )}
            </Space>
          </div>
          <Text code className={styles.sourceText}>
            {extension.source}
          </Text>
          {extension.path && (
            <Text type="secondary" className={styles.pathText}>
              {extension.path}
            </Text>
          )}
        </div>
        {!extension.builtIn && (
          <Button
            danger
            size="small"
            className={styles.installedActionButton}
            icon={<DeleteOutlined />}
            loading={uninstallingSource === extension.source}
            onClick={() => setPendingUninstall(extension)}
          >
            {actionText}
          </Button>
        )}
      </div>
    );
  };

  return (
    <>
      <Collapse
        className={styles.collapseCard}
        items={[
          {
            key: 'extensions',
            label: (
              <Space>
                <AppstoreAddOutlined />
                <Text strong>{t('pi.extensions.title')}</Text>
              </Space>
            ),
            extra: (
              <Space onClick={(event) => event.stopPropagation()}>
                <Button
                  type="link"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  disabled={!data?.extensionsPath}
                  onClick={handleOpenExtensionsFolder}
                >
                  {t('pi.extensions.openDirectory')}
                </Button>
                <Button
                  type="link"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  disabled={!data?.packagesPath}
                  onClick={handleOpenPackagesFolder}
                >
                  {t('pi.extensions.openPackagesDirectory')}
                </Button>
                <Button
                  type="link"
                  size="small"
                  icon={<SyncOutlined />}
                  loading={updating}
                  onClick={handleUpdateAll}
                >
                  {t('pi.extensions.updateAll')}
                </Button>
                <Button
                  type="link"
                  size="small"
                  icon={<ReloadOutlined />}
                  loading={loading}
                  onClick={loadExtensions}
                >
                  {t('common.refresh')}
                </Button>
              </Space>
            ),
            children: (
              <div className={styles.content}>
                {error && (
                  <Alert
                    type="error"
                    showIcon
                    message={t('pi.extensions.loadFailed')}
                    description={error}
                  />
                )}
                <div className={styles.metaRow}>
                  <Text type="secondary">{t('pi.extensions.pathLabel')}</Text>
                  <Text code className={styles.pathText}>
                    {data?.extensionsPath || '-'}
                  </Text>
                  <Text type="secondary">{t('pi.extensions.packagesPathLabel')}</Text>
                  <Text code className={styles.pathText}>
                    {data?.packagesPath || '-'}
                  </Text>
                  <Text type="secondary">{t('pi.extensions.restartHint')}</Text>
                </div>

                <div className={styles.customInstallRow}>
                  <Input
                    value={customSource}
                    onChange={(event) => setCustomSource(event.target.value)}
                    onPressEnter={() => {
                      void handleInstall(customSource);
                    }}
                    placeholder={t('pi.extensions.sourcePlaceholder')}
                    allowClear
                  />
                  <Button
                    type="primary"
                    icon={<PlusOutlined />}
                    loading={installingSources.has(customSource.trim())}
                    onClick={() => {
                      void handleInstall(customSource);
                    }}
                  >
                    {t('pi.extensions.install')}
                  </Button>
                </div>

                <Collapse
                  className={styles.innerCollapse}
                  size="small"
                  bordered={false}
                  items={[
                    {
                      key: 'recommended',
                      label: (
                        <Space>
                          <Text strong>{t('pi.extensions.recommendedTitle')}</Text>
                          <Button
                            type="link"
                            size="small"
                            className={styles.officialPackagesLink}
                            icon={<LinkOutlined />}
                            onClick={(event) => {
                              event.stopPropagation();
                              void openUrl(PI_PACKAGES_URL);
                            }}
                          >
                            {t('pi.extensions.officialPackages')}
                          </Button>
                          <Text type="secondary">
                            {t('pi.extensions.recommendedCount', {
                              count: RECOMMENDED_PI_EXTENSIONS.length,
                            })}
                          </Text>
                        </Space>
                      ),
                      children: (
                        <div className={styles.recommendedList}>
                          {RECOMMENDED_PI_EXTENSIONS.map(renderRecommendedExtension)}
                        </div>
                      ),
                    },
                  ]}
                />

                <Collapse
                  className={styles.innerCollapse}
                  size="small"
                  bordered={false}
                  defaultActiveKey={['installed']}
                  items={[
                    {
                      key: 'installed',
                      label: (
                        <Space>
                          <Text strong>{t('pi.extensions.installedTitle')}</Text>
                          <Text type="secondary">
                            {t('pi.extensions.count', { count: extensions.length })}
                          </Text>
                        </Space>
                      ),
                      children: loading && !data ? (
                        <div className={styles.loadingText}>{t('pi.extensions.loading')}</div>
                      ) : extensions.length > 0 ? (
                        <div className={styles.installedList}>
                          {extensions.map(renderInstalledExtension)}
                        </div>
                      ) : (
                        <Empty
                          image={Empty.PRESENTED_IMAGE_SIMPLE}
                          description={t('pi.extensions.emptyInstalled')}
                        />
                      ),
                    },
                  ]}
                />

                {magicContextInstalled && (
                  <MagicContextSettings harness="pi" />
                )}
              </div>
            ),
          },
        ]}
      />

      <Modal
        title={pendingUninstall?.kind === 'package'
          ? t('pi.extensions.confirmUninstallTitle')
          : t('pi.extensions.confirmDeleteTitle')}
        open={!!pendingUninstall}
        okText={pendingUninstall?.kind === 'package'
          ? t('pi.extensions.uninstall')
          : t('pi.extensions.deleteLocal')}
        okButtonProps={{
          danger: true,
          loading: Boolean(pendingUninstall && uninstallingSource === pendingUninstall.source),
        }}
        cancelText={t('common.cancel')}
        onOk={handleConfirmUninstall}
        onCancel={() => setPendingUninstall(null)}
        destroyOnHidden
      >
        {pendingUninstall && (
          <div className={styles.confirmContent}>
            <Paragraph>
              {pendingUninstall.kind === 'package'
                ? t('pi.extensions.confirmUninstallContent')
                : t('pi.extensions.confirmDeleteContent')}
            </Paragraph>
            <Text code>{pendingUninstall.source}</Text>
            {pendingUninstall.path && (
              <Text type="secondary" className={styles.pathText}>
                {pendingUninstall.path}
              </Text>
            )}
          </div>
        )}
      </Modal>

      <Modal
        title={t('pi.extensions.updateResultTitle')}
        open={!!commandResult}
        footer={[
          <Button key="close" type="primary" onClick={() => setCommandResult(null)}>
            {t('common.close')}
          </Button>,
        ]}
        onCancel={() => setCommandResult(null)}
        destroyOnHidden
      >
        {commandResult && (
          <pre className={styles.commandOutput}>
            {`${commandResult.command}\n${commandResult.output || t('pi.extensions.emptyCommandOutput')}`}
          </pre>
        )}
      </Modal>
    </>
  );
};

export default PiExtensionsSection;
