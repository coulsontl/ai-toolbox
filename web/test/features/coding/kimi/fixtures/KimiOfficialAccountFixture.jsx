/**
 * Mounts the **real** `KimiPage` against a stubbed Tauri IPC layer so the
 * official-account card can be inspected and dragged with real pointer events.
 *
 * Why the real page: the card is only half the feature. The other half is the
 * page's slot arithmetic — which index the card is drawn at, and whether the
 * drag reports the *merged* ordering (provider cards plus the card) rather than
 * a provider-only index. A card-only fixture would re-create the page half,
 * which is exactly where the disagreement can live.
 *
 * Only `KimiPage` is mounted, so the stub table below is the page's own command
 * set — an unstubbed command throws loudly instead of silently returning
 * `undefined`, which would turn a missing stub into a confusing render failure.
 */
import './KimiOfficialAccountStubs.js';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { updateGatewayProviderProfiles } from '@/features/coding/shared/gateway/providerProfiles';
// Imported before the page on purpose, mirroring `web/App.tsx`. The page sits in
// a real import cycle through `shared/providerShare`, so the fixture has to
// enter that cycle at the same point the app does; importing the page first
// throws `Cannot access 'KimiPage' before initialization`. That is an
// import-order artifact, not a product bug.
import '@/app/routes';
import KimiPage from '@/features/coding/kimi/pages/KimiPage';
import gatewayProfiles from '../../../../../../tauri/resources/gateway_provider_profiles.json';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const themeMode = parameters.get('theme') || 'light';
const resolvedTheme = themeMode === 'system'
  ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
  : themeMode;
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = resolvedTheme;
updateGatewayProviderProfiles(gatewayProfiles);

/** Provider card title, and the label the official-account card renders with. */
const OFFICIAL_ACCOUNT_TITLE = '官方账号';

const createProvider = (id, name, createdAt) => ({
  id,
  name,
  category: 'custom',
  settingsConfig: JSON.stringify({
    auth: { API_KEY: `fixture-key-${id}` },
    defaultModelKey: `${id}-model-a`,
    providerConfigs: {
      fixture: { type: 'openai', base_url: `https://${id}.example.test/v1` },
    },
    modelCatalog: {
      models: [
        {
          key: `${id}-model-a`,
          model: 'kimi-k2',
          provider: 'fixture',
          displayName: 'Model A',
          maxContextSize: 128000,
        },
      ],
    },
  }),
  isApplied: false,
  isDisabled: false,
  createdAt,
  updatedAt: createdAt,
});

const allProviders = [
  createProvider('provider-a', 'Provider A', '2026-01-03T00:00:00.000Z'),
  createProvider('provider-b', 'Provider B', '2026-01-02T00:00:00.000Z'),
  createProvider('provider-c', 'Provider C', '2026-01-01T00:00:00.000Z'),
];

const createAccount = (id, email, isApplied) => ({
  id,
  providerId: 'provider-official',
  name: email,
  kind: 'official',
  email,
  isApplied,
  createdAt: '2026-01-01T00:00:00.000Z',
  updatedAt: '2026-01-01T00:00:00.000Z',
});

const state = {
  runId: parameters.get('runId'),
  requests: [],
  sortMode: parameters.get('sortMode') || 'custom',
  accountIndex: Number(parameters.get('accountIndex') ?? 0),
  providers: parameters.get('providers') === '0' ? [] : structuredClone(allProviders),
  officialAccounts: parameters.get('accounts') === '0'
    ? []
    : [
        createAccount('account-1', 'first@example.invalid', true),
        createAccount('account-2', 'second@example.invalid', false),
      ],
};

const gatewayCliStatus = {
  cli_key: 'kimi',
  state: 'direct',
  dot: 'idle',
  can_takeover: true,
  can_restore_direct: false,
  gateway_origin: null,
  runtime_root: null,
  managed_targets: [],
  mode: null,
  primary_provider_id: null,
  provider_priorities: [],
  message: null,
};

const eventListeners = new Map();
let nextEventId = 0;

window.__TAURI_INTERNALS__ = {
  transformCallback(callback) {
    const id = nextEventId++;
    eventListeners.set(id, callback);
    return id;
  },
  unregisterCallback(id) {
    eventListeners.delete(id);
  },
  async invoke(command, args) {
    state.requests.push({ command, args: structuredClone(args ?? null) });
    switch (command) {
      // --- provider list ---------------------------------------------------
      case 'get_kimi_config_file_path': return 'C:\\Users\\tester\\.kimi-code\\config.toml';
      case 'get_kimi_root_path_info': return { path: 'C:\\Users\\tester\\.kimi-code', source: 'default' };
      case 'list_kimi_providers': return structuredClone(state.providers);
      case 'list_kimi_official_accounts': return structuredClone(state.officialAccounts);
      case 'reorder_kimi_providers': {
        const byId = new Map(state.providers.map(provider => [provider.id, provider]));
        state.providers = args.ids.map(id => byId.get(id)).filter(Boolean);
        return null;
      }
      case 'save_kimi_official_account_index': {
        state.accountIndex = args.index;
        return null;
      }
      case 'list_kimi_plugins': return [];
      case 'refresh_tray_menu': return null;
      case 'get_kimi_common_config':
        return { config: '', officialAccountIndex: state.accountIndex };
      case 'read_kimi_settings': return { config: '' };

      // --- provider list UI state -----------------------------------------
      case 'get_provider_list_state':
        return { sort_modes: { kimi: state.sortMode }, last_used: {} };
      case 'save_provider_sort_mode': {
        state.sortMode = args.mode;
        return null;
      }
      case 'record_provider_last_used': return null;

      // --- gateway ---------------------------------------------------------
      case 'proxy_gateway_cli_status': return structuredClone(gatewayCliStatus);
      case 'proxy_gateway_cli_statuses': return [structuredClone(gatewayCliStatus)];
      case 'proxy_gateway_supported_cli_keys': return ['kimi'];
      case 'proxy_gateway_status': return { running: false, mode: null };
      case 'proxy_gateway_get_settings': return {};

      // --- global prompt list ---------------------------------------------
      case 'list_kimi_prompt_configs': return [];

      // --- sessions --------------------------------------------------------
      case 'list_tool_sessions':
        return { sessions: [], total: 0, has_more: false, next_cursor: null };

      // --- feature-availability probes ------------------------------------
      case 'has_all_api_hub_extension': return false;
      case 'has_cc_switch_db': return false;

      // --- event plumbing --------------------------------------------------
      case 'plugin:event|listen': return 1;
      case 'plugin:event|listen_any': return 1;
      case 'plugin:event|unlisten': return null;
      case 'plugin:opener|reveal_item_in_dir': return null;

      default:
        throw new Error('Unstubbed fixture command: ' + command);
    }
  },
};

const wait = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

/** Every card inside the provider section, in the order it is drawn. The
 *  official-account card is named by its title; provider cards by their name. */
const cardsInProviderSection = () => [...document.querySelectorAll('#kimi-providers .ant-card')];

const cardFor = name => cardsInProviderSection().find(card => card.textContent.includes(name));

const handleFor = name => {
  const card = cardFor(name);
  if (!card) throw new Error('No card rendered for: ' + name);
  const handle = card.querySelector('.anticon-holder');
  return handle ? handle.parentElement : null;
};

const centerOf = element => {
  const rect = element.getBoundingClientRect();
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
};

window.kimiOfficialAccountFixture = {
  state,
  OFFICIAL_ACCOUNT_TITLE,
  /** Card titles in the order the page draws them. */
  renderedMembers() {
    return cardsInProviderSection().map(card => {
      const provider = state.providers.find(item => card.textContent.includes(item.name));
      if (provider) return provider.name;
      return card.textContent.includes(OFFICIAL_ACCOUNT_TITLE) ? OFFICIAL_ACCOUNT_TITLE : '?';
    });
  },
  hasHandle(name) {
    return Boolean(handleFor(name));
  },
  handleCenter(name) {
    const handle = handleFor(name);
    if (!handle) throw new Error('No drag handle rendered for: ' + name);
    return centerOf(handle);
  },
  /** Row center of another card — the drop target for a drag. */
  cardCenter(name) {
    const card = cardFor(name);
    if (!card) throw new Error('No card rendered for: ' + name);
    return centerOf(card);
  },
  /** The card's own text, so labels ("登录", "切换") can be read off the screen. */
  cardText(name) {
    return cardFor(name)?.textContent ?? null;
  },
  /** Labels of every button drawn inside the provider section's toolbar. */
  toolbarLabels() {
    return [...document.querySelectorAll('#kimi-providers .ant-collapse-header button')]
      .map(button => button.textContent.trim());
  },
  /** Labels of every button drawn inside a card, in DOM order. */
  cardButtonLabels(name) {
    return [...(cardFor(name)?.querySelectorAll('button') ?? [])]
      .map(button => button.textContent.trim());
  },
  /**
   * Sentences the card renders more than once.
   *
   * Only long leaf texts are considered: short ones are legitimately repeated
   * (every row has a "切换"), while a duplicated *sentence* means a block was
   * added in a new place and not removed from the old one — which is what
   * happened when the section hint became the title's subtitle.
   */
  cardRepeatedSentences(name, minimumLength = 20) {
    const counts = new Map();
    for (const node of cardFor(name)?.querySelectorAll('*') ?? []) {
      if (node.children.length > 0) continue;
      const text = node.textContent.trim();
      if (text.length < minimumLength) continue;
      counts.set(text, (counts.get(text) ?? 0) + 1);
    }
    return [...counts.entries()].filter(([, count]) => count > 1).map(([text]) => text);
  },
  /** Same buttons with their enabled state — a row's action is drawn even when
   *  it is inert, so label presence alone says nothing about what is offered. */
  cardButtonStates(name) {
    return [...(cardFor(name)?.querySelectorAll('button') ?? [])]
      .map(button => ({
        label: button.textContent.trim(),
        disabled: button.disabled,
      }));
  },
  async waitForMembers(titles, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (window.kimiOfficialAccountFixture.renderedMembers().join('|') === titles.join('|')) {
        return titles;
      }
      await wait(50);
    }
    return window.kimiOfficialAccountFixture.renderedMembers();
  },
  /** The last `reorder_kimi_providers` payload the page sent, if any. */
  lastReorder() {
    const call = [...state.requests].reverse().find(entry => entry.command === 'reorder_kimi_providers');
    return call ? call.args.ids : null;
  },
  /** Every `save_kimi_official_account_index` payload the page sent, in order. */
  savedIndices() {
    return state.requests
      .filter(entry => entry.command === 'save_kimi_official_account_index')
      .map(entry => entry.args.index);
  },
  storedIndex() {
    return state.accountIndex;
  },
  storedOrder() {
    return state.providers.map(provider => provider.name);
  },
};

createRoot(document.getElementById('root')).render(
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <App>
      {/* The cards call `useNavigate`, so the page needs router context. */}
      <MemoryRouter initialEntries={['/coding/kimi']}>
        <KimiPage />
      </MemoryRouter>
    </App>
  </ConfigProvider>,
);