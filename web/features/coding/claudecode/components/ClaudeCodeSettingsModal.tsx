import React from 'react';
import { message } from 'antd';
import { useTranslation } from 'react-i18next';
import SidebarSettingsModal, {
  SettingsToggleRow,
} from '@/components/common/SidebarSettingsModal';
import {
  getClaudePluginStatus,
  applyClaudePluginConfig,
  getClaudeOnboardingStatus,
  applyClaudeOnboardingSkip,
  clearClaudeOnboardingSkip,
} from '@/services/claudeCodeApi';
import { useSettingsStore } from '@/stores/settingsStore';

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
  const claudeCliLaunchFullAccess = useSettingsStore((state) => state.claudeCliLaunchFullAccess);
  const setClaudeCliLaunchFullAccess = useSettingsStore(
    (state) => state.setClaudeCliLaunchFullAccess,
  );
  const [vscodeEnabled, setVscodeEnabled] = React.useState(false);
  const [skipOnboarding, setSkipOnboarding] = React.useState(false);
  const [vscodeLoading, setVscodeLoading] = React.useState(false);
  const [onboardingLoading, setOnboardingLoading] = React.useState(false);
  const [cliLaunchFullAccessLoading, setCliLaunchFullAccessLoading] = React.useState(false);

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

  return (
    <SidebarSettingsModal
      open={open}
      onClose={onClose}
      sidebarVisible={sidebarVisible}
      onSidebarVisibleChange={onSidebarVisibleChange}
    >
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
    </SidebarSettingsModal>
  );
};

export default ClaudeCodeSettingsModal;
