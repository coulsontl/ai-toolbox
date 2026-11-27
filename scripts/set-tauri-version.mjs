#!/usr/bin/env node
/**
 * Writes the given semver string into tauri/tauri.conf.json's `version`, and
 * the same version into the `aitoolbox` CLI crate (`cli/Cargo.toml` +
 * `cli/Cargo.lock`) so the bundled CLI reports the app version.
 *
 * Used by the release workflow to inject the rolling beta version right
 * before building, so the built executable, `app.getVersion()`,
 * `aitoolbox --version` and the generated `latest.json` all carry it. The
 * committed files always keep the stable version; the injection never lands
 * in the repository.
 *
 * Usage: node scripts/set-tauri-version.mjs 1.2.1-beta.261024a
 */

import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { updateCargoLockVersion, updateCargoTomlVersion } from './sync-tauri-version.mjs';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const configPath = path.join(repositoryRoot, 'tauri', 'tauri.conf.json');
const cliCargoTomlPath = path.join(repositoryRoot, 'cli', 'Cargo.toml');
const cliCargoLockPath = path.join(repositoryRoot, 'cli', 'Cargo.lock');

const SEMVER_PATTERN = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

export async function writeTauriVersion(configPath_, version) {
  if (!SEMVER_PATTERN.test(version)) {
    throw new Error(`Not a valid semver version: ${version}`);
  }
  const raw = await readFile(configPath_, 'utf8');
  const config = JSON.parse(raw);
  const previous = config.version;
  config.version = version;
  await writeFile(configPath_, `${JSON.stringify(config, null, 2)}\n`, 'utf8');
  return { previous, version };
}

/** Updates `[package].version` and the matching lockfile package entry. */
export async function writeCrateVersion(cargoTomlPath, cargoLockPath, packageName, version) {
  if (!SEMVER_PATTERN.test(version)) {
    throw new Error(`Not a valid semver version: ${version}`);
  }
  const cargoTomlContent = await readFile(cargoTomlPath, 'utf8');
  const cargoTomlUpdate = updateCargoTomlVersion(cargoTomlContent, version);
  if (cargoTomlUpdate.changed) {
    await writeFile(cargoTomlPath, cargoTomlUpdate.content, 'utf8');
  }

  const cargoLockContent = await readFile(cargoLockPath, 'utf8');
  const cargoLockUpdate = updateCargoLockVersion(cargoLockContent, version, packageName);
  if (cargoLockUpdate.changed) {
    await writeFile(cargoLockPath, cargoLockUpdate.content, 'utf8');
  }

  return {
    previous: cargoTomlUpdate.previousVersion,
    cargoTomlChanged: cargoTomlUpdate.changed,
    cargoLockChanged: cargoLockUpdate.changed,
  };
}

async function main() {
  const version = process.argv[2];
  if (!version) {
    console.error('Usage: node scripts/set-tauri-version.mjs <semver>');
    process.exit(1);
  }
  const { previous } = await writeTauriVersion(configPath, version);
  console.log(`tauri.conf.json version: ${previous} -> ${version}`);
  await writeCrateVersion(cliCargoTomlPath, cliCargoLockPath, 'aitoolbox', version);
  console.log(`cli/Cargo.toml + cli/Cargo.lock version: ${version}`);
}

if (process.argv[1] && process.argv[1].endsWith('set-tauri-version.mjs')) {
  await main();
}
