import assert from 'node:assert/strict';
import test from 'node:test';

import {
  bumpLetters,
  bumpPatch,
  nextBetaVersion,
  parseBetaStamp,
  shanghaiDayStamp,
} from '../../../scripts/next-beta-version.mjs';
import { writeTauriVersion } from '../../../scripts/set-tauri-version.mjs';

test('bumpPatch carries the .9 boundaries', () => {
  assert.equal(bumpPatch('1.2.0'), '1.2.1');
  assert.equal(bumpPatch('1.2.9'), '1.3.0');
  assert.equal(bumpPatch('1.9.9'), '2.0.0');
  assert.throws(() => bumpPatch('1.2.0-beta.1'), /plain semver/);
});

test('shanghaiDayStamp renders YYMMDD in the Beijing clock', () => {
  // 2026-10-24 17:06 UTC is already 2026-10-25 01:06 in Asia/Shanghai.
  assert.equal(shanghaiDayStamp(new Date('2026-10-24T17:06:00Z')), '261025');
  // Still the same Beijing day.
  assert.equal(shanghaiDayStamp(new Date('2026-10-24T01:06:00Z')), '261024');
});

test('parseBetaStamp extracts day and letters', () => {
  assert.deepEqual(parseBetaStamp('1.2.1-beta.261024a'), { day: '261024', letters: 'a' });
  assert.deepEqual(parseBetaStamp('1.2.1-beta.261024za'), { day: '261024', letters: 'za' });
  assert.equal(parseBetaStamp('1.2.1'), null);
  assert.equal(parseBetaVersion_stable(), null);
});

// A stable version has no stamp to parse; helper keeps the assertion readable.
function parseBetaVersion_stable() {
  return parseBetaStamp('1.2.1-beta.24a');
}

test('nextBetaVersion restarts at a for a new day or a new base', () => {
  const now = new Date('2026-10-24T08:00:00Z'); // 261024 in Beijing
  assert.equal(
    nextBetaVersion({ stableVersion: '1.2.0', currentBetaVersion: null, now }),
    '1.2.1-beta.261024a'
  );
  // Same day, same base: the letter rolls forward.
  assert.equal(
    nextBetaVersion({ stableVersion: '1.2.0', currentBetaVersion: '1.2.1-beta.261024a', now }),
    '1.2.1-beta.261024b'
  );
  // Same day but a new stable shipped (base moved): restart at a.
  assert.equal(
    nextBetaVersion({ stableVersion: '1.2.1', currentBetaVersion: '1.2.1-beta.261024a', now }),
    '1.2.2-beta.261024a'
  );
  // Next day with the same base: the day segment decides, restart at a.
  const nextDay = new Date('2026-10-25T08:00:00Z');
  assert.equal(
    nextBetaVersion({ stableVersion: '1.2.0', currentBetaVersion: '1.2.1-beta.261024z', now: nextDay }),
    '1.2.1-beta.261025a'
  );
});

test('the rolling stamp keeps dictionary order equal to time order', () => {
  // Every later build must compare greater as a plain string, which is what
  // semver does with alphanumeric prerelease identifiers.
  const sequence = ['261024a', '261024b', '261024z', '261024za', '261025a', '261101a', '270301a'];
  for (let index = 1; index < sequence.length; index += 1) {
    assert.ok(sequence[index] > sequence[index - 1], `${sequence[index]} > ${sequence[index - 1]}`);
  }
  assert.equal(bumpLetters('z'), 'za');
});

test('writeTauriVersion injects the version and preserves the rest', async () => {
  const { mkdtemp, readFile, rm, writeFile } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const path = await import('node:path');
  const directory = await mkdtemp(path.join(tmpdir(), 'tauri-version-'));
  const configPath = path.join(directory, 'tauri.conf.json');
  await writeFile(
    configPath,
    `${JSON.stringify({ productName: 'AI Toolbox', version: '1.2.0', identifier: 'com.example' }, null, 2)}\n`,
    'utf8'
  );

  const { previous, version } = await writeTauriVersion(configPath, '1.2.1-beta.261024a');
  assert.equal(previous, '1.2.0');
  assert.equal(version, '1.2.1-beta.261024a');

  const written = JSON.parse(await readFile(configPath, 'utf8'));
  assert.equal(written.version, '1.2.1-beta.261024a');
  assert.equal(written.productName, 'AI Toolbox');
  assert.equal(written.identifier, 'com.example');

  await assert.rejects(writeTauriVersion(configPath, 'not-a-version'), /valid semver/);
  await rm(directory, { recursive: true, force: true });
});
