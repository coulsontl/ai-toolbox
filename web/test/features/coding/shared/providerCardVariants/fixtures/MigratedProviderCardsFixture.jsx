/**
 * Mounts the four migrated provider mapping layers directly, with no page and
 * no IPC stubs beyond what the modules touch at import time.
 *
 * Why not the real pages: the four pages each call dozens of Tauri commands on
 * mount, and the claims this fixture guards belong to the mapping layers — the
 * layout they pick, the facts they state, and the guards they honour. A page
 * fixture would break the day a page gains an unrelated command.
 *
 * What it guards, per CLI:
 *
 * - Claude Desktop: role bindings render as `Role: code`, a `tierAlias` becomes
 *   a real tag, notes live in the footer *below* the header columns, and a
 *   disabled drag renders no handle.
 * - Gemini CLI: the official card's name row is `🔗 官方账号 (n)`, its second
 *   line is the switching explanation, the account block's rows span the card's
 *   content width, and a virtual row offers only 「保存当前登录」.
 * - Grok: the official row's sixth action (refresh token) is rendered through
 *   the shared section's extra-action slot, and the model toolbar keeps the
 *   documented order.
 * - OmO Native: the grid cell states the npm package (plus the default model
 *   for the provider that owns it), and the default provider's "more" menu
 *   greys out delete.
 */
import './ProviderCardSpacingStubs.js';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { updateGatewayProviderProfiles } from '@/features/coding/shared/gateway/providerProfiles';
import ClaudeDesktopProviderCard from '@/features/coding/claudedesktop/components/ClaudeDesktopProviderCard';
import GeminiCliProviderCard from '@/features/coding/geminicli/components/GeminiCliProviderCard';
import GrokProviderCard from '@/features/coding/grok/components/GrokProviderCard';
import OmoNativeProviderCard from '@/features/coding/omo_native/components/OmoNativeProviderCard';
import OpenClawProviderCard from '@/features/coding/openclaw/components/OpenClawProviderCard';
import HermesProviderCard from '@/features/coding/hermes/components/HermesProviderCard';
import gatewayProfiles from '../../../../../../../tauri/resources/gateway_provider_profiles.json';
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

const noop = () => {};

/* ------------------------------------------------------------------ Claude Desktop */

const claudeDesktopProvider = {
  id: 'desktop-a',
  name: 'Desktop Relay',
  category: 'custom',
  settingsConfig: JSON.stringify({
    env: {
      ANTHROPIC_BASE_URL: 'https://desktop.example.test',
      ANTHROPIC_AUTH_TOKEN: 'sk-ant-desktop-fixture-key',
    },
  }),
  meta: {
    claudeDesktopModelRoutes: {
      'claude-sonnet-5': { model: 'claude-sonnet-4-5', tierAlias: 'sonnet' },
      'claude-opus-5': { model: 'claude-opus-4-8' },
    },
  },
  notes: '团队共享额度',
  isApplied: true,
  isDisabled: false,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

const claudeDesktopProps = {
  provider: claudeDesktopProvider,
  isApplied: true,
  onEdit: noop,
  onDelete: noop,
  onCopy: noop,
  onShare: noop,
  onTest: noop,
  onSelect: noop,
  onToggleDisabled: noop,
  dragDisabled: true,
  gatewayStatus: null,
  gatewayTakeoverActive: false,
};

/**
 * The official channel with no role bindings — the one case the card has to
 * state the notes without a binding line to carry them. The bespoke card drew
 * notes on the second line in that state; the shared card must not drop them.
 */
const claudeDesktopOfficialProvider = {
  id: 'desktop-official',
  name: 'Claude Desktop Official',
  category: 'official',
  settingsConfig: JSON.stringify({ env: {} }),
  notes: '官方渠道备注',
  isApplied: false,
  isDisabled: false,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

/**
 * The shape that made the meta line wrap (user-reported 2026-10-21): a long
 * endpoint plus all four role bindings. The facts must compress; 「模型测试」
 * stays on the same row.
 */
const claudeDesktopLongProvider = {
  id: 'desktop-long',
  name: 'Desktop Long',
  category: 'custom',
  settingsConfig: JSON.stringify({
    env: {
      ANTHROPIC_BASE_URL: 'https://a-very-long-endpoint-name.cn-shanghai.fcapp.run/anthropic/v1/messages',
      ANTHROPIC_AUTH_TOKEN: 'sk-ant-long-fixture-key',
    },
  }),
  meta: {
    claudeDesktopModelRoutes: {
      'claude-sonnet-5': { model: 'claude-sonnet-4-5' },
      'claude-opus-5': { model: 'claude-opus-4-8' },
      'claude-fable-5': { model: 'claude-fable-5' },
      'claude-haiku-4-5': { model: 'claude-haiku-4-5-20251001' },
    },
  },
  isApplied: false,
  isDisabled: false,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

/**
 * A claude-safe provider whose role routes *map* claude ids onto each other
 * (`claude-sonnet-5` → `claude-opus-5`, the AnyRouter shape). Direct 3P mode
 * forbids model mapping, so this provider must read as "needs the gateway":
 * its only apply action is 「应用并代理」, never a bare 应用.
 */
const claudeDesktopMappedProvider = {
  id: 'desktop-mapped',
  name: 'Desktop Mapped',
  category: 'custom',
  settingsConfig: JSON.stringify({
    env: {
      ANTHROPIC_BASE_URL: 'https://mapped.example.test',
      ANTHROPIC_AUTH_TOKEN: 'sk-ant-mapped-fixture-key',
    },
  }),
  meta: {
    apiFormat: 'anthropic',
    claudeDesktopModelRoutes: {
      'claude-sonnet-5': { model: 'claude-opus-5', supports1m: false },
      'claude-opus-5': { model: 'claude-opus-5', supports1m: false },
    },
  },
  isApplied: false,
  isDisabled: false,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

/* ------------------------------------------------------------------ Gemini CLI */

const geminiOfficialProvider = {
  id: 'gemini-official',
  name: 'Gemini Official',
  category: 'official',
  settingsConfig: JSON.stringify({ env: {} }),
  isApplied: true,
  isDisabled: false,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

const geminiOfficialAccounts = [
  {
    id: '__local__',
    providerId: 'gemini-official',
    name: 'Local login',
    kind: 'local',
    email: 'local@example.test',
    isApplied: false,
    isVirtual: true,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  },
  {
    id: 'account-oauth',
    providerId: 'gemini-official',
    name: 'Work login',
    kind: 'oauth',
    email: 'work@example.test',
    projectId: 'proj-123',
    planType: 'Pro',
    limit5hText: '87%',
    isApplied: true,
    isVirtual: false,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  },
];

const geminiProps = {
  provider: geminiOfficialProvider,
  isApplied: true,
  onEdit: noop,
  onDelete: noop,
  onCopy: noop,
  onShare: noop,
  onSelect: noop,
  onToggleDisabled: noop,
  officialAccounts: geminiOfficialAccounts,
  onOfficialAccountLogin: noop,
  onOfficialLocalAccountSave: noop,
  onOfficialAccountApply: noop,
  onOfficialAccountDelete: noop,
  onOfficialAccountRefresh: noop,
  onOfficialAccountViewDetails: noop,
  gatewayStatus: null,
  gatewayTakeoverActive: false,
};

/* ------------------------------------------------------------------ Grok */

const grokOfficialProvider = {
  id: 'grok-official',
  name: 'Grok Official',
  category: 'official',
  settingsConfig: JSON.stringify({ auth: {}, config: {}, modelCatalog: { models: [] } }),
  // Notes are editable in official mode too, and the bespoke card drew them on
  // the second line next to the switching hint.
  notes: '官方渠道备注',
  isApplied: false,
  isDisabled: false,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

const grokOfficialAccounts = [
  {
    id: 'grok-account-oauth',
    providerId: 'grok-official',
    name: 'Grok login',
    kind: 'oauth',
    email: 'grok@example.test',
    planType: 'SuperGrok',
    limitWeeklyText: '89%',
    isApplied: true,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  },
];

const grokCustomProvider = {
  id: 'grok-custom',
  name: 'Grok Relay',
  category: 'custom',
  settingsConfig: JSON.stringify({
    auth: { API_KEY: 'sk-grok-fixture-key-0123456789' },
    // Channel-level endpoint: `extractGrokSettingsBaseUrl` reads this before
    // falling back to the model catalog / the `config` TOML text.
    baseUrl: 'https://grok.example.test/v1',
    config: { model: 'grok-4' },
    defaultModelKey: 'relay-model',
    modelCatalog: {
      models: [
        { key: 'relay-model', model: 'grok-4', displayName: 'Grok 4', reasoningEffort: 'high' },
        { key: 'relay-mini', model: 'grok-4-mini', displayName: 'Grok 4 Mini' },
      ],
    },
  }),
  isApplied: false,
  isDisabled: false,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

const grokCommonProps = {
  onEdit: noop,
  onDelete: noop,
  onCopy: noop,
  onShare: noop,
  onTest: noop,
  onSelect: noop,
  onToggleDisabled: noop,
  onAddModel: noop,
  onEditModel: noop,
  onDeleteModel: noop,
  onSetDefaultModel: noop,
  onFetchModels: noop,
  onToggleBatchDeleteMode: noop,
  onBatchDeleteModels: noop,
  onOfficialAccountLogin: noop,
  onOfficialLocalAccountSave: noop,
  onOfficialAccountApply: noop,
  onOfficialAccountDelete: noop,
  onOfficialAccountRefresh: noop,
  onOfficialAccountRefreshToken: noop,
  onOfficialAccountViewDetails: noop,
  gatewayStatus: null,
  gatewayTakeoverActive: false,
};

/* ------------------------------------------------------------------ OmO Native */

const omoProviders = [
  {
    key: 'omo-default',
    builtin: false,
    oauth: false,
    custom: true,
    hasKey: true,
    apiKey: 'sk-omo-default',
    config: {
      name: 'OmO Default',
      baseUrl: 'https://omo.example.test/v1',
      api: 'openai-completions',
      models: [{ id: 'model-default', name: 'Default Model' }],
    },
  },
  {
    key: 'omo-other',
    builtin: false,
    oauth: false,
    custom: true,
    hasKey: true,
    apiKey: 'sk-omo-other',
    config: {
      name: 'OmO Other',
      baseUrl: 'https://omo-other.example.test/v1',
      api: 'anthropic-messages',
      models: [{ id: 'model-other', name: 'Other Model' }],
    },
  },
];

const omoProps = {
  onEdit: noop,
  onCopy: noop,
  onDelete: noop,
  selectable: false,
  selected: false,
  onSelectChange: noop,
  onAddModel: noop,
  onEditModel: noop,
  onCopyModel: noop,
  onDeleteModel: noop,
  onReorderModels: noop,
  onTestModels: noop,
  onFetchModels: noop,
  onToggleModelSelection: noop,
  onToggleBatchDeleteMode: noop,
  onBatchDeleteModels: noop,
};

/* ------------------------------------------------------------------ OpenClaw */

const openClawProvider = {
  baseUrl: 'https://relay.openclaw.example.test/v1',
  apiKey: 'sk-openclaw-fixture-key',
  api: 'openai-completions',
  models: [
    { id: 'gpt-5', name: 'GPT-5', contextWindow: 400000, maxTokens: 128000 },
    { id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5' },
  ],
};

const openClawProps = {
  onEdit: noop,
  onCopy: noop,
  onShare: noop,
  onDelete: noop,
  onAddModel: noop,
  onEditModel: noop,
  onCopyModel: noop,
  onDeleteModel: noop,
  onReorderModels: noop,
  onTestModels: noop,
  testModelsDisabled: false,
  testModelsDisabledTooltip: '未配置可测试模型',
  onFetchModels: noop,
  fetchDisabled: false,
  fetchDisabledTooltip: '请先填写 Base URL 与 API Key',
  onSelectChange: noop,
  onToggleModelSelection: noop,
  onToggleBatchDeleteMode: noop,
  onBatchDeleteModels: noop,
};

/* ------------------------------------------------------------------ Hermes */

const hermesProvider = {
  providerKey: 'anthropic-direct',
  displayName: 'Anthropic Direct',
  apiMode: 'anthropic',
  provider: {
    id: 'anthropic-direct',
    api_mode: 'anthropic',
    base_url: 'https://api.anthropic.com',
    api_key: 'sk-ant-hermes-fixture-key',
    models: [
      { id: 'claude-opus-4-8', name: 'Claude Opus 4.8' },
      { id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5' },
    ],
  },
  modelIds: ['claude-opus-4-8', 'claude-sonnet-4-5'],
  isBuiltin: false,
  isReadOnly: false,
  isDefault: true,
};

/** A built-in `providers:` entry: no edit, no copy, no delete, no catalog. */
const hermesReadOnlyProvider = {
  providerKey: 'openrouter',
  displayName: 'OpenRouter Built-in',
  apiMode: 'openai',
  provider: { id: 'openrouter', base_url: 'https://openrouter.ai/api/v1' },
  modelIds: [],
  isBuiltin: true,
  isReadOnly: true,
  isDefault: false,
};

const hermesProps = {
  onEdit: noop,
  onCopy: noop,
  onShare: noop,
  onDelete: noop,
  onAddModel: noop,
  onEditModel: noop,
  onCopyModel: noop,
  onDeleteModel: noop,
  onSetPrimaryModel: noop,
  onTestModels: noop,
  testModelsDisabled: false,
  onFetchModels: noop,
  fetchDisabled: false,
  onSelectChange: noop,
  onToggleModelSelection: noop,
  onToggleBatchDeleteMode: noop,
  onBatchDeleteModels: noop,
};

/* ------------------------------------------------------------------ fixture API */

const cards = (name) => [...document.querySelectorAll(`[data-fixture="${name}"]`)];

window.migratedProviderCardsFixture = {
  /** The rendered text of one fixture block, whitespace-normalised. */
  text(name) {
    const node = cards(name)[0];
    if (!node) throw new Error('No fixture block: ' + name);
    return node.textContent.replace(/\s+/g, ' ').trim();
  },

  /** Left/right edges of the first element matching `selector` inside a block. */
  edges(name, selector) {
    const root = cards(name)[0];
    const node = root?.querySelector(selector);
    if (!node) return null;
    const rect = node.getBoundingClientRect();
    return { left: Math.round(rect.left), right: Math.round(rect.right), top: Math.round(rect.top), bottom: Math.round(rect.bottom) };
  },

  /**
   * Expands a collapsed account block (the list card's fold starts closed, as
   * it always has) by clicking its list title.
   */
  async expandAccounts(name) {
    const root = cards(name)[0];
    const titleButton = [...root.querySelectorAll('button')]
      .find(button => button.textContent.includes('账号列表'));
    if (!titleButton) return false;
    titleButton.click();
    await new Promise(resolve => setTimeout(resolve, 300));
    return true;
  },

  /** Whether a block renders a drag handle. */
  hasHandle(name) {
    return Boolean(cards(name)[0]?.querySelector('[data-provider-card-handle]'));
  },

  /**
   * Opens the list card's model fold.
   *
   * The rows exist only after the fold opens: `ModelListSection` keeps the body
   * lazily mounted, so reading a row's state before this clicks returns nothing
   * and would read as "the row says nothing".
   */
  async expandModels(name) {
    const root = cards(name)[0];
    // The fold's header is antd's `role="button"` header div, not a <button>.
    const header = [...root.querySelectorAll('[role="button"], button')]
      .find(node => node.textContent.includes('模型列表'));
    if (!header) return false;
    header.click();
    await new Promise(resolve => setTimeout(resolve, 300));
    return true;
  },

  /** The second line of a compact cell, as written. */
  subtitle(name) {
    const node = cards(name)[0]?.querySelector('[data-provider-subtitle]');
    return node ? node.textContent.replace(/\s+/g, ' ').trim() : null;
  },

  /**
   * Opens the compact card's "more" menu and reports the delete entry's state.
   * Returns `null` when the block has no menu button.
   */
  async deleteMenuItem(name) {
    const root = cards(name)[0];
    const button = root?.querySelector('[data-provider-card-actions] button');
    if (!button) return null;
    // The hover-revealed trigger is opacity 0 until the pointer enters, and a
    // synthetic click on it is dispatched but the dropdown may not open while
    // the button is considered hidden. Reveal it for the duration of the check.
    const host = button.closest('[data-provider-card-actions]');
    const previousOpacity = host?.style.opacity ?? '';
    if (host) host.style.opacity = '1';
    // Snapshot before the click: an earlier card's menu stays mounted after it
    // closes, and both cards carry a 删除 entry (only one disabled), so a query
    // by visibility alone can answer with the wrong menu. The node that appears
    // is the one this click opened.
    const mountedBefore = new Set(document.querySelectorAll('.ant-dropdown'));
    button.click();
    await new Promise(resolve => setTimeout(resolve, 300));
    const menu = [...document.querySelectorAll('.ant-dropdown')]
      .find(node => !mountedBefore.has(node));
    const items = [...(menu?.querySelectorAll('[role="menuitem"]') ?? [])];
    const deleteItem = items.find(item => item.textContent.includes('删除'));
    const result = deleteItem
      ? {
          found: true,
          disabled: deleteItem.getAttribute('aria-disabled') === 'true'
            || deleteItem.classList.contains('ant-dropdown-menu-item-disabled'),
          title: deleteItem.getAttribute('title'),
        }
      : { found: false };
    document.body.click();
    if (host) host.style.opacity = previousOpacity;
    await new Promise(resolve => setTimeout(resolve, 200));
    return result;
  },
};

createRoot(document.getElementById('root')).render(
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <MemoryRouter>
      <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div data-fixture="claude-desktop-list">
          <ClaudeDesktopProviderCard {...claudeDesktopProps} viewMode="list" />
        </div>
        <div data-fixture="claude-desktop-official-list">
          <ClaudeDesktopProviderCard
            {...claudeDesktopProps}
            provider={claudeDesktopOfficialProvider}
            isOfficialChannel
            isApplied={false}
            viewMode="list"
          />
        </div>
        <div data-fixture="claude-desktop-long-list">
          <ClaudeDesktopProviderCard
            {...claudeDesktopProps}
            provider={claudeDesktopLongProvider}
            isApplied={false}
            viewMode="list"
          />
        </div>
        <div data-fixture="claude-desktop-mapped-list">
          <ClaudeDesktopProviderCard
            {...claudeDesktopProps}
            provider={claudeDesktopMappedProvider}
            isApplied={false}
            viewMode="list"
          />
        </div>
        <div data-fixture="claude-desktop-card">
          <ClaudeDesktopProviderCard {...claudeDesktopProps} viewMode="card" />
        </div>
        <div data-fixture="claude-desktop-official-card">
          <ClaudeDesktopProviderCard
            {...claudeDesktopProps}
            provider={claudeDesktopOfficialProvider}
            isOfficialChannel
            isApplied={false}
            viewMode="card"
          />
        </div>

        <div data-fixture="gemini-list">
          <GeminiCliProviderCard {...geminiProps} viewMode="list" />
        </div>
        <div data-fixture="gemini-card">
          <GeminiCliProviderCard {...geminiProps} viewMode="card" />
        </div>

        <div data-fixture="grok-official-list">
          <GrokProviderCard
            {...grokCommonProps}
            provider={grokOfficialProvider}
            isApplied={false}
            officialAccounts={grokOfficialAccounts}
            viewMode="list"
          />
        </div>
        <div data-fixture="grok-custom-list">
          <GrokProviderCard
            {...grokCommonProps}
            provider={grokCustomProvider}
            isApplied={false}
            viewMode="list"
          />
        </div>
        {/* The one official channel that never signed in: its apply action
            must stay visible but locked, and explain itself on hover. */}
        <div data-fixture="grok-official-empty-list">
          <GrokProviderCard
            {...grokCommonProps}
            provider={grokOfficialProvider}
            isApplied={false}
            officialAccounts={[]}
            viewMode="list"
          />
        </div>

        <div data-fixture="omo-default-card">
          <OmoNativeProviderCard
            {...omoProps}
            provider={omoProviders[0]}
            viewMode="card"
            defaultModelId="model-default"
            deleteDisabledReason="默认渠道不能删除"
          />
        </div>
        <div data-fixture="omo-other-card">
          <OmoNativeProviderCard
            {...omoProps}
            provider={omoProviders[1]}
            viewMode="card"
          />
        </div>

        <div data-fixture="openclaw-list">
          <OpenClawProviderCard
            {...openClawProps}
            providerId="relay-main"
            config={openClawProvider}
            viewMode="list"
          />
        </div>
        <div data-fixture="openclaw-card">
          <OpenClawProviderCard
            {...openClawProps}
            providerId="relay-main"
            config={openClawProvider}
            viewMode="card"
          />
        </div>
        {/* The default provider: delete stays visible but explains itself. */}
        <div data-fixture="openclaw-default-list">
          <OpenClawProviderCard
            {...openClawProps}
            providerId="relay-main"
            config={openClawProvider}
            viewMode="list"
            deleteDisabledReason="该渠道已设为默认，不可删除"
          />
        </div>

        <div data-fixture="hermes-list">
          <HermesProviderCard
            {...hermesProps}
            provider={hermesProvider}
            defaultModelId="claude-opus-4-8"
            viewMode="list"
          />
        </div>
        <div data-fixture="hermes-card">
          <HermesProviderCard
            {...hermesProps}
            provider={hermesProvider}
            defaultModelId="claude-opus-4-8"
            viewMode="card"
          />
        </div>
        {/* The page owns the read-only gating: it passes no callbacks at all,
            which is what the card must render as "no actions". */}
        <div data-fixture="hermes-readonly-list">
          <HermesProviderCard
            provider={hermesReadOnlyProvider}
            viewMode="list"
          />
        </div>
        {/* Selection mode: an eligible row gets a live checkbox, an ineligible
            one a disabled checkbox that explains itself on hover. */}
        <div data-fixture="hermes-selection-list">
          <HermesProviderCard
            {...hermesProps}
            provider={hermesProvider}
            defaultModelId="claude-opus-4-8"
            viewMode="list"
            selectable
          />
        </div>
        <div data-fixture="hermes-selection-disabled-list">
          <HermesProviderCard
            {...hermesProps}
            provider={hermesProvider}
            defaultModelId="claude-opus-4-8"
            viewMode="list"
            selectDisabledReason="该渠道已设为默认，不可多选"
          />
        </div>
        <div data-fixture="hermes-readonly-selection-list">
          <HermesProviderCard
            provider={hermesReadOnlyProvider}
            viewMode="list"
            selectDisabledReason="引擎内置渠道，不可编辑"
          />
        </div>
      </div>
    </MemoryRouter>
  </ConfigProvider>,
);
