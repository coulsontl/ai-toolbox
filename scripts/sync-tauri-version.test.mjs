import test from 'node:test';
import assert from 'node:assert/strict';

import {
  updateCargoLockVersion,
  updateCargoTomlVersion,
  updatePackageJsonVersion,
} from './sync-tauri-version.mjs';
import { writeCrateVersion } from './set-tauri-version.mjs';

test('updateCargoLockVersion keeps a single trailing newline unchanged', () => {
  const before = [
    '[[package]]',
    'name = "ai-toolbox"',
    'version = "0.7.6"',
    'dependencies = [',
    ' "anyhow",',
    ']',
    '',
  ].join('\n');

  const result = updateCargoLockVersion(before, '0.7.7');

  assert.equal(result.changed, true);
  assert.equal(result.previousVersion, '0.7.6');
  assert.equal(result.content, before.replace('0.7.6', '0.7.7'));
});

test('updateCargoTomlVersion preserves multiple trailing newlines exactly', () => {
  const before = '[package]\nname = "ai-toolbox"\nversion = "0.7.6"\n\n\n';

  const result = updateCargoTomlVersion(before, '0.7.7');

  assert.equal(result.changed, true);
  assert.equal(result.content, '[package]\nname = "ai-toolbox"\nversion = "0.7.7"\n\n\n');
});

test('updatePackageJsonVersion preserves missing trailing newline', () => {
  const before = '{\n  "name": "ai-toolbox",\n  "version": "0.7.6"\n}';

  const result = updatePackageJsonVersion(before, '0.7.7');

  assert.equal(result.changed, true);
  assert.equal(result.content, '{\n  "name": "ai-toolbox",\n  "version": "0.7.7"\n}');
});

test('updateCargoLockVersion can target the aitoolbox CLI package', () => {
  const before = [
    '[[package]]',
    'name = "ai-toolbox"',
    'version = "1.2.4"',
    '',
    '[[package]]',
    'name = "aitoolbox"',
    'version = "1.2.4"',
    '',
  ].join('\n');

  const result = updateCargoLockVersion(before, '1.2.5-beta.261128a', 'aitoolbox');

  assert.equal(result.changed, true);
  assert.equal(result.previousVersion, '1.2.4');
  assert.match(result.content, /name = "ai-toolbox"\nversion = "1\.2\.4"/);
  assert.match(result.content, /name = "aitoolbox"\nversion = "1\.2\.5-beta\.261128a"/);
});

test('writeCrateVersion updates both the manifest and its lockfile', async () => {
  const { mkdtemp, readFile, rm, writeFile } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const path = await import('node:path');
  const directory = await mkdtemp(path.join(tmpdir(), 'crate-version-'));
  const manifestPath = path.join(directory, 'Cargo.toml');
  const lockPath = path.join(directory, 'Cargo.lock');
  await writeFile(
    manifestPath,
    '[package]\nname = "aitoolbox"\nversion = "1.2.4"\n',
    'utf8',
  );
  await writeFile(
    lockPath,
    '[[package]]\nname = "aitoolbox"\nversion = "1.2.4"\n',
    'utf8',
  );

  const result = await writeCrateVersion(
    manifestPath,
    lockPath,
    'aitoolbox',
    '1.2.5-beta.261128a',
  );

  assert.equal(result.cargoTomlChanged, true);
  assert.equal(result.cargoLockChanged, true);
  assert.match(await readFile(manifestPath, 'utf8'), /version = "1\.2\.5-beta\.261128a"/);
  assert.match(await readFile(lockPath, 'utf8'), /version = "1\.2\.5-beta\.261128a"/);
  await rm(directory, { recursive: true, force: true });
});

test('updateCargoLockVersion is a no-op when version is already synced', () => {
  const before = [
    '[[package]]',
    'name = "ai-toolbox"',
    'version = "0.7.7"',
    '',
  ].join('\n');

  const result = updateCargoLockVersion(before, '0.7.7');

  assert.equal(result.changed, false);
  assert.equal(result.previousVersion, '0.7.7');
  assert.equal(result.content, before);
});
