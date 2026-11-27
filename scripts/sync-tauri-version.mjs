import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const currentFilePath = fileURLToPath(import.meta.url);
const scriptsDirectory = path.dirname(currentFilePath);
const projectRoot = path.resolve(scriptsDirectory, '..');

const packageJsonPath = path.join(projectRoot, 'package.json');
const tauriConfigPath = path.join(projectRoot, 'tauri', 'tauri.conf.json');
const cargoTomlPath = path.join(projectRoot, 'tauri', 'Cargo.toml');
const cargoLockPath = path.join(projectRoot, 'tauri', 'Cargo.lock');
const cliCargoTomlPath = path.join(projectRoot, 'cli', 'Cargo.toml');
const cliCargoLockPath = path.join(projectRoot, 'cli', 'Cargo.lock');

function detectLineEnding(fileContent) {
  return fileContent.includes('\r\n') ? '\r\n' : '\n';
}

function extractTrailingLineEndingSuffix(fileContent) {
  return fileContent.match(/(?:\r?\n)+$/)?.[0] ?? '';
}

function splitLines(fileContent) {
  const trailingLineEndingSuffix = extractTrailingLineEndingSuffix(fileContent);
  const contentWithoutTrailingLineEndings = trailingLineEndingSuffix
    ? fileContent.slice(0, -trailingLineEndingSuffix.length)
    : fileContent;

  if (contentWithoutTrailingLineEndings === '') {
    return [];
  }

  return contentWithoutTrailingLineEndings.split(/\r?\n/);
}

function joinLines(lines, lineEnding, trailingLineEndingSuffix) {
  const nextContent = lines.join(lineEnding);
  return `${nextContent}${trailingLineEndingSuffix}`;
}

export function updatePackageJsonVersion(packageJsonContent, tauriVersion) {
  const lineEnding = detectLineEnding(packageJsonContent);
  const trailingLineEndingSuffix = extractTrailingLineEndingSuffix(packageJsonContent);
  const lines = splitLines(packageJsonContent);

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const versionMatch = lines[lineIndex].match(/^(\s*"version"\s*:\s*")([^"]+)(".*)$/);
    if (!versionMatch) {
      continue;
    }

    const originalVersion = versionMatch[2];
    if (originalVersion === tauriVersion) {
      return {
        changed: false,
        previousVersion: originalVersion,
        content: packageJsonContent,
      };
    }

    lines[lineIndex] = `${versionMatch[1]}${tauriVersion}${versionMatch[3]}`;
    return {
      changed: true,
      previousVersion: originalVersion,
      content: joinLines(lines, lineEnding, trailingLineEndingSuffix),
    };
  }

  throw new Error('Failed to locate version in package.json');
}

export function updateCargoTomlVersion(cargoTomlContent, tauriVersion) {
  const lineEnding = detectLineEnding(cargoTomlContent);
  const trailingLineEndingSuffix = extractTrailingLineEndingSuffix(cargoTomlContent);
  const lines = splitLines(cargoTomlContent);
  let currentSectionName = '';

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const sectionMatch = lines[lineIndex].match(/^\s*\[([^\]]+)\]\s*$/);
    if (sectionMatch) {
      currentSectionName = sectionMatch[1];
      continue;
    }

    if (currentSectionName !== 'package') {
      continue;
    }

    const versionMatch = lines[lineIndex].match(/^(\s*version\s*=\s*")([^"]+)(".*)$/);
    if (!versionMatch) {
      continue;
    }

    const originalVersion = versionMatch[2];
    if (originalVersion === tauriVersion) {
      return {
        changed: false,
        previousVersion: originalVersion,
        content: cargoTomlContent,
      };
    }

    lines[lineIndex] = `${versionMatch[1]}${tauriVersion}${versionMatch[3]}`;
    return {
      changed: true,
      previousVersion: originalVersion,
      content: joinLines(lines, lineEnding, trailingLineEndingSuffix),
    };
  }

  throw new Error('Failed to locate [package].version in tauri/Cargo.toml');
}

export function updateCargoLockVersion(cargoLockContent, tauriVersion, packageName = 'ai-toolbox') {
  const lineEnding = detectLineEnding(cargoLockContent);
  const trailingLineEndingSuffix = extractTrailingLineEndingSuffix(cargoLockContent);
  const lines = splitLines(cargoLockContent);
  let isInsidePackageBlock = false;
  let currentPackageName = '';

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const line = lines[lineIndex];

    if (/^\s*\[\[package\]\]\s*$/.test(line)) {
      isInsidePackageBlock = true;
      currentPackageName = '';
      continue;
    }

    if (!isInsidePackageBlock) {
      continue;
    }

    const nameMatch = line.match(/^(\s*name\s*=\s*")([^"]+)(".*)$/);
    if (nameMatch) {
      currentPackageName = nameMatch[2];
      continue;
    }

    if (currentPackageName !== packageName) {
      continue;
    }

    const versionMatch = line.match(/^(\s*version\s*=\s*")([^"]+)(".*)$/);
    if (!versionMatch) {
      continue;
    }

    const originalVersion = versionMatch[2];
    if (originalVersion === tauriVersion) {
      return {
        changed: false,
        previousVersion: originalVersion,
        content: cargoLockContent,
      };
    }

    lines[lineIndex] = `${versionMatch[1]}${tauriVersion}${versionMatch[3]}`;
    return {
      changed: true,
      previousVersion: originalVersion,
      content: joinLines(lines, lineEnding, trailingLineEndingSuffix),
    };
  }

  throw new Error(`Failed to locate ${packageName} version in the Cargo.lock`);
}

export async function syncVersionFromTauriConfig() {
  const tauriConfigContent = await readFile(tauriConfigPath, 'utf8');
  const tauriConfig = JSON.parse(tauriConfigContent);
  const tauriVersion = tauriConfig.version;

  if (typeof tauriVersion !== 'string' || tauriVersion.trim() === '') {
    throw new Error('tauri.conf.json version is missing or invalid');
  }

  const packageJsonContent = await readFile(packageJsonPath, 'utf8');
  const packageJsonUpdate = updatePackageJsonVersion(packageJsonContent, tauriVersion);
  if (packageJsonUpdate.changed) {
    await writeFile(packageJsonPath, packageJsonUpdate.content, 'utf8');
    console.log(`Synced package.json version: ${packageJsonUpdate.previousVersion} -> ${tauriVersion}`);
  }

  const cargoTomlContent = await readFile(cargoTomlPath, 'utf8');
  const cargoTomlUpdate = updateCargoTomlVersion(cargoTomlContent, tauriVersion);
  if (cargoTomlUpdate.changed) {
    await writeFile(cargoTomlPath, cargoTomlUpdate.content, 'utf8');
    console.log(`Synced tauri/Cargo.toml version: ${cargoTomlUpdate.previousVersion} -> ${tauriVersion}`);
  }

  const cargoLockContent = await readFile(cargoLockPath, 'utf8');
  const cargoLockUpdate = updateCargoLockVersion(cargoLockContent, tauriVersion);
  if (cargoLockUpdate.changed) {
    await writeFile(cargoLockPath, cargoLockUpdate.content, 'utf8');
    console.log(`Synced tauri/Cargo.lock version: ${cargoLockUpdate.previousVersion} -> ${tauriVersion}`);
  }

  // The `aitoolbox` CLI ships inside the installer, so `aitoolbox --version`
  // must match the desktop app version.
  const cliCargoTomlContent = await readFile(cliCargoTomlPath, 'utf8');
  const cliCargoTomlUpdate = updateCargoTomlVersion(cliCargoTomlContent, tauriVersion);
  if (cliCargoTomlUpdate.changed) {
    await writeFile(cliCargoTomlPath, cliCargoTomlUpdate.content, 'utf8');
    console.log(`Synced cli/Cargo.toml version: ${cliCargoTomlUpdate.previousVersion} -> ${tauriVersion}`);
  }

  const cliCargoLockContent = await readFile(cliCargoLockPath, 'utf8');
  const cliCargoLockUpdate = updateCargoLockVersion(cliCargoLockContent, tauriVersion, 'aitoolbox');
  if (cliCargoLockUpdate.changed) {
    await writeFile(cliCargoLockPath, cliCargoLockUpdate.content, 'utf8');
    console.log(`Synced cli/Cargo.lock version: ${cliCargoLockUpdate.previousVersion} -> ${tauriVersion}`);
  }

  if (
    !packageJsonUpdate.changed &&
    !cargoTomlUpdate.changed &&
    !cargoLockUpdate.changed &&
    !cliCargoTomlUpdate.changed &&
    !cliCargoLockUpdate.changed
  ) {
    console.log(`Versions already in sync: ${tauriVersion}`);
  }
}

const invokedScriptPath = process.argv[1] ? path.resolve(process.argv[1]) : null;

if (invokedScriptPath === currentFilePath) {
  syncVersionFromTauriConfig().catch((error) => {
    console.error(`Failed to sync Tauri version: ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  });
}
