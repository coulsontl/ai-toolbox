/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';
import { Worker } from 'node:worker_threads';

import { INVALID_UNCLOSED_DOUBLE_QUOTE_STRING_PATTERN } from '../../../../components/common/TomlEditor/invalidDoubleQuoteStringPattern.ts';

/**
 * Monarch applies rules from the current cursor, so a match only counts when it
 * starts at index 0. `RegExp#test` alone is wrong here: on a closed string the
 * trailing `"` would still match the pattern from mid-string.
 */
function matchesFromStart(pattern: RegExp, input: string): boolean {
  const flags = pattern.flags.includes('g') ? pattern.flags : `${pattern.flags}g`;
  const stickyPattern = new RegExp(pattern.source, flags);
  stickyPattern.lastIndex = 0;
  const match = stickyPattern.exec(input);
  return match !== null && match.index === 0;
}

function matchesFromStartInWorker(
  pattern: RegExp,
  input: string,
  timeoutMs = 2_000,
): Promise<boolean> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(`
      const { parentPort, workerData } = require('node:worker_threads');
      const flags = workerData.flags.replaceAll('g', '').replaceAll('y', '') + 'y';
      const pattern = new RegExp(workerData.source, flags);
      parentPort.postMessage(pattern.test(workerData.input));
    `, {
      eval: true,
      workerData: { source: pattern.source, flags: pattern.flags, input },
    });

    const timeout = setTimeout(() => {
      void worker.terminate();
      reject(new Error(`TOML tokenizer pattern exceeded ${timeoutMs}ms`));
    }, timeoutMs);

    worker.once('message', (matched: boolean) => {
      clearTimeout(timeout);
      void worker.terminate();
      resolve(matched);
    });
    worker.once('error', (error) => {
      clearTimeout(timeout);
      void worker.terminate();
      reject(error);
    });
  });
}

test('matches unclosed double-quoted strings from the start of the remaining input', () => {
  assert.equal(matchesFromStart(INVALID_UNCLOSED_DOUBLE_QUOTE_STRING_PATTERN, '"hello'), true);
  assert.equal(
    matchesFromStart(INVALID_UNCLOSED_DOUBLE_QUOTE_STRING_PATTERN, '"C:\\\\Users\\\\Admin'),
    true,
  );
});

test('does not match properly closed double-quoted strings from the start', () => {
  assert.equal(matchesFromStart(INVALID_UNCLOSED_DOUBLE_QUOTE_STRING_PATTERN, '"hello"'), false);
  assert.equal(
    matchesFromStart(
      INVALID_UNCLOSED_DOUBLE_QUOTE_STRING_PATTERN,
      '"C:\\\\Users\\\\Admin\\\\hook.exe"',
    ),
    false,
  );
});

test('does not backtrack exponentially on an adversarial closed notify-style line', async () => {
  // The legacy alternatives can each consume `\a`, producing exponential paths before failure.
  const closedBackslashHeavy = `"${String.raw`\a`.repeat(128)}" ]`;

  assert.equal(
    await matchesFromStartInWorker(
      INVALID_UNCLOSED_DOUBLE_QUOTE_STRING_PATTERN,
      closedBackslashHeavy,
    ),
    false,
  );
});
