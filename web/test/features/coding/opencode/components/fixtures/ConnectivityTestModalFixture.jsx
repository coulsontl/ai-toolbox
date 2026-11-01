/**
 * Mounts the shared connectivity test modal directly, with no page and no IPC,
 * so its collapsed-config layout can be measured in a browser.
 *
 * Why a component fixture: the modal takes every input as a prop and reaches
 * nothing outside React, so a page fixture would add dozens of stub commands
 * to answer a question the component already owns.
 *
 * What it guards: the config section must be collapsed by default, so the
 * model list (the thing the user opened the modal to see) is inside the first
 * viewport. Before the collapse it was pushed below the fold by ~460px of
 * prompt field, switch panel, select panel and help text.
 */
import React from 'react';
import { createRoot } from 'react-dom/client';
import { ConfigProvider, theme } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import enUS from 'antd/locale/en_US';
import i18n from '@/i18n';
import ConnectivityTestModal from '@/features/coding/opencode/components/ConnectivityTestModal';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const themeMode = parameters.get('theme') || 'light';
const resolvedTheme = themeMode === 'system'
  ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
  : themeMode;
await i18n.changeLanguage(language);
document.documentElement.dataset.theme = resolvedTheme;
// The app supplies antd's own locale (empty-state text, date pickers, …) the
// same way; without it the modal renders antd's Chinese defaults in English.
const antdLocale = language === 'en-US' ? enUS : zhCN;

const MODEL_IDS = [
  'claude-opus-4-8[1M]',
  'claude-haiku-4-5',
  'openai/gpt-5.5',
  'gemini-2.5-pro',
  'grok-4-fast',
  'a-very-long-upstream-model-id-that-must-not-break-the-summary-row',
];

const PROVIDER = {
  id: 'axonhub',
  name: 'AxonHub',
  npm: '@ai-sdk/openai-compatible',
  options: { baseURL: 'https://api.example.invalid/v1', apiKey: 'test-key' },
  models: Object.fromEntries(MODEL_IDS.map((modelId) => [modelId, { name: modelId }])),
};

/**
 * Every connectivity probe fails, which is the only state where the footer can
 * offer removal. Answering here beats stubbing the whole Tauri bridge: the
 * modal reaches the backend exclusively through these two commands.
 */
let failEveryProbe = false;
window.__CONNECTIVITY_FIXTURE_FAIL_PROBES__ = () => {
  failEveryProbe = true;
};
window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    if (failEveryProbe && command === 'test_provider_model_connectivity') {
      // The modal matches results to rows by model id, so echo the requested
      // id back instead of inventing one.
      const [modelId] = args?.request?.modelIds ?? [];
      return { results: [{ modelId, status: 'error', errorMessage: 'fixture failure' }] };
    }
    return undefined;
  },
};

let root = createRoot(document.getElementById('root'));

const mountModal = ({
  diagnostics = { prompt: 'say hi!' },
  // Left undefined by default: removal is opt-in, and the default mount is the
  // "caller did not declare what it can delete" case the fixture pins.
  removableModelIds = undefined,
  onRemoveModels = async (modelIds) => {
    window.__CONNECTIVITY_FIXTURE_REMOVED__ = modelIds;
  },
} = {}) => {
  window.__CONNECTIVITY_FIXTURE_REMOVED__ = [];
  // The modal only re-initialises on a closed→open transition, so re-rendering
  // it with new props would keep the previous run's selections. A root cannot
  // render after `unmount`, so each mount gets a fresh one.
  root.unmount();
  root = createRoot(document.getElementById('root'));
  // The page owns `open`, exactly as every real caller does: the modal closes
  // itself by calling `onCancel`, so a fixture with a no-op handler would keep
  // showing a modal the app had already dismissed.
  root.render(<Harness />);

  function Harness() {
    const [open, setOpen] = React.useState(true);
    return (
      <ConfigProvider
        locale={antdLocale}
        theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}
      >
        <ConnectivityTestModal
          open={open}
          onCancel={() => setOpen(false)}
          providerId={PROVIDER.id}
          providerName={PROVIDER.name}
          providerConfig={PROVIDER}
          modelIds={MODEL_IDS}
          removableModelIds={removableModelIds}
          diagnostics={diagnostics}
          onSaveDiagnostics={async () => {}}
          onRemoveModels={onRemoveModels}
        />
      </ConfigProvider>
    );
  }
};

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

const sectionTitles = () => [...document.querySelectorAll('.ant-modal-body section')];

/** The config section is the first section card; the results section is the second. */
const configSection = () => sectionTitles()[0];
const resultsSection = () => sectionTitles()[1];

const configHeader = () => configSection().querySelector('[role="button"]');

window.connectivityModalFixture = {
  mountModal,

  /** Simulates a user click on the config header (mouse path). */
  async toggleConfigByClick() {
    configHeader().dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await delay(250);
  },

  /** Simulates a user pressing Enter on the focused config header (keyboard path). */
  async toggleConfigByKeyboard() {
    const header = configHeader();
    header.focus();
    header.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await delay(250);
  },

  configHeaderExpanded() {
    return configHeader().getAttribute('aria-expanded') === 'true';
  },

  /** Height of the config section card, in px. */
  configSectionHeight() {
    return configSection().getBoundingClientRect().height;
  },

  /** Distance from the top of the modal body to the top of the model table. */
  distanceFromBodyTopToTable() {
    const body = document.querySelector('.ant-modal-body');
    const table = resultsSection().querySelector('.ant-table');
    return table.getBoundingClientRect().top - body.getBoundingClientRect().top;
  },

  /** True when the model table's first row is inside the viewport. */
  modelTableInViewport() {
    const table = resultsSection().querySelector('.ant-table');
    const rect = table.getBoundingClientRect();
    return rect.top >= 0 && rect.top < window.innerHeight;
  },

  /** True when the collapsed config body keeps its controls out of the tab order. */
  hiddenConfigControlsAreNotFocusable() {
    const configBody = configSection().querySelector('.ant-form');
    return configBody === null || configBody.closest('[hidden]') !== null;
  },

  /** Summary text shown in the collapsed header. */
  configSummaryText() {
    const summary = configHeader().querySelector('span');
    return summary ? summary.textContent : '';
  },

  /**
   * True when the collapsed header sits centered in its card. The card's own
   * padding and the header's negative margins used to disagree, leaving 12px
   * above the row and 30px below.
   */
  collapsedHeaderIsCentered() {
    const card = configSection().getBoundingClientRect();
    const header = configHeader().getBoundingClientRect();
    const above = header.top - card.top;
    const below = card.bottom - header.bottom;
    return Math.abs(above - below) <= 1;
  },

  /**
   * Distance from the card's left edge to the header's content box, in px.
   * Collapsed the header owns the card's padding; expanded the card does. The
   * two must agree, including under the narrow-viewport padding.
   */
  headerInsetFromCardLeft() {
    const card = configSection().getBoundingClientRect();
    const title = configHeader().querySelector('div');
    return title.getBoundingClientRect().left - card.left;
  },

  /** True when the summary row stays within the header's own bounds. */
  summaryDoesNotOverflowHeader() {
    const header = configHeader();
    const summary = header.querySelector('span');
    if (!summary) return false;
    return summary.getBoundingClientRect().right <= header.getBoundingClientRect().right + 1;
  },

  /** True when the modal body fits inside the viewport without scrolling. */
  modalBodyFitsViewport() {
    const body = document.querySelector('.ant-modal-body');
    return body.scrollHeight <= body.clientHeight + 1;
  },

  /**
   * Clears the prompt through the real textarea, so the antd Form field state
   * — not just the DOM — goes empty. React tracks the last value it wrote, so
   * the native setter has to be used to get past its value tracker.
   */
  async clearPrompt() {
    const textarea = configSection().querySelector('textarea');
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
    setter.call(textarea, '');
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
    await delay(250);
  },

  /**
   * Checks the first real model row. The `ant-table-measure-row` antd renders
   * first is `aria-hidden` and clones the header cells — including the
   * select-all checkbox — so matching plain `tr` would click "select all".
   */
  async selectFirstModel() {
    const checkbox = resultsSection().querySelector('.ant-table-tbody tr.ant-table-row input[type="checkbox"]');
    checkbox.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await delay(250);
  },

  /** Clicks the footer's primary "start test" action. */
  async clickStartTest() {
    const button = [...document.querySelectorAll('.ant-modal-footer button')]
      .find((node) => node.classList.contains('ant-btn-primary'));
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await delay(400);
  },

  /**
   * Footer's removal action text, or `null` when the caller offered no removal.
   * It is the only danger button in the footer, so the selector cannot pick up
   * the plain "close" button next to it.
   */
  removalButtonText() {
    const button = document.querySelector('.ant-modal-footer .ant-btn-dangerous');
    return button ? button.textContent.trim() : null;
  },

  /**
   * Runs a test whose every probe fails, then reports the removal button.
   *
   * `removableModelIds` is the switch under test: with it, the footer offers
   * removal and the failed rows are pre-checked; without it, a caller that
   * merely passed `onRemoveModels` gets no button at all.
   */
  async runFailingTest() {
    window.__CONNECTIVITY_FIXTURE_FAIL_PROBES__();
    // antd's Checkbox is controlled: the real input never toggles itself, so a
    // synthetic `click` would fire `onChange(true)` and React would write the
    // old value straight back. Clicking the wrapping label is what a user does
    // and what antd listens for.
    const selectAll = resultsSection().querySelector('.ant-table-thead .ant-checkbox-wrapper');
    selectAll.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await delay(250);
    await this.clickStartTest();
    // The probes settle independently; the footer only re-renders after the
    // last one resolves.
    await delay(1200);
  },

  /** Model ids the modal pre-checked for removal. */
  checkedModelIds() {
    return [...resultsSection().querySelectorAll('.ant-table-tbody tr.ant-table-row')]
      .filter((row) => row.querySelector('input[type="checkbox"]')?.checked)
      .map((row) => row.querySelector('td:nth-child(2)')?.textContent?.trim())
      .filter(Boolean);
  },

  /** Ids the caller's `onRemoveModels` has been handed so far. */
  removedModelIds() {
    return window.__CONNECTIVITY_FIXTURE_REMOVED__;
  },

  /**
   * Opens the Popconfirm and reports where its confirm button sits, in
   * viewport coordinates. The caller clicks it with a real CDP mouse event:
   * antd closes the popover on any document-level mousedown, so a synthetic
   * `click()` on the inner button races the dismissal and loses.
   */
  async openRemovalPopconfirm() {
    const button = document.querySelector('.ant-modal-footer .ant-btn-dangerous');
    const triggerRect = button.getBoundingClientRect();
    return { x: triggerRect.left + triggerRect.width / 2, y: triggerRect.top + triggerRect.height / 2 };
  },

  /** Where the open Popconfirm's confirm button sits, or `null`. */
  confirmButtonPoint() {
    const popover = document.querySelector('.ant-popover:not(.ant-popover-hidden)');
    const confirmButton = popover?.querySelector('.ant-btn-primary');
    if (!confirmButton) {
      return null;
    }
    const rect = confirmButton.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },

  /**
   * True once the modal has been closed by its own success path.
   *
   * antd keeps the dialog's DOM after closing (no `destroyOnClose`), so
   * presence is not the signal — the wrapper's `display` is.
   */
  modalClosed() {
    const wrap = document.querySelector('.ant-modal-wrap');
    if (!wrap) return true;
    return window.getComputedStyle(wrap).display === 'none';
  },

};

mountModal();
