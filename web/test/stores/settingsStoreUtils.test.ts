import assert from 'node:assert/strict';
import test from 'node:test';

import type { AppSettings } from '../../services/settingsApi.ts';
import {
  backupSettingsStatePatch,
  buildLaunchOnStartupSettings,
} from '../../stores/settingsStoreUtils.ts';

function createSettings(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    language: 'zh-CN',
    current_module: 'coding',
    current_sub_tab: 'opencode',
    backup_type: 'local',
    local_backup_path: '',
    webdav: {
      url: '',
      username: '',
      password: '',
      remote_path: '',
      host_label: '',
    },
    s3: {
      access_key: '',
      secret_key: '',
      bucket: '',
      region: '',
      prefix: '',
      endpoint_url: '',
      force_path_style: false,
      public_domain: '',
    },
    last_backup_time: null,
    backup_image_assets_enabled: true,
    backup_cli_config_files_enabled: true,
    backup_custom_entries: [],
    backup_file_filter_rules: [],
    backup_encryption: {
      enabled: false,
      credential_ref: 'keyring:ai-toolbox-backup-encryption',
    },
    launch_on_startup: true,
    minimize_to_tray_on_close: true,
    start_minimized: false,
    start_lightweight: false,
    lightweight_on_close: false,
    keep_computer_awake: false,
    proxy_mode: 'system',
    proxy_url: '',
    theme: 'system',
    auto_backup_enabled: false,
    auto_backup_interval_days: 7,
    auto_backup_max_keep: 10,
    last_auto_backup_time: null,
    auto_check_update: true,
    receive_beta_updates: false,
    visible_tabs: ['opencode', 'claudecode', 'codex', 'geminicli', 'openclaw', 'pi', 'oh_my_pi', 'gateway', 'image', 'ssh', 'wsl'],
    sidebar_hidden_by_page: {
      opencode: false,
      claudecode: false,
      claudedesktop: false,
      codex: false,
      grok: false,
      geminicli: false,
      antigravity: false,
      kimi: false,
      openclaw: false,
      pi: false,
      oh_my_pi: false,
      omo_native: false,
      hermes: false,
      dsh: false,
      zcode: false,
    },
    opencode_allow_clear_applied_oh_my_config: false,
    opencode_use_legacy_oh_my_config: false,
    opencode_omo_upgrade_confirmed: false,
    opencode_dual_write_reasoning_variant: false,
    codex_preserve_official_auth_on_switch: false,
    codex_unified_session_history_enabled: false,
    claude_cli_launch_full_access: false,
    preferred_terminal: null,
    cli_manual_paths: {},
    provider_sort_modes: {},
    provider_last_used: {},
    provider_view_modes: {},
    ...overrides,
  };
}

test('buildLaunchOnStartupSettings clears start_minimized when launch on startup is disabled', () => {
  const currentSettings = createSettings({
    launch_on_startup: true,
    start_minimized: true,
  });

  const updatedSettings = buildLaunchOnStartupSettings(currentSettings, false);

  assert.equal(updatedSettings.launch_on_startup, false);
  assert.equal(updatedSettings.start_minimized, false);
  assert.equal(currentSettings.launch_on_startup, true);
  assert.equal(currentSettings.start_minimized, true);
});

test('buildLaunchOnStartupSettings preserves start_minimized when launch on startup is enabled', () => {
  const currentSettings = createSettings({
    launch_on_startup: false,
    start_minimized: true,
  });

  const updatedSettings = buildLaunchOnStartupSettings(currentSettings, true);

  assert.equal(updatedSettings.launch_on_startup, true);
  assert.equal(updatedSettings.start_minimized, true);
});

test('unified backup save patch syncs every submitted field including auto-backup params', () => {
  const config = {
    backupType: 'repository' as const,
    localBackupPath: 'D:/backups',
    webdav: {
      url: 'https://dav.example.com',
      username: 'user',
      password: 'pass',
      remotePath: '/backup',
      hostLabel: 'Office PC',
    },
    backupImageAssetsEnabled: true,
    backupCliConfigFilesEnabled: false,
    backupCustomEntries: [{
      id: 'custom-backup-1',
      name: 'notes',
      source_path: 'D:/notes',
      restore_path: null,
      entry_type: 'file' as const,
      enabled: true,
      exclude_patterns: [],
    }],
    backupFileFilterRules: [{ tool: 'opencode', file_path: 'auth.json' }],
    autoBackupEnabled: true,
    autoBackupIntervalDays: 2,
    autoBackupMaxKeep: 3,
  };

  const patch = backupSettingsStatePatch(config, { encryption: { enabled: true } });

  // Regression (review F04): a save that only changed auto-backup parameters used
  // to leave the store holding the old values, so reopening the modal and saving
  // again silently reverted them.
  assert.equal(patch.autoBackupEnabled, true);
  assert.equal(patch.autoBackupIntervalDays, 2);
  assert.equal(patch.autoBackupMaxKeep, 3);
  assert.equal(patch.backupEncryptionEnabled, true);
  assert.equal(patch.backupType, 'repository');
  assert.equal(patch.localBackupPath, 'D:/backups');
  assert.equal(patch.webdav.hostLabel, 'Office PC');
  assert.equal(patch.backupImageAssetsEnabled, true);
  assert.equal(patch.backupCliConfigFilesEnabled, false);
  assert.deepEqual(patch.backupCustomEntries, config.backupCustomEntries);
  assert.deepEqual(patch.backupFileFilterRules, config.backupFileFilterRules);
});
