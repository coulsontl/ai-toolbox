import React from 'react';
import { message } from 'antd';
import { useTranslation } from 'react-i18next';
import { platform } from '@tauri-apps/plugin-os';
import SidebarSettingsModal, {
  SettingsSelectRow,
  SettingsToggleRow,
} from '@/components/common/SidebarSettingsModal';
import CliManualPathSetting from '@/components/common/CliManualPathSetting';
import {
  getClaudePluginStatus,
  applyClaudePluginConfig,
  getClaudeOnboardingStatus,
  applyClaudeOnboardingSkip,
  clearClaudeOnboardingSkip,
} from '@/services/claudeCodeApi';
import { useSettingsStore } from '@/stores/settingsStore';
import { paseoSyncWarningKey } from '@/features/coding/shared/paseoSyncWarning';

interface ClaudeCodeSettingsModalProps {
  open: boolean;
  onClose: () => void;
  sidebarVisible: boolean;
  onSidebarVisibleChange: (visible: boolean) => void | Promise<void>;
}

export const ClaudeCodeSettingsModal: React.FC<ClaudeCodeSettingsModalProps> = ({
  open,
  onClose,
  sidebarVisible,
  onSidebarVisibleChange,
}) => {
  const { t } = useTranslation();
  const isWindows = React.useMemo(() => platform() === 'windows', []);
  const claudeCliLaunchFullAccess = useSettingsStore((state) => state.claudeCliLaunchFullAccess);
  const setClaudeCliLaunchFullAccess = useSettingsStore(
    (state) => state.setClaudeCliLaunchFullAccess,
  );
  const preferredTerminal = useSettingsStore((state) => state.preferredTerminal);
  const setPreferredTerminal = useSettingsStore((state) => state.setPreferredTerminal);
  const syncClaudeModelsToPaseo = useSettingsStore((state) => state.syncClaudeModelsToPaseo);
  const setSyncClaudeModelsToPaseo = useSettingsStore(
    (state) => state.setSyncClaudeModelsToPaseo,
  );
  const [vscodeEnabled, setVscodeEnabled] = React.useState(false);
  const [skipOnboarding, setSkipOnboarding] = React.useState(false);
  const [vscodeLoading, setVscodeLoading] = React.useState(false);
  const [onboardingLoading, setOnboardingLoading] = React.useState(false);
  const [cliLaunchFullAccessLoading, setCliLaunchFullAccessLoading] = React.useState(false);
  const [paseoSyncLoading, setPaseoSyncLoading] = React.useState(false);

  React.useEffect(() => {
    if (open) {
      void loadSettings();
    }
  }, [open]);

  const loadSettings = async () => {
    try {
      const [pluginStatus, onboardingStatus] = await Promise.all([
        getClaudePluginStatus(),
        getClaudeOnboardingStatus(),
      ]);
      setVscodeEnabled(pluginStatus.enabled);
      setSkipOnboarding(onboardingStatus);
    } catch (error) {
      console.error('Failed to load settings:', error);
    }
  };

  const handleVscodeToggle = async (checked: boolean) => {
    setVscodeLoading(true);
    try {
      await applyClaudePluginConfig(checked);
      setVscodeEnabled(checked);
      message.success(
        checked ? t('claudecode.plugin.enabled') : t('claudecode.plugin.disabled'),
      );
    } catch (error) {
      console.error('Failed to toggle VSCode integration:', error);
      message.error(t('common.error'));
    } finally {
      setVscodeLoading(false);
    }
  };

  const handleOnboardingToggle = async (checked: boolean) => {
    setOnboardingLoading(true);
    try {
      if (checked) {
        await applyClaudeOnboardingSkip();
      } else {
        await clearClaudeOnboardingSkip();
      }
      setSkipOnboarding(checked);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to toggle onboarding skip:', error);
      message.error(t('common.error'));
    } finally {
      setOnboardingLoading(false);
    }
  };

  const handleCliLaunchFullAccessToggle = async (checked: boolean) => {
    setCliLaunchFullAccessLoading(true);
    try {
      await setClaudeCliLaunchFullAccess(checked);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to toggle Claude CLI full access:', error);
      message.error(t('common.error'));
    } finally {
      setCliLaunchFullAccessLoading(false);
    }
  };

  const handlePaseoSyncToggle = async (checked: boolean) => {
    setPaseoSyncLoading(true);
    try {
      const result = await setSyncClaudeModelsToPaseo(checked);
      if (result.syncWarning) {
        message.warning(t(paseoSyncWarningKey(result.syncWarning)));
      } else {
        message.success(t('common.success'));
      }
    } catch (error) {
      console.error('Failed to update Claude Paseo model sync setting:', error);
      message.error(t('common.error'));
    } finally {
      setPaseoSyncLoading(false);
    }
  };

  const handlePreferredTerminalChange = async (terminal: string) => {
    try {
      await setPreferredTerminal(terminal);
    } catch (error) {
      console.error('Failed to save preferred terminal:', error);
      message.error(t('common.error'));
    }
  };

  return (
    <SidebarSettingsModal
      open={open}
      onClose={onClose}
      sidebarVisible={sidebarVisible}
      onSidebarVisibleChange={onSidebarVisibleChange}
    >
      <CliManualPathSetting
        commandName="claude"
        labelKey="subModules.claudecode"
      />
      <SettingsToggleRow
        title={t('claudecode.settings.vscode')}
        hint={t('claudecode.settings.vscodeHint')}
        checked={vscodeEnabled}
        loading={vscodeLoading}
        onChange={handleVscodeToggle}
      />
      <SettingsToggleRow
        title={t('claudecode.settings.skipOnboarding')}
        hint={t('claudecode.settings.skipOnboardingHint')}
        checked={skipOnboarding}
        loading={onboardingLoading}
        onChange={handleOnboardingToggle}
      />
      <SettingsToggleRow
        title={t('claudecode.settings.cliLaunchFullAccess')}
        hint={t('claudecode.settings.cliLaunchFullAccessHint')}
        checked={claudeCliLaunchFullAccess}
        loading={cliLaunchFullAccessLoading}
        onChange={handleCliLaunchFullAccessToggle}
      />
      <SettingsToggleRow
        title={t('common.paseoSync.title')}
        hint={t('common.paseoSync.hint')}
        checked={syncClaudeModelsToPaseo}
        loading={paseoSyncLoading}
        onChange={handlePaseoSyncToggle}
      />
      {isWindows && (
        <SettingsSelectRow
          title={t('common.preferredTerminal')}
          hint={t('common.preferredTerminalHint')}
          value={preferredTerminal ?? 'cmd'}
          options={[
            { value: 'cmd', label: t('common.terminalCmd') },
            { value: 'powershell', label: t('common.terminalPowershell') },
            { value: 'wt', label: t('common.terminalWt') },
            { value: 'gitbash', label: t('common.terminalGitBash') },
          ]}
          onChange={handlePreferredTerminalChange}
        />
      )}
    </SidebarSettingsModal>
  );
};

export default ClaudeCodeSettingsModal;
