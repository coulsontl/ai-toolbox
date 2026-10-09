/**
 * Mounts the **real** `CodexPage` against a stubbed Tauri IPC layer so the
 * provider-list drag interaction can be driven with real pointer events in a
 * browser.
 *
 * Why the real page and not a card fixture: the drag path spans three files —
 * the page's `DndContext` sensors, the mapping layer's `draggable` gate, and
 * `CardShell`'s `useSortable` registration. A card-only fixture would have to
 * re-create the page half, which is exactly where the disagreement lives. The
 * bug this guards against ("the handle renders but nothing moves") is invisible
 * to any check that does not start a real drag on a real handle.
 *
 * Only `CodexPage` is mounted, so the stub table below is the page's own command
 * set — an unstubbed command throws loudly instead of silently returning
 * `undefined`, which would turn a missing stub into a confusing render failure.
 */
import './ProviderListDragStubs.js';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { updateGatewayProviderProfiles } from '@/features/coding/shared/gateway/providerProfiles';
// Imported before the page on purpose. The page sits in a real import cycle:
// `CodexPage` → `shared/providerShare` → `ProviderTransferModal` →
// `deeplinkImportAction` → `app/routes` → `routeConfig` → `CodexPage`. The app
// entry (`web/App.tsx`) enters that cycle at `app/routes`, so the fixture must
// too; importing the page first instead throws `Cannot access 'CodexPage'
// before initialization`. That is an import-order artifact, not a product bug —
// the app itself never evaluates the page before the router.
import '@/app/routes';
import CodexPage from '@/features/coding/codex/pages/CodexPage';
import gatewayProfiles from '../../../../../../../tauri/resources/gateway_provider_profiles.json';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
/** How many providers to list. Three (the default) fit on screen; the locate
 *  check asks for more so the applied card starts below the fold. */
const providerCount = Number(parameters.get('count') || 3);
/** Which provider record carries `isApplied`, i.e. the applied tag and the
 *  locate action's target. Any value that matches no provider id (the checks
 *  use `applied=none`) leaves nothing applied; `applied=__local__` puts the
 *  flag on the local-file bridge record instead (see `bridge` below). */
const appliedProviderId = parameters.get('applied') || 'provider-a';
/** `provider-a`…`provider-n`, in the order the page lists them. */
const providerIds = Array.from('abcdefghijklmnopqrstuvwxyz'.slice(0, providerCount));
/**
 * The local-file bridge record.
 *
 * The backend flags this record applied, but every card suppresses the applied
 * chrome for it (`showRuntimeApplied = isApplied && !isLocalProvider`) — it is
 * a mirror of the on-disk config, not a preset the user applied. The page must
 * not offer it as the locate target, and this is the only state where the
 * backend's `is_applied` and the card's badge disagree.
 */
const isBridgeApplied = appliedProviderId === '__local__';
const themeMode = parameters.get('theme') || 'light';
const resolvedTheme = themeMode === 'system'
  ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
  : themeMode;
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = resolvedTheme;
updateGatewayProviderProfiles(gatewayProfiles);

/** Providers, in the order the page will list them. `createdAt` is set so the
 *  "creation time" sort mode produces a *different* order than the stored one —
 *  otherwise a drag assertion could not tell the two apart. */
const createProvider = (id, name, createdAt, category = 'custom') => ({
  id,
  name,
  category,
  settingsConfig: JSON.stringify({
    auth: { OPENAI_API_KEY: 'fixture-key-' + id },
    config: 'model_provider = "custom"\nmodel = "gpt-5"\n[model_providers.custom]\n'
      + 'name = "Fixture"\nwire_api = "responses"\nbase_url = "https://' + id + '.example.test/v1"',
    modelCatalog: {
      models: [
        { model: id + '-model-a', displayName: 'Model A', contextWindow: 128000 },
        { model: id + '-model-b', displayName: 'Model B', contextWindow: 64000 },
      ],
    },
  }),
  isApplied: id === appliedProviderId,
  isDisabled: false,
  createdAt,
  updatedAt: createdAt,
});

/** Two saved accounts, one applied — enough to see both row states. Pass
 *  `?accounts=2` to render them on every card; the default keeps the drag
 *  fixture's page as bare as the drag assertions expect. */
const createOfficialAccount = (id, email, isApplied) => ({
  id,
  providerId: 'provider-a',
  name: email,
  kind: 'oauth',
  email,
  planType: 'plus',
  limitMonthlyText: '100%',
  isApplied,
  isVirtual: false,
  createdAt: '2026-01-01T00:00:00.000Z',
  updatedAt: '2026-01-01T00:00:00.000Z',
});

const state = {
  runId: parameters.get('runId'),
  requests: [],
  sortMode: parameters.get('sortMode') || 'custom',
  providers: [
    ...(isBridgeApplied
      ? [createProvider('__local__', 'default', '2026-01-04T00:00:00.000Z')]
      : []),
    ...providerIds.map((letter, index) => createProvider(
      `provider-${letter}`,
      `Provider ${letter.toUpperCase()}`,
      new Date(Date.UTC(2026, 0, 3 - index)).toISOString(),
      // The account rows only render on an `official` provider, so `?accounts=2`
      // turns provider A into one — that is the shape the rows appear in.
      index === 0 && parameters.get('accounts') === '2' ? 'official' : 'custom',
    )),
  ],
  officialAccounts: parameters.get('accounts') === '2'
    ? [
        createOfficialAccount('account-1', 'first@example.invalid', true),
        createOfficialAccount('account-2', 'second@example.invalid', false),
      ]
    : [],
};

const gatewayCliStatus = {
  cli_key: 'codex',
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
      case 'get_codex_config_file_path': return 'C:\\Users\\tester\\.codex\\config.toml';
      case 'get_codex_root_path_info': return { path: 'C:\\Users\\tester\\.codex', source: 'default' };
      case 'list_codex_providers': return structuredClone(state.providers);
      case 'list_codex_official_accounts': return structuredClone(state.officialAccounts);
      case 'reorder_codex_providers': {
        const byId = new Map(state.providers.map(provider => [provider.id, provider]));
        state.providers = args.ids.map(id => byId.get(id));
        return null;
      }
      case 'list_opencode_favorite_providers': return [];
      case 'refresh_tray_menu': return null;
      case 'get_codex_common_config': return null;
      case 'read_codex_settings': return {};

      // --- provider list UI state -----------------------------------------
      case 'get_provider_list_state':
        return { sort_modes: { codex: state.sortMode }, last_used: {} };
      case 'save_provider_sort_mode': {
        state.sortMode = args.mode;
        return null;
      }
      case 'record_provider_last_used': return null;

      // --- gateway ---------------------------------------------------------
      case 'proxy_gateway_cli_status': return structuredClone(gatewayCliStatus);
      case 'proxy_gateway_cli_statuses': return [structuredClone(gatewayCliStatus)];
      case 'proxy_gateway_supported_cli_keys': return ['codex'];
      case 'proxy_gateway_status': return { running: false, mode: null };
      case 'proxy_gateway_get_settings': return {};

      // --- plugins / memories / sessions ----------------------------------
      case 'get_codex_plugin_runtime_status':
        return { feature_enabled: false, runtime_available: false, runtime_error: null, plugin_count: 0 };
      case 'list_codex_installed_plugins': return [];
      case 'list_codex_plugin_workspace_roots': return [];
      case 'list_codex_memories': return { entries: [], root_path: 'C:\\Users\\tester\\.codex\\memories' };
      case 'list_tool_sessions':
        return { sessions: [], total: 0, has_more: false, next_cursor: null };

      // --- global prompt list ---------------------------------------------
      case 'list_codex_prompt_configs': return [];

      // --- feature-availability probes ------------------------------------
      case 'has_all_api_hub_extension': return false;
      case 'has_cc_switch_db': return false;

      // --- event plumbing --------------------------------------------------
      case 'plugin:event|listen': return 1;
      case 'plugin:event|unlisten': return null;
      case 'plugin:opener|reveal_item_in_dir': return null;

      default:
        throw new Error('Unstubbed fixture command: ' + command);
    }
  },
};

const wait = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

/** The page's own drag handle, addressed the way a user reaches it: the grip
 *  inside the card whose title matches. */
const cardFor = name => [...document.querySelectorAll('.ant-card')]
  .find(card => card.textContent.includes(name));

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

window.providerListDragFixture = {
  state,
  /** Provider names in the order the page currently renders them. */
  renderedOrder() {
    return [...document.querySelectorAll('.ant-card')]
      .map(card => state.providers.find(provider => card.textContent.includes(provider.name))?.name)
      .filter(Boolean);
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
  async setSortMode(mode) {
    window.providerListDragFixture.state.sortMode = mode;
  },
  async waitForOrder(names, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const order = window.providerListDragFixture.renderedOrder();
      if (order.join('|') === names.join('|')) return order;
      await wait(50);
    }
    return window.providerListDragFixture.renderedOrder();
  },
  /** The last `reorder_codex_providers` payload the page sent, if any. */
  lastReorder() {
    const call = [...state.requests].reverse().find(entry => entry.command === 'reorder_codex_providers');
    return call ? call.args.ids : null;
  },
  /** Stored order, as the backend would persist it. */
  storedOrder() {
    return state.providers.map(provider => provider.name);
  },
  /** The record the fixture marked applied — the locate action's target. */
  appliedProviderId,
  /** The applied card's box relative to the viewport: how the locate check
   *  tells "scrolled into view" from "still below the fold". */
  appliedCardBox() {
    const card = document.querySelector(`[data-provider-id="${appliedProviderId}"]`);
    if (!card) return null;
    const rect = card.getBoundingClientRect();
    return { top: rect.top, bottom: rect.bottom, viewportHeight: window.innerHeight };
  },
  /**
   * The applied card wrapper's computed shadow — the locate flash's *visible*
   * effect. Read from the rendered style, not the class list: the CSS-module
   * build hashes the class name, and a check that matched a hash would pass
   * even if the rule never applied.
   */
  appliedCardShadow() {
    const card = document.querySelector(`[data-provider-id="${appliedProviderId}"]`);
    return card ? getComputedStyle(card).boxShadow : null;
  },
};

createRoot(document.getElementById('root')).render(
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <App>
      {/* The cards call `useNavigate`, so the page needs router context. The
          app provides it through `RouterProvider`; a memory router is the
          equivalent for a single mounted page. */}
      <MemoryRouter initialEntries={['/coding/codex']}>
        {/* The app scrolls `main` (MainLayout's `styles.main`), and App.css
            pins `html/body/#root` to `overflow: hidden` — so without a
            scroll container here the page is simply clipped and
            `scrollIntoView` has nothing to scroll. Any check that drives the
            provider list's locate action needs this, or it would be asserting
            against a fixture in which scrolling is impossible. */}
        <main style={{ height: '100%', overflowY: 'auto' }}>
          <CodexPage />
        </main>
      </MemoryRouter>
    </App>
  </ConfigProvider>,
);
