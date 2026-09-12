import assert from 'node:assert/strict';
import test from 'node:test';

import {
  reconcileSessionSelection,
  toggleLoadedSessionSelection,
} from '../../../../../features/coding/shared/sessionManager/sessionSelection.ts';
import type { SessionMeta } from '../../../../../features/coding/shared/sessionManager/types.ts';

const makeSession = (sourcePath: string, sessionId = sourcePath): SessionMeta => ({
  providerId: 'codex',
  sessionId,
  sourcePath,
});

test('select all covers the complete loaded list including sessions outside the viewport', () => {
  const sessions = Array.from({ length: 5000 }, (_, index) => makeSession(`session-${index}.jsonl`));
  const selected = toggleLoadedSessionSelection([], sessions);

  assert.equal(selected.length, 5000);
  assert.equal(selected[0], 'session-0.jsonl');
  assert.equal(selected[selected.length - 1], 'session-4999.jsonl');
  assert.deepEqual(toggleLoadedSessionSelection(selected, sessions), []);
});

test('local and WSL sessions with the same id remain independently selectable', () => {
  const sessions = [
    makeSession('C:\\Users\\user\\.codex\\sessions\\same.jsonl', 'same-id'),
    makeSession('\\\\wsl.localhost\\Debian\\home\\user\\.codex\\sessions\\same.jsonl', 'same-id'),
  ];

  assert.deepEqual(toggleLoadedSessionSelection([sessions[0].sourcePath], sessions), sessions.map((session) => session.sourcePath));
});

test('a refreshed or filtered list clears selection for sessions that are no longer loaded', () => {
  const selected = ['deleted.jsonl', 'remaining.jsonl', 'filtered-out.jsonl'];

  assert.deepEqual(reconcileSessionSelection(selected, [makeSession('remaining.jsonl')]), ['remaining.jsonl']);
  assert.deepEqual(reconcileSessionSelection(selected, []), []);
});

test('background completion preserves valid selection without selecting newly loaded sessions', () => {
  const selected = ['recent.jsonl'];
  const refreshed = [makeSession('new.jsonl'), makeSession('recent.jsonl'), makeSession('older.jsonl')];

  assert.strictEqual(reconcileSessionSelection(selected, refreshed), selected);
  assert.deepEqual(selected, ['recent.jsonl']);
});

test('partial deletion keeps the failed session selected after the refreshed list arrives', () => {
  const selected = ['deleted.jsonl', 'failed.jsonl'];
  const refreshed = [makeSession('failed.jsonl'), makeSession('unselected.jsonl')];

  assert.deepEqual(reconcileSessionSelection(selected, refreshed), ['failed.jsonl']);
});

test('select all handles empty lists and repeated source paths without duplicating selection', () => {
  assert.deepEqual(toggleLoadedSessionSelection([], []), []);
  assert.deepEqual(toggleLoadedSessionSelection([], [makeSession('same.jsonl'), makeSession('same.jsonl')]), ['same.jsonl']);
});
