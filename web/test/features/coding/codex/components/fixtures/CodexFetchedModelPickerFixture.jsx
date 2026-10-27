import React from 'react';
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import CodexProviderFormModal from '@/features/coding/codex/components/CodexProviderFormModal';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const dark = parameters.get('theme') === 'dark';
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = dark ? 'dark' : 'light';
const models = Array.from({ length: 8 }, (_, index) => ({
  id: `synthetic-model-${index}`,
  name: `Synthetic Model ${index}`,
}));
const state = { runId: parameters.get('runId'), requests: [], saved: null, models, pending: [] };
let deferNext = false;
let rejectNext = false;
// Strict command allowlist: this fixture never reaches a native bridge or a
// provider. Even "official" model discovery is an in-memory synthetic response.
window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    state.requests.push({ command, args });
    if (command === 'read_opencode_config') return { status: 'success', config: { provider: {} } };
    if (command === 'proxy_gateway_supported_cli_keys') return ['codex'];
    if (command === 'fetch_codex_official_models') {
      if (rejectNext) { rejectNext = false; throw new Error('Synthetic catalog failure'); }
      if (deferNext) {
        deferNext = false;
        return new Promise(resolve => state.pending.push(resolve));
      }
      return { models: structuredClone(state.models), source: 'remote' };
    }
    throw new Error('Unexpected fixture command: ' + command);
  },
};
const wait = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const visible = element => element.getBoundingClientRect().height > 0 && getComputedStyle(element).visibility !== 'hidden';
const visibleElements = selector => [...document.querySelectorAll(selector)].filter(visible);
const createProvider = id => ({
  id, name: 'Synthetic official provider', category: 'official',
  settingsConfig: JSON.stringify({ config: 'model = "gpt-6.1-sol"\nmodel_reasoning_effort = "high"' }),
});
const fixture = window.modelPickerFixture = {
  state,
  t: key => i18n.t(key),
  pickerInput: () => document.querySelector('[class*="modelPicker"] input'),
  modelInput: () => document.querySelector('[class*="modelInput"] input'),
  toml: () => document.querySelector('textarea[aria-label="Fixture TOML editor"]').value,
  options: () => visibleElements('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option').map(element => element.textContent),
  async click(key) {
    const button = visibleElements('button').find(element => element.textContent.replace(/\s/g, '') === String(i18n.t(key)).replace(/\s/g, ''));
    if (!button || button.disabled) throw new Error('Button unavailable: ' + key);
    button.click(); await wait(220);
  },
  async search(value) {
    const input = fixture.pickerInput();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await wait(120);
  },
  deferNext: () => { deferNext = true; },
  rejectNext: () => { rejectNext = true; },
  async resolveNext(ids) {
    const resolve = state.pending.shift();
    if (!resolve) throw new Error('No deferred catalog request');
    resolve({ models: ids.map(id => ({ id, name: id })), source: 'remote' });
    await wait(180);
  },
};
function FixtureApp() {
  const [open, setOpen] = React.useState(true);
  const [provider, setProvider] = React.useState(() => createProvider('synthetic-a'));
  fixture.reopen = async () => { setOpen(true); await wait(250); };
  fixture.switchProvider = async () => { setProvider(createProvider('synthetic-b')); await wait(250); };
  return (
    <ConfigProvider modal={{ centered: true }} theme={{ algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
      <App>
        <CodexProviderFormModal open={open} provider={provider} onCancel={() => setOpen(false)}
          onSubmit={async values => { state.saved = structuredClone(values); }} />
      </App>
    </ConfigProvider>
  );
}
createRoot(document.getElementById('root')).render(<FixtureApp />);
