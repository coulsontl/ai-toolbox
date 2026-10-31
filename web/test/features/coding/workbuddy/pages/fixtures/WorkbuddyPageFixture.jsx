/**
 * Visual check for the WorkBuddy page.
 *
 * Mounts the real `WorkbuddyPage` against a stubbed Tauri bridge so the page
 * renders exactly as it does in the app: same CSS modules, same antd theme,
 * same `SectionSidebarLayout` chrome. Only the backend commands are faked —
 * everything that decides layout, spacing and typography is production code.
 *
 * Why a fixture instead of driving the running app: only one dev server can own
 * port 5173 (vite is pinned there), so a second worktree cannot start the app
 * alongside the one the user is running.
 */
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { useSettingsStore } from '@/stores/settingsStore';
import WorkbuddyPage from '@/features/coding/workbuddy/pages/WorkbuddyPage';
import { updatePresetModels } from '@/constants/presetModels';
import bundledPresetModels from '../../../../../../../tauri/resources/preset_models.json';
import '@/App.css';

/**
 * The catalogue the real app loads at startup (`load_cached_preset_models` →
 * `fetch_remote_preset_models`). The fixture never runs that bootstrap, so
 * without this the 「选择预设模型」 picker renders as an empty panel and any
 * assertion about it either fails for the wrong reason or passes vacuously.
 */
updatePresetModels(bundledPresetModels);

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const dark = parameters.get('theme') === 'dark';
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = dark ? 'dark' : 'light';

// Two channels, one of them with two models, so the card list exercises both
// the single-row and multi-row shapes.
//
// `id` must be present: the backend derives it from `(baseUrl, apiKey)`
// (`channel_id`), the card forwards it as `data-provider-id`, and the locate
// action queries that attribute. A stub without `id` renders cards that look
// right but cannot be located.
const runtimeConfig = {
  providers: [
    {
      id: 'channel-example',
      name: 'Example',
      vendor: 'Example',
      baseUrl: 'https://api.example.com/v1',
      apiKey: 'sk-example-key-aaaaaaaaaaaaaaaa',
      models: [
        { id: 'example-model-a', name: 'Example A', maxInputTokens: 128000, maxOutputTokens: 8192 },
        { id: 'example-model-b', name: 'Example B' },
      ],
    },
    {
      id: 'channel-other',
      name: 'Other',
      vendor: 'Other',
      baseUrl: 'https://api.other.com/v1',
      apiKey: 'sk-other-key-bbbbbbbbbbbbbbbb',
      models: [{ id: 'other-model', name: 'Other Model' }],
    },
  ],
  availableModels: [],
};

const rawModelsFile = `[
  {
    "id": "example-model-a",
    "name": "Example A",
    "url": "https://api.example.com/v1/chat/completions",
    "apiKey": "sk-example-key-aaaaaaaaaaaaaaaa"
  }
]
`;

const state = { requests: [] };

window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    state.requests.push({ command, args });
    switch (command) {
      case 'read_workbuddy_runtime_config':
        return structuredClone(runtimeConfig);
      case 'get_workbuddy_path_info':
        return { path: 'C:/Users/Example/.workbuddy', source: 'default' };
      case 'read_workbuddy_models_file':
        return rawModelsFile;
      case 'get_settings':
        return { language, sidebar_hidden_by_page: {}, visible_tabs: ['workbuddy'] };
      case 'get_workbuddy_settings_config':
        return null;
      default:
        // The session manager renders its empty state for anything it cannot
        // list, which is the shape a fresh install shows.
        return null;
    }
  },
  transformCallback: (callback) => callback,
};

useSettingsStore.setState({ sidebarHiddenByPage: {} });

const root = createRoot(document.getElementById('root'));
root.render(
  <ConfigProvider theme={{ algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    {/* `App` is required: the page calls `App.useApp()` for `message`, which
        is a no-op stub without the provider (`t.error is not a function`). */}
    <App>
      <WorkbuddyPage />
    </App>
  </ConfigProvider>,
);

window.__workbuddyFixture = {
  state,
  providerCardCount: () => document.querySelectorAll('[data-provider-id]').length,
  bodyText: () => document.body.innerText,
  sidebarSectionTitles: () =>
    [...document.querySelectorAll('[data-workbuddy-sidebar-section]')].map((node) =>
      node.getAttribute('data-sidebar-title'),
    ),
  hardcodedColors: () => {
    const offenders = [];
    for (const sheet of document.styleSheets) {
      let rules;
      try {
        rules = sheet.cssRules;
      } catch {
        continue;
      }
      for (const rule of rules) {
        if (!rule.cssText) continue;
        if (/#[0-9a-fA-F]{3,8}\b/.test(rule.cssText) && !rule.cssText.includes('--')) {
          offenders.push(rule.cssText.slice(0, 120));
        }
      }
    }
    return offenders;
  },
};
