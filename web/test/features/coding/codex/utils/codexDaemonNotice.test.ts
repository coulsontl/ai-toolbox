/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  describeCodexDaemonHolders,
  resolveCodexDaemonRestartAction,
  shouldPromptCodexDaemonRestart,
} from '../../../../../features/coding/codex/utils/codexDaemonNotice.ts';

test('prompts when any holder is running', () => {
  assert.equal(
    shouldPromptCodexDaemonRestart({
      running: true,
      holders: ['daemon'],
      appServerVersion: '0.164.0',
    }),
    true,
  );
  // The desktop app alone must prompt too: it bundles its own app-server and
  // holds the same config (issue #443 was reported from the CLI, but the app
  // reproduces it identically).
  assert.equal(
    shouldPromptCodexDaemonRestart({ running: true, holders: ['desktopApp'] }),
    true,
  );
  assert.equal(
    shouldPromptCodexDaemonRestart({
      running: true,
      holders: ['daemon', 'desktopApp'],
    }),
    true,
  );
});

test('stays silent when nothing is running', () => {
  assert.equal(
    shouldPromptCodexDaemonRestart({ running: false, holders: [] }),
    false,
  );
});

test('stays silent when the probe is missing or failed', () => {
  // The apply succeeded; a failed probe is not evidence of stale settings, and
  // prompting here would turn a backend hiccup into user-facing noise.
  assert.equal(shouldPromptCodexDaemonRestart(null), false);
  assert.equal(shouldPromptCodexDaemonRestart(undefined), false);
});

test('the mode decides the action once a holder is running', () => {
  const running = { running: true, holders: ['daemon'] as const };
  // `prompt` is the pre-existing behaviour and stays the default.
  assert.equal(resolveCodexDaemonRestartAction({ ...running, holders: ['daemon'] }, 'prompt'), 'prompt');
  assert.equal(resolveCodexDaemonRestartAction({ ...running, holders: ['daemon'] }, 'auto'), 'auto');
  assert.equal(resolveCodexDaemonRestartAction({ ...running, holders: ['daemon'] }, 'off'), 'none');
});

test('no action in any mode when nothing holds the config', () => {
  const idle = { running: false, holders: [] as const };
  assert.equal(resolveCodexDaemonRestartAction({ ...idle, holders: [] }, 'prompt'), 'none');
  assert.equal(resolveCodexDaemonRestartAction({ ...idle, holders: [] }, 'auto'), 'none');
  assert.equal(resolveCodexDaemonRestartAction({ ...idle, holders: [] }, 'off'), 'none');
});

test('a failed probe stays silent in every mode', () => {
  // Even `auto` must not act on a probe that never answered: there is nothing
  // to restart, and a backend hiccup must not look like a holder.
  assert.equal(resolveCodexDaemonRestartAction(null, 'prompt'), 'none');
  assert.equal(resolveCodexDaemonRestartAction(null, 'auto'), 'none');
  assert.equal(resolveCodexDaemonRestartAction(undefined, 'auto'), 'none');
});

test('describes each holder kind so the prompt can tell them apart', () => {
  assert.deepEqual(describeCodexDaemonHolders([]), {
    hasDaemon: false,
    hasDesktopApp: false,
  });
  assert.deepEqual(describeCodexDaemonHolders(['daemon']), {
    hasDaemon: true,
    hasDesktopApp: false,
  });
  assert.deepEqual(describeCodexDaemonHolders(['desktopApp']), {
    hasDaemon: false,
    hasDesktopApp: true,
  });
  assert.deepEqual(describeCodexDaemonHolders(['daemon', 'desktopApp']), {
    hasDaemon: true,
    hasDesktopApp: true,
  });
});
