/**
 * Mounts the **real** `KimiPage` against a stubbed Tauri IPC layer so the
 * official channel's account list can be inspected with real DOM queries.
 *
 * Why the real page rather than the card: the account list lives *inside* the
 * official provider card, so what has to hold is a property of the page — the
 * official row is an ordinary list member (draggable, searchable, counted like
 * any other), and its account rows line up with the card's own content edges
 * rather than with some narrower column.
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

/** The official channel's row id, and the two labels every official card shows. */
const OFFICIAL_PROVIDER_ID = 'provider-official';
const OFFICIAL_PROVIDER_NAME = 'Kimi Official';
/**
 * The heading every CLI's official card shows, and the title of the list under
 * it. Both are shared strings (`common.officialAccount.*`), which is the point:
 * the card *displays* 官方账号 even though the row is named "Kimi Official" in
 * the data, so the card can no longer be found by the words it shows.
 */
const OFFICIAL_CARD_HEADING = '官方账号';
const ACCOUNT_LIST_TITLE = '账号列表';

const createProvider = (id, name, createdAt, category = 'custom') => ({
  id,
  name,
  category,
  settingsConfig: JSON.stringify({
    auth: { API_KEY: category === 'official' ? '' : `fixture-key-${id}` },
    defaultModelKey: `${id}-model-a`,
    providerConfigs: category === 'official'
      ? {}
      : { fixture: { type: 'openai', base_url: `https://${id}.example.test/v1` } },
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
  createProvider('provider-official', OFFICIAL_PROVIDER_NAME, '2026-01-04T00:00:00.000Z', 'official'),
  createProvider('provider-a', 'Provider A', '2026-01-03T00:00:00.000Z'),
  createProvider('provider-b', 'Provider B', '2026-01-02T00:00:00.000Z'),
  createProvider('provider-c', 'Provider C', '2026-01-01T00:00:00.000Z'),
];

/**
 * A row is labelled with the platform nickname the backend read from `/me`;
 * the capture time is only the fallback for a login the platform could not
 * identify.
 */
const createAccount = (id, createdAt, isApplied, { isVirtual = false, nickname = '' } = {}) => ({
  id,
  providerId: isVirtual ? '' : 'provider-official',
  name: '',
  kind: isVirtual ? 'local' : 'official',
  nickname,
  isApplied,
  isVirtual,
  createdAt,
  updatedAt: createdAt,
});

const savedAccounts = [
  createAccount('account-1', '2026-03-31T00:00:00.000Z', true, { nickname: 'moonwalker' }),
  createAccount('account-2', '2026-04-01T00:00:00.000Z', false, { nickname: 'nightowl' }),
];
const virtualAccount = createAccount('__local__', '2026-04-02T00:00:00.000Z', false, {
  isVirtual: true,
  nickname: 'moonwalker',
});

const accountMode = parameters.get('accounts') || '2';
const providersMode = parameters.get('providers') || 'all';
const state = {
  runId: parameters.get('runId'),
  requests: [],
  sortMode: parameters.get('sortMode') || 'custom',
  providers:
    providersMode === '0'
      ? []
      : structuredClone(
          providersMode === 'custom-only'
            ? allProviders.filter(provider => provider.category !== 'official')
            : allProviders,
        ),
  officialAccounts:
    accountMode === '0'
      ? []
      : accountMode === 'virtual'
        ? [structuredClone(virtualAccount)]
        : structuredClone(savedAccounts),
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
      case 'save_kimi_official_local_account': {
        const virtual = state.officialAccounts.find(account => account.isVirtual);
        if (!virtual) throw new Error('No live Kimi login to save');
        virtual.isVirtual = false;
        virtual.providerId = 'provider-official';
        return structuredClone(virtual);
      }
      case 'reorder_kimi_providers': {
        const byId = new Map(state.providers.map(provider => [provider.id, provider]));
        state.providers = args.ids.map(id => byId.get(id)).filter(Boolean);
        return null;
      }
      case 'list_kimi_plugins': return [];
      case 'refresh_tray_menu': return null;
      case 'get_kimi_common_config': return { config: '' };
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

/**
 * Every card inside the provider section, in the order it is drawn.
 *
 * Addressed by the wrapper's `data-provider-id` — the same anchor the page's own
 * "locate" action uses — rather than by the words on the card. The official
 * card's row is named "Kimi Official" but the card *displays* the shared account
 * heading (官方账号), so a text anchor finds nothing and the helper below
 * degrades to '?'.
 */
const cardsInProviderSection = () =>
  [...document.querySelectorAll('#kimi-providers [data-provider-id]')];

const cardFor = providerId =>
  cardsInProviderSection().find(card => card.dataset.providerId === providerId);

const centerOf = element => {
  const rect = element.getBoundingClientRect();
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
};

/**
 * The account rows of the official card.
 *
 * A row is the element the section draws a separator under (it sets the border
 * inline), so the marker is structural rather than a stylesheet class name.
 * Anchoring on the action cluster alone is not enough: the card header's own
 * "apply" action is a `Space` carrying a check icon too.
 */
const accountRowsIn = card =>
  [...card.querySelectorAll('div')].filter(node =>
    String(node.style?.borderBottom || '').includes('1px solid'));

window.kimiOfficialAccountFixture = {
  state,
  OFFICIAL_PROVIDER_ID,
  OFFICIAL_CARD_HEADING,
  ACCOUNT_LIST_TITLE,
  /** The id of every member card, in the order the page draws them. */
  renderedMemberIds() {
    return cardsInProviderSection().map(card => card.dataset.providerId);
  },
  /**
   * The list's collapse toggle inside a card, if it renders one.
   *
   * Structural rather than by label: the caret is the toggle button's only icon,
   * and the model list's own collapse sits outside the provider section's cards.
   */
  listToggle(id) {
    return cardFor(id)?.querySelector('button:has(.anticon-right), button:has(.anticon-down)') ?? null;
  },
  /** Does the card render that toggle at all? */
  hasListToggle(id) {
    return Boolean(window.kimiOfficialAccountFixture.listToggle(id));
  },
  hasHandle(id) {
    const card = cardFor(id);
    return Boolean(card?.querySelector('.anticon-holder'));
  },
  /** Does the card's heading line carry the account block's 🔗? */
  hasHeadingGlyph(id) {
    const card = cardFor(id);
    return Boolean(card?.querySelector('.anticon-link'));
  },
  handleCenter(id) {
    const card = cardFor(id);
    const handle = card?.querySelector('.anticon-holder');
    if (!handle) throw new Error('No drag handle rendered for: ' + id);
    return centerOf(handle.parentElement ?? handle);
  },
  /** Row center of another card — the drop target for a drag. */
  cardCenter(id) {
    const card = cardFor(id);
    if (!card) throw new Error('No card rendered for: ' + id);
    return centerOf(card);
  },
  /** The card's own text, so labels ("登录", "切换") can be read off the screen. */
  cardText(id) {
    return cardFor(id)?.textContent ?? null;
  },
  /**
   * The measured boxes of the sign-in entry, the list's title and the heading.
   *
   * The three are separate lines, and which line the button belongs on is a
   * placement rule no type or snapshot check can see — the button renders either
   * way. Callers compare the boxes (see the browser checks).
   */
  loginPlacement(id) {
    const card = cardFor(id);
    if (!card) throw new Error('No card rendered for: ' + id);
    const box = node => {
      const rect = node.getBoundingClientRect();
      return {
        left: Math.round(rect.left),
        right: Math.round(rect.right),
        top: Math.round(rect.top),
        bottom: Math.round(rect.bottom),
      };
    };
    /** The deepest element whose whole text is exactly `text`. */
    const leafWithText = text => [...card.querySelectorAll('*')]
      .filter(node => node.children.length === 0 && node.textContent.trim() === text)
      .at(-1) ?? null;
    const login = [...card.querySelectorAll('button')]
      .find(button => button.textContent.includes('登录')) ?? null;
    const listTitle = leafWithText(ACCOUNT_LIST_TITLE);
    const heading = leafWithText(OFFICIAL_CARD_HEADING);
    return {
      login: login ? box(login) : null,
      listTitle: listTitle ? box(listTitle) : null,
      heading: heading ? box(heading) : null,
    };
  },
  /** Labels of every button drawn inside the provider section's toolbar. */
  toolbarLabels() {
    return [...document.querySelectorAll('#kimi-providers .ant-collapse-header button')]
      .map(button => button.textContent.trim());
  },
  /** Labels of every button drawn inside a card, in DOM order. */
  cardButtonLabels(id) {
    return [...(cardFor(id)?.querySelectorAll('button') ?? [])]
      .map(button => button.textContent.trim());
  },
  /** Same buttons with their enabled state. */
  cardButtonStates(id) {
    return [...(cardFor(id)?.querySelectorAll('button') ?? [])]
      .map(button => ({
        label: button.textContent.trim(),
        disabled: button.disabled,
      }));
  },
  /**
   * Sentences the card renders more than once.
   *
   * Only long leaf texts are considered: short ones are legitimately repeated
   * (every row has a "切换"), while a duplicated *sentence* means a block was
   * added in a new place and not removed from the old one — which is what
   * happened when the section hint became the title's subtitle.
   */
  cardRepeatedSentences(id, minimumLength = 20) {
    const counts = new Map();
    for (const node of cardFor(id)?.querySelectorAll('*') ?? []) {
      if (node.children.length > 0) continue;
      const text = node.textContent.trim();
      if (text.length < minimumLength) continue;
      counts.set(text, (counts.get(text) ?? 0) + 1);
    }
    return [...counts.entries()].filter(([, count]) => count > 1).map(([text]) => text);
  },
  /** How many account rows the official card draws. */
  accountRowCount(id = OFFICIAL_PROVIDER_ID) {
    const card = cardFor(id);
    return card ? accountRowsIn(card).length : 0;
  },
  /** Debug aid: the outer HTML of the official card's account area. */
  debugAccountHtml(id = OFFICIAL_PROVIDER_ID) {
    const card = cardFor(id);
    return card ? card.innerHTML.slice(0, 4000) : null;
  },
  /**
   * Right edge of the account rows, of the card's content box, and of the
   * header's action links. The three must agree: a block that renders inside the
   * header's content column is inset by the action links' width, which is the
   * "empty space on the right" defect that a type check cannot see.
   */
  accountAlignment(id = OFFICIAL_PROVIDER_ID) {
    const card = cardFor(id);
    if (!card) throw new Error('No card rendered for: ' + id);
    const rows = accountRowsIn(card);
    if (rows.length === 0) throw new Error('No account rows rendered for: ' + id);
    // The card body's *content* edge, not its border box: the padding is part of
    // the card, and measuring the border box would call a correctly aligned row
    // inset. (The Codex guard subtracts the same padding.)
    const body = card.querySelector('.ant-card-body');
    const bodyStyle = body ? getComputedStyle(body) : null;
    const headerActions = card.querySelector('.anticon-ellipsis')?.closest('span, button, div');
    return {
      rowRight: Math.round(Math.max(...rows.map(row => row.getBoundingClientRect().right))),
      contentRight: body
        ? Math.round(body.getBoundingClientRect().right - parseFloat(bodyStyle.paddingRight || '0'))
        : null,
      headerRight: headerActions ? Math.round(headerActions.getBoundingClientRect().right) : null,
    };
  },
  async waitForMembers(ids, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (window.kimiOfficialAccountFixture.renderedMemberIds().join('|') === ids.join('|')) {
        return ids;
      }
      await wait(50);
    }
    return window.kimiOfficialAccountFixture.renderedMemberIds();
  },
  /** The last `reorder_kimi_providers` payload the page sent, if any. */
  lastReorder() {
    const call = [...state.requests].reverse().find(entry => entry.command === 'reorder_kimi_providers');
    return call ? call.args.ids : null;
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
