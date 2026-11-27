#!/usr/bin/env node
// Live smoke test for the local control API (`aitoolbox`).
//
// This is the repeatable part of the "Live E2E 验证 SOP" in
// `tauri/src/control_api/AGENTS.md`: it talks to a *running* AI Toolbox app
// through the real `aitoolbox` binary, exercising read-only routes for all 17 CLIs
// plus the gateway lifecycle.
//
// Safety:
// - Refuses to run without `--live`, so `pnpm test` never touches a user app.
// - Never writes to a CLI: no provider/model/config/prompt mutations, and no
//   gateway engage/restore/switch. Only `gateway restart` / `reset-health`
//   (both L1) are attempted, and only when the persisted approval mode lets L1
//   writes pass without a desktop click.
// - Does not create or delete data; it only restarts the gateway it found.
//
// Usage:
//   node scripts/control-api-live-e2e.mjs --live
//   AITOOLBOX_BIN=/path/to/aitoolbox node scripts/control-api-live-e2e.mjs --live
//
// The default binary is `cli/target/debug/aitoolbox(.exe)` next to this repo.

import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const live = process.argv.includes('--live');
if (!live) {
  console.log('[control-api-live-e2e] skipped (pass --live to run against the running app)');
  process.exit(0);
}

const aitoolbox =
  process.env.AITOOLBOX_BIN ??
  path.join(projectRoot, 'cli', 'target', 'debug', process.platform === 'win32' ? 'aitoolbox.exe' : 'aitoolbox');

let failures = 0;

function run(args, { timeout = 60_000 } = {}) {
  const result = spawnSync(aitoolbox, [...args, '--json'], {
    encoding: 'utf8',
    timeout,
    windowsHide: true,
  });
  let body = null;
  if (result.stdout?.trim()) {
    try {
      body = JSON.parse(result.stdout);
    } catch {
      body = result.stdout;
    }
  }
  return { code: result.status ?? -1, body, stderr: result.stderr?.trim() ?? '' };
}

function check(label, args, validate = () => true, options) {
  const result = run(args, options);
  let ok = result.code === 0;
  let detail = '';
  if (ok) {
    try {
      ok = validate(result.body, result);
    } catch (error) {
      ok = false;
      detail = error.message;
    }
  }
  if (!ok && !detail) {
    detail = `exit ${result.code}: ${result.stderr || JSON.stringify(result.body)}`;
  }
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${detail ? `  (${detail})` : ''}`);
  if (!ok) failures += 1;
  return result.body;
}

function skip(label, reason) {
  console.log(`SKIP  ${label}  (${reason})`);
}

const status = check('status', ['status'], (body) => body?.running === true);
if (status) {
  console.log(`      control API ${status.url}  approval=${status.approvalMode}`);
}

const clis = check('cli list', ['cli', 'list'], (body) => body?.clis?.length === 17);
if (clis) {
  for (const key of clis.clis.map((row) => row.cliKey)) {
    check(`cli paths ${key}`, ['cli', 'paths', '--cli', key], (body) => Boolean(body?.configPath || body?.rootDir));
  }
}

check('gateway status', ['gateway', 'status'], (body) => typeof body?.running === 'boolean');
check('gateway clis', ['gateway', 'clis'], (body) => Array.isArray(body));
check('gateway health', ['gateway', 'health'], (body) => Array.isArray(body));
check('gateway logs', ['gateway', 'logs', '--limit', '5'], (body) => Array.isArray(body?.data));
check('plugin list', ['plugin', 'list'], (body) => Array.isArray(body));
check('plugin providers', ['plugin', 'providers'], (body) => Array.isArray(body));
check('plugin accounts', ['plugin', 'accounts'], (body) => Array.isArray(body));
check('plugin middleware', ['plugin', 'middleware'], (body) => Array.isArray(body));
check('plugin pages', ['plugin', 'pages'], (body) => Array.isArray(body));
check('plugin settings', ['plugin', 'settings'], (body) => typeof body?.npmRegistry === 'string');
check('plugin bun-status', ['plugin', 'bun-status'], (body) => typeof body?.available === 'boolean');
check('plugin host-status', ['plugin', 'host-status'], (body) => typeof body?.running === 'boolean');

// L1 writes only when the approval policy will not block on a modal.
const l1Allowed = status?.approvalMode === 'auto_approve' || status?.approvalMode === 'ask_high_risk';
if (!l1Allowed) {
  skip('gateway reset-health', `approvalMode=${status?.approvalMode ?? 'unknown'} would require a desktop click`);
  skip('gateway restart', `approvalMode=${status?.approvalMode ?? 'unknown'} would require a desktop click`);
} else {
  check('gateway reset-health', ['gateway', 'reset-health'], (body) => body?.reset === true);
  const restart = run(['gateway', 'restart'], { timeout: 90_000 });
  if (restart.code === 0) {
    console.log('PASS  gateway restart');
  } else {
    // Guarantee the app is left with a working gateway even when the restart
    // regression comes back.
    run(['gateway', 'start'], { timeout: 90_000 });
    console.log(
      `FAIL  gateway restart  (exit ${restart.code}: ${restart.stderr || JSON.stringify(restart.body)}); attempted gateway start recovery`,
    );
    failures += 1;
  }
  check('gateway status after restart', ['gateway', 'status'], (body) => body?.running === true);
}

console.log(failures === 0 ? '\nAll live control API checks passed.' : `\n${failures} check(s) failed.`);
process.exit(failures === 0 ? 0 : 1);
