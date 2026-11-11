import { readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { run } from 'node:test';
import { spec } from 'node:test/reporters';

const currentFilePath = fileURLToPath(import.meta.url);
const scriptsDirectory = path.dirname(currentFilePath);
const projectRoot = path.resolve(scriptsDirectory, '..');
const webTestDirectory = path.join(projectRoot, 'web', 'test');
const typeScriptExtensionRegisterUrl = pathToFileURL(
  path.join(scriptsDirectory, 'register-node-ts-extension-loader.mjs'),
).href;

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

    // The modelsJson source-contract tests predate the TypeScript runner and
    // are plain .test.mjs files; include them so `pnpm test` actually enforces
    // the regressions AGENTS.md points at.
    if (
      entry.name.endsWith('.test.ts')
      || entry.name.endsWith('.spec.ts')
      || entry.name.endsWith('.test.mjs')
    ) {
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
  execArgv: ['--import', typeScriptExtensionRegisterUrl],
});

// Node emits per-file summaries before the final run summary. A passing first
// file must not hide a failure reported by another worker later in the run.
testStream.on('test:fail', () => {
  process.exitCode = 1;
});

// File-level completeness guard: test events carry the owning file, and the
// runner has been observed (Windows, Node 22) to silently end a run with one
// discovered file contributing zero test points while still passing. Detect
// that class of false pass instead of trusting the aggregate summary.
const filesWithTestEvents = new Set();
const recordFileActivity = (event) => {
  if (typeof event?.file === 'string') {
    filesWithTestEvents.add(path.resolve(event.file));
  }
};
for (const eventName of ['test:start', 'test:pass', 'test:fail', 'test:complete']) {
  testStream.on(eventName, recordFileActivity);
}

testStream.compose(spec).pipe(process.stdout);

await new Promise((resolve, reject) => {
  testStream.once('end', resolve);
  testStream.once('error', reject);
});

const filesWithoutTestEvents = testFiles.filter((file) => !filesWithTestEvents.has(path.resolve(file)));
if (filesWithoutTestEvents.length > 0) {
  console.error(
    `Web test runner finished without running any tests from ${filesWithoutTestEvents.length} file(s): `
    + `${filesWithoutTestEvents.join(', ')}`,
  );
  process.exitCode = 1;
}
