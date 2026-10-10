/**
 * Mounts the **real** `ZcodePage` against a stubbed Tauri IPC layer so the
 * official-account card can be inspected with real DOM queries.
 *
 * Why the real page rather than the card: this card is a *member* of the
 * provider list — it is inserted at `officialAccountIndex` among the provider
 * cards, dragged with the same handle, and its heading is drawn by the shared
 * section because ZCode has no provider row to head it. A card-only fixture
 * cannot see any of that.
 *
 * The three things this fixture exists to make checkable are the ones that
 * drifted while the card was hand-written: the heading (🔗 官方账号 + count),
 * the row indentation under it, and the hint's colour — which must match the
 * one Codex and Kimi render, even though this host draws it through a different
 * code path.
 *
 * Only `ZcodePage` is mounted, so the stub table below is the page's own command
 * set — an unstubbed command throws loudly instead of silently returning
 * `undefined`, which would turn a missing stub into a confusing render failure.
 */
import './ZcodeOfficialAccountStubs.js';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { updateGatewayProviderProfiles } from '@/features/coding/shared/gateway/providerProfiles';
// Imported before the page on purpose, mirroring `web/App.tsx`: the page sits in
// a real import cycle through `shared/providerShare`, so the fixture has to
// enter that cycle at the same point the app does; importing the page first
// throws `Cannot access 'ZcodePage' before initialization`.
import '@/app/routes';
import ZcodePage from '@/features/coding/zcode/pages/ZcodePage';
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

/** The official card's stable id, and the labels it must show. */
const OFFICIAL_ACCOUNT_CARD_ID = 'zcode-official-account';
const OFFICIAL_CARD_HEADING = '官方账号';
const ACCOUNT_LIST_TITLE = '账号列表';

const createProvider = (id, name, createdAt) => ({
  id,
  name,
  category: 'custom',
  settingsConfig: JSON.stringify({
    auth: { API_KEY: `fixture-key-${id}` },
    defaultModelKey: `${id}-model-a`,
    providerConfigs: { fixture: { type: 'openai', base_url: `https://${id}.example.test/v1` } },
    modelCatalog: {
      models: [
        {
          key: `${id}-model-a`,
          model: 'zai-model',
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

const createAccount = (id, createdAt, isApplied, { isVirtual = false, email = '' } = {}) => ({
  id,
  providerId: 'zai',
  name: '',
  kind: isVirtual ? 'local' : 'oauth',
  email,
  isApplied,
  isVirtual,
  createdAt,
  updatedAt: createdAt,
});

const savedAccounts = [
  createAccount('account-1', '2026-03-31T00:00:00.000Z', true, { email: 'user@example.invalid' }),
  createAccount('account-2', '2026-04-01T00:00:00.000Z', false, { email: 'second@example.invalid' }),
];

const accountMode = parameters.get('accounts') || '2';
const state = {
  runId: parameters.get('runId'),
  requests: [],
  sortMode: parameters.get('sortMode') || 'custom',
  officialAccountIndex: Number(parameters.get('index') ?? 1),
  providers: [
    createProvider('provider-a', 'Provider A', '2026-01-03T00:00:00.000Z'),
    createProvider('provider-b', 'Provider B', '2026-01-02T00:00:00.000Z'),
    createProvider('provider-c', 'Provider C', '2026-01-01T00:00:00.000Z'),
  ],
  officialAccounts: accountMode === '0' ? [] : structuredClone(savedAccounts),
};

const gatewayCliStatus = {
  cli_key: 'zcode',
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
      case 'get_zcode_config_file_path': return 'C:\\Users\\tester\\.zcode\\cli\\config.json';
      case 'get_zcode_root_path_info': return { path: 'C:\\Users\\tester\\.zcode', source: 'default' };
      case 'list_zcode_providers': return structuredClone(state.providers);
      case 'get_zcode_generation_status': return true;
      case 'list_zcode_official_accounts': return structuredClone(state.officialAccounts);
      case 'get_zcode_common_config':
        return { config: '', officialAccountIndex: state.officialAccountIndex };
      case 'save_zcode_official_account_index': {
        state.officialAccountIndex = args.index;
        return null;
      }
      case 'reorder_zcode_providers': {
        const byId = new Map(state.providers.map(provider => [provider.id, provider]));
        state.providers = args.ids
          .filter(id => id !== OFFICIAL_ACCOUNT_CARD_ID)
          .map(id => byId.get(id))
          .filter(Boolean);
        return null;
      }
      case 'save_zcode_official_local_account': {
        const virtual = state.officialAccounts.find(account => account.isVirtual);
        if (!virtual) throw new Error('No live ZCode login to save');
        virtual.isVirtual = false;
        return structuredClone(virtual);
      }
      case 'get_zcode_preview': return { config: '' };
      case 'read_zcode_settings': return {};
      case 'refresh_tray_menu': return null;
      case 'list_zcode_prompt_configs': return [];

      // --- provider list UI state -----------------------------------------
      case 'get_provider_list_state':
        return { sort_modes: { zcode: state.sortMode }, last_used: {} };
      case 'save_provider_sort_mode': {
        state.sortMode = args.mode;
        return null;
      }
      case 'record_provider_last_used': return null;

      // --- gateway ---------------------------------------------------------
      case 'proxy_gateway_cli_status': return structuredClone(gatewayCliStatus);
      case 'proxy_gateway_cli_statuses': return [structuredClone(gatewayCliStatus)];
      case 'proxy_gateway_supported_cli_keys': return ['zcode'];
      case 'proxy_gateway_status': return { running: false, mode: null };
      case 'proxy_gateway_get_settings': return {};

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
 * Every member card inside the provider section, in the order it is drawn.
 *
 * Addressed by the wrapper's `data-provider-id` — the same anchor the page's own
 * "locate" action uses — rather than by the words on a card: the official card
 * displays the shared heading, so a text anchor would find the wrong thing.
 */
const cardsInProviderSection = () =>
  [...document.querySelectorAll('#zcode-providers [data-provider-id]')];

const cardFor = providerId =>
  cardsInProviderSection().find(card => card.dataset.providerId === providerId);

const centerOf = element => {
  const rect = element.getBoundingClientRect();
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
};

/** The account rows of the official card: the ones the section draws a
 *  separator under (it sets the border inline), so the marker is structural. */
const accountRowsIn = card =>
  [...card.querySelectorAll('div')].filter(node =>
    String(node.style?.borderBottom || '').includes('1px solid'));

window.zcodeOfficialAccountFixture = {
  state,
  OFFICIAL_ACCOUNT_CARD_ID,
  OFFICIAL_CARD_HEADING,
  ACCOUNT_LIST_TITLE,
  /** The id of every member card, in the order the page draws them. */
  renderedMemberIds() {
    return cardsInProviderSection().map(card => card.dataset.providerId);
  },
  hasHandle(id) {
    return Boolean(cardFor(id)?.querySelector('.anticon-holder'));
  },
  /** Does the heading line carry the account block's 🔗? */
  hasHeadingGlyph(id) {
    return Boolean(cardFor(id)?.querySelector('.anticon-link'));
  },
  cardText(id) {
    return cardFor(id)?.textContent ?? null;
  },
  /** Labels of every button drawn inside a card, in DOM order. */
  cardButtonLabels(id) {
    return [...(cardFor(id)?.querySelectorAll('button') ?? [])]
      .map(button => button.textContent.trim());
  },
  /** The list's collapse toggle inside a card, if it renders one. */
  listToggle(id) {
    return cardFor(id)?.querySelector('button:has(.anticon-right), button:has(.anticon-down)') ?? null;
  },
  hasListToggle(id) {
    return Boolean(window.zcodeOfficialAccountFixture.listToggle(id));
  },
  /** How many account rows the official card draws. */
  accountRowCount(id = OFFICIAL_ACCOUNT_CARD_ID) {
    const card = cardFor(id);
    return card ? accountRowsIn(card).length : 0;
  },
  /**
   * The measured boxes and the computed styles of the three lines that have to
   * agree across the three CLIs: the heading, the list title and the hint.
   *
   * Colour is read off the rendered element on purpose — the hint is the one
   * sentence this host draws itself, and it silently came out a shade darker
   * than the same sentence on Codex and Kimi (`--color-text-secondary` vs
   * antd's `colorTextDescription`), which no type or source check can see.
   */
  lineMetrics(id = OFFICIAL_ACCOUNT_CARD_ID) {
    const card = cardFor(id);
    if (!card) throw new Error('No official-account card rendered');
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
    /** The deepest element whose text *contains* `text`. */
    const leafContaining = text => [...card.querySelectorAll('*')]
      .filter(node => node.children.length === 0 && node.textContent.includes(text))
      .at(-1) ?? null;
    const styleOf = node => {
      if (!node) return null;
      const computed = getComputedStyle(node);
      return { color: computed.color, fontSize: computed.fontSize };
    };
    const hint = leafContaining('切换会覆盖');
    const heading = leafWithText(OFFICIAL_CARD_HEADING);
    const listTitle = leafWithText(ACCOUNT_LIST_TITLE);
    const login = [...card.querySelectorAll('button')]
      .find(button => button.textContent.includes('登录')) ?? null;
    const rows = accountRowsIn(card);
    return {
      heading: heading ? box(heading) : null,
      listTitle: listTitle ? box(listTitle) : null,
      login: login ? box(login) : null,
      hint: hint ? { ...box(hint), ...styleOf(hint) } : null,
      // The rows' text boxes, for the left-edge alignment check.
      rowLefts: rows.map(row => Math.round(row.getBoundingClientRect().left)),
      listTitleLeft: listTitle ? Math.round(listTitle.getBoundingClientRect().left) : null,
    };
  },
  /** The last `reorder_zcode_providers` payload the page sent, if any. */
  lastReorder() {
    const call = [...state.requests].reverse().find(entry => entry.command === 'reorder_zcode_providers');
    return call ? call.args.ids : null;
  },
};

createRoot(document.getElementById('root')).render(
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <App>
      {/* The cards call `useNavigate`, so the page needs router context. */}
      <MemoryRouter initialEntries={['/coding/zcode']}>
        <ZcodePage />
      </MemoryRouter>
    </App>
  </ConfigProvider>,
);
