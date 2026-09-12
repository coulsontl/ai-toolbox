import { execFileSync, spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { access, copyFile, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { cpus, platform, tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { build } from 'vite';

import { verifySessionList } from '../web/test/features/coding/shared/sessionManager/sessionListBrowserChecks.mjs';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const fixtureDirectory = path.join(projectRoot, 'web/test/features/coding/shared/sessionManager');
const { values } = parseArgs({
  options: {
    baseline: { type: 'string', default: 'HEAD' },
    repetitions: { type: 'string', default: '3' },
    browser: { type: 'string' },
    output: { type: 'string' },
    'verify-only': { type: 'boolean', default: false },
  },
});
const repetitions = Number(values.repetitions);
if (!Number.isInteger(repetitions) || repetitions < 1) throw new Error('--repetitions must be a positive integer');
const outputParent = path.resolve(values.output ?? tmpdir());
await mkdir(outputParent, { recursive: true });
const artifactRoot = await mkdtemp(path.join(outputParent, 'session-list-benchmark-'));
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

async function findBrowser() {
  const candidates = [
    values.browser,
    process.env['PROGRAMFILES(X86)'] && path.join(process.env['PROGRAMFILES(X86)'], 'Microsoft/Edge/Application/msedge.exe'),
    process.env.PROGRAMFILES && path.join(process.env.PROGRAMFILES, 'Google/Chrome/Application/chrome.exe'),
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
    '/usr/bin/google-chrome',
  ].filter(Boolean);
  for (const candidate of candidates) {
    try { await access(candidate); return candidate; } catch { /* Try the next installed browser. */ }
  }
  throw new Error('No Chromium browser found. Pass --browser with an installed Chrome or Edge executable.');
}

const browserPath = await findBrowser();
const packageJson = JSON.parse(await readFile(path.join(projectRoot, 'package.json'), 'utf8'));
const aliases = [
  { find: /^react-dom\/client$/, replacement: path.join(projectRoot, 'node_modules/react-dom/profiling.js') },
  { find: '@', replacement: path.join(projectRoot, 'web') },
  ...Object.keys(packageJson.dependencies).map(name => ({
    find: new RegExp('^' + name.replace(/[.*+?^$()|[\]\\]/g, '\\$&') + '(?=/|$)'),
    replacement: path.join(projectRoot, 'node_modules', name),
  })),
];
await copyFile(path.join(fixtureDirectory, 'fixtures/sessionListBenchmark.jsx'), path.join(artifactRoot, 'entry.jsx'));
await writeFile(path.join(artifactRoot, 'index.html'), '<!DOCTYPE html><html><head><meta charset="UTF-8"></head><body style="margin:0"><div id="root"></div><script type="module" src="./entry.jsx"></script></body></html>');

async function buildFixture(variant) {
  const baselineSources = new Map();
  if (variant === 'baseline') {
    for (const fileName of ['SessionManagerPanel.tsx', 'SessionManagerPanel.module.less']) {
      const relativePath = 'web/features/coding/shared/sessionManager/' + fileName;
      baselineSources.set(path.join(projectRoot, relativePath).replaceAll('\\', '/'), execFileSync(
        'git', ['show', values.baseline + ':' + relativePath], { cwd: projectRoot, encoding: 'utf8', windowsHide: true },
      ));
    }
  }
  await build({
    configFile: false,
    root: artifactRoot,
    base: './',
    resolve: { alias: aliases },
    plugins: [{ name: 'baseline-session-panel', load: id => baselineSources.get(id.replaceAll('\\', '/')) }],
    css: { preprocessorOptions: { less: { javascriptEnabled: true } } },
    esbuild: { jsx: 'automatic' },
    build: {
      outDir: path.join(artifactRoot, variant),
      emptyOutDir: false,
      reportCompressedSize: false,
      chunkSizeWarningLimit: 3000,
      rollupOptions: { onwarn(warning, warn) { if (warning.code !== 'MODULE_LEVEL_DIRECTIVE') warn(warning); } },
    },
  });
}

async function openBrowser() {
  const profilePath = await mkdtemp(path.join(artifactRoot, 'browser-profile-'));
  // Chromium's random debugging port can be a Fetch-blocked port (e.g. 5060).
  // Ask the OS for an ephemeral port before launching the isolated browser.
  const portReservation = createServer();
  await new Promise(resolve => portReservation.listen(0, '127.0.0.1', resolve));
  const debugPort = portReservation.address().port;
  await new Promise(resolve => portReservation.close(resolve));
  const browser = spawn(browserPath, [
    '--headless=new', '--remote-debugging-port=' + debugPort, '--user-data-dir=' + profilePath,
    '--no-first-run', '--no-default-browser-check', '--disable-extensions', '--disable-background-networking',
    '--window-size=1280,900', '--force-device-scale-factor=1', 'about:blank',
  ], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
  let browserErrors = '';
  browser.stderr.on('data', data => { browserErrors += data; });
  let socket;
  try {
    let targets;
    for (let attempt = 0; attempt < 150; attempt++) {
      try { targets = await (await fetch('http://127.0.0.1:' + debugPort + '/json/list')).json(); break; }
      catch { await delay(100); }
    }
    if (!targets) throw new Error('Browser did not start: ' + browserErrors.slice(-1500));
    socket = new WebSocket(targets.find(target => target.type === 'page').webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', reject, { once: true });
    });
    const pendingRequests = new Map();
    const exceptions = [];
    let nextRequestId = 0;
    socket.addEventListener('message', event => {
      const payload = JSON.parse(event.data);
      if (payload.method === 'Runtime.exceptionThrown') exceptions.push(payload.params.exceptionDetails);
      const request = pendingRequests.get(payload.id);
      if (!request) return;
      pendingRequests.delete(payload.id);
      clearTimeout(request.timeout);
      payload.error ? request.reject(new Error(JSON.stringify(payload.error))) : request.resolve(payload.result);
    });
    const send = (method, params = {}) => new Promise((resolve, reject) => {
      const id = ++nextRequestId;
      const timeout = setTimeout(() => {
        pendingRequests.delete(id);
        reject(new Error('Browser command timed out: ' + method));
      }, 30000);
      pendingRequests.set(id, { resolve, reject, timeout });
      socket.send(JSON.stringify({ id, method, params }));
    });
    const evaluate = async expression => {
      const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
      if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    await send('Page.enable');
    await send('Runtime.enable');
    await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
    return {
      send, evaluate, exceptions,
      async close() {
        try { await send('Browser.close'); } finally { socket.close(); browser.kill(); }
      },
    };
  } catch (error) {
    socket?.close();
    browser.kill();
    throw error;
  }
}

async function serveFixture(variant) {
  const fixtureRoot = path.join(artifactRoot, variant);
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://localhost');
    const filePath = path.resolve(fixtureRoot, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
    if (!filePath.startsWith(fixtureRoot + path.sep)) { response.statusCode = 403; response.end(); return; }
    try {
      response.setHeader('Content-Type', filePath.endsWith('.js') ? 'text/javascript' : filePath.endsWith('.css') ? 'text/css' : 'text/html');
      response.end(await readFile(filePath));
    } catch { response.statusCode = 404; response.end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { baseUrl: 'http://127.0.0.1:' + server.address().port, close: () => new Promise(resolve => server.close(resolve)) };
}

async function measureScenario(browser, baseUrl, count) {
  const { send, evaluate } = browser;
  const fixtureRunId = randomUUID();
  await send('Page.navigate', { url: baseUrl + '/?count=' + count + '&fixtureRunId=' + fixtureRunId });
  let ready = false;
  for (let attempt = 0; attempt < 300; attempt++) {
    ready = await evaluate('window.benchmark?.fixtureRunId === ' + JSON.stringify(fixtureRunId) + ' && !!benchmark.metrics.initialSnapshot');
    if (ready) break;
    await delay(50);
  }
  if (!ready) throw new Error('Session fixture did not become ready');
  const initial = await evaluate('benchmark.metrics.initialSnapshot');
  await delay(350);
  // A space updates the controlled input without triggering a different backend query.
  const typing = await evaluate('benchmark.measure(() => benchmark.setInput(" "))');
  await delay(300);
  const selection = await evaluate('benchmark.measure(() => benchmark.findButton("sessionManager.select").click())');
  const selectAll = await evaluate('benchmark.measure(() => benchmark.findButton("sessionManager.selectLoaded").click())');
  const scroll = await evaluate('benchmark.scroll()');
  return { count, initial, typing, selection, selectAll, scroll };
}

const variants = values['verify-only'] ? ['current'] : ['baseline', 'current'];
for (const variant of variants) await buildFixture(variant);
const report = {
  generatedAt: new Date().toISOString(), platform: platform(), cpu: cpus()[0]?.model, node: process.version,
  baseline: values.baseline, viewport: { width: 1280, height: 900 }, artifactRoot, runs: [], checks: [],
};
console.log('Artifacts: ' + artifactRoot);
// Run each variant sequentially so two browsers do not compete during measurements.
for (let repetition = 0; repetition < (values['verify-only'] ? 1 : repetitions); repetition++) {
  for (const variant of variants) {
    const server = await serveFixture(variant);
    let browser;
    try {
      browser = await openBrowser();
      report.browser ??= await browser.send('Browser.getVersion');
      if (!values['verify-only']) {
        for (const count of [500, 2000, 5000]) {
          const sample = { variant, repetition: repetition + 1, ...await measureScenario(browser, server.baseUrl, count) };
          report.runs.push(sample);
          console.log(JSON.stringify(sample));
        }
      }
      if (variant === 'current' && repetition === 0) {
        report.checks = await verifySessionList({ ...browser, baseUrl: server.baseUrl, benchDirectory: artifactRoot });
      }
      if (browser.exceptions.length) throw new Error('Uncaught browser exceptions: ' + JSON.stringify(browser.exceptions));
    } finally {
      try {
        await browser?.close();
      } finally {
        await server.close();
        await writeFile(path.join(artifactRoot, 'report.json'), JSON.stringify(report, null, 2));
      }
    }
  }
}
console.log('Report: ' + path.join(artifactRoot, 'report.json'));
