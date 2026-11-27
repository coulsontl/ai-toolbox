import assert from 'node:assert/strict';
import test from 'node:test';

import type { CodexHistoryProviderGroup } from '../../../../../types/codex.ts';
import {
  countCodexHistorySelectedGroups,
  defaultCodexHistorySyncSelection,
  emptyCodexHistorySyncSelection,
  hasCrossUpstreamCodexHistorySelection,
  isCodexHistoryGroupSelectable,
  isCodexHistorySyncSelectionEmpty,
  setCodexHistoryMissingProviderSelected,
  setCodexHistoryProviderSelected,
} from '../../../../../features/coding/codex/utils/codexHistorySyncSelection.ts';

const group = (
  providerId: string | null,
  kind: CodexHistoryProviderGroup['kind'],
  threadCount = 1,
  sessionFileCount = 1,
): CodexHistoryProviderGroup => ({
  providerId,
  kind,
  threadCount,
  sessionFileCount,
});

const groups: CodexHistoryProviderGroup[] = [
  group('ai-toolbox-gateway', 'ai_toolbox_legacy', 3, 2),
  group('aitb-launch-abc', 'ai_toolbox_legacy', 1, 1),
  group(null, 'missing', 2, 1),
  group('openai', 'official', 4, 4),
  group('third-party', 'other', 1, 1),
  group('custom', 'current', 8, 8),
];

test('default selection only preselects AI Toolbox legacy buckets', () => {
  const selection = defaultCodexHistorySyncSelection(groups);

  assert.deepEqual(selection.sourceProviders, ['ai-toolbox-gateway', 'aitb-launch-abc']);
  assert.equal(selection.includeMissingProvider, false);
  assert.equal(countCodexHistorySelectedGroups(selection), 2);
});

test('missing and empty status select nothing', () => {
  assert.deepEqual(defaultCodexHistorySyncSelection(null), emptyCodexHistorySyncSelection());
  assert.deepEqual(defaultCodexHistorySyncSelection([]), emptyCodexHistorySyncSelection());
  assert.equal(isCodexHistorySyncSelectionEmpty(emptyCodexHistorySyncSelection()), true);
});

test('the active provider group is never selectable', () => {
  assert.equal(isCodexHistoryGroupSelectable(group('custom', 'current')), false);
  for (const kind of ['ai_toolbox_legacy', 'missing', 'official', 'other'] as const) {
    assert.equal(isCodexHistoryGroupSelectable(group('x', kind)), true);
  }
});

test('provider toggles are idempotent and ignore blank ids', () => {
  const initial = emptyCodexHistorySyncSelection();
  const added = setCodexHistoryProviderSelected(initial, ' openai ', true);
  assert.deepEqual(added.sourceProviders, ['openai']);
  assert.equal(setCodexHistoryProviderSelected(added, 'openai', true), added);
  assert.equal(setCodexHistoryProviderSelected(added, '   ', true), added);

  const removed = setCodexHistoryProviderSelected(added, 'openai', false);
  assert.deepEqual(removed.sourceProviders, []);
  assert.equal(setCodexHistoryProviderSelected(removed, 'openai', false), removed);
});

test('missing provider is an independent toggle', () => {
  const withMissing = setCodexHistoryMissingProviderSelected(
    emptyCodexHistorySyncSelection(),
    true,
  );
  assert.deepEqual(withMissing, { sourceProviders: [], includeMissingProvider: true });
  assert.equal(isCodexHistorySyncSelectionEmpty(withMissing), false);
  assert.equal(countCodexHistorySelectedGroups(withMissing), 1);
  assert.equal(
    setCodexHistoryMissingProviderSelected(withMissing, true),
    withMissing,
  );
});

test('only official and other selections are cross-upstream', () => {
  const legacy = defaultCodexHistorySyncSelection(groups);
  assert.equal(hasCrossUpstreamCodexHistorySelection(legacy, groups), false);

  assert.equal(
    hasCrossUpstreamCodexHistorySelection(
      setCodexHistoryProviderSelected(legacy, 'openai', true),
      groups,
    ),
    true,
  );
  assert.equal(
    hasCrossUpstreamCodexHistorySelection(
      setCodexHistoryProviderSelected(legacy, 'third-party', true),
      groups,
    ),
    true,
  );
  assert.equal(
    hasCrossUpstreamCodexHistorySelection(
      setCodexHistoryMissingProviderSelected(legacy, true),
      groups,
    ),
    false,
  );
});
