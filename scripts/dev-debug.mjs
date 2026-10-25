// Start `tauri dev` with the WebView2 DevTools Protocol port enabled, so an
// agent (or any CDP client) can attach to the real webview: read the console,
// exceptions and network traffic, evaluate JS, take screenshots.
//
// Why an environment variable and not a code change: the main window is built
// in code (`build_main_window` in tauri/src/lib.rs) and nothing in the repo
// passes `additional_browser_args`. WebView2 *appends*
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS after whatever the app passes, so the
// wry defaults stay intact and we only add the debug port. Doing it in code
// would also require a separate data directory for the differing browser args.
//
// Scope: this flag is inherited by the whole process tree we spawn, so only
// this run of the app gets the port. A plain `pnpm tauri dev` from another
// terminal does not, which is the point of keeping this as a separate script.
import { execFileSync, spawn } from 'node:child_process';
import { createServer } from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

// 9222 is taken on this machine by Pot (com.pot-app.desktop), which runs its own
// WebView2 with the same flag. Probing for a free port keeps two dev instances
// (say, two worktrees) from fighting over one debugging endpoint.
async function findFreePort(preferred) {
  const isFree = (port) =>
    new Promise((resolve) => {
      const probe = createServer();
      probe.once('error', () => resolve(false));
      probe.listen(port, '127.0.0.1', () => probe.close(() => resolve(true)));
    });

  if (await isFree(preferred)) return preferred;
  for (let port = preferred + 1; port < preferred + 20; port += 1) {
    if (await isFree(port)) return port;
  }
  throw new Error(`No free debugging port near ${preferred}`);
}

const preferredPort = Number(process.env.AI_TOOLBOX_CDP_PORT ?? 9223);
const debugPort = await findFreePort(preferredPort);

console.log(`[dev:debug] WebView2 CDP enabled on http://127.0.0.1:${debugPort}`);
console.log(`[dev:debug] targets: http://127.0.0.1:${debugPort}/json/list`);
console.log('[dev:debug] devtools: open the ws query string from /json/list in a browser');
console.log(`[dev:debug] backend log: %APPDATA%\\com.ai-toolbox\\logs\\ai-toolbox_<date>.log`);

// Run the tauri CLI's JavaScript entry directly instead of going through
// `pnpm exec`. Two reasons:
//   1. pnpm is a `.cmd` shim on Windows and Node 24 refuses to spawn a `.cmd`
//      without a shell (`spawn EINVAL`).
//   2. A shell adds an extra `cmd.exe` layer between us and the tree, which
//      complicates stopping it cleanly.
// The file is the same one `tauri.js` dispatches to, so this is not a workaround
// for a missing API — it is the CLI's own entry point.
const tauriCliEntry = path.join(projectRoot, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');

const child = spawn(process.execPath, [tauriCliEntry, 'dev', ...process.argv.slice(2)], {
  cwd: projectRoot,
  stdio: 'inherit',
  env: {
    ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort}`,
  },
});

// Ctrl+C reaches the whole console process group on Windows, so the tauri CLI
// and the app usually stop on their own. What survives is Vite: it can outlive
// its parent and keep port 5173, which makes the next run die on
// `strictPort: true`. Record the descendants now, then sweep whatever is still
// alive once the direct child exits.
// `wmic` was removed from Windows 11 (24H2+), where it now fails with ENOENT —
// and because this runs inside the Ctrl+C handler, that crash aborted the whole
// shutdown. PowerShell's CIM query is the supported replacement.
function collectDescendants(rootPid) {
  const script =
    'Get-CimInstance Win32_Process | ForEach-Object { "$($_.ParentProcessId),$($_.ProcessId)" }';
  let all = '';
  try {
    all = execFileSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', script], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
  } catch {
    return []; // Snapshot is best effort; the taskkill /T below is the fallback.
  }
  const childrenByParent = new Map();
  for (const line of all.split(/\r?\n/)) {
    const [parent, pid] = line.trim().split(',');
    if (!pid) continue;
    const parentPid = Number(parent);
    if (!childrenByParent.has(parentPid)) childrenByParent.set(parentPid, []);
    childrenByParent.get(parentPid).push(Number(pid));
  }
  const descendants = [];
  const queue = [rootPid];
  while (queue.length) {
    for (const childPid of childrenByParent.get(queue.shift()) ?? []) {
      descendants.push(childPid);
      queue.push(childPid);
    }
  }
  return descendants;
}

function sweepRemaining(trackedPids) {
  for (const pid of trackedPids) {
    try {
      process.kill(pid, 0);
    } catch {
      continue;
    }
    try {
      execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
      console.log(`[dev:debug] swept leftover process ${pid}`);
    } catch {
      // Already gone between the liveness check and the kill.
    }
  }
}

let trackedDescendants = [];
let stopping = false;

function stopChildTree() {
  if (stopping) return;
  stopping = true;
  if (process.platform === 'win32') {
    // Snapshot before killing: re-parented grandchildren vanish from the tree.
    trackedDescendants = collectDescendants(child.pid);
    spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
  } else {
    child.kill('SIGTERM');
  }
}
process.on('SIGINT', stopChildTree);
process.on('SIGTERM', stopChildTree);

child.once('exit', (code, signal) => {
  if (trackedDescendants.length) sweepRemaining(trackedDescendants);
  if (signal) {
    console.log(`[dev:debug] tauri dev stopped by ${signal}`);
    process.exit(0);
  }
  process.exit(code ?? 0);
});
