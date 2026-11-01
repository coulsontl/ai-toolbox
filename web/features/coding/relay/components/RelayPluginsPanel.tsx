import React from 'react';
import { Empty, Input, Modal, Spin, Tooltip } from 'antd';
import {
  AlertCircle,
  CircleStop,
  Download,
  KeyRound,
  Package,
  Play,
  RefreshCw,
  Trash2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import {
  completeRelayOAuth,
  getRelayBunStatus,
  getRelayHostStatus,
  installRelayBun,
  installRelayPlugin,
  listRelayHostProviders,
  listRelayPluginAccounts,
  listRelayPlugins,
  restartRelayHost,
  signInRelayProvider,
  signOutRelayProvider,
  stopRelayHost,
  uninstallRelayBun,
  uninstallRelayPlugin,
  type RelayHostProvider,
  type RelayHostStatus,
  type RelayPlugin,
  type RelayPluginAccount,
  type BunDownloadProgress,
  type BunRuntimeStatus,
} from '@/services';
import styles from './RelayPluginsPanel.module.less';

/** Tauri event the Bun installer reports progress on. */
const BUN_PROGRESS_EVENT = 'relay://bun-progress';

interface PendingSignIn {
  provider: string;
  session: string;
  url: string;
  instructions: string;
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

const RelayPluginsPanel: React.FC = () => {
  const { t } = useTranslation();

  const [bun, setBun] = React.useState<BunRuntimeStatus | null>(null);
  const [host, setHost] = React.useState<RelayHostStatus | null>(null);
  const [providers, setProviders] = React.useState<RelayHostProvider[]>([]);
  const [plugins, setPlugins] = React.useState<RelayPlugin[]>([]);
  const [accounts, setAccounts] = React.useState<RelayPluginAccount[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [progress, setProgress] = React.useState<BunDownloadProgress | null>(null);
  const [installOpen, setInstallOpen] = React.useState(false);
  const [installSpec, setInstallSpec] = React.useState('');
  const [installError, setInstallError] = React.useState('');
  const [signInProvider, setSignInProvider] = React.useState<RelayHostProvider | null>(null);
  const [signInKey, setSignInKey] = React.useState('');
  const [signInError, setSignInError] = React.useState('');
  const [pending, setPending] = React.useState<PendingSignIn | null>(null);
  const [oauthCode, setOauthCode] = React.useState('');
  const [oauthError, setOauthError] = React.useState('');

  const load = React.useCallback(async () => {
    try {
      setError(null);
      const [bunStatus, hostStatus, installed] = await Promise.all([
        getRelayBunStatus(),
        getRelayHostStatus(),
        listRelayPlugins(),
      ]);
      setBun(bunStatus);
      setHost(hostStatus);
      setPlugins(installed);
      if (bunStatus.installed) {
        // Providers come from the live process, so a plugin that failed to load
        // is absent here rather than showing as if it worked.
        const [liveProviders, storedAccounts] = await Promise.all([
          listRelayHostProviders().catch(() => [] as RelayHostProvider[]),
          listRelayPluginAccounts(),
        ]);
        setProviders(liveProviders);
        setAccounts(storedAccounts);
      } else {
        setProviders([]);
        setAccounts([]);
      }
    } catch (loadError) {
      setError(String(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  React.useEffect(() => {
    const unlisten = listen<BunDownloadProgress>(BUN_PROGRESS_EVENT, (event) => {
      setProgress(event.payload);
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  const run = async (label: string, action: () => Promise<void>) => {
    setBusy(label);
    setError(null);
    try {
      await action();
    } catch (actionError) {
      setError(String(actionError));
    } finally {
      setBusy(null);
    }
  };

  const handleInstallBun = () =>
    run('bun', async () => {
      setProgress({ downloaded: 0, total: 0, speed: 0 });
      try {
        const status = await installRelayBun();
        setBun(status);
      } finally {
        setProgress(null);
      }
      await load();
    });

  const handleInstallPlugin = () =>
    run('install', async () => {
      try {
        const result = await installRelayPlugin(installSpec.trim());
        if (result.hostError) {
          setInstallError(result.hostError);
          await load();
          return;
        }
        setInstallOpen(false);
        setInstallSpec('');
        setInstallError('');
        await load();
      } catch (installFailure) {
        setInstallError(String(installFailure));
      }
    });

  const handleSignIn = () =>
    run('signin', async () => {
      if (!signInProvider) return;
      try {
        const result = await signInRelayProvider({
          provider: signInProvider.id,
          key: signInKey,
          method: 'api',
        });
        if (result.session) {
          // An OAuth flow: the host is waiting for the code the user pastes.
          setPending({
            provider: result.provider,
            session: result.session,
            url: result.url,
            instructions: result.instructions,
          });
          setSignInProvider(null);
          setSignInKey('');
          return;
        }
        setSignInProvider(null);
        setSignInKey('');
        setSignInError('');
        await load();
      } catch (signInFailure) {
        setSignInError(String(signInFailure));
      }
    });

  const handleOAuth = () =>
    run('oauth', async () => {
      if (!pending) return;
      try {
        await completeRelayOAuth(pending.session, oauthCode.trim());
        setPending(null);
        setOauthCode('');
        setOauthError('');
        await load();
      } catch (callbackFailure) {
        setOauthError(String(callbackFailure));
      }
    });

  const accountsFor = (providerId: string) =>
    accounts.filter((account) => account.plugin_provider_id === providerId);

  if (loading) {
    return (
      <div className={styles.center}>
        <Spin />
      </div>
    );
  }

  return (
    <div className={styles.panel}>
      {error ? (
        <div className={styles.notice}>
          <AlertCircle size={14} />
          <span>{error}</span>
        </div>
      ) : null}

      {/* Bun 运行时：插件宿主跑在它上面，没装就整页降级。 */}
      <section className={styles.section}>
        <div className={styles.sectionHead}>
          <div>
            <div className={styles.sectionTitle}>{t('relay.plugins.runtime.title')}</div>
            <div className={styles.sectionHint}>
              {bun?.installed
                ? t('relay.plugins.runtime.installed', {
                    version: bun.version,
                    asset: bun.asset,
                  })
                : t('relay.plugins.runtime.missing')}
            </div>
          </div>
          <div className={styles.actions}>
            {bun?.installed ? (
              <>
                <span className={styles.pill}>
                  <span className={`${styles.dot} ${host?.running ? styles.dotOn : ''}`} />
                  {host?.running
                    ? t('relay.plugins.runtime.hostRunning')
                    : t('relay.plugins.runtime.hostStopped')}
                </span>
                <button
                  type="button"
                  className={styles.button}
                  disabled={busy !== null}
                  onClick={() =>
                    run('restart', async () => {
                      await restartRelayHost();
                      await load();
                    })
                  }
                >
                  <Play size={13} />
                  {t('relay.plugins.runtime.restart')}
                </button>
                <button
                  type="button"
                  className={styles.button}
                  disabled={busy !== null}
                  onClick={() =>
                    run('stop', async () => {
                      await stopRelayHost();
                      await load();
                    })
                  }
                >
                  <CircleStop size={13} />
                  {t('relay.plugins.runtime.stop')}
                </button>
                <button
                  type="button"
                  className={styles.button}
                  disabled={busy !== null}
                  onClick={() => run('uninstall-bun', async () => {
                    await uninstallRelayBun();
                    await load();
                  })}
                >
                  <Trash2 size={13} />
                  {t('relay.plugins.runtime.remove')}
                </button>
              </>
            ) : (
              <button
                type="button"
                className={`${styles.button} ${styles.buttonPrimary}`}
                disabled={busy !== null || bun?.verified === false}
                onClick={handleInstallBun}
              >
                <Download size={13} />
                {busy === 'bun'
                  ? t('relay.plugins.runtime.downloading')
                  : t('relay.plugins.runtime.install')}
              </button>
            )}
          </div>
        </div>
        {progress ? (
          <div className={styles.progressRow}>
            <div className={styles.progressTrack}>
              <div
                className={styles.progressFill}
                style={{
                  width: `${
                    progress.total > 0
                      ? Math.min(100, (progress.downloaded / progress.total) * 100)
                      : 0
                  }%`,
                }}
              />
            </div>
            <span className={styles.progressText}>
              {formatBytes(progress.downloaded)}
              {progress.total > 0 ? ` / ${formatBytes(progress.total)}` : ''}
              {progress.speed > 0 ? ` · ${formatBytes(progress.speed)}/s` : ''}
            </span>
          </div>
        ) : null}
        {host?.disabled ? (
          <div className={styles.notice}>
            <AlertCircle size={14} />
            <span>
              {t('relay.plugins.runtime.crashed', { count: host.failures })}
            </span>
          </div>
        ) : null}
      </section>

      {/* 已安装的包。一行一个包，删除连目录一起清掉。 */}
      <section className={styles.section}>
        <div className={styles.sectionHead}>
          <div>
            <div className={styles.sectionTitle}>{t('relay.plugins.packages.title')}</div>
            <div className={styles.sectionHint}>{t('relay.plugins.packages.hint')}</div>
          </div>
          <div className={styles.actions}>
            <button
              type="button"
              className={`${styles.button} ${styles.buttonPrimary}`}
              disabled={busy !== null || !bun?.installed}
              onClick={() => {
                setInstallError('');
                setInstallOpen(true);
              }}
            >
              <Package size={13} />
              {t('relay.plugins.packages.install')}
            </button>
          </div>
        </div>
        {plugins.length === 0 ? (
          <div className={styles.empty}>
            {bun?.installed
              ? t('relay.plugins.packages.empty')
              : t('relay.plugins.packages.needsRuntime')}
          </div>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>{t('relay.plugins.packages.columns.package')}</th>
                <th>{t('relay.plugins.packages.columns.version')}</th>
                <th>{t('relay.plugins.packages.columns.exports')}</th>
                <th aria-label={t('relay.plugins.packages.columns.actions')} />
              </tr>
            </thead>
            <tbody>
              {plugins.map((plugin) => {
                const owned = providers.filter((provider) => provider.spec === plugin.spec);
                return (
                  <tr key={plugin.id}>
                    <td className={styles.cellStrong}>{plugin.spec}</td>
                    <td className={styles.cellMono}>{plugin.version || '—'}</td>
                    <td className={styles.cellMuted}>
                      {owned.length > 0
                        ? owned.map((provider) => provider.exportName).join(', ')
                        : '—'}
                    </td>
                    <td className={styles.cellActions}>
                      <button
                        type="button"
                        className={styles.iconButton}
                        title={t('relay.plugins.packages.remove')}
                        disabled={busy !== null}
                        onClick={() =>
                          run('uninstall', async () => {
                            await uninstallRelayPlugin(plugin.id);
                            await load();
                          })
                        }
                      >
                        <Trash2 size={13} />
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>

      {/* 已装的插件与它们申报的 provider。 */}
      <section className={styles.section}>
        <div className={styles.sectionHead}>
          <div>
            <div className={styles.sectionTitle}>{t('relay.plugins.providers.title')}</div>
            <div className={styles.sectionHint}>
              {t('relay.plugins.providers.hint')}
            </div>
          </div>
          <div className={styles.actions}>
            <button
              type="button"
              className={styles.button}
              disabled={busy !== null}
              onClick={() => run('refresh', load)}
            >
              <RefreshCw size={13} />
              {t('relay.plugins.providers.refresh')}
            </button>
          </div>
        </div>

        {providers.length === 0 ? (
          <Empty
            description={
              bun?.installed
                ? t('relay.plugins.providers.empty')
                : t('relay.plugins.providers.needsRuntime')
            }
            image={Empty.PRESENTED_IMAGE_SIMPLE}
          />
        ) : (
          <div className={styles.providerGrid}>
            {providers.map((provider) => {
              const signingIn = accountsFor(provider.id).length > 0 || provider.signedIn;
              return (
                <div key={provider.id} className={styles.providerCard}>
                  <div className={styles.providerHead}>
                    <div className={styles.providerName}>{provider.name || provider.id}</div>
                    <Tooltip title={provider.spec}>
                      <span className={styles.badge}>{provider.exportName || 'default'}</span>
                    </Tooltip>
                  </div>
                  <div className={styles.providerMeta}>
                    {provider.npm || '—'}
                    {provider.models.length > 0
                      ? ` · ${t('relay.plugins.providers.modelCount', {
                          count: provider.models.length,
                        })}`
                      : ''}
                  </div>
                  <div className={styles.providerMeta}>
                    {provider.checkin ? t('relay.plugins.providers.checkin') : null}
                    {provider.checkin && provider.usage ? ' · ' : ''}
                    {provider.usage ? t('relay.plugins.providers.usage') : null}
                  </div>
                  {provider.listError ? (
                    <div className={styles.providerError}>{provider.listError}</div>
                  ) : null}
                  <div className={styles.providerFoot}>
                    <span className={styles.pill}>
                      <span
                        className={`${styles.dot} ${signingIn ? styles.dotOn : ''}`}
                      />
                      {signingIn
                        ? t('relay.plugins.providers.signedIn')
                        : t('relay.plugins.providers.signedOut')}
                    </span>
                    {signingIn ? (
                      <button
                        type="button"
                        className={styles.button}
                        disabled={busy !== null}
                        onClick={() =>
                          run('signout', async () => {
                            await signOutRelayProvider(provider.id, provider.account);
                            await load();
                          })
                        }
                      >
                        {t('relay.plugins.providers.signOut')}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className={styles.button}
                        disabled={busy !== null}
                        onClick={() => {
                          setSignInProvider(provider);
                          setSignInKey('');
                          setSignInError('');
                        }}
                      >
                        <KeyRound size={13} />
                        {t('relay.plugins.providers.signIn')}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <Modal
        open={installOpen}
        title={t('relay.plugins.install.title')}
        okText={t('relay.plugins.install.confirm')}
        cancelText={t('relay.common.cancel')}
        confirmLoading={busy === 'install'}
        onOk={handleInstallPlugin}
        onCancel={() => setInstallOpen(false)}
        destroyOnHidden
      >
        <div className={styles.modalBody}>
          <Input
            autoFocus
            value={installSpec}
            placeholder="@scope/opencode-plugin-name"
            onChange={(event) => setInstallSpec(event.target.value)}
            onPressEnter={handleInstallPlugin}
          />
          <div className={styles.sectionHint}>{t('relay.plugins.install.hint')}</div>
          {installError ? <div className={styles.providerError}>{installError}</div> : null}
        </div>
      </Modal>

      <Modal
        open={signInProvider !== null}
        title={
          signInProvider
            ? t('relay.plugins.signIn.title', { provider: signInProvider.name })
            : ''
        }
        okText={t('relay.plugins.signIn.confirm')}
        cancelText={t('relay.common.cancel')}
        confirmLoading={busy === 'signin'}
        onOk={handleSignIn}
        onCancel={() => setSignInProvider(null)}
        destroyOnHidden
      >
        <div className={styles.modalBody}>
          <Input.Password
            autoFocus
            value={signInKey}
            placeholder={
              signInProvider?.methods.find((method) => method.placeholder)?.placeholder ??
              t('relay.plugins.signIn.keyPlaceholder')
            }
            onChange={(event) => setSignInKey(event.target.value)}
            onPressEnter={handleSignIn}
          />
          <div className={styles.sectionHint}>{t('relay.plugins.signIn.hint')}</div>
          {signInError ? <div className={styles.providerError}>{signInError}</div> : null}
        </div>
      </Modal>

      <Modal
        open={pending !== null}
        title={t('relay.plugins.oauth.title')}
        okText={t('relay.plugins.oauth.confirm')}
        cancelText={t('relay.common.cancel')}
        confirmLoading={busy === 'oauth'}
        onOk={handleOAuth}
        onCancel={() => setPending(null)}
        destroyOnHidden
      >
        <div className={styles.modalBody}>
          {pending?.url ? (
            <a className={styles.link} href={pending.url} target="_blank" rel="noreferrer">
              {pending.url}
            </a>
          ) : null}
          {pending?.instructions ? (
            <div className={styles.sectionHint}>{pending.instructions}</div>
          ) : null}
          <Input
            value={oauthCode}
            placeholder={t('relay.plugins.oauth.codePlaceholder')}
            onChange={(event) => setOauthCode(event.target.value)}
            onPressEnter={handleOAuth}
          />
          {oauthError ? <div className={styles.providerError}>{oauthError}</div> : null}
        </div>
      </Modal>
    </div>
  );
};

export default RelayPluginsPanel;
