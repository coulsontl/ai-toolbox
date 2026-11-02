import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const readSource = (relativePath: string) =>
  readFile(new URL(relativePath, import.meta.url), 'utf8');

/**
 * The startup recovery screen boots with the SQLite database unreadable (its
 * `user_version` is newer than this build supports), which is exactly where
 * `receive_beta_updates` is stored. Without an explicit opt-in the beta channel
 * could never be reached from that screen, leaving the user stuck on a stable
 * build that cannot open their data while the only build that can — the
 * rolling beta — is never even offered.
 */
test('the recovery screen can force the beta channel past the unreadable setting', async () => {
  const backend = await readSource('../../../tauri/src/update.rs');

  // Both entry points must take the flag: the install path re-selects the
  // channel on the backend, so a beta check followed by a stable-only install
  // only rediscovers the stable release and fails with "No update available".
  assert.match(backend, /pub async fn check_for_updates\([\s\S]{0,200}force_beta: Option<bool>/);
  assert.match(backend, /pub async fn install_update\([\s\S]{0,200}force_beta: Option<bool>/);

  // The flag has to reach the shared selector from both paths.
  assert.match(backend, /select_update_candidate\(&app, force_beta\.unwrap_or\(false\)\)/);
  assert.match(backend, /run_updater_download\(&app, force_beta\.unwrap_or\(false\)\)/);

  // A forced beta fetch must fail loudly. Degrading to the stable candidate
  // would render "already up to date" while no in-app alternative is left.
  assert.match(backend, /Err\(error\) if force_beta => Err\(error\)/);
});

test('the recovery screen keeps one beta flag across check, retry, and install', async () => {
  const screen = await readSource('../../app/RecoveryApp.tsx');

  assert.match(screen, /checkForUpdates\(\{ forceBeta: withBeta \}\)/);
  assert.match(screen, /installUpdate\(\{ forceBeta \}\)/);

  // Retry must not silently downgrade a failed beta check back to stable-only.
  assert.match(screen, /runCheck\(forceBeta\)/);

  // The opt-in button is only offered where it can actually help: the stable
  // channel had nothing newer, and the install is not package-managed (Scoop /
  // deb can only ever upgrade the stable build).
  assert.match(
    screen,
    /phase === 'noUpdate' && !forceBeta && !scoopManaged && !debManaged[\s\S]{0,200}recovery\.tryBeta/,
  );
});

/**
 * `startInstall` is a `useCallback`, so a missing `forceBeta` dependency
 * freezes the flag at its initial value. The symptom is a fully working beta
 * check followed by an install that re-selects the stable channel and fails —
 * the exact dead end this feature exists to remove.
 */
test('the install callback is not memoized away from the beta flag', async () => {
  const screen = await readSource('../../app/RecoveryApp.tsx');
  const startInstall = screen.slice(
    screen.indexOf('const startInstall = React.useCallback'),
    screen.indexOf('}, [', screen.indexOf('const startInstall = React.useCallback')),
  );

  assert.match(startInstall, /installUpdate\(\{ forceBeta \}\)/);
  assert.match(
    screen.slice(screen.indexOf('const startInstall = React.useCallback')),
    /^\s*\}, \[forceBeta\]\);/m,
  );
});
