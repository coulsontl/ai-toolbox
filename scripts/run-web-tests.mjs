import { readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { run } from 'node:test';
import { spec } from 'node:test/reporters';

const currentFilePath = fileURLToPath(import.meta.url);
const scriptsDirectory = path.dirname(currentFilePath);
const projectRoot = path.resolve(scriptsDirectory, '..');
const webTestDirectory = path.join(projectRoot, 'web', 'test');

async function collectTestFiles(directoryPath) {
  let entries;
  try {
    entries = await readdir(directoryPath, { withFileTypes: true });
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
      return [];
    }
    throw error;
  }
  const collectedFiles = [];

  for (const entry of entries) {
    const entryPath = path.join(directoryPath, entry.name);
    if (entry.isDirectory()) {
      collectedFiles.push(...await collectTestFiles(entryPath));
      continue;
    }

    if (!entry.isFile()) {
      continue;
    }

    if (entry.name.endsWith('.test.ts') || entry.name.endsWith('.spec.ts')) {
      collectedFiles.push(entryPath);
    }
  }

  return collectedFiles;
}

const testFiles = (await collectTestFiles(webTestDirectory)).sort();

if (testFiles.length === 0) {
  console.log('No web tests found under web/test.');
  process.exit(0);
}

const testStream = run({
  files: testFiles,
  concurrency: true,
});

testStream.compose(spec).pipe(process.stdout);

const summary = await new Promise((resolve, reject) => {
  testStream.once('test:summary', resolve);
  testStream.once('error', reject);
});

if (summary.success === false) {
  process.exitCode = 1;
}
