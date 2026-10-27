/**
 * Mounts the **real** shared `FetchModelsModal` — the dialog the Codex provider
 * card's 「获取模型」 action opens — against a stubbed Tauri IPC layer.
 *
 * Why this fixture exists: the catalog moved out of the provider form and onto
 * the card (2026-09-19), and the old form-level import fixture lost the button
 * it drove. The behaviours the old suite guarded did not disappear with it —
 * they moved here, because every one of them lives inside `FetchModelsModal`
 * itself: which `sdkType`/`apiType`/`customUrl` reaches the backend, that a
 * closed dialog cannot apply a late response, that a failed refresh keeps the
 * previous rows, and that only an explicit cleanup opt-in removes models.
 *
 * The component is mounted directly rather than through `CodexPage`: the page
 * adds the card, the sidebar and the mapping layer, none of which this dialog's
 * contract depends on. The card's own toolbar is asserted in
 * `codexCardViewBrowserChecks.mjs`; what it passes here is asserted below.
 */
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const dark = parameters.get('theme') === 'dark';
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = dark ? 'dark' : 'light';

/** The provider shape the page derives before opening the dialog. */
const providers = {
  responses: {
    providerId: 'provider-responses',
    providerName: 'Responses Upstream',
    baseUrl: 'https://responses.example.test/v1',
    apiKey: 'sk-responses-fixture',
    sdkType: '@ai-sdk/openai',
  },
  anthropic: {
    providerId: 'provider-anthropic',
    providerName: 'Anthropic Upstream',
    baseUrl: 'https://anthropic.example.test/v1',
    apiKey: 'sk-anthropic-fixture',
    sdkType: '@ai-sdk/anthropic',
  },
  google: {
    providerId: 'provider-google',
    providerName: 'Google Upstream',
    baseUrl: 'https://google.example.test/v1',
    apiKey: 'sk-google+fixture',
    sdkType: '@ai-sdk/google',
  },
  // Same protocol, but the stored base URL carries no version segment, so the
  // dialog has to add one.
  'google-unversioned': {
    providerId: 'provider-google-unversioned',
    providerName: 'Google Upstream Unversioned',
    baseUrl: 'https://google-unversioned.example.test',
    apiKey: 'sk-google-fixture',
    sdkType: '@ai-sdk/google',
  },
};

const state = {
  runId: parameters.get('runId'),
  scenario: parameters.get('scenario') || 'responses',
  requests: [],
  applied: null,
  /** Assigned by `FixtureApp` so the fixture can close/reopen the real state. */
  setOpen: () => {},
  models: [],
};

let pendingFetch = null;
let deferNextFetch = false;
let nextFetchError = null;
/** When set, a deferred request rejects with this message once released. */
let pendingFetchError = null;

// Strict allowlist: the fixture never reaches a native bridge or a provider.
// `fetch_provider_models` is answered from memory.
window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    state.requests.push({ command, args: structuredClone(args) });
    if (command !== 'fetch_provider_models') {
      throw new Error('Unexpected fixture command: ' + command);
    }
    if (nextFetchError) {
      const message = nextFetchError;
      nextFetchError = null;
      throw new Error(message);
    }
    if (deferNextFetch) {
      deferNextFetch = false;
      return new Promise((resolve, reject) => {
        pendingFetch = (value) => {
          pendingFetch = null;
          if (pendingFetchError) {
            const message = pendingFetchError;
            pendingFetchError = null;
            reject(new Error(message));
            return;
          }
          resolve(value);
        };
      });
    }
    return { models: structuredClone(state.models), total: state.models.length };
  },
};

const wait = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const visible = (element) => element.getBoundingClientRect().height > 0
  && getComputedStyle(element).visibility !== 'hidden';
const visibleElements = (selector) => [...document.querySelectorAll(selector)].filter(visible);
const lastRequest = () => state.requests.filter((request) => request.command === 'fetch_provider_models').at(-1);

const fixture = window.fetchedModelsModalFixture = {
  state,
  t: (key, options) => i18n.t(key, options),
  visibleModalCount: () => visibleElements('.ant-modal').length,
  rows: () => visibleElements('tr[data-row-key]').map((row) => row.dataset.rowKey),
  rowDisabled: (modelId) => visibleElements('tr[data-row-key]')
    .find((row) => row.dataset.rowKey === modelId)
    ?.querySelector('input[type=checkbox]')?.disabled,
  rowHasExistingTag: (modelId) => {
    const row = visibleElements('tr[data-row-key]').find((item) => item.dataset.rowKey === modelId);
    return Boolean(row && row.textContent.includes(i18n.t('opencode.fetchModels.alreadyExists')));
  },
  bodyText: () => document.body.textContent,
  lastRequest,
  errorVisible: () => Boolean(document.querySelector('.ant-alert-error')),
  async clickButton(key, options) {
    const label = String(i18n.t(key, options)).replace(/\s/g, '');
    const button = visibleElements('button')
      .find((element) => element.textContent.replace(/\s/g, '') === label);
    if (!button || button.disabled) {
      throw new Error('Button unavailable: ' + key);
    }
    button.click();
    await wait(220);
  },
  confirmButtonDisabled: () => {
    const footer = visibleElements('.ant-modal-footer').at(-1);
    return footer?.querySelector('button.ant-btn-primary')?.disabled ?? null;
  },
  // The fetch/refresh control carries `loading` from the component, and antd
  // renders that as `.ant-btn-loading`. Apply is not a proxy for it: Apply stays
  // disabled whenever nothing is selected, regardless of an in-flight request.
  fetchButtonLoading: () => visibleElements('.ant-modal-body button.ant-btn-loading').length > 0,
  async fetchModels() {
    await fixture.clickButton('opencode.fetchModels.fetch');
  },
  async refreshModels() {
    await fixture.clickButton('opencode.fetchModels.refresh');
  },
  async select(modelId) {
    const checkbox = visibleElements('tr[data-row-key]')
      .find((row) => row.dataset.rowKey === modelId)
      ?.querySelector('input[type=checkbox]');
    if (!checkbox || checkbox.disabled) {
      throw new Error('Selectable model was not found: ' + modelId);
    }
    checkbox.click();
    await wait(100);
  },
  async search(value) {
    const input = visibleElements('input')
      .find((element) => element.placeholder === i18n.t('opencode.fetchModels.searchPlaceholder'));
    if (!input) {
      throw new Error('Search input was not found');
    }
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await wait(120);
  },
  async setApiType(value) {
    const radio = visibleElements('input[type=radio]').find((input) => input.value === value);
    if (!radio) {
      throw new Error('API type radio was not found: ' + value);
    }
    radio.click();
    await wait(150);
  },
  apiTypeOptionCount: () => visibleElements('input[type=radio]').length,
  urlValue: () => visibleElements('.ant-input-group-wrapper input').at(0)?.value ?? null,
  // The cleanup opt-in is the one checkbox the dialog renders outside its table.
  cleanupCheckbox: () => visibleElements('.ant-modal-body .ant-checkbox-input')
    .find((input) => !input.closest('.ant-table')),
  async toggleCleanup() {
    const checkbox = fixture.cleanupCheckbox();
    if (!checkbox) {
      throw new Error('Cleanup checkbox was not found');
    }
    checkbox.click();
    await wait(120);
  },
  async apply() {
    const button = visibleElements('.ant-modal-footer').at(-1)?.querySelector('button.ant-btn-primary');
    if (!button || button.disabled) {
      throw new Error('Apply is disabled');
    }
    button.click();
    await wait(200);
  },
  async cancel() {
    const button = [...visibleElements('.ant-modal button')]
      .find((element) => element.textContent.replace(/\s/g, '') === String(i18n.t('common.cancel')).replace(/\s/g, ''));
    if (!button) {
      throw new Error('Cancel button was not found');
    }
    button.click();
    await wait(250);
  },
  setModels: (models) => { state.models = models; },
  deferNextFetch: () => { deferNextFetch = true; },
  failNextFetch: (message) => { nextFetchError = message; },
  async resolvePending(models) {
    if (!pendingFetch) {
      throw new Error('No deferred fetch request');
    }
    pendingFetch({ models, total: models.length });
    await wait(200);
  },
  /** Release a deferred request as a failure, so the in-flight state can be
   *  observed before the error lands. */
  async failPending(message) {
    if (!pendingFetch) {
      throw new Error('No deferred fetch request');
    }
    pendingFetchError = message;
    pendingFetch(null);
    await wait(250);
  },
  // Close and reopen through the real component state so the dialog's own
  // reset effect runs, exactly as it does when the user reopens it. Clearing
  // `applied` keeps a later `waitFor(applied)` from matching the previous run.
  async reopen() {
    state.setOpen(false);
    await wait(250);
    state.applied = null;
    state.setOpen(true);
    await wait(300);
  },
};

function FixtureApp() {
  const [open, setOpen] = React.useState(true);
  const provider = providers[state.scenario] ?? providers.responses;
  state.setOpen = setOpen;
  return (
    <ConfigProvider
      modal={{ centered: true }}
      theme={{ algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm }}
    >
      <App>
        <FetchModelsModal
          open={open}
          providerId={provider.providerId}
          providerName={provider.providerName}
          baseUrl={provider.baseUrl}
          apiKey={provider.apiKey}
          sdkType={provider.sdkType}
          existingModelIds={state.scenario === 'existing' ? ['model-kept'] : []}
          priorityOwnedBy={['openai']}
          onCancel={() => setOpen(false)}
          onSuccess={(result) => {
            state.applied = result;
            setOpen(false);
          }}
        />
      </App>
    </ConfigProvider>
  );
}

createRoot(document.getElementById('root')).render(<FixtureApp />);
