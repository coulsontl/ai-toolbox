/**
 * Visual check for the CodeBuddy page.
 *
 * Same rationale as the WorkBuddy fixture: mounts the real page against a
 * stubbed Tauri bridge (only one dev server can own port 5173, so a second
 * worktree cannot start the app beside the one the user is running).
 */
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { useSettingsStore } from '@/stores/settingsStore';
import CodebuddyPage from '@/features/coding/codebuddy/pages/CodebuddyPage';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const dark = parameters.get('theme') === 'dark';
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = dark ? 'dark' : 'light';

const runtimeConfig = {
  providers: [
    {
      id: 'channel-alpha',
      name: 'Alpha',
      vendor: 'Alpha',
      baseUrl: 'https://api.alpha.example/v1',
      apiKey: 'sk-alpha-key-cccccccccccccccc',
      models: [
        { id: 'alpha-large', name: 'Alpha Large' },
        { id: 'alpha-small', name: 'Alpha Small' },
      ],
    },
    {
      id: 'channel-beta',
      name: 'Beta',
      vendor: 'Beta',
      baseUrl: 'https://api.beta.example/v1',
      apiKey: 'sk-beta-key-dddddddddddddddd',
      models: [{ id: 'beta-one', name: 'Beta One' }],
    },
  ],
  availableModels: ['alpha-large'],
  exists: true,
  path: 'C:/Users/Example/.codebuddy/models.json',
};

const rawModelsFile = `[
  {
    "id": "alpha-large",
    "name": "Alpha Large",
    "url": "https://api.alpha.example/v1/chat/completions",
    "apiKey": "sk-alpha-key-cccccccccccccccc"
  }
]
`;

const state = { requests: [] };

window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    state.requests.push({ command, args });
    switch (command) {
      case 'read_codebuddy_runtime_config':
        return structuredClone(runtimeConfig);
      case 'get_codebuddy_path_info':
        return { path: 'C:/Users/Example/.codebuddy', source: 'default' };
      case 'read_codebuddy_models_file':
        return rawModelsFile;
      case 'read_codebuddy_settings_file':
        // `string | null`, not an object: an absent file is the fresh-install
        // shape, and the page renders this value directly (React error #31 on
        // an object).
        return null;
      case 'get_settings':
        return { language, sidebar_hidden_by_page: {}, visible_tabs: ['codebuddy'] };
      case 'get_codebuddy_settings_config':
        return null;
      case 'list_codebuddy_prompt_configs':
        return [];
      case 'list_global_prompt_configs':
        return [];
      default:
        return null;
    }
  },
  transformCallback: (callback) => callback,
};

useSettingsStore.setState({ sidebarHiddenByPage: {} });

const root = createRoot(document.getElementById('root'));
root.render(
  <ConfigProvider theme={{ algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <App>
      <CodebuddyPage />
    </App>
  </ConfigProvider>,
);

window.__codebuddyFixture = {
  state,
  providerCardCount: () => document.querySelectorAll('[data-provider-id]').length,
  sidebarSectionTitles: () =>
    [...document.querySelectorAll('[data-codebuddy-sidebar-section]')].map((node) =>
      node.getAttribute('data-sidebar-title'),
    ),
};
