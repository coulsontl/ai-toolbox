/**
 * Mounts the **real** `CodexPage` against a stubbed Tauri IPC layer so the
 * provider list's card layout can be driven with real pointer events.
 *
 * Why the real page and not a card fixture: the layout switch spans the page's
 * view preference + `DndContext` strategy, the shared toolbar, the mapping
 * layer's `viewMode` branch and the compact card's own sortable registration.
 * The failures this guards — "the toggle renders but nothing changes", "the
 * model catalog disappeared when the grid came in" — are invisible to any check
 * that does not mount the real page and click the real control.
 *
 * The command table is shared with the drag fixture (`codexPageStubs.js`);
 * only the provider records differ, because the card view states different
 * things (a primary model, a protocol, an official account block).
 */
import '../../shared/providerList/fixtures/ProviderListDragStubs.js';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import { updateGatewayProviderProfiles } from '@/features/coding/shared/gateway/providerProfiles';
import { createCodexPageInvoke } from '../../shared/providerList/fixtures/codexPageStubs.js';
// Imported before the page on purpose: the page sits in a real import cycle
// (`CodexPage` → `shared/providerShare` → … → `app/routes` → `routeConfig` →
// the page). The app entry enters that cycle at `app/routes`, so the fixture
// must too; importing the page first throws
// `Cannot access 'CodexPage' before initialization`.
import '@/app/routes';
import CodexPage from '@/features/coding/codex/pages/CodexPage';
import gatewayProfiles from '../../../../../../tauri/resources/gateway_provider_profiles.json';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const themeMode = parameters.get('theme') || 'light';
const resolvedTheme = themeMode === 'system'
  ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
  : themeMode;
/** Initial layout, so a check can start in either one without clicking first. */
const initialViewMode = parameters.get('viewMode') || 'list';
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = resolvedTheme;
updateGatewayProviderProfiles(gatewayProfiles);

/** Long enough to overflow the sidebar's value column, so the middle-elision
 *  and the copy button can be told apart: elided on screen, whole on the
 *  clipboard. */
const longApiKey = 'sk-codex-fixture-0123456789abcdefghijklmnopqrstuvwxyz-TAIL';

/**
 * One provider record.
 *
 * Everything Codex knows lives inside `settingsConfig`: the endpoint and the
 * active model are inside the `config.toml` text, the catalog is its own JSON
 * field, and the protocol comes from `meta.apiFormat` when the user picked one
 * (otherwise it is derived from the wire API in the TOML).
 */
const createProvider = ({ id, name, createdAt, category = 'custom', model, effort, apiFormat, apiKey, notes, models = [], autoReview }) => ({
  id,
  name,
  category,
  settingsConfig: JSON.stringify({
    auth: { OPENAI_API_KEY: apiKey ?? `fixture-key-${id}` },
    config: [
      'model_provider = "custom"',
      model ? `model = "${model}"` : null,
      effort ? `model_reasoning_effort = "${effort}"` : null,
      '[model_providers.custom]',
      'name = "Fixture"',
      'wire_api = "responses"',
      `base_url = "https://${id}.example.test/v1"`,
    ].filter(Boolean).join('\n'),
    modelCatalog: { models: models.map((entry) => ({ model: entry.model, displayName: entry.displayName, contextWindow: 128000 })) },
    // Only provider-a declares one: the auto-review line and the per-row
    // action are Codex-only affordances of the catalog, and the sidebar has to
    // carry them with it.
    ...(autoReview ? { autoReviewModelOverride: autoReview } : {}),
  }),
  meta: apiFormat ? { apiFormat } : undefined,
  notes,
  isApplied: id === 'provider-a',
  isDisabled: false,
  createdAt,
  updatedAt: createdAt,
});

const accountsForProviderC = [
  {
    id: 'account-1',
    providerId: 'provider-c',
    name: 'first@example.invalid',
    kind: 'oauth',
    email: 'first@example.invalid',
    planType: 'plus',
    limitMonthlyText: '100%',
    isApplied: true,
    isVirtual: false,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
  },
  {
    id: 'account-2',
    providerId: 'provider-c',
    name: 'second@example.invalid',
    kind: 'oauth',
    email: 'second@example.invalid',
    planType: 'plus',
    limitMonthlyText: '100%',
    isApplied: false,
    isVirtual: false,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
  },
];

/**
 * The login the CLI itself is holding, when no saved row represents it.
 *
 * `auth.json` can carry a login the app has never captured, and the backend
 * hands that to the frontend as one virtual row (`isVirtual`, no snapshot of its
 * own, `isApplied: false` by construction) — the shape the block draws as 本地
 * and the cell names as the login in use.
 */
const liveLoginAccount = {
  id: '__local__',
  // The backend hands the virtual row to the official provider that asked for
  // it (`assign_provider_id`), which is the list it belongs to.
  providerId: 'provider-c',
  name: '__local__',
  kind: 'local',
  email: 'live@example.invalid',
  planType: null,
  limitMonthlyText: null,
  isApplied: false,
  isVirtual: true,
  createdAt: '2026-01-01T00:00:00.000Z',
  updatedAt: '2026-01-01T00:00:00.000Z',
};

/**
 * Which accounts the official channel holds.
 *
 * `idle` is the state the cell used to render as nothing at all — accounts
 * saved, none of them in use. `virtual` is the state the reporter's machine was
 * actually in: a saved account that is not in use **plus** the login the CLI's
 * own `auth.json` still carries, which is the pair the cell has to choose from
 * (it must name the live one, not the idle one) and the pair that made the
 * heading read `官方账号 (2)`.
 */
const accountMode = parameters.get('accounts') || 'saved';

const state = {
  runId: parameters.get('runId'),
  requests: [],
  sortMode: 'custom',
  viewMode: initialViewMode,
  officialAccounts:
    accountMode === 'idle'
      ? accountsForProviderC.map((account) => ({ ...account, isApplied: false }))
      : accountMode === 'virtual'
        // The saved account first, the live login last: the order the backend
        // lists them in, and the order the cell must not be fooled by.
        ? [
            { ...accountsForProviderC[0], isApplied: false },
            liveLoginAccount,
          ]
        : accountsForProviderC,
  providers: [
    // Applied, with a full catalog, a protocol the user picked, a long key and
    // a note: the record whose sidebar states every row.
    createProvider({
      id: 'provider-a',
      name: 'Provider Main',
      createdAt: '2026-01-04T00:00:00.000Z',
      model: 'gpt-5',
      effort: 'high',
      apiFormat: 'openai_chat',
      apiKey: longApiKey,
      notes: 'fixture note',
      models: [
        { model: 'gpt-5', displayName: 'GPT-5' },
        { model: 'gpt-5-mini', displayName: 'GPT-5 mini' },
      ],
      autoReview: 'gpt-5-mini',
    }),
    // No catalog, no note: the plain case.
    createProvider({
      id: 'provider-b',
      name: 'Provider Plain',
      createdAt: '2026-01-03T00:00:00.000Z',
      model: 'gpt-5.1',
    }),
    // The official channel: named for what it holds, headed by 🔗 + count, and
    // carrying the account block the grid cell has no room for.
    createProvider({
      id: 'provider-c',
      name: 'Official Codex',
      createdAt: '2026-01-02T00:00:00.000Z',
      category: 'official',
    }),
    // A custom provider whose config declares no model at all.
    createProvider({
      id: 'provider-d',
      name: 'Provider No Model',
      createdAt: '2026-01-01T00:00:00.000Z',
    }),
  ],
};

// Opt-in usage scenarios exercise the real page and command wrapper with synthetic data.
const pendingUsage = [];
const pendingAccountLists = [];
let deferAccountList = false;
const baseInvoke = createCodexPageInvoke(state);
const invoke = async (command, args) => {
  if (parameters.has('credits') && command === 'list_codex_official_accounts' && deferAccountList) {
    deferAccountList = false;
    const snapshot = await baseInvoke(command, args);
    return new Promise(resolve => pendingAccountLists.push(() => resolve(snapshot)));
  }
  if (parameters.has('credits') && command === 'refresh_codex_official_account_limits') {
    state.requests.push({ command, args: structuredClone(args) });
    const account = structuredClone(state.officialAccounts.find((item) =>
      item.id === args.accountId && item.providerId === args.providerId));
    if (!account) throw new Error('Synthetic account not found');
    return new Promise((resolve, reject) => pendingUsage.push({ account, resolve, reject }));
  }
  return baseInvoke(command, args);
};

const eventListeners = new Map();
let nextEventId = 0;
/** The clipboard is recorded, not written to: the sidebar's copy buttons are
 *  only worth having if they put the *whole* value on it, and a real clipboard
 *  needs a permission a headless fixture would have to grant. */
const clipboardWrites = [];

Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: { writeText: async (text) => { clipboardWrites.push(text); } },
});

window.__TAURI_INTERNALS__ = {
  transformCallback(callback) {
    const id = nextEventId++;
    eventListeners.set(id, callback);
    return id;
  },
  unregisterCallback(id) {
    eventListeners.delete(id);
  },
  invoke,
};

const wait = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

/**
 * A card by provider id, falling back to the text it contains.
 *
 * Two identifiers, because the page gives a card two names: the record's id
 * (`data-provider-id`, the same anchor the locate action uses) and whatever it
 * prints. The official channel prints 官方账号 rather than its own label, so its
 * checks address it by id — and a check that wants the *title* still has to say
 * which text it expects to find.
 */
const cardFor = (key) => document.querySelector(`[data-provider-id="${key}"]`)
  ?? [...document.querySelectorAll('[data-provider-id]')]
    .find((card) => card.textContent.includes(key));

/** The card's title: the span whose text is exactly the provider name. */
const titleOf = (card, name) => [...(card?.querySelectorAll('span') ?? [])]
  .find((span) => span.textContent.trim() === name);

const iconName = (element) => element?.querySelector('.anticon')?.getAttribute('aria-label') ?? null;

window.codexCardViewFixture = {
  state,
  pendingUsageCount: () => pendingUsage.length,
  deferNextAccountList() { deferAccountList = true; },
  pendingAccountListCount: () => pendingAccountLists.length,
  respondAccountList() {
    const resolve = pendingAccountLists.shift();
    if (!resolve) throw new Error('No pending synthetic account list');
    resolve();
  },
  respondUsage(patch, error) {
    const pending = pendingUsage.shift();
    if (!pending) throw new Error('No pending synthetic usage request');
    if (error) {
      state.officialAccounts = state.officialAccounts.map((current) =>
        current.id === pending.account.id && current.providerId === pending.account.providerId
          && current.email === pending.account.email
          ? { ...current, credits: undefined, lastError: error } : current);
      pending.reject(new Error(error));
      return;
    }
    const next = { ...pending.account, lastError: undefined,
      lastLimitsFetchedAt: '2026-10-24T12:34:56Z', ...patch };
    // A delayed response must not rewrite the fixture's newly selected login.
    state.officialAccounts = state.officialAccounts.map((current) =>
      current.id === next.id && current.providerId === next.providerId
        && current.email === next.email && current.accountId === next.accountId ? next : current);
    pending.resolve(next);
  },
  reloadAccounts(accounts) {
    state.officialAccounts = accounts;
    window.dispatchEvent(new Event(TRAY_CONFIG_REFRESH_EVENT));
  },
  accountRow(email) {
    return [...document.querySelectorAll('div')].find((element) =>
      element.style.borderBottom && element.textContent.includes(email)) ?? null;
  },
  accountAction(email, icon) {
    // AntD replaces the action icon with a spinner while pending; the translated
    // button label remains stable and lets a real second click test suppression.
    const label = icon === 'sync' ? i18n.t('common.officialAccount.refresh')
      : icon === 'eye' ? i18n.t('common.officialAccount.details') : null;
    const button = label
      ? [...(this.accountRow(email)?.querySelectorAll('button') ?? [])]
        .find((item) => item.textContent.trim() === label)
      : this.accountRow(email)?.querySelector(`.anticon-${icon}`)?.closest('button');
    if (!button) throw new Error('Account action not found: ' + email + '/' + icon);
    return button;
  },
  clickAccountAction(email, icon) {
    this.accountAction(email, icon).click();
  },
  /** The layout toggle's centre, found by its data attribute: the label is
   *  卡片/列表 whichever way it reads, and copy is free to change. */
  layoutToggleCenter() {
    const button = document.querySelector('[data-provider-view-toggle]');
    if (!button) return null;
    const rect = button.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  layoutToggleMode() {
    return document.querySelector('[data-provider-view-toggle]')
      ?.getAttribute('data-provider-view-toggle') ?? null;
  },
  layoutToggleLabel() {
    return document.querySelector('[data-provider-view-toggle]')?.textContent.trim() ?? null;
  },
  layoutToggleIconName() {
    return iconName(document.querySelector('[data-provider-view-toggle]'));
  },
  /**
   * Provider ids in the order the page renders them.
   *
   * By id, not by name: an official provider's card is headed 官方账号 rather
   * than by the row's own label — that is the product's rule, so matching on
   * `provider.name` would silently drop that card from the order and every
   * count would be one short.
   */
  renderedOrder() {
    return [...document.querySelectorAll('[data-provider-id]')]
      .map((card) => card.getAttribute('data-provider-id'))
      .filter(Boolean);
  },
  gridColumnCount() {
    const grid = document.querySelector('[data-provider-grid]');
    if (!grid) return 0;
    return getComputedStyle(grid).gridTemplateColumns.split(' ').filter(Boolean).length;
  },
  cardText(name) {
    return cardFor(name)?.textContent ?? null;
  },
  /**
   * antd's icon names rendered inside a card, in document order.
   *
   * These elements *are* the icons, so the name is read straight off them —
   * `iconName` searches *within* an element, which is what a button wrapping an
   * icon needs and the wrong thing here.
   */
  cardIconNames(key) {
    const card = cardFor(key);
    if (!card) return null;
    return [...card.querySelectorAll('.anticon')]
      .map((icon) => icon.getAttribute('aria-label'))
      .filter(Boolean);
  },
  /**
   * Whether the link glyph sits **before** the title it heads.
   *
   * The heading reads `🔗 官方账号 (2)`, so the order is the claim — a check that
   * only asked "is there a link icon in this card" would pass on a heading that
   * put the glyph after the words.
   */
  headingGlyphPrecedesTitle(key, titleText) {
    const card = cardFor(key);
    const glyph = card?.querySelector('.anticon-link');
    const title = titleOf(card, titleText);
    if (!glyph || !title) return false;
    return Boolean(glyph.compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING);
  },
  /**
   * The card's second line, or null when it renders none.
   *
   * Every gap on the line is a CSS margin rather than a text node, so
   * `textContent` would read `主模型:gpt-5`. Rebuilding it reports what the user
   * actually sees.
   */
  subtitleText(name) {
    const card = cardFor(name);
    const subtitle = card?.querySelector('[data-provider-subtitle]');
    if (!subtitle) return null;
    return [...subtitle.children]
      .map((entry) => [...entry.children].map((part) => part.textContent.trim()).join(' '))
      .join('  ');
  },
  /** The state tags a card states beside its title, in order. */
  cardTagLabels(name) {
    const row = cardFor(name)?.querySelector('[data-provider-card-tags]');
    if (!row) return null;
    return [...row.querySelectorAll('.ant-tag')].map((tag) => tag.textContent.trim());
  },
  /**
   * What the card states, as one object — the type scale and the wording.
   *
   * `titleText` is separate from the card key because the title has to be
   * matched by what it *prints*, and the card may be addressed by id.
   */
  cardSummary(key, titleText) {
    const card = cardFor(key);
    if (!card) return null;
    const title = titleOf(card, titleText);
    const subtitle = card.querySelector('[data-provider-subtitle]');
    const button = card.querySelector('[data-provider-card-footer] .ant-btn');
    return {
      title: title?.textContent?.trim() ?? null,
      titleFontSize: title ? getComputedStyle(title).fontSize : null,
      subtitleFontSize: subtitle ? getComputedStyle(subtitle).fontSize : null,
      footerButtonFontSize: button ? getComputedStyle(button).fontSize : null,
    };
  },
  footerActions(name) {
    const footer = cardFor(name)?.querySelector('[data-provider-card-footer]');
    if (!footer) return null;
    return [...footer.querySelectorAll('button')]
      .map((button) => button.textContent.trim())
      .filter(Boolean);
  },
  cardCenter(name) {
    const card = cardFor(name);
    if (!card) throw new Error('No card rendered for: ' + name);
    const rect = card.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  handleCenter(name) {
    const handle = cardFor(name)?.querySelector('[data-provider-card-handle]');
    if (!handle) throw new Error('No drag handle rendered for: ' + name);
    const rect = handle.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  titleCenter(name) {
    const title = titleOf(cardFor(name), name);
    if (!title) throw new Error('No title rendered for: ' + name);
    const rect = title.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  openTooltipText() {
    return document.querySelector('.ant-tooltip:not(.ant-tooltip-hidden)')?.textContent?.trim() ?? null;
  },
  openedUrls() {
    return [...(state.openedUrls ?? [])];
  },
  moreButtonOpacity(name) {
    const wrapper = cardFor(name)?.querySelector('[data-provider-card-actions]');
    return wrapper ? Number(getComputedStyle(wrapper).opacity) : null;
  },
  moreButtonCenter(name) {
    const button = cardFor(name)?.querySelector('[data-provider-card-actions] button');
    if (!button) return null;
    const rect = button.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  openMenuItemLabels() {
    return [...document.querySelectorAll('.ant-dropdown-menu-item')]
      .map((item) => item.textContent.trim());
  },
  /** The open detail sidebar, or null when none is open. */
  detailPanel() {
    return document.querySelector('[data-provider-detail-panel]');
  },
  detailSectionTitles() {
    const panel = window.codexCardViewFixture.detailPanel();
    if (!panel) return null;
    return [...panel.querySelectorAll('[data-provider-detail-section-title]')]
      .map((title) => title.textContent.trim());
  },
  /** A field row's value, as rendered. */
  detailFieldText(field) {
    const panel = window.codexCardViewFixture.detailPanel();
    const cell = panel?.querySelector(`[data-provider-detail-field="${field}"]`);
    return cell?.textContent?.trim() ?? null;
  },
  detailCopyFields() {
    const panel = window.codexCardViewFixture.detailPanel();
    if (!panel) return null;
    return [...panel.querySelectorAll('[data-provider-detail-copy]')]
      .map((button) => button.getAttribute('data-provider-detail-copy'));
  },
  copyDetailField(field) {
    const panel = window.codexCardViewFixture.detailPanel();
    const button = panel?.querySelector(`[data-provider-detail-copy="${field}"]`);
    if (!button) return false;
    button.click();
    return true;
  },
  clipboardWrites() {
    return [...clipboardWrites];
  },
  detailFooterActions() {
    const footer = document.querySelector('[data-provider-detail-footer]');
    if (!footer) return null;
    return [...footer.querySelectorAll('button')]
      .map((button) => button.textContent.trim())
      .filter(Boolean);
  },
  /**
   * The state tags the sidebar's title line states, in order.
   *
   * The same fragment the cell's tag row renders — a sidebar that listed the
   * tags by hand would drift the moment either side gained one.
   */
  detailStatusTagLabels() {
    const line = document.querySelector('[data-provider-detail-status]');
    if (!line) return null;
    return [...line.querySelectorAll('.ant-tag')].map((tag) => tag.textContent.trim());
  },
  detailStatusActions() {
    const line = document.querySelector('[data-provider-detail-status]');
    if (!line) return null;
    return [...line.querySelectorAll('button')]
      .map((button) => button.textContent.trim())
      .filter(Boolean);
  },
  /**
   * Waits for the sidebar's model catalog to be on screen.
   *
   * The section used to be a collapsed Collapse, so a check had to click it
   * open first. The sidebar's copy is a plain always-open block now
   * (user-directed, 2026-10-12) — the list card's copy still folds, which is
   * why the attribute this waits on is only on the sidebar's one — so this only
   * waits, and it still exists because the sidebar mounts behind a `Drawer`
   * whose content arrives a frame or two later.
   */
  async openDetailCatalog() {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (document.querySelector('[data-model-list-section]')) return true;
      await wait(50);
    }
    return false;
  },
  /** Clicks a sidebar button by the label it prints. */
  clickDetailButton(label) {
    const panel = window.codexCardViewFixture.detailPanel();
    const button = [...(panel?.querySelectorAll('button') ?? [])]
      .find((candidate) => candidate.textContent.includes(label));
    if (!button) return false;
    button.click();
    return true;
  },
  detailTextIncludes(text) {
    const panel = window.codexCardViewFixture.detailPanel();
    return panel ? panel.textContent.includes(text) : false;
  },
  closeDetail() {
    const button = window.codexCardViewFixture.detailPanel()
      ?.querySelector('[data-provider-detail-close]');
    if (button) button.click();
  },
  lastViewModeSave() {
    const call = [...state.requests].reverse().find((entry) => entry.command === 'save_provider_view_mode');
    return call ? call.args : null;
  },
  lastReorder() {
    const call = [...state.requests].reverse().find((entry) => entry.command === 'reorder_codex_providers');
    return call ? call.args.ids : null;
  },
  async waitForOrder(names, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const order = window.codexCardViewFixture.renderedOrder();
      if (order.join('|') === names.join('|')) return order;
      await wait(50);
    }
    return window.codexCardViewFixture.renderedOrder();
  },
  };

createRoot(document.getElementById('root')).render(
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <App>
      {/* The cards call `useNavigate`, so the page needs router context. */}
      <MemoryRouter initialEntries={['/coding/codex']}>
        {/* App.css pins `html/body/#root` to `overflow: hidden` and the app
            scrolls `main`, so any check that drives the provider list needs
            this wrapper or it would assert against a page that cannot scroll. */}
        <main style={{ height: '100%', overflowY: 'auto' }}>
          <CodexPage />
        </main>
      </MemoryRouter>
    </App>
  </ConfigProvider>,
);
