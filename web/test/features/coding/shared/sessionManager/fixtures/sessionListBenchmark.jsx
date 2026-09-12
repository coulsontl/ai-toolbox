// Browser-only fixture using the real panel with isolated in-memory Tauri APIs.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter, useLocation, useNavigate } from 'react-router-dom';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import KeepAliveOutlet from '@/components/layout/KeepAliveOutlet';
import SessionManagerPanel from '@/features/coding/shared/sessionManager/SessionManagerPanel';
import styles from '@/features/coding/shared/sessionManager/SessionManagerPanel.module.less';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const sessionCount = Number(parameters.get('count') ?? 500);
const selectedTheme = parameters.get('theme') ?? 'light';
const initialTool = parameters.get('tool') ?? 'codex';
const longPaths = parameters.get('long') === '1';
const recentFirst = parameters.get('recent') === '1';
const fixtureRunId = parameters.get('fixtureRunId');
const metrics = { requests: [], commits: [], longTasks: [], dataReadyAt: 0, expectedTotal: sessionCount };
const sessions = Array.from({ length: sessionCount }, (_, index) => ({
  providerId: initialTool,
  sessionId: 'session-' + String(index).padStart(6, '0'),
  title: 'Session ' + index + ' — shared session list performance fixture',
  summary: 'Synthetic session metadata for rendering verification',
  sourcePath: 'C:\\session-fixtures\\project-' + (index % 20) + '\\session-' + index + '.jsonl',
  projectDir: 'D:\\Projects\\project-' + (index % 20) + (longPaths ? '\\目录很长的项目路径'.repeat(index % 7 + 1) : ''),
  createdAt: 1700000000000 + index * 1000,
  lastActiveAt: 1700000000000 + index * 1000,
  resumeCommand: index % 13 ? 'codex resume session-' + index : null,
  runtimeSource: index % 2 ? 'wsl' : 'local',
  runtimeDistro: index % 2 ? 'Debian' : null,
}));
let deletedPaths = new Set();
Object.defineProperty(navigator, 'clipboard', { value: { writeText: async text => { window.fixtureCopiedText = text; } } });
window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    metrics.requests.push({ command, args, at: performance.now() });
    if (command === 'list_tool_sessions') {
      let result = sessions.filter(item => !deletedPaths.has(item.sourcePath));
      const availableSources = [{ source: 'local' }, { source: 'wsl', distro: 'Debian' }];
      if (args.sourceMode !== 'all') result = result.filter(item => item.runtimeSource === args.sourceMode);
      const availablePaths = Array.from(new Set(result.map(item => item.projectDir)));
      if (args.pathFilter) result = result.filter(item => item.projectDir.includes(args.pathFilter));
      if (args.query) {
        const exact = result.filter(item => item.sessionId.toLowerCase() === args.query.toLowerCase());
        result = exact.length ? exact : result.filter(item => [item.title, item.summary, item.sessionId, item.projectDir].some(value => value.toLowerCase().includes(args.query.toLowerCase())));
      }
      const partial = recentFirst && args.loadMode === 'cache-first' && !args.query;
      if (partial) result = result.slice(0, 10);
      if (args.loadMode === 'full' && recentFirst) await new Promise(resolve => setTimeout(resolve, 300));
      metrics.dataReadyAt = performance.now();
      metrics.expectedTotal = result.length;
      return { items: result, total: result.length, page: 1, pageSize: 10, hasMore: false,
        partial, cacheState: partial ? 'quick' : 'fresh', metaComplete: !partial,
        messageSearchComplete: true, availableSources, availablePaths };
    }
    if (command === 'delete_tool_sessions') {
      const failedItems = [];
      args.sourcePaths.forEach(path => path === window.fixtureFailDelete ? failedItems.push({ sourcePath: path, error: 'Fixture deletion failure' }) : deletedPaths.add(path));
      return { deletedCount: args.sourcePaths.length - failedItems.length, failedItems };
    }
    if (command === 'delete_tool_session') { deletedPaths.add(args.sourcePath); return; }
    if (command === 'plugin:dialog|open') return null;
    throw new Error('Unexpected fixture command: ' + command);
  },
};
new PerformanceObserver(list => metrics.longTasks.push(...list.getEntries().map(entry => ({ start: entry.startTime, duration: entry.duration })))).observe({ type: 'longtask', buffered: true });
const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
const cards = () => Array.from(document.getElementsByClassName(styles.sessionCard));
let initialPaintScheduled = false;
const initialPaintObserver = new MutationObserver(() => {
  if (initialPaintScheduled || !metrics.dataReadyAt || cards().length === 0) return;
  initialPaintScheduled = true;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    metrics.initialSnapshot = {
      mountedCards: cards().length,
      elements: document.querySelectorAll('*').length,
      reactDurationMs: metrics.commits.reduce((sum, item) => sum + item.duration, 0),
      dataToPaintMs: performance.now() - metrics.dataReadyAt,
      longTasks: metrics.longTasks.slice(),
    };
    initialPaintObserver.disconnect();
  }));
});
initialPaintObserver.observe(document.body, { childList: true, subtree: true });
const findButton = (key, options) => Array.from(document.querySelectorAll('button')).find(button => button.textContent.trim() === i18n.t(key, options) && button.getBoundingClientRect().height > 0);
const input = () => Array.from(document.querySelectorAll('input')).find(node => node.placeholder === i18n.t('sessionManager.searchPlaceholder') && node.getBoundingClientRect().height > 0);
function setInput(value) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input(), value);
  input().dispatchEvent(new Event('input', { bubbles: true }));
}
window.benchmark = {
  metrics, cards, findButton, setInput, sessionCount, sessions, fixtureRunId,
  async nextPaint() { await frame(); await frame(); },
  async measure(action) {
    metrics.commits.length = 0;
    const started = performance.now();
    action();
    await frame(); await frame();
    return { elapsedMs: performance.now() - started, reactDurationMs: metrics.commits.reduce((sum, item) => sum + item.duration, 0), mountedCards: cards().length };
  },
  async scroll(frames = 120) {
    const main = document.querySelector('main');
    const intervals = [];
    let previous = performance.now();
    let maximumMounted = 0;
    for (let index = 0; index < frames; index++) {
      await frame();
      const now = performance.now();
      intervals.push(now - previous);
      previous = now;
      main.scrollTop = (main.scrollHeight - main.clientHeight) * index / (frames - 1);
      maximumMounted = Math.max(maximumMounted, document.getElementsByClassName(styles.sessionCard).length);
    }
    await frame(); await frame();
    const sorted = intervals.slice(2).sort((left, right) => left - right);
    return { fps: 1000 / (sorted.reduce((sum, item) => sum + item, 0) / sorted.length), p95FrameMs: sorted[Math.floor(sorted.length * .95)], maxFrameMs: Math.max(...sorted), maximumMounted, lastTitle: cards().at(-1)?.textContent, scrollTop: main.scrollTop };
  },
  async setTheme(value) {
    window.fixtureSetTheme(value);
    await frame(); await frame();
  },
  async navigate(path) {
    window.fixtureNavigate(path);
    await frame(); await frame();
  },
};
const panelProfiler = (_id, phase, duration) => metrics.commits.push({ phase, duration, at: performance.now() });
const SessionFixturePage = () => {
  const [leadingHeight, setLeadingHeight] = React.useState(Number(parameters.get('leadingHeight') ?? 0));
  window.fixtureSetLeadingHeight = setLeadingHeight;
  return <>
    {leadingHeight > 0 ? <div style={{ height: leadingHeight }} /> : null}
    <React.Profiler id="sessions" onRender={panelProfiler}><SessionManagerPanel tool={initialTool} expandNonce={1} /></React.Profiler>
  </>;
};
const OtherPage = () => <SessionManagerPanel tool="claudecode" expandNonce={1} />;
const DetailPage = () => {
  const navigate = useNavigate();
  const location = useLocation();
  return <button id="back" onClick={() => navigate(location.state.from, { state: { restoreScrollTop: location.state.fromScrollTop } })}>Back to sessions</button>;
};
const routes = [
  { path: '/coding/' + initialTool, component: SessionFixturePage },
  { path: '/coding/' + initialTool + '/sessions/detail', component: DetailPage, chrome: { mode: 'secondary' } },
  ...(initialTool === 'claudecode' ? [] : [{ path: '/coding/claudecode', component: OtherPage }]),
  { path: '/empty', component: () => <p>Other page</p> },
];
function Fixture() {
  const mainRef = React.useRef(null);
  const [currentTheme, setCurrentTheme] = React.useState(selectedTheme);
  const navigate = useNavigate();
  window.fixtureNavigate = navigate;
  window.fixtureSetTheme = setCurrentTheme;
  const dark = currentTheme === 'dark' || currentTheme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches;
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  return <ConfigProvider theme={{ algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm, token: { colorPrimary: '#1890ff' } }}><App>
    <main ref={mainRef} style={{ height: '100vh', overflowY: 'auto', overflowX: 'hidden', padding: 24, boxSizing: 'border-box', background: 'var(--color-bg-layout)' }}>
      <KeepAliveOutlet routes={routes} max={12} scrollContainerRef={mainRef} />
    </main>
  </App></ConfigProvider>;
}
createRoot(document.getElementById('root')).render(<MemoryRouter initialEntries={['/coding/' + initialTool]}><Fixture /></MemoryRouter>);
