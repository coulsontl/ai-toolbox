#!/usr/bin/env node
/**
 * Writes the given semver string into tauri/tauri.conf.json's `version`.
 *
 * Used by the release workflow to inject the rolling beta version right
 * before building, so the built executable, `app.getVersion()` and the
 * generated `latest.json` all carry it. The committed file always keeps the
 * stable version; the injection never lands in the repository.
 *
 * Usage: node scripts/set-tauri-version.mjs 1.2.1-beta.261024a
 */

import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const configPath = path.join(repositoryRoot, 'tauri', 'tauri.conf.json');

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

async function main() {
  const version = process.argv[2];
  if (!version) {
    console.error('Usage: node scripts/set-tauri-version.mjs <semver>');
    process.exit(1);
  }
  const { previous } = await writeTauriVersion(configPath, version);
  console.log(`tauri.conf.json version: ${previous} -> ${version}`);
}

if (process.argv[1] && process.argv[1].endsWith('set-tauri-version.mjs')) {
  await main();
}
