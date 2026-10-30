import React from 'react';
import { CodeOutlined, SettingOutlined } from '@ant-design/icons';

export interface SubTab {
  key: string;
  labelKey: string;
  path: string;
}

export interface Module {
  key: string;
  labelKey: string;
  icon: React.ReactNode;
  path: string;
  subTabs: SubTab[];
}

export const MODULES: Module[] = [
  {
    key: 'coding',
    labelKey: 'modules.coding',
    icon: React.createElement(CodeOutlined),
    path: '/coding',
    subTabs: [
      { key: 'opencode', labelKey: 'subModules.opencode', path: '/coding/opencode' },
      { key: 'claudecode', labelKey: 'subModules.claudecode', path: '/coding/claudecode' },
      { key: 'codex', labelKey: 'subModules.codex', path: '/coding/codex' },
      { key: 'grok', labelKey: 'subModules.grok', path: '/coding/grok' },
      { key: 'geminicli', labelKey: 'subModules.geminicli', path: '/coding/geminicli' },
      { key: 'antigravity', labelKey: 'subModules.antigravity', path: '/coding/antigravity' },
      { key: 'zcode', labelKey: 'subModules.zcode', path: '/coding/zcode' },
      { key: 'kimi', labelKey: 'subModules.kimi', path: '/coding/kimi' },
      { key: 'openclaw', labelKey: 'subModules.openclaw', path: '/coding/openclaw' },
      { key: 'pi', labelKey: 'subModules.pi', path: '/coding/pi' },
      { key: 'oh_my_pi', labelKey: 'subModules.ohMyPi', path: '/coding/oh-my-pi' },
      { key: 'omo_native', labelKey: 'subModules.omoNative', path: '/coding/omo-native' },
      { key: 'claudedesktop', labelKey: 'subModules.claudedesktop', path: '/coding/claudedesktop' },
      { key: 'hermes', labelKey: 'subModules.hermes', path: '/coding/hermes' },
      { key: 'dsh', labelKey: 'subModules.dsh', path: '/coding/dsh' },
      { key: 'workbuddy', labelKey: 'subModules.workbuddy', path: '/coding/workbuddy' },
      { key: 'codebuddy', labelKey: 'subModules.codebuddy', path: '/coding/codebuddy' },
    ],
  },
  // {
  //   key: 'daily',
  //   labelKey: 'modules.daily',
  //   icon: React.createElement(CalendarOutlined),
  //   path: '/daily',
  //   subTabs: [
  //     { key: 'notes', labelKey: 'subModules.notes', path: '/daily/notes' },
  //   ],
  // },
];

export const SETTINGS_MODULE: Module = {
  key: 'settings',
  labelKey: 'modules.settings',
  icon: React.createElement(SettingOutlined),
  path: '/settings',
  subTabs: [],
};

export const DEFAULT_MODULE = MODULES[0];
export const DEFAULT_PATH = DEFAULT_MODULE.subTabs[0]?.path || DEFAULT_MODULE.path;
