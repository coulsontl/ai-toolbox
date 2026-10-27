/**
 * Shared "a Codex process still holds the old config" prompt (issue #443).
 *
 * Codex reads `config.toml` / `auth.json` once when an app-server starts. Two
 * kinds of process keep one alive — the CLI's background daemon and the ChatGPT
 * desktop app — and both serve the previous provider until they restart.
 *
 * Every Codex surface that applies a provider or an official account goes
 * through this hook, so the check and its wording cannot drift between the
 * direct-apply path, the official-account path and the gateway paths.
 */

import { useCallback } from 'react';
import { App, Button, Space } from 'antd';
import { useTranslation } from 'react-i18next';
import { getCodexDaemonStatus, restartCodexDaemon } from '@/services/codexApi';
import { useSettingsStore } from '@/stores';
import {
  describeCodexDaemonHolders,
  resolveCodexDaemonRestartAction,
} from '../utils/codexDaemonNotice';

export const useCodexDaemonNotice = () => {
  const { t } = useTranslation();
  const { notification, message, modal } = App.useApp();
  // Read lazily at call time rather than capturing it in the callback: the mode
  // can change in the settings modal while a page stays mounted.
  const codexDaemonRestartMode = useSettingsStore(
    (state) => state.codexDaemonRestartMode,
  );

  /**
   * Run the restart and report the outcome. Shared by the confirmed (manual)
   * and `auto` paths so the two cannot report differently.
   */
  const runRestart = useCallback(
    async (hasDesktopApp: boolean): Promise<boolean> => {
      try {
        await restartCodexDaemon();
        message.success(
          hasDesktopApp
            ? t('codex.daemon.desktopAppRestarted')
            : t('codex.daemon.restartSuccess'),
        );
        return true;
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : String(error);
        message.error(t('codex.daemon.restartFailed', { error: errorMessage }));
        return false;
      }
    },
    [message, t],
  );

  /**
   * Probe the holders and, when one is running, explain that the just-applied
   * config is not live yet and offer the restart.
   *
   * The user's `codexDaemonRestartMode` decides whether this prompts, restarts
   * silently, or does nothing (see `resolveCodexDaemonRestartAction`).
   *
   * Never throws: a failed probe must not turn a successful apply into an
   * error in the caller's catch block.
   */
  const notifyIfDaemonHoldsStaleConfig = useCallback(async () => {
    let status = null;
    try {
      status = await getCodexDaemonStatus();
    } catch (error) {
      console.error('Failed to probe Codex config holders:', error);
      return;
    }
    const action = resolveCodexDaemonRestartAction(
      status,
      codexDaemonRestartMode,
    );
    if (action === 'none') {
      return;
    }
    const { hasDaemon, hasDesktopApp } = describeCodexDaemonHolders(
      status?.holders ?? [],
    );

    // `auto`: restart without asking, then report the outcome. A failure here
    // is surfaced, not swallowed — the user chose to skip the prompt, but they
    // still must not be left believing the new config is live.
    if (action === 'auto') {
      console.info(
        'A Codex process is holding the previous config; restarting it automatically:',
        status,
      );
      // The backend bounds this (8s waiting for the desktop app to exit, 60s
      // for the daemon), but it can still take seconds with no other feedback,
      // which reads as a frozen UI. Hold a loading toast for the duration.
      const autoKey = 'codex-daemon-auto-restart';
      message.loading({
        key: autoKey,
        content: t('codex.daemon.autoRestarting'),
        duration: 0,
      });
      try {
        await runRestart(hasDesktopApp);
      } finally {
        // Destroy only the loading toast; the outcome message has its own
        // (implicit) key and must survive this call.
        message.destroy(autoKey);
      }
      return;
    }

    console.info(
      'A Codex process is holding the previous config; prompting to restart it:',
      status,
    );

    // The body names what the restart will actually touch: the desktop app is
    // terminated and relaunched, which drops any conversation open inside it.
    const description = hasDaemon && hasDesktopApp
      ? t('codex.daemon.staleDescriptionBoth')
      : hasDesktopApp
        ? t('codex.daemon.staleDescriptionDesktop')
        : t('codex.daemon.staleDescription');

    const notificationKey = 'codex-daemon-stale-config';
    notification.info({
      key: notificationKey,
      message: hasDesktopApp && !hasDaemon
        ? t('codex.daemon.staleTitleDesktop')
        : t('codex.daemon.staleTitle'),
      description,
      duration: 0,
      btn: (
        <Space>
          <Button
            size="small"
            onClick={() => notification.destroy(notificationKey)}
          >
            {t('codex.daemon.dismiss')}
          </Button>
          <Button
            type="primary"
            size="small"
            onClick={() => {
              modal.confirm({
                title: t('codex.daemon.restartConfirmTitle'),
                content: hasDesktopApp
                  ? t('codex.daemon.restartConfirmDescriptionDesktop')
                  : t('codex.daemon.restartConfirmDescription'),
                okText: hasDesktopApp
                  ? t('codex.daemon.restartDesktopApp')
                  : t('codex.daemon.restart'),
                cancelText: t('common.cancel'),
                onOk: async () => {
                  // Drop the prompt either way; the outcome arrives as a
                  // message, so leaving it up would contradict the toast.
                  notification.destroy(notificationKey);
                  await runRestart(hasDesktopApp);
                },
              });
            }}
          >
            {hasDesktopApp
              ? t('codex.daemon.restartDesktopApp')
              : t('codex.daemon.restart')}
          </Button>
        </Space>
      ),
    });
  }, [codexDaemonRestartMode, modal, notification, runRestart, t]);

  return { notifyIfDaemonHoldsStaleConfig };
};
