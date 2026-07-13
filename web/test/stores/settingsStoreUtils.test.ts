import assert from 'node:assert/strict';
import test from 'node:test';

import type { AppSettings } from '../../services/settingsApi.ts';
import { buildLaunchOnStartupSettings } from '../../stores/settingsStoreUtils.ts';

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
    backup_custom_entries: [],
    backup_file_filter_rules: [],
    launch_on_startup: true,
    minimize_to_tray_on_close: true,
    start_minimized: false,
    proxy_mode: 'system',
    proxy_url: '',
    theme: 'system',
    auto_backup_enabled: false,
    auto_backup_interval_days: 7,
    auto_backup_max_keep: 10,
    last_auto_backup_time: null,
    auto_check_update: true,
    visible_tabs: ['opencode', 'claudecode', 'codex', 'geminicli', 'openclaw', 'pi', 'gateway', 'image', 'ssh', 'wsl'],
    sidebar_hidden_by_page: {
      opencode: false,
      claudecode: false,
    codex: false,
    grok: false,
      openclaw: false,
      geminicli: false,
      pi: false,
    },
    opencode_allow_clear_applied_oh_my_config: false,
    codex_preserve_official_auth_on_switch: false,
    codex_unified_session_history_enabled: false,
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
