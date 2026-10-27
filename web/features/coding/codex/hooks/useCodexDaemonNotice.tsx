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
import {
  describeCodexDaemonHolders,
  shouldPromptCodexDaemonRestart,
} from '../utils/codexDaemonNotice';

export const useCodexDaemonNotice = () => {
  const { t } = useTranslation();
  const { notification, message, modal } = App.useApp();

  /**
   * Probe the holders and, when one is running, explain that the just-applied
   * config is not live yet and offer the restart.
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
    if (!shouldPromptCodexDaemonRestart(status)) {
      return;
    }
    const { hasDaemon, hasDesktopApp } = describeCodexDaemonHolders(
      status?.holders ?? [],
    );
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
                  try {
                    await restartCodexDaemon();
                    notification.destroy(notificationKey);
                    message.success(
                      hasDesktopApp
                        ? t('codex.daemon.desktopAppRestarted')
                        : t('codex.daemon.restartSuccess'),
                    );
                  } catch (error) {
                    const errorMessage =
                      error instanceof Error ? error.message : String(error);
                    message.error(
                      t('codex.daemon.restartFailed', { error: errorMessage }),
                    );
                  }
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
  }, [message, modal, notification, t]);

  return { notifyIfDaemonHoldsStaleConfig };
};
