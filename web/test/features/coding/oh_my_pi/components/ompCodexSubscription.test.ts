import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

import { getOmpModelThinkingLevelOptions, getProviderModelRecords } from '../../../../../utils/ompModelMetadata.ts';
import { isOmpCodexSubscriptionProvider, OMP_CODEX_PROVIDER_KEY } from '../../../../../features/coding/oh_my_pi/utils/ompCodexSubscription.ts';
import type { OmpCodexSubscription, OmpRuntimeProviderView } from '../../../../../types/ohMyPi.ts';

const moduleRoot = new URL('../../../../../features/coding/oh_my_pi/', import.meta.url);
const subscription: OmpCodexSubscription = {
  status: 'not_configured', hasApiKeyOverride: false, shell: 'posix',
  loginCommand: "PI_CODING_AGENT_DIR='/tmp/omp agent' omp --profile default login openai-codex",
  modelsCommand: "PI_CODING_AGENT_DIR='/tmp/omp agent' omp --profile default models --json --no-extensions",
};

// Run the production handlers/components, replacing only React hooks and external IO.
function declarations(file: string, names: string[]): string {
  const url = new URL(file, moduleRoot);
  const source = ts.createSourceFile(url.pathname, readFileSync(url, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const found = new Map<string, string>();
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && names.includes(node.name.text)) {
      found.set(node.name.text, `const ${node.name.text} = ${node.initializer!.getText(source)};`);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.equal(found.size, names.length);
  return names.map(name => found.get(name)).join('\n');
}

function evaluate(source: string, context: Record<string, unknown>) {
  return vm.runInNewContext(ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText, Object.assign(context, { queueMicrotask }));
}

type Element = { type: string; props: Record<string, any> };
const t = (key: string, values?: Record<string, unknown>) => values ? `${key}:${JSON.stringify(values)}` : key;
function elements(tree: any, type: string): Element[] {
  if (!tree || typeof tree !== 'object') return [];
  if (Array.isArray(tree)) return tree.flatMap(child => elements(child, type));
  return [...(tree.type === type ? [tree] : []), ...elements(tree.props?.children, type), ...elements(tree.props?.footer, type)];
}
function button(tree: Element, label: string) {
  const result = elements(tree, 'Button').find(node => node.props.children.includes(label));
  assert.ok(result, `Missing button: ${label}`);
  return result;
}

function componentHarness(file: string, name: string, extra: Record<string, unknown> = {}) {
  const hooks: any[] = [];
  let index = 0;
  let effects: (() => void)[] = [];
  const messages: string[] = [];
  const copies: string[] = [];
  const React = {
    createElement: (type: string, props: Record<string, unknown> | null, ...children: unknown[]) => ({ type, props: { ...props, children } }),
    useState: (initial: unknown) => {
      const slot = index++;
      if (!(slot in hooks)) hooks[slot] = initial;
      return [hooks[slot], (value: unknown) => { hooks[slot] = value; }];
    },
    useRef: (initial: unknown) => {
      const slot = index++;
      if (!(slot in hooks)) hooks[slot] = { current: initial };
      return hooks[slot];
    },
    useEffect: (effect: () => void, dependencies: unknown[]) => {
      const slot = index++;
      if (!hooks[slot] || dependencies.some((value, i) => hooks[slot][i] !== value)) {
        effects.push(effect);
        hooks[slot] = dependencies;
      }
    },
  };
  const component = evaluate(`${declarations(file, name === 'OmpCodexSubscriptionSection' ? ['STATUS_KEYS', name] : [name])}\n${name};`, {
    React, useTranslation: () => ({ t }),
    App: { useApp: () => ({ message: { success: (value: string) => messages.push(value), error: (value: string) => messages.push(value) } }) },
    Typography: { Text: 'Text', Paragraph: 'Paragraph' },
    Button: 'Button', Modal: 'Modal', Tag: 'Tag', Alert: 'Alert', Copy: 'Copy', Terminal: 'Terminal', RefreshCw: 'RefreshCw',
    ImeSafeAutoComplete: 'ImeSafeAutoComplete',
    styles: new Proxy({}, { get: (_target, key) => key }),
    copyTextToClipboard: async (value: string) => { copies.push(value); },
    ...extra,
  });
  return {
    copies, messages,
    render: (props: Record<string, unknown>): Element => {
      index = 0;
      effects = [];
      const result = component(props);
      effects.forEach(effect => effect());
      return result;
    },
  };
}

function pageHarness() {
  const fields: Record<string, any> = { defaultProvider: 'openai', defaultModel: 'api-model', defaultThinkingLevel: 'high' };
  const providers: OmpRuntimeProviderView[] = [
    { providerKey: 'openai', displayName: 'OpenAI API', sources: ['models_yml'], categories: ['api_key'], credentialKind: 'api_key', runtimeFiles: [], isBuiltin: true, isOverride: true, isDefault: true, modelIds: ['api-model'] },
    { providerKey: OMP_CODEX_PROVIDER_KEY, displayName: 'OpenAI Codex', sources: ['official_builtin'], categories: ['subscription'], credentialKind: 'oauth', runtimeFiles: [], isBuiltin: true, isOverride: false, isDefault: false, modelIds: [] },
  ];
  const persisted: Record<string, any> = { providers, modelSettings: { providerKey: 'openai', modelId: 'api-model', thinkingLevel: 'high' }, codexSubscription: { ...subscription }, otherSettings: {}, modelsContent: 'providers:\n  openai:\n    apiKey: KEEP' };
  const saves: Record<string, unknown>[] = [];
  let reads = 0;
  let failRead = false;
  let trayRefreshes = 0;
  const noop = () => {};
  const context: Record<string, any> = {
    runtimeConfig: structuredClone(persisted), modelSettingsSaveSeqRef: { current: 0 },
    React: { useCallback: (handler: unknown) => handler, useMemo: (factory: () => unknown) => factory() },
    getOmpModelThinkingLevelOptions, getProviderModelRecords, isOmpCodexSubscriptionProvider, OMP_CODEX_PROVIDER_KEY,
    modelForm: { setFieldsValue: (values: Record<string, unknown>) => Object.assign(fields, values), setFieldValue: (key: string, value: unknown) => { fields[key] = value; } },
    setSaving: noop, setLoading: noop, setOtherSettings: noop,
    setRuntimeConfig: (next: any) => { context.runtimeConfig = typeof next === 'function' ? next(context.runtimeConfig) : next; },
    saveOmpModelSettings: async (input: Record<string, unknown>) => {
      saves.push(structuredClone(input));
      persisted.modelSettings = { providerKey: input.defaultProvider || persisted.modelSettings.providerKey, modelId: input.defaultModel || persisted.modelSettings.modelId, thinkingLevel: persisted.modelSettings.thinkingLevel };
      return structuredClone(persisted);
    },
    readOmpRuntimeConfig: async () => { reads++; if (failRead) throw new Error('unreadable'); return structuredClone(persisted); },
    refreshTrayMenu: async () => { trayRefreshes++; },
    message: { error: noop }, console: { error: noop }, t,
  };
  const handlers = evaluate(`${declarations('pages/OhMyPiPage.tsx', ['isOmpThinkingLevelSupported', 'loadConfig', 'handleModelSettingsChange'])}\n({ loadConfig, handleModelSettingsChange });`, context);
  return {
    fields, handlers, persisted, saves, config: () => context.runtimeConfig, reads: () => reads, trayRefreshes: () => trayRefreshes,
    failRead: (value: boolean) => { failRead = value; },
    apiKeyProviders: () => evaluate(`(() => { ${declarations('pages/OhMyPiPage.tsx', ['ompProviders'])}\nreturn ompProviders; })();`, context),
  };
}

test('opening and cancelling native login never executes commands or changes configuration', async () => {
  const page = pageHarness();
  const view = componentHarness('components/OmpCodexSubscriptionSection.tsx', 'OmpCodexSubscriptionSection');
  const props = { subscription, onRefresh: () => page.handlers.loadConfig(true) };
  let tree = view.render(props);
  button(tree, 'ohMyPi.codexSubscription.loginGuide').props.onClick();
  tree = view.render(props);
  const modal = elements(tree, 'Modal')[0];
  assert.equal(modal.props.open, true);
  assert.equal(modal.props.style.maxWidth, '100%');
  assert.equal(modal.props.style.marginBlock, 0);
  assert.ok(elements(tree, 'code').some(node => node.props.children.includes(subscription.loginCommand)));
  modal.props.onCancel();
  assert.equal(elements(view.render(props), 'Modal')[0].props.open, false);
  assert.equal(page.reads(), 0);
  assert.equal(page.saves.length, 0);
  assert.equal(view.copies.length, 0);
});

test('repeated refresh is single-flight and re-reads native status without writes', async () => {
  const page = pageHarness();
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  const view = componentHarness('components/OmpCodexSubscriptionSection.tsx', 'OmpCodexSubscriptionSection');
  let refreshes = 0;
  const props = { subscription, onRefresh: async () => { refreshes++; await pending; await page.handlers.loadConfig(true); } };
  const tree = view.render(props);
  const first = button(tree, 'common.refresh').props.onClick();
  const second = button(tree, 'common.refresh').props.onClick();
  assert.equal(refreshes, 1);
  assert.equal(button(view.render(props), 'common.refresh').props.loading, true);
  page.persisted.codexSubscription.status = 'configured';
  release();
  await Promise.all([first, second]);
  assert.equal(page.config().codexSubscription.status, 'configured');
  assert.equal(page.reads(), 1);
  assert.equal(page.saves.length, 0);
  assert.equal(button(view.render(props), 'common.refresh').props.loading, false);
  await button(view.render(props), 'common.refresh').props.onClick();
  assert.equal(page.reads(), 2);
});

test('unknown status, override warning and WSL guidance remain explicit and read-only', async () => {
  const page = pageHarness();
  page.persisted.codexSubscription.status = 'configured';
  await page.handlers.loadConfig(true);
  page.failRead(true);
  await page.handlers.loadConfig(true);
  assert.equal(page.config().codexSubscription.status, 'unknown');
  const view = componentHarness('components/OmpCodexSubscriptionSection.tsx', 'OmpCodexSubscriptionSection');
  const tree = view.render({ subscription: { ...page.config().codexSubscription, hasApiKeyOverride: true, wslDistro: 'Ubuntu' }, onRefresh: async () => {} });
  assert.ok(elements(tree, 'Tag').every(tag => tag.props.children.includes('ohMyPi.codexSubscription.unknown')));
  assert.ok(elements(tree, 'Alert').some(alert => alert.props.title === 'ohMyPi.codexSubscription.apiKeyOverride'));
  assert.ok(elements(tree, 'Text').some(text => String(text.props.children).includes('ohMyPi.codexSubscription.wslShell')));
  assert.equal(page.saves.length, 0);
  assert.equal(page.apiKeyProviders().length, 1);
  assert.equal(page.apiKeyProviders()[0].providerKey, 'openai');
});

test('copy preserves backend commands exactly and unavailable commands cannot be copied', async () => {
  const view = componentHarness('components/OmpCodexSubscriptionSection.tsx', 'OmpCodexSubscriptionSection');
  const tree = view.render({ subscription, onRefresh: async () => {} });
  const copies = elements(tree, 'Button').filter(node => node.props.children.includes('common.copy'));
  for (const copy of copies) await copy.props.onClick();
  assert.deepEqual(view.copies, [subscription.loginCommand, subscription.modelsCommand]);
  const unavailable = view.render({ subscription: { ...subscription, loginCommand: '', modelsCommand: '' }, onRefresh: async () => {} });
  for (const copy of elements(unavailable, 'Button').filter(node => node.props.children.includes('common.copy'))) {
    assert.equal(copy.props.disabled, true);
    await copy.props.onClick();
  }
  assert.equal(view.copies.length, 2);
});

test('empty Codex catalog accepts an exact typed ID and saves only after commit, then reloads', async () => {
  const page = pageHarness();
  page.fields.defaultProvider = OMP_CODEX_PROVIDER_KEY;
  await page.handlers.handleModelSettingsChange({ defaultProvider: OMP_CODEX_PROVIDER_KEY }, page.fields);
  assert.equal(page.fields.defaultModel, undefined);
  assert.equal(page.saves.length, 0, 'A provider-only change must not reuse the old API model');
  const beforeModels = page.persisted.modelsContent;
  const input = componentHarness('components/OmpCodexModelInput.tsx', 'OmpCodexModelInput');
  const pending: Promise<void>[] = [];
  const props = { options: [], value: undefined, placeholder: 'model', onChange: (value: string) => {
    page.fields.defaultModel = value;
    pending.push(page.handlers.handleModelSettingsChange({ defaultModel: value }, page.fields));
  } };
  let tree = input.render(props);
  tree.props.onChange('Custom-Codex-2026:exact');
  assert.equal(page.saves.length, 0);
  tree = input.render(props);
  tree.props.onBlur();
  await Promise.all(pending);
  assert.equal(page.saves.length, 1);
  assert.equal(page.saves[0].defaultProvider, OMP_CODEX_PROVIDER_KEY);
  assert.equal(page.saves[0].defaultModel, 'Custom-Codex-2026:exact');
  assert.equal(page.saves[0].clearThinkingLevel, false);
  assert.equal(page.persisted.modelsContent, beforeModels);
  await page.handlers.loadConfig(true);
  assert.equal(page.fields.defaultModel, 'Custom-Codex-2026:exact');
  assert.equal(page.fields.defaultThinkingLevel, 'high');
  assert.equal(page.trayRefreshes(), 1);
});

test('selection and Enter/blur commit once, including a highlighted option and IME', async () => {
  const input = componentHarness('components/OmpCodexModelInput.tsx', 'OmpCodexModelInput');
  const changes: string[] = [];
  const props = { value: '', options: [{ value: 'native-model', label: 'native-model' }], placeholder: 'model', onChange: (value: string) => changes.push(value) };
  let tree = input.render(props);
  tree.props.onChange('native');
  tree.props.onInputKeyDown({ key: 'Enter', keyCode: 13, nativeEvent: { isComposing: false } });
  tree.props.onSelect('native-model');
  await new Promise<void>(resolve => queueMicrotask(resolve));
  tree = input.render(props);
  tree.props.onBlur();
  assert.deepEqual(changes, ['native-model']);
  tree.props.onChange('manual-model');
  tree.props.onInputKeyDown({ key: 'Enter', keyCode: 229, nativeEvent: { isComposing: true } });
  await new Promise<void>(resolve => queueMicrotask(resolve));
  assert.equal(changes.length, 1);
  tree.props.onInputKeyDown({ key: 'Enter', keyCode: 13, nativeEvent: { isComposing: false } });
  await new Promise<void>(resolve => queueMicrotask(resolve));
  assert.deepEqual(changes, ['native-model', 'manual-model']);
});

test('existing Codex API-key overrides keep their ordinary management entry', async () => {
  const page = pageHarness();
  page.persisted.providers[1].modelsProvider = { api: 'openai-codex-responses', baseUrl: 'https://relay.example', apiKey: 'USER_OVERRIDE' };
  page.persisted.providers[1].credentialKind = 'api_key';
  await page.handlers.loadConfig(true);
  const visible = page.apiKeyProviders();
  assert.equal(visible.length, 2);
  assert.equal(visible[1].modelsProvider.apiKey, 'USER_OVERRIDE');
  assert.equal(page.saves.length, 0);
});

test('cached Codex IDs preserve global thinking when no capability metadata exists', async () => {
  const page = pageHarness();
  page.persisted.providers[1].modelIds = ['cached-native-model'];
  await page.handlers.loadConfig(true);
  Object.assign(page.fields, { defaultProvider: OMP_CODEX_PROVIDER_KEY, defaultModel: 'cached-native-model' });
  await page.handlers.handleModelSettingsChange({ defaultProvider: OMP_CODEX_PROVIDER_KEY }, page.fields);
  assert.equal(page.saves.length, 1);
  assert.equal(page.saves[0].clearThinkingLevel, false);
  assert.equal(page.saves[0].defaultThinkingLevel, 'high');
  assert.equal(page.saves[0].defaultModel, 'cached-native-model');
});
