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
      { key: 'openclaw', labelKey: 'subModules.openclaw', path: '/coding/openclaw' },
      { key: 'pi', labelKey: 'subModules.pi', path: '/coding/pi' },
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
