import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const currentFilePath = fileURLToPath(import.meta.url);
const scriptsDirectory = path.dirname(currentFilePath);
const projectRoot = path.resolve(scriptsDirectory, '..');

const fixtureMappings = [
  {
    source: ['llm', 'transformer', 'anthropic', 'testdata'],
    target: ['tauri', 'src', 'coding', 'proxy_gateway', 'transformer', 'fixtures', 'reference', 'anthropic'],
  },
  {
    source: ['llm', 'transformer', 'openai', 'testdata'],
    target: ['tauri', 'src', 'coding', 'proxy_gateway', 'transformer', 'fixtures', 'reference', 'openai_chat'],
  },
  {
    source: ['llm', 'transformer', 'openai', 'responses', 'testdata'],
    target: ['tauri', 'src', 'coding', 'proxy_gateway', 'transformer', 'fixtures', 'reference', 'openai_responses'],
  },
  {
    source: ['llm', 'transformer', 'gemini', 'testdata'],
    target: ['tauri', 'src', 'coding', 'proxy_gateway', 'transformer', 'fixtures', 'reference', 'gemini'],
  },
];

function parseArgs(argv) {
  const options = {
    // No baked-in default: the AxonHub checkout is a local reference repo, not
    // something every contributor has at the same path. Pass --axonhub or set
    // AXONHUB_ROOT.
    axonhubRoot: process.env.AXONHUB_ROOT ?? '',
    write: false,
    prune: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--write') {
      options.write = true;
      continue;
    }
    if (arg === '--prune') {
      options.prune = true;
      continue;
    }
    if (arg === '--axonhub') {
      const value = argv[index + 1];
      if (!value) {
        throw new Error('--axonhub requires a path');
      }
      options.axonhubRoot = value;
      index += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }

  return options;
}

function isFixtureFile(filePath) {
  return filePath.endsWith('.json') || filePath.endsWith('.jsonl');
}

async function collectFixtureFiles(rootDirectory) {
  const files = [];

  async function visit(directory, relativeDirectory) {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      const relativePath = path.join(relativeDirectory, entry.name);
      const absolutePath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        await visit(absolutePath, relativePath);
        continue;
      }
      if (entry.isFile() && isFixtureFile(entry.name)) {
        files.push({
          relativePath: relativePath.split(path.sep).join('/'),
          absolutePath,
        });
      }
    }
  }

  await visit(rootDirectory, '');
  return files;
}

async function hashFile(filePath) {
  const content = await readFile(filePath);
  return createHash('sha256').update(content).digest('hex');
}

async function pathExists(filePath) {
  try {
    await readFile(filePath);
    return true;
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

async function syncMapping(options, mapping) {
  const sourceDirectory = path.join(options.axonhubRoot, ...mapping.source);
  const targetDirectory = path.join(projectRoot, ...mapping.target);
  const sourceFiles = await collectFixtureFiles(sourceDirectory);
  const targetFiles = await collectFixtureFiles(targetDirectory);
  const targetFileSet = new Set(targetFiles.map((file) => file.relativePath));
  const sourceFileSet = new Set(sourceFiles.map((file) => file.relativePath));
  const operations = [];

  for (const sourceFile of sourceFiles) {
    const targetPath = path.join(targetDirectory, sourceFile.relativePath);
    const targetExists = await pathExists(targetPath);
    if (!targetExists) {
      operations.push({ type: 'add', source: sourceFile.absolutePath, target: targetPath });
      continue;
    }
    const [sourceHash, targetHash] = await Promise.all([
      hashFile(sourceFile.absolutePath),
      hashFile(targetPath),
    ]);
    if (sourceHash !== targetHash) {
      operations.push({ type: 'update', source: sourceFile.absolutePath, target: targetPath });
    }
  }

  if (options.prune) {
    for (const targetFile of targetFiles) {
      if (!sourceFileSet.has(targetFile.relativePath)) {
        operations.push({ type: 'remove', target: targetFile.absolutePath });
      }
    }
  }

  if (options.write) {
    for (const operation of operations) {
      if (operation.type === 'remove') {
        await rm(operation.target);
        continue;
      }
      await mkdir(path.dirname(operation.target), { recursive: true });
      const content = await readFile(operation.source);
      await writeFile(operation.target, content);
    }
  }

  return {
    label: mapping.target.at(-1),
    sourceCount: sourceFiles.length,
    targetCount: targetFileSet.size,
    operations,
  };
}

function printResult(result, options) {
  const mode = options.write ? 'write' : 'dry-run';
  console.log(`${result.label}: ${result.sourceCount} source fixtures, ${result.targetCount} target fixtures, ${result.operations.length} ${mode} operations`);
  for (const operation of result.operations) {
    const target = path.relative(projectRoot, operation.target).split(path.sep).join('/');
    console.log(`  ${operation.type.padEnd(6)} ${target}`);
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (!options.axonhubRoot) {
    throw new Error(
      'Missing AxonHub checkout path. Pass --axonhub <path> or set AXONHUB_ROOT.'
    );
  }
  const results = [];
  for (const mapping of fixtureMappings) {
    results.push(await syncMapping(options, mapping));
  }
  for (const result of results) {
    printResult(result, options);
  }
  const operationCount = results.reduce((sum, result) => sum + result.operations.length, 0);
  if (!options.write && operationCount > 0) {
    console.log('Dry run only. Re-run with --write to apply fixture updates; add --prune to remove stale local fixtures.');
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
