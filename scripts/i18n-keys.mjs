import { mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as ts from 'typescript';

const currentFilePath = fileURLToPath(import.meta.url);
const projectRoot = path.resolve(path.dirname(currentFilePath), '..');

const DEFAULT_LOCALE_FILES = [
  path.join('web', 'i18n', 'locales', 'zh-CN.json'),
  path.join('web', 'i18n', 'locales', 'en-US.json'),
];
const I18N_WRITE_LOCK_DIR = path.join('.tmp', 'i18n-keys-write.lock');
const I18N_WRITE_LOCK_TIMEOUT_MS = 30_000;
const I18N_WRITE_LOCK_RETRY_MS = 100;

const SCAN_ROOTS = [
  'web/app',
  'web/components',
  'web/constants',
  'web/features',
  'web/hooks',
  'web/services',
  'web/stores',
  'web/types',
  'web/utils',
  'tauri/src',
];

const TYPESCRIPT_SOURCE_EXTENSIONS = new Set(['.ts', '.tsx', '.js', '.jsx']);
const RUST_SOURCE_EXTENSIONS = new Set(['.rs']);
const SOURCE_EXTENSIONS = new Set([
  ...TYPESCRIPT_SOURCE_EXTENSIONS,
  ...RUST_SOURCE_EXTENSIONS,
]);

const DEFAULT_DYNAMIC_PROTECTED_PREFIXES = [
  'subModules.',
  'settings.gateway.cli.',
  'settings.gateway.cliStatus.',
  'gateway.page.statistics.range.',
  'gateway.page.requests.detailTabs.',
  'gateway.takeover.state.',
  'gateway.failover.mode.',
  'image.modes.',
  'image.status.',
  'image.sizePicker.modes.',
  'skills.enabledFilter.',
  'opencode.ohMyOpenCode.agentsMeta.',
  'opencode.ohMyOpenCode.categoriesMeta.',
  'opencode.ohMyOpenCodeSlim.agents.',
  // Rendered by the shared CliExtensionsSection as
  // `${i18nPrefix}.extensions.${descriptionKeySuffix}` — two stacked variables
  // the scanner cannot expand, so the family needs blanket protection (Pi's
  // recommended-extension descriptions live here).
  'pi.extensions.',
  'settings.backup.builtinMappings.',
];

const DEFAULT_DYNAMIC_IDENTIFIER_VALUES_BY_FILE = {
  'web/components/common/ModelFormModal/index.tsx': {
    i18nPrefix: ['opencode'],
  },
  'web/components/common/ModelItem/index.tsx': {
    i18nPrefix: ['opencode', 'openclaw'],
  },
  'web/components/common/OfficialProviderCard/index.tsx': {
    // OpenCode mounts it with the default 'opencode'; OmO Native's builtin
    // providers section mounts it with 'omoNative'. A prefix missing here
    // made `i18n:prune` delete the omoNative copy the card still renders.
    i18nPrefix: ['opencode', 'omoNative'],
  },
  'web/features/coding/shared/prompt/GlobalPromptConfigCard.tsx': {
    translationKeyPrefix: ['claudecode.prompt', 'codex.prompt', 'geminicli.prompt', 'opencode.prompt'],
  },
  'web/features/coding/shared/prompt/GlobalPromptConfigModal.tsx': {
    translationKeyPrefix: ['claudecode.prompt', 'codex.prompt', 'geminicli.prompt', 'opencode.prompt'],
  },
  'web/features/coding/shared/prompt/GlobalPromptSettings.tsx': {
    translationKeyPrefix: ['claudecode.prompt', 'codex.prompt', 'geminicli.prompt', 'opencode.prompt'],
  },
  // Every module that renders the shared root-directory modal. A prefix missing
  // here makes that module's whole `rootPathSource.*` block look unused, and
  // `i18n:prune` then deletes copy the modal still asks for at runtime.
  'web/features/coding/shared/useRootDirectoryConfig.ts': {
    translationKeyPrefix: [
      'antigravity',
      'claudecode',
      'codex',
      'dsh',
      'geminicli',
      'grok',
      'hermes',
      'kimi',
      'ohMyPi',
      'omoNative',
      'pi',
      'zcode',
    ],
    // The second placeholder in
    // `${translationKeyPrefix}.rootPathSource.modal.${pathInfo.source}Hint`.
    // Without it the expression never expands and `envHint` / `shellHint` look
    // unused — `i18n:prune` then deletes copy the modal renders.
    'pathInfo.source': ['env', 'shell'],
  },
};

const PLURAL_SUFFIXES = ['zero', 'one', 'two', 'few', 'many', 'other'];
const UNKNOWN_CONSTANT = Symbol('unknown constant');

const HELP_TEXT = `Usage:
  node scripts/i18n-keys.mjs check [--root dir] [--locale-files a,b] [--scan-roots a,b]
  node scripts/i18n-keys.mjs report [--json] [--root dir] [--locale-files a,b] [--scan-roots a,b]
  node scripts/i18n-keys.mjs audit-unused [--json] [--root dir] [--locale-files a,b] [--scan-roots a,b]
  node scripts/i18n-keys.mjs prune [--prefix key.prefix | --all-confirmed] [--write] [--root dir] [--locale-files a,b] [--scan-roots a,b]
  node scripts/i18n-keys.mjs set-key <key> --zh-CN text --en-US text [--write] [--allow-overwrite] [--root dir] [--locale-files a,b]
  node scripts/i18n-keys.mjs find-text <text> [--locale zh-CN] [--root dir] [--locale-files a,b] [--scan-roots a,b]
  node scripts/i18n-keys.mjs find-key <key-or-prefix> [--root dir] [--locale-files a,b] [--scan-roots a,b]

Commands:
  check       Fails when static i18n usages are missing from any locale, or locale key sets differ.
  report      Prints used keys, unused keys, missing keys, locale mismatches, and dynamic calls.
  audit-unused Prints unused-key audit results, including exact source literal blockers.
  prune       Removes high-confidence unused keys under --prefix, or all audited confirmed unused keys with --all-confirmed.
  set-key     Adds or updates one locale key in every configured locale file. Requires --write to update files.
  find-text   Finds locale keys by translated text.
  find-key    Finds locale values and code usage locations by key or prefix.

Options:
  --root         Project root to scan. Defaults to the repository root.
  --locale-files Comma-separated locale file paths relative to --root.
  --scan-roots  Comma-separated source directories relative to --root.
  --all-confirmed Prune every unused key confirmed by audit. Requires --write to update files.
  --allow-overwrite Allow set-key to replace an existing value that differs from the provided value.
`;

export async function analyzeProject(options = {}) {
  const rootDirectory = options.rootDirectory ?? projectRoot;
  const localeFilePaths = options.localeFilePaths ?? DEFAULT_LOCALE_FILES;
  const scanRoots = options.scanRoots ?? SCAN_ROOTS;
  const dynamicProtectedPrefixes = [
    ...DEFAULT_DYNAMIC_PROTECTED_PREFIXES,
    ...(options.dynamicProtectedPrefixes ?? []),
  ];
  const dynamicIdentifierValuesByFile = mergeDynamicIdentifierValuesByFile(
    DEFAULT_DYNAMIC_IDENTIFIER_VALUES_BY_FILE,
    options.dynamicIdentifierValuesByFile ?? {},
  );

  const localeFiles = await readLocaleFiles(rootDirectory, localeFilePaths);
  const allActualLocaleKeys = new Set(localeFiles.flatMap((localeFile) => localeFile.entries.map((entry) => entry.key)));
  const allCanonicalLocaleKeys = new Set([...allActualLocaleKeys].map(canonicalizeLocaleKey));
  const sourceFiles = await collectSourceFiles(rootDirectory, scanRoots);
  const sourceAnalysis = await analyzeSourceFiles(sourceFiles, allActualLocaleKeys);
  const expandedDynamicKeyUsages = expandDynamicKeyUsages(
    sourceAnalysis.dynamicUsages,
    dynamicIdentifierValuesByFile,
  );
  const localeKeysByLocale = new Map(
    localeFiles.map((localeFile) => [localeFile.locale, new Set(localeFile.entries.map((entry) => entry.key))]),
  );
  const canonicalLocaleKeysByLocale = new Map(
    localeFiles.map((localeFile) => [
      localeFile.locale,
      new Set(localeFile.entries.map((entry) => canonicalizeLocaleKey(entry.key))),
    ]),
  );
  const usedKeys = new Set([
    ...sourceAnalysis.staticUsages.map((usage) => usage.key),
    ...expandedDynamicKeyUsages.map((usage) => usage.key),
  ]);
  const protectedPredicates = buildProtectedPredicates(sourceAnalysis.dynamicUsages, dynamicProtectedPrefixes);

  const missingStaticKeys = [];
  const checkedUsages = [
    ...sourceAnalysis.staticUsages,
    ...expandedDynamicKeyUsages,
  ];
  for (const usage of checkedUsages) {
    for (const localeFile of localeFiles) {
      if (!hasLocaleKey(localeKeysByLocale.get(localeFile.locale), usage.key)) {
        missingStaticKeys.push({
          key: usage.key,
          locale: localeFile.locale,
          filePath: usage.filePath,
          line: usage.line,
          column: usage.column,
        });
      }
    }
  }

  const localeMismatches = [];
  for (const localeFile of localeFiles) {
    const ownKeys = canonicalLocaleKeysByLocale.get(localeFile.locale) ?? new Set();
    for (const key of allCanonicalLocaleKeys) {
      if (!ownKeys.has(key)) {
        localeMismatches.push({ key, locale: localeFile.locale });
      }
    }
  }

  const usageLocationsByKey = groupUsagesByKey(checkedUsages);
  const unusedLocaleKeys = [];
  for (const key of [...allActualLocaleKeys].sort()) {
    if (usedKeys.has(canonicalizeLocaleKey(key))) {
      continue;
    }

    const protectedBy = findProtectionReason(key, protectedPredicates);
    unusedLocaleKeys.push({
      key,
      protected: Boolean(protectedBy),
      protectedBy,
      locales: localeFiles
        .filter((localeFile) => localeKeysByLocale.get(localeFile.locale)?.has(key))
        .map((localeFile) => localeFile.locale),
    });
  }

  return {
    rootDirectory,
    localeFiles,
    sourceFiles,
    usedKeys: [...usedKeys].sort(),
    literalUsages: sourceAnalysis.literalUsages,
    staticUsages: sourceAnalysis.staticUsages,
    dynamicUsages: sourceAnalysis.dynamicUsages,
    parseErrors: sourceAnalysis.parseErrors,
    expandedDynamicKeyUsages,
    unresolvedDynamicUsages: sourceAnalysis.dynamicUsages.filter((usage) => usage.protectPrefix === ''),
    missingStaticKeys,
    localeMismatches,
    unusedLocaleKeys,
    removableUnusedKeys: unusedLocaleKeys.filter((entry) => !entry.protected),
    usageLocationsByKey,
  };
}

function hasLocaleKey(localeKeys, key) {
  if (!localeKeys) {
    return false;
  }
  if (localeKeys.has(key)) {
    return true;
  }
  return PLURAL_SUFFIXES.some((suffix) => localeKeys.has(`${key}_${suffix}`));
}

function canonicalizeLocaleKey(key) {
  for (const suffix of PLURAL_SUFFIXES) {
    const suffixText = `_${suffix}`;
    if (key.endsWith(suffixText)) {
      return key.slice(0, -suffixText.length);
    }
  }
  return key;
}

export async function pruneUnusedKeys(options = {}) {
  const analysis = options.analysis ?? await analyzeProject(options);
  const prefixes = normalizePrefixList(options.prefixes ?? []);
  const auditEntries = auditUnusedKeys(analysis);
  const candidateKeys = options.allConfirmed
    ? auditEntries
      .filter((entry) => entry.status === 'confirmed-unused')
      .map((entry) => entry.key)
    : prefixes.length === 0
      ? []
      : analysis.removableUnusedKeys.map((entry) => entry.key);
  const keysToRemove = new Set(
    candidateKeys.filter((key) => prefixes.length === 0 || matchesAnyPrefix(key, prefixes)),
  );

  if (keysToRemove.size === 0) {
    return { analysis, removedKeys: [] };
  }

  if (!options.write) {
    return {
      analysis,
      removedKeys: [...keysToRemove].sort(),
    };
  }

  if (options.write) {
    await withLocaleWriteLock(analysis.rootDirectory, async () => {
      const currentLocaleFiles = await readLocaleFiles(
        analysis.rootDirectory,
        analysis.localeFiles.map((localeFile) => localeFile.relativePath),
      );
      for (const localeFile of currentLocaleFiles) {
        for (const key of keysToRemove) {
          deleteNestedKey(localeFile.data, key);
        }

        await writeLocaleFile(localeFile);
      }
    });
  } else {
    for (const localeFile of analysis.localeFiles) {
      for (const key of keysToRemove) {
        deleteNestedKey(localeFile.data, key);
      }
    }
  }

  return {
    analysis,
    removedKeys: [...keysToRemove].sort(),
  };
}

export async function setLocaleKey(options = {}) {
  const rootDirectory = options.rootDirectory ?? projectRoot;
  const localeFilePaths = options.localeFilePaths ?? DEFAULT_LOCALE_FILES;
  const key = String(options.key ?? '').trim();
  const valuesByLocale = options.valuesByLocale ?? {};

  if (!key) {
    throw new Error('set-key requires a locale key.');
  }
  if (key.split('.').some((part) => part.trim() === '')) {
    throw new Error(`Invalid locale key: ${key}`);
  }

  const applySetKey = async () => {
    const localeFiles = await readLocaleFiles(rootDirectory, localeFilePaths);
    const missingLocales = localeFiles
      .map((localeFile) => localeFile.locale)
      .filter((locale) => !Object.prototype.hasOwnProperty.call(valuesByLocale, locale));
    if (missingLocales.length > 0) {
      throw new Error(`set-key requires values for locale(s): ${missingLocales.map((locale) => `--${locale}`).join(', ')}`);
    }

    const changes = [];
    const conflicts = [];

    for (const localeFile of localeFiles) {
      const nextValue = valuesByLocale[localeFile.locale];
      const previousValue = getNestedKey(localeFile.data, key);
      const exists = previousValue !== undefined;

      if (exists && previousValue !== nextValue && !options.allowOverwrite) {
        conflicts.push({
          locale: localeFile.locale,
          previousValue,
          nextValue,
        });
        continue;
      }

      if (!exists || previousValue !== nextValue) {
        changes.push({
          locale: localeFile.locale,
          previousValue,
          nextValue,
          action: exists ? 'update' : 'add',
        });
      }
    }

    if (conflicts.length > 0) {
      const lines = conflicts.map((conflict) =>
        `${conflict.locale}: existing value ${JSON.stringify(conflict.previousValue)} differs from ${JSON.stringify(conflict.nextValue)}`,
      );
      throw new Error(`set-key refused to overwrite existing value(s). Use --allow-overwrite to replace them.\n${lines.join('\n')}`);
    }

    if (options.write) {
      for (const localeFile of localeFiles) {
        setNestedKey(localeFile.data, key, valuesByLocale[localeFile.locale]);
        await writeLocaleFile(localeFile);
      }
    }

    return {
      key,
      write: Boolean(options.write),
      localeFiles,
      changes,
    };
  };

  if (options.write) {
    return withLocaleWriteLock(rootDirectory, applySetKey);
  }

  return applySetKey();
}

function normalizePrefixList(prefixes) {
  return prefixes
    .flatMap((prefix) => String(prefix).split(','))
    .map((prefix) => prefix.trim())
    .filter(Boolean);
}

export function auditUnusedKeys(analysis) {
  const literalUsagesByKey = groupUsagesByKey(analysis.literalUsages ?? []);

  return analysis.unusedLocaleKeys.map((entry) => {
    const exactLiteralUsages = literalUsagesByKey.get(entry.key) ?? [];
    let status = 'confirmed-unused';
    let reason = 'unused key has no exact source string literal usage';

    if (entry.protected) {
      status = 'protected';
      reason = entry.protectedBy ?? 'protected by dynamic i18n usage';
    } else if (exactLiteralUsages.length > 0) {
      status = 'needs-review';
      reason = 'exact source string literal still exists';
    }

    return {
      ...entry,
      status,
      reason,
      exactLiteralUsages,
      dynamicSuffixMatches: findDynamicSuffixMatches(entry.key, analysis.dynamicUsages),
    };
  });
}

function mergeDynamicIdentifierValuesByFile(baseValues, overrideValues) {
  const merged = {};
  for (const [filePath, identifierValues] of Object.entries(baseValues)) {
    merged[filePath] = { ...identifierValues };
  }

  for (const [filePath, identifierValues] of Object.entries(overrideValues)) {
    merged[filePath] = {
      ...(merged[filePath] ?? {}),
      ...identifierValues,
    };
  }

  return merged;
}

function expandDynamicKeyUsages(dynamicUsages, dynamicIdentifierValuesByFile) {
  const usages = [];

  for (const usage of dynamicUsages) {
    const identifierValues = dynamicIdentifierValuesByFile[usage.filePath];
    if (!identifierValues) {
      continue;
    }

    const keys = expandDynamicExpression(usage.expression, identifierValues);
    for (const key of keys) {
      usages.push({
        key,
        filePath: usage.filePath,
        line: usage.line,
        column: usage.column,
        fromDynamicExpression: usage.expression,
      });
    }
  }

  return usages.sort(compareUsage);
}

function expandDynamicExpression(expression, identifierValues) {
  let candidates = [expression];
  let expanded = false;

  for (const [identifier, values] of Object.entries(identifierValues)) {
    const token = `\${${identifier}}`;
    if (!candidates.some((candidate) => candidate.includes(token))) {
      continue;
    }

    expanded = true;
    candidates = candidates.flatMap((candidate) =>
      values.map((value) => candidate.split(token).join(value)),
    );
  }

  if (!expanded) {
    return [];
  }

  return [...new Set(candidates.filter((candidate) => !candidate.includes('${')))];
}

function matchesAnyPrefix(key, prefixes) {
  if (prefixes.length === 0) {
    return false;
  }
  return prefixes.some((prefix) => key === prefix || key.startsWith(`${prefix}.`));
}

export function findKeysByText(analysis, query, options = {}) {
  const normalizedQuery = normalizeSearchText(query);
  const localeFilter = options.locale;

  return analysis.localeFiles.flatMap((localeFile) => {
    if (localeFilter && localeFile.locale !== localeFilter) {
      return [];
    }

    return localeFile.entries
      .filter((entry) => normalizeSearchText(String(entry.value)).includes(normalizedQuery))
      .map((entry) => ({
        locale: localeFile.locale,
        key: entry.key,
        value: entry.value,
      }));
  });
}

export function findKeysByPrefix(analysis, query) {
  return [...analysis.localeFiles]
    .flatMap((localeFile) => localeFile.entries
      .filter((entry) => entry.key === query || entry.key.startsWith(`${query}.`))
      .map((entry) => ({
        locale: localeFile.locale,
        key: entry.key,
        value: entry.value,
        usages: analysis.usageLocationsByKey.get(entry.key) ?? [],
      })));
}

async function readLocaleFiles(rootDirectory, localeFilePaths) {
  const localeFiles = [];

  for (const relativePath of localeFilePaths) {
    const absolutePath = path.join(rootDirectory, relativePath);
    const rawContent = await readFile(absolutePath, 'utf8');
    const data = JSON.parse(rawContent);
    const locale = path.basename(relativePath, '.json');
    localeFiles.push({
      locale,
      relativePath,
      absolutePath,
      data,
      entries: flattenLocaleEntries(data),
    });
  }

  return localeFiles;
}

async function withLocaleWriteLock(rootDirectory, action) {
  const lockDirectory = path.join(rootDirectory, I18N_WRITE_LOCK_DIR);
  await mkdir(path.dirname(lockDirectory), { recursive: true });

  const startedAt = Date.now();
  while (true) {
    try {
      await mkdir(lockDirectory);
      break;
    } catch (error) {
      if (error?.code !== 'EEXIST') {
        throw error;
      }
      if (Date.now() - startedAt > I18N_WRITE_LOCK_TIMEOUT_MS) {
        throw new Error(`Timed out waiting for i18n write lock: ${lockDirectory}`);
      }
      await sleep(I18N_WRITE_LOCK_RETRY_MS);
    }
  }

  try {
    return await action();
  } finally {
    await removeLockDirectory(lockDirectory);
  }
}

async function removeLockDirectory(lockDirectory) {
  const startedAt = Date.now();
  while (true) {
    try {
      await rm(lockDirectory, { recursive: true, force: true });
      return;
    } catch {
      // A scanner holding the directory briefly must not fail the run, and a
      // leftover lock would stall the next writer, so retry within budget.
      if (Date.now() - startedAt > I18N_WRITE_LOCK_TIMEOUT_MS) {
        return;
      }
      await sleep(I18N_WRITE_LOCK_RETRY_MS);
    }
  }
}

async function writeLocaleFile(localeFile) {
  const tempPath = `${localeFile.absolutePath}.${process.pid}.${Date.now()}.tmp`;
  await writeFile(tempPath, `${JSON.stringify(localeFile.data, null, 2)}\n`, 'utf8');
  const startedAt = Date.now();
  while (true) {
    try {
      await rename(tempPath, localeFile.absolutePath);
      return;
    } catch (error) {
      // Windows: Defender or the search indexer can briefly hold the target
      // file open, which surfaces as EPERM/EACCES on rename; retry within the
      // lock budget instead of failing a write whose content is already done.
      const retryable = ['EPERM', 'EACCES', 'EBUSY'].includes(error?.code);
      if (!retryable || Date.now() - startedAt > I18N_WRITE_LOCK_TIMEOUT_MS) {
        await rm(tempPath, { force: true }).catch(() => {});
        throw error;
      }
      await sleep(I18N_WRITE_LOCK_RETRY_MS);
    }
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function flattenLocaleEntries(value, prefix = '') {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    return [{ key: prefix, value }];
  }

  return Object.entries(value).flatMap(([key, nestedValue]) => {
    const nextPrefix = prefix ? `${prefix}.${key}` : key;
    return flattenLocaleEntries(nestedValue, nextPrefix);
  });
}

async function collectSourceFiles(rootDirectory, scanRoots) {
  const files = [];

  for (const scanRoot of scanRoots) {
    const absoluteRoot = path.join(rootDirectory, scanRoot);
    files.push(...await collectSourceFilesFromDirectory(rootDirectory, absoluteRoot));
  }

  return files.sort((left, right) => left.relativePath.localeCompare(right.relativePath));
}

async function collectSourceFilesFromDirectory(rootDirectory, directoryPath) {
  let entries;
  try {
    entries = await readdir(directoryPath, { withFileTypes: true });
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
      return [];
    }
    throw error;
  }

  const files = [];
  for (const entry of entries) {
    const entryPath = path.join(directoryPath, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name === 'dist' || entry.name === 'target') {
        continue;
      }
      files.push(...await collectSourceFilesFromDirectory(rootDirectory, entryPath));
      continue;
    }

    if (!entry.isFile() || !SOURCE_EXTENSIONS.has(path.extname(entry.name))) {
      continue;
    }

    files.push({
      absolutePath: entryPath,
      relativePath: path.relative(rootDirectory, entryPath).split(path.sep).join('/'),
    });
  }

  return files;
}

async function analyzeSourceFiles(sourceFiles, localeKeys) {
  const staticUsages = [];
  const dynamicUsages = [];
  const literalUsages = [];
  const parseErrors = [];

  for (const sourceFile of sourceFiles) {
    const content = await readFile(sourceFile.absolutePath, 'utf8');
    const result = RUST_SOURCE_EXTENSIONS.has(path.extname(sourceFile.relativePath))
      ? analyzeRustSourceFile(sourceFile.relativePath, content, localeKeys)
      : analyzeSourceFileWithAst(sourceFile.relativePath, content, localeKeys);
    staticUsages.push(...result.staticUsages);
    dynamicUsages.push(...result.dynamicUsages);
    literalUsages.push(...result.literalUsages);
    parseErrors.push(...result.parseErrors);
  }

  staticUsages.sort(compareUsage);
  dynamicUsages.sort(compareUsage);
  literalUsages.sort(compareUsage);
  parseErrors.sort(compareUsage);

  return { staticUsages, dynamicUsages, literalUsages, parseErrors };
}

function analyzeRustSourceFile(filePath, content, localeKeys) {
  const literalUsages = collectRustLocaleKeyStringUsages(filePath, content, localeKeys);
  return {
    staticUsages: literalUsages,
    dynamicUsages: [],
    literalUsages,
    parseErrors: [],
  };
}

function collectRustLocaleKeyStringUsages(filePath, content, localeKeys) {
  const usages = [];
  const stringRegex = /"((?:\\.|[^"\\])*)"/g;
  let match;

  while ((match = stringRegex.exec(content)) !== null) {
    const value = match[1];
    if (!localeKeys.has(value)) {
      continue;
    }

    const location = offsetToLocation(content, match.index);
    usages.push({
      key: value,
      filePath,
      line: location.line,
      column: location.column,
    });
  }

  return usages;
}

function analyzeSourceFileWithAst(filePath, content, localeKeys) {
  const ast = ts.createSourceFile(
    filePath,
    content,
    ts.ScriptTarget.Latest,
    true,
    getScriptKind(filePath),
  );
  const parseErrors = ast.parseDiagnostics.map((diagnostic) => diagnosticToParseError(filePath, ast, diagnostic));

  if (parseErrors.length > 0) {
    return {
      staticUsages: [],
      dynamicUsages: [],
      literalUsages: [],
      parseErrors,
    };
  }

  const context = new AstContext(ast);
  const staticUsages = [];
  const dynamicUsages = [];
  const literalUsages = collectSourceLiteralUsages(ast, filePath, localeKeys);

  visitSourceFileStatements(ast, context, filePath, staticUsages, dynamicUsages);

  return {
    staticUsages,
    dynamicUsages,
    literalUsages,
    parseErrors,
  };
}

function collectSourceLiteralUsages(ast, filePath, localeKeys) {
  const usages = [];

  const visit = (node) => {
    if ((ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) && localeKeys.has(node.text)) {
      usages.push({
        key: node.text,
        filePath,
        ...nodeToLocation(ast, node),
      });
    }
    ts.forEachChild(node, visit);
  };

  visit(ast);
  return usages;
}

function getScriptKind(filePath) {
  if (filePath.endsWith('.tsx')) {
    return ts.ScriptKind.TSX;
  }
  if (filePath.endsWith('.jsx')) {
    return ts.ScriptKind.JSX;
  }
  if (filePath.endsWith('.js')) {
    return ts.ScriptKind.JS;
  }
  return ts.ScriptKind.TS;
}

function diagnosticToParseError(filePath, ast, diagnostic) {
  const position = diagnostic.start ?? 0;
  const location = ast.getLineAndCharacterOfPosition(position);
  return {
    filePath,
    line: location.line + 1,
    column: location.character + 1,
    message: ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n'),
  };
}

class AstContext {
  constructor(ast) {
    this.ast = ast;
    this.scopes = [{ constants: new Map(), keyBuilders: new Map(), objectStringMaps: new Map() }];
  }

  pushScope() {
    this.scopes.push({ constants: new Map(), keyBuilders: new Map(), objectStringMaps: new Map() });
  }

  popScope() {
    this.scopes.pop();
  }

  setConstant(name, value) {
    this.currentScope().constants.set(name, value);
  }

  assignConstant(name, value) {
    for (let index = this.scopes.length - 1; index >= 0; index -= 1) {
      if (this.scopes[index].constants.has(name)) {
        const current = this.scopes[index].constants.get(name);
        this.scopes[index].constants.set(name, mergeStaticConstantValues(current, value));
        return;
      }
    }

    this.setConstant(name, value);
  }

  getConstant(name) {
    for (let index = this.scopes.length - 1; index >= 0; index -= 1) {
      if (this.scopes[index].constants.has(name)) {
        return this.scopes[index].constants.get(name);
      }
    }
    return undefined;
  }

  setKeyBuilder(name, builder) {
    this.currentScope().keyBuilders.set(name, builder);
  }

  getKeyBuilder(name) {
    for (let index = this.scopes.length - 1; index >= 0; index -= 1) {
      const value = this.scopes[index].keyBuilders.get(name);
      if (value) {
        return value;
      }
    }
    return undefined;
  }

  setObjectStringMap(name, value) {
    this.currentScope().objectStringMaps.set(name, value);
  }

  getObjectStringMap(name) {
    for (let index = this.scopes.length - 1; index >= 0; index -= 1) {
      const value = this.scopes[index].objectStringMaps.get(name);
      if (value) {
        return value;
      }
    }
    return undefined;
  }

  currentScope() {
    return this.scopes[this.scopes.length - 1];
  }
}

function visitSourceFileStatements(ast, context, filePath, staticUsages, dynamicUsages) {
  for (const statement of ast.statements) {
    visitAstNode(statement, context, filePath, staticUsages, dynamicUsages);
  }
}

function visitAstNode(node, context, filePath, staticUsages, dynamicUsages) {
  if (ts.isBlock(node) || ts.isModuleBlock(node)) {
    context.pushScope();
    for (const statement of node.statements) {
      visitAstNode(statement, context, filePath, staticUsages, dynamicUsages);
    }
    context.popScope();
    return;
  }

  if (ts.isFunctionDeclaration(node)) {
    collectFunctionKeyBuilder(node, context);
    visitFunctionLikeNode(node, context, filePath, staticUsages, dynamicUsages);
    return;
  }

  if (isFunctionLikeWithBody(node)) {
    visitFunctionLikeNode(node, context, filePath, staticUsages, dynamicUsages);
    return;
  }

  if (ts.isVariableStatement(node)) {
    collectVariableStatementBindings(node, context);
    visitAstChildren(node, context, filePath, staticUsages, dynamicUsages);
    return;
  }

  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken) {
    collectAssignmentBinding(node, context);
  }

  if (ts.isCallExpression(node)) {
    collectTranslationCallUsage(node, context, filePath, staticUsages, dynamicUsages);
  } else if (ts.isPropertyAssignment(node)) {
    collectStaticKeyPropertyUsage(node, context, filePath, staticUsages, dynamicUsages);
  } else if (ts.isJsxAttribute(node)) {
    collectJsxStaticKeyAttributeUsage(node, context, filePath, staticUsages, dynamicUsages);
  }

  visitAstChildren(node, context, filePath, staticUsages, dynamicUsages);
}

function visitFunctionLikeNode(node, context, filePath, staticUsages, dynamicUsages) {
  context.pushScope();
  for (const parameter of node.parameters ?? []) {
    if (ts.isIdentifier(parameter.name)) {
      context.setConstant(parameter.name.text, UNKNOWN_CONSTANT);
    }
  }
  if (node.body) {
    visitAstNode(node.body, context, filePath, staticUsages, dynamicUsages);
  }
  context.popScope();
}

function visitAstChildren(node, context, filePath, staticUsages, dynamicUsages) {
  ts.forEachChild(node, (child) => {
    visitAstNode(child, context, filePath, staticUsages, dynamicUsages);
  });
}

function isFunctionLikeWithBody(node) {
  return (
    ts.isFunctionExpression(node)
    || ts.isArrowFunction(node)
    || ts.isMethodDeclaration(node)
    || ts.isGetAccessorDeclaration(node)
    || ts.isSetAccessorDeclaration(node)
    || ts.isConstructorDeclaration(node)
  ) && Boolean(node.body);
}

function collectVariableStatementBindings(node, context) {
  const isConst = (node.declarationList.flags & ts.NodeFlags.Const) !== 0;
  const isLet = (node.declarationList.flags & ts.NodeFlags.Let) !== 0;

  for (const declaration of node.declarationList.declarations) {
    if (!ts.isIdentifier(declaration.name)) {
      continue;
    }

    if (!declaration.initializer) {
      if (isLet) {
        context.setConstant(declaration.name.text, UNKNOWN_CONSTANT);
      }
      continue;
    }

    const keyBuilder = isConst ? createKeyBuilderFromFunctionLike(declaration.initializer) : null;
    if (keyBuilder) {
      context.setKeyBuilder(declaration.name.text, keyBuilder);
      continue;
    }

    const objectStringMap = isConst ? createObjectStringMap(declaration.initializer) : null;
    if (objectStringMap) {
      context.setObjectStringMap(declaration.name.text, objectStringMap);
      continue;
    }

    const evaluated = evaluateI18nKeyExpression(declaration.initializer, context);
    const staticValues = getStaticEvaluationValues(evaluated);
    if (staticValues.length > 0) {
      context.setConstant(declaration.name.text, staticValues);
    } else if (isLet) {
      context.setConstant(declaration.name.text, UNKNOWN_CONSTANT);
    }
  }
}

function collectAssignmentBinding(node, context) {
  if (!ts.isIdentifier(node.left)) {
    return;
  }

  const evaluated = evaluateI18nKeyExpression(node.right, context);
  const staticValues = getStaticEvaluationValues(evaluated);
  context.assignConstant(node.left.text, staticValues.length > 0 ? staticValues : UNKNOWN_CONSTANT);
}

function collectFunctionKeyBuilder(node, context) {
  if (!node.name) {
    return;
  }

  const keyBuilder = createKeyBuilderFromFunctionLike(node);
  if (keyBuilder) {
    context.setKeyBuilder(node.name.text, keyBuilder);
  }
}

function collectTranslationCallUsage(node, context, filePath, staticUsages, dynamicUsages) {
  const argumentIndex = getTranslationCallKeyArgumentIndex(node);
  if (argumentIndex === null) {
    return;
  }

  const argument = node.arguments[argumentIndex];
  if (!argument) {
    return;
  }

  addEvaluationUsages(
    evaluateI18nKeyExpression(argument, context),
    context.ast,
    filePath,
    node,
    staticUsages,
    dynamicUsages,
  );
}

function getTranslationCallKeyArgumentIndex(node) {
  if (ts.isIdentifier(node.expression) && node.expression.text === 't') {
    return 0;
  }

  if (
    ts.isPropertyAccessExpression(node.expression)
    && node.expression.name.text === 't'
    && node.expression.expression.getText() === 'i18n'
  ) {
    return 0;
  }

  if (ts.isIdentifier(node.expression) && node.expression.text === 'getMetaText') {
    return 1;
  }

  if (ts.isIdentifier(node.expression) && node.expression.text === 'withDetail') {
    return 0;
  }

  return null;
}

function collectStaticKeyPropertyUsage(node, context, filePath, staticUsages, dynamicUsages) {
  if (getPropertyNameText(node.name) !== 'labelKey') {
    return;
  }

  addEvaluationUsages(
    evaluateI18nKeyExpression(node.initializer, context),
    context.ast,
    filePath,
    node.name,
    staticUsages,
    dynamicUsages,
  );
}

function collectJsxStaticKeyAttributeUsage(node, context, filePath, staticUsages, dynamicUsages) {
  if (node.name.text !== 'labelKey' || !node.initializer) {
    return;
  }

  if (ts.isStringLiteral(node.initializer)) {
    addEvaluationUsages(
      [{ type: 'static', value: node.initializer.text }],
      context.ast,
      filePath,
      node.name,
      staticUsages,
      dynamicUsages,
    );
    return;
  }

  if (ts.isJsxExpression(node.initializer) && node.initializer.expression) {
    addEvaluationUsages(
      evaluateI18nKeyExpression(node.initializer.expression, context),
      context.ast,
      filePath,
      node.name,
      staticUsages,
      dynamicUsages,
    );
  }
}

function getPropertyNameText(name) {
  if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) {
    return name.text;
  }
  return '';
}

function evaluateI18nKeyExpression(node, context) {
  const expression = skipExpressionWrappers(node);

  if (ts.isStringLiteral(expression) || ts.isNoSubstitutionTemplateLiteral(expression)) {
    return [{ type: 'static', value: expression.text }];
  }

  if (ts.isIdentifier(expression)) {
    const constant = context.getConstant(expression.text);
    if (Array.isArray(constant)) {
      return constant.map((value) => ({ type: 'static', value }));
    }
    return [createUnresolvedDynamicUsage(expression, context.ast)];
  }

  if (ts.isTemplateExpression(expression)) {
    return [evaluateTemplateExpression(expression, context)];
  }

  if (ts.isBinaryExpression(expression) && expression.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    return [evaluateBinaryPlusExpression(expression, context)];
  }

  if (ts.isConditionalExpression(expression)) {
    return [
      ...evaluateI18nKeyExpression(expression.whenTrue, context),
      ...evaluateI18nKeyExpression(expression.whenFalse, context),
    ];
  }

  if (ts.isPropertyAccessExpression(expression) || ts.isElementAccessExpression(expression)) {
    const mapUsage = evaluateObjectStringMapAccess(expression, context);
    if (mapUsage) {
      return mapUsage;
    }
  }

  if (ts.isCallExpression(expression)) {
    const keyBuilderUsage = evaluateKeyBuilderCall(expression, context);
    if (keyBuilderUsage) {
      return keyBuilderUsage;
    }
  }

  return [createUnresolvedDynamicUsage(expression, context.ast)];
}

function skipExpressionWrappers(node) {
  let current = node;
  while (
    ts.isParenthesizedExpression(current)
    || ts.isAsExpression(current)
    || ts.isTypeAssertionExpression(current)
    || ts.isNonNullExpression(current)
  ) {
    current = current.expression;
  }
  return current;
}

function evaluateTemplateExpression(node, context) {
  let raw = node.head.text;

  for (const span of node.templateSpans) {
    raw += expressionToTemplatePart(span.expression, context);
    raw += span.literal.text;
  }

  if (!raw.includes('${')) {
    return { type: 'static', value: raw };
  }

  return buildAstDynamicTemplateUsage(raw);
}

function evaluateBinaryPlusExpression(node, context) {
  const raw = flattenBinaryPlusOperands(node)
    .map((operand) => expressionToTemplatePart(operand, context))
    .join('');

  if (!raw.includes('${')) {
    return { type: 'static', value: raw };
  }

  return buildAstDynamicTemplateUsage(raw);
}

function flattenBinaryPlusOperands(node) {
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    return [
      ...flattenBinaryPlusOperands(node.left),
      ...flattenBinaryPlusOperands(node.right),
    ];
  }
  return [node];
}

function expressionToTemplatePart(node, context) {
  const evaluated = evaluateI18nKeyExpression(node, context);
  if (evaluated.length === 1 && evaluated[0].type === 'static') {
    return evaluated[0].value;
  }
  return `\${${skipExpressionWrappers(node).getText(context.ast)}}`;
}

function evaluateKeyBuilderCall(node, context) {
  if (!ts.isIdentifier(node.expression) || node.arguments.length !== 1) {
    return null;
  }

  const builder = context.getKeyBuilder(node.expression.text);
  if (!builder) {
    return null;
  }

  const argument = evaluateI18nKeyExpression(node.arguments[0], context);
  if (argument.length !== 1 || argument[0].type !== 'static') {
    return null;
  }

  context.pushScope();
  context.setConstant(builder.parameterName, [argument[0].value]);
  const result = evaluateI18nKeyExpression(builder.returnExpression, context);
  context.popScope();

  return result;
}

function evaluateObjectStringMapAccess(node, context) {
  if (ts.isPropertyAccessExpression(node)) {
    const objectName = ts.isIdentifier(node.expression) ? node.expression.text : '';
    const objectMap = objectName ? context.getObjectStringMap(objectName) : undefined;
    const value = objectMap?.properties.get(node.name.text);
    return value === undefined ? null : [{ type: 'static', value }];
  }

  if (!ts.isElementAccessExpression(node) || !ts.isIdentifier(node.expression)) {
    return null;
  }

  const objectMap = context.getObjectStringMap(node.expression.text);
  if (!objectMap) {
    return null;
  }

  const argument = evaluateI18nKeyExpression(node.argumentExpression, context);
  if (argument.length === 1 && argument[0].type === 'static') {
    const value = objectMap.properties.get(argument[0].value);
    return value === undefined ? null : [{ type: 'static', value }];
  }

  return objectMap.values.map((value) => ({ type: 'static', value }));
}

function createObjectStringMap(node) {
  const expression = skipExpressionWrappers(node);
  if (!ts.isObjectLiteralExpression(expression)) {
    return null;
  }

  const properties = new Map();
  for (const property of expression.properties) {
    if (!ts.isPropertyAssignment(property)) {
      return null;
    }

    const propertyName = getPropertyNameText(property.name);
    const valueExpression = skipExpressionWrappers(property.initializer);
    if (!propertyName || !(ts.isStringLiteral(valueExpression) || ts.isNoSubstitutionTemplateLiteral(valueExpression))) {
      return null;
    }

    properties.set(propertyName, valueExpression.text);
  }

  if (properties.size === 0) {
    return null;
  }

  return {
    properties,
    values: uniqueStrings([...properties.values()]),
  };
}

function createKeyBuilderFromFunctionLike(node) {
  if (!isSupportedKeyBuilderFunction(node) || node.parameters.length !== 1) {
    return null;
  }

  const parameter = node.parameters[0];
  if (!ts.isIdentifier(parameter.name)) {
    return null;
  }

  const returnExpression = getSingleReturnExpression(node);
  if (!returnExpression) {
    return null;
  }

  return {
    parameterName: parameter.name.text,
    returnExpression,
  };
}

function isSupportedKeyBuilderFunction(node) {
  return ts.isFunctionDeclaration(node)
    || ts.isFunctionExpression(node)
    || ts.isArrowFunction(node);
}

function getSingleReturnExpression(node) {
  if (ts.isArrowFunction(node) && node.body && !ts.isBlock(node.body)) {
    return node.body;
  }

  if (!node.body || !ts.isBlock(node.body) || node.body.statements.length !== 1) {
    return null;
  }

  const statement = node.body.statements[0];
  if (!ts.isReturnStatement(statement) || !statement.expression) {
    return null;
  }

  return statement.expression;
}

function createUnresolvedDynamicUsage(node, ast) {
  return {
    type: 'dynamic',
    expression: node.getText(ast),
    value: node.getText(ast),
    protectPrefix: '',
    protectSuffix: '',
  };
}

function buildAstDynamicTemplateUsage(raw) {
  const firstExpressionIndex = raw.indexOf('${');
  const lastExpressionStart = raw.lastIndexOf('${');
  const lastExpressionEnd = raw.indexOf('}', lastExpressionStart);
  const prefix = firstExpressionIndex === -1 ? '' : raw.slice(0, firstExpressionIndex);
  const suffix = lastExpressionEnd === -1 ? '' : raw.slice(lastExpressionEnd + 1);

  return {
    type: 'dynamic',
    expression: raw,
    value: raw,
    protectPrefix: normalizeProtectionPrefix(prefix),
    protectSuffix: suffix,
  };
}

function addEvaluationUsages(evaluated, ast, filePath, node, staticUsages, dynamicUsages) {
  for (const usage of evaluated) {
    addAstParsedUsage(usage, ast, filePath, node, staticUsages, dynamicUsages);
  }
}

function addAstParsedUsage(parsed, ast, filePath, node, staticUsages, dynamicUsages) {
  const location = nodeToLocation(ast, node);
  if (parsed.type === 'static') {
    staticUsages.push({
      key: parsed.value,
      filePath,
      line: location.line,
      column: location.column,
    });
    return;
  }

  dynamicUsages.push({
    expression: parsed.expression,
    protectPrefix: parsed.protectPrefix,
    protectSuffix: parsed.protectSuffix,
    filePath,
    line: location.line,
    column: location.column,
  });
}

function nodeToLocation(ast, node) {
  const position = node.getStart(ast);
  const location = ast.getLineAndCharacterOfPosition(position);
  return {
    line: location.line + 1,
    column: location.character + 1,
  };
}

function offsetToLocation(content, offset) {
  const before = content.slice(0, offset);
  const lines = before.split(/\r?\n/);
  return {
    line: lines.length,
    column: lines[lines.length - 1].length + 1,
  };
}

function uniqueStrings(values) {
  return [...new Set(values)];
}

function getStaticEvaluationValues(evaluated) {
  if (evaluated.length === 0 || !evaluated.every((entry) => entry.type === 'static')) {
    return [];
  }
  return uniqueStrings(evaluated.map((entry) => entry.value).filter(Boolean));
}

function mergeStaticConstantValues(current, next) {
  if (next === UNKNOWN_CONSTANT) {
    return UNKNOWN_CONSTANT;
  }
  if (current === UNKNOWN_CONSTANT || current === undefined) {
    return next;
  }
  if (Array.isArray(current) && Array.isArray(next)) {
    return uniqueStrings([...current, ...next]);
  }
  return UNKNOWN_CONSTANT;
}

function normalizeProtectionPrefix(prefix) {
  if (!prefix || prefix.includes('${')) {
    return '';
  }
  return prefix;
}

function compareUsage(left, right) {
  return left.filePath.localeCompare(right.filePath)
    || left.line - right.line
    || left.column - right.column;
}

function groupUsagesByKey(usages) {
  const grouped = new Map();
  for (const usage of usages) {
    const list = grouped.get(usage.key) ?? [];
    list.push(usage);
    grouped.set(usage.key, list);
  }
  return grouped;
}

function buildProtectedPredicates(dynamicUsages, dynamicProtectedPrefixes) {
  const predicates = [];

  for (const prefix of dynamicProtectedPrefixes) {
    predicates.push({
      reason: `configured prefix ${prefix}`,
      matches: (key) => key.startsWith(prefix),
    });
  }

  for (const usage of dynamicUsages) {
    if (!usage.protectPrefix) {
      continue;
    }
    predicates.push({
      reason: `${usage.filePath}:${usage.line} dynamic ${usage.protectPrefix}*${usage.protectSuffix}`,
      matches: (key) => key.startsWith(usage.protectPrefix) && key.endsWith(usage.protectSuffix),
    });
  }

  return predicates;
}

function findProtectionReason(key, predicates) {
  return predicates.find((predicate) => predicate.matches(key))?.reason;
}

function findDynamicSuffixMatches(key, dynamicUsages) {
  return dynamicUsages
    .filter((usage) => usage.protectSuffix && key.endsWith(usage.protectSuffix))
    .map((usage) => ({
      expression: usage.expression,
      filePath: usage.filePath,
      line: usage.line,
      column: usage.column,
    }));
}

function deleteNestedKey(root, key) {
  const parts = key.split('.');
  let current = root;
  const parents = [];

  for (const part of parts.slice(0, -1)) {
    if (!current || typeof current !== 'object' || Array.isArray(current)) {
      return false;
    }
    parents.push([current, part]);
    current = current[part];
  }

  if (!current || typeof current !== 'object' || Array.isArray(current)) {
    return false;
  }

  delete current[parts[parts.length - 1]];

  for (let index = parents.length - 1; index >= 0; index -= 1) {
    const [parent, part] = parents[index];
    const child = parent[part];
    if (child && typeof child === 'object' && !Array.isArray(child) && Object.keys(child).length === 0) {
      delete parent[part];
    }
  }

  return true;
}

function getNestedKey(root, key) {
  let current = root;
  for (const part of key.split('.')) {
    if (!current || typeof current !== 'object' || Array.isArray(current)) {
      return undefined;
    }
    if (!Object.prototype.hasOwnProperty.call(current, part)) {
      return undefined;
    }
    current = current[part];
  }
  return current;
}

function setNestedKey(root, key, value) {
  const parts = key.split('.');
  let current = root;

  for (const part of parts.slice(0, -1)) {
    const existingValue = current[part];
    if (!existingValue || typeof existingValue !== 'object' || Array.isArray(existingValue)) {
      current[part] = {};
    }
    current = current[part];
  }

  current[parts[parts.length - 1]] = value;
}

function normalizeSearchText(value) {
  return value.trim().toLocaleLowerCase();
}

function formatLocation(usage) {
  return `${usage.filePath}:${usage.line}:${usage.column}`;
}

function printCheckReport(analysis) {
  if (analysis.parseErrors.length > 0) {
    console.error('Source files with parse errors:');
    for (const parseError of analysis.parseErrors) {
      console.error(`- ${parseError.filePath}:${parseError.line}:${parseError.column} ${parseError.message}`);
    }
  }

  if (analysis.missingStaticKeys.length > 0) {
    console.error('Missing locale keys used by code:');
    for (const missing of analysis.missingStaticKeys) {
      console.error(`- ${missing.key} missing in ${missing.locale} (${missing.filePath}:${missing.line}:${missing.column})`);
    }
  }

  if (analysis.localeMismatches.length > 0) {
    console.error('Locale key mismatches:');
    for (const mismatch of analysis.localeMismatches) {
      console.error(`- ${mismatch.key} missing in ${mismatch.locale}`);
    }
  }

  if (
    analysis.parseErrors.length === 0
    && analysis.missingStaticKeys.length === 0
    && analysis.localeMismatches.length === 0
  ) {
    console.log('i18n check passed.');
  }
}

function printReport(analysis) {
  console.log(`Locales: ${analysis.localeFiles.map((localeFile) => localeFile.locale).join(', ')}`);
  console.log(`Source files scanned: ${analysis.sourceFiles.length}`);
  console.log(`Static used keys: ${analysis.usedKeys.length}`);
  console.log(`Dynamic calls: ${analysis.dynamicUsages.length}`);
  console.log(`Parse errors: ${analysis.parseErrors.length}`);
  console.log(`Missing static keys: ${analysis.missingStaticKeys.length}`);
  console.log(`Locale mismatches: ${analysis.localeMismatches.length}`);
  console.log(`Unused locale keys: ${analysis.unusedLocaleKeys.length}`);
  console.log(`Removable unused keys: ${analysis.removableUnusedKeys.length}`);

  if (analysis.parseErrors.length > 0) {
    console.log('\nParse errors:');
    for (const parseError of analysis.parseErrors) {
      console.log(`- ${parseError.filePath}:${parseError.line}:${parseError.column} ${parseError.message}`);
    }
  }

  if (analysis.missingStaticKeys.length > 0) {
    console.log('\nMissing static keys:');
    for (const missing of analysis.missingStaticKeys) {
      console.log(`- ${missing.key} missing in ${missing.locale} (${missing.filePath}:${missing.line}:${missing.column})`);
    }
  }

  if (analysis.localeMismatches.length > 0) {
    console.log('\nLocale mismatches:');
    for (const mismatch of analysis.localeMismatches) {
      console.log(`- ${mismatch.key} missing in ${mismatch.locale}`);
    }
  }

  if (analysis.removableUnusedKeys.length > 0) {
    console.log('\nHigh-confidence unused keys:');
    for (const entry of analysis.removableUnusedKeys) {
      console.log(`- ${entry.key} (${entry.locales.join(', ')})`);
    }
  }

  if (analysis.dynamicUsages.length > 0) {
    console.log('\nDynamic i18n calls:');
    for (const usage of analysis.dynamicUsages) {
      const protection = usage.protectPrefix
        ? ` protects ${usage.protectPrefix}*${usage.protectSuffix}`
        : ' unresolved';
      console.log(`- ${formatLocation(usage)} ${usage.expression}${protection}`);
    }
  }
}

function printAuditUnusedReport(analysis, auditEntries) {
  const confirmed = auditEntries.filter((entry) => entry.status === 'confirmed-unused');
  const needsReview = auditEntries.filter((entry) => entry.status === 'needs-review');
  const protectedEntries = auditEntries.filter((entry) => entry.status === 'protected');

  console.log(`Unused locale keys: ${analysis.unusedLocaleKeys.length}`);
  console.log(`Confirmed unused keys: ${confirmed.length}`);
  console.log(`Needs review: ${needsReview.length}`);
  console.log(`Protected dynamic keys: ${protectedEntries.length}`);

  if (needsReview.length > 0) {
    console.log('\nUnused keys with exact source string literals:');
    for (const entry of needsReview) {
      console.log(`- ${entry.key}: ${entry.reason}`);
      for (const usage of entry.exactLiteralUsages) {
        console.log(`  literal at ${formatLocation(usage)}`);
      }
    }
  }

  if (confirmed.length > 0) {
    console.log('\nConfirmed unused keys:');
    for (const entry of confirmed) {
      console.log(`- ${entry.key} (${entry.locales.join(', ')})`);
    }
  }
}

function printFindTextResults(results) {
  if (results.length === 0) {
    console.log('No matching translation text found.');
    return;
  }

  for (const result of results) {
    console.log(`${result.locale} ${result.key}: ${result.value}`);
  }
}

function printFindKeyResults(results) {
  if (results.length === 0) {
    console.log('No matching translation key found.');
    return;
  }

  for (const result of results) {
    console.log(`${result.locale} ${result.key}: ${result.value}`);
    for (const usage of result.usages) {
      console.log(`  used at ${formatLocation(usage)}`);
    }
  }
}

function printSetKeyResult(result) {
  if (result.changes.length === 0) {
    console.log(`No changes for i18n key ${result.key}.`);
    return;
  }

  console.log(`${result.write ? 'Updated' : 'Would update'} i18n key ${result.key}:`);
  for (const change of result.changes) {
    const previousText = change.previousValue === undefined
      ? '<missing>'
      : JSON.stringify(change.previousValue);
    console.log(`- ${change.locale}: ${change.action} ${previousText} -> ${JSON.stringify(change.nextValue)}`);
  }
  if (!result.write) {
    console.log('\nRun with --write to update locale files.');
  }
}

function parseArgs(args) {
  const flags = new Map();
  const positional = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--') {
      positional.push(...args.slice(index + 1));
      break;
    }

    if (!arg.startsWith('--')) {
      positional.push(arg);
      continue;
    }

    const [flagName, inlineValue] = arg.slice(2).split('=', 2);
    if (inlineValue !== undefined) {
      flags.set(flagName, inlineValue);
      continue;
    }

    const next = args[index + 1];
    if (next && !next.startsWith('--')) {
      flags.set(flagName, next);
      index += 1;
    } else {
      flags.set(flagName, true);
    }
  }

  return { flags, positional };
}

function collectSetKeyCliOptions(localeFiles, flags, positionalArgs) {
  const localeNames = new Set(localeFiles.map((localeFile) => localeFile.locale));
  const valuesByLocale = {};
  let write = flags.has('write');
  let allowOverwrite = flags.has('allow-overwrite');

  for (const localeFile of localeFiles) {
    if (!flags.has(localeFile.locale)) {
      continue;
    }
    const value = flags.get(localeFile.locale);
    if (value === true) {
      throw new Error(`set-key --${localeFile.locale} requires a value.`);
    }
    valuesByLocale[localeFile.locale] = String(value);
  }

  for (let index = 0; index < positionalArgs.length; index += 1) {
    const token = positionalArgs[index];
    if (token === '--write') {
      write = true;
      continue;
    }
    if (token === '--allow-overwrite') {
      allowOverwrite = true;
      continue;
    }
    if (!token.startsWith('--')) {
      continue;
    }

    const [flagName, inlineValue] = token.slice(2).split('=', 2);
    if (!localeNames.has(flagName)) {
      continue;
    }

    if (inlineValue !== undefined) {
      valuesByLocale[flagName] = inlineValue;
      continue;
    }

    const nextValue = positionalArgs[index + 1];
    if (nextValue === undefined) {
      throw new Error(`set-key --${flagName} requires a value.`);
    }
    valuesByLocale[flagName] = String(nextValue);
    index += 1;
  }

  return {
    valuesByLocale,
    write,
    allowOverwrite,
  };
}

function buildAnalyzeOptionsFromFlags(flags) {
  const options = {};

  if (flags.has('root')) {
    options.rootDirectory = path.resolve(String(flags.get('root')));
  }
  if (flags.has('locale-files')) {
    options.localeFilePaths = splitCommaSeparatedFlag(flags.get('locale-files'));
  }
  if (flags.has('scan-roots')) {
    options.scanRoots = splitCommaSeparatedFlag(flags.get('scan-roots'));
  }

  return options;
}

function splitCommaSeparatedFlag(value) {
  return String(value)
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
}

async function main() {
  const { flags, positional } = parseArgs(process.argv.slice(2));
  const command = positional[0];

  if (!command || command === 'help' || flags.has('help')) {
    console.log(HELP_TEXT);
    return;
  }

  const analyzeOptions = buildAnalyzeOptionsFromFlags(flags);

  if (command === 'check') {
    const analysis = await analyzeProject(analyzeOptions);
    printCheckReport(analysis);
    if (
      analysis.parseErrors.length > 0
      || analysis.missingStaticKeys.length > 0
      || analysis.localeMismatches.length > 0
    ) {
      process.exitCode = 1;
    }
    return;
  }

  if (command === 'report') {
    const analysis = await analyzeProject(analyzeOptions);
    if (flags.has('json')) {
      console.log(JSON.stringify({
        usedKeys: analysis.usedKeys,
        dynamicUsages: analysis.dynamicUsages,
        parseErrors: analysis.parseErrors,
        expandedDynamicKeyUsages: analysis.expandedDynamicKeyUsages,
        missingStaticKeys: analysis.missingStaticKeys,
        localeMismatches: analysis.localeMismatches,
        unusedLocaleKeys: analysis.unusedLocaleKeys,
        removableUnusedKeys: analysis.removableUnusedKeys,
      }, null, 2));
      return;
    }
    printReport(analysis);
    return;
  }

  if (command === 'audit-unused') {
    const analysis = await analyzeProject(analyzeOptions);
    const auditEntries = auditUnusedKeys(analysis);
    if (flags.has('json')) {
      console.log(JSON.stringify({
        unusedLocaleKeys: analysis.unusedLocaleKeys.length,
        confirmedUnusedKeys: auditEntries.filter((entry) => entry.status === 'confirmed-unused'),
        needsReviewKeys: auditEntries.filter((entry) => entry.status === 'needs-review'),
        protectedUnusedKeys: auditEntries.filter((entry) => entry.status === 'protected'),
      }, null, 2));
      return;
    }
    printAuditUnusedReport(analysis, auditEntries);
    return;
  }

  if (command === 'prune') {
    const write = flags.has('write');
    const allConfirmed = flags.has('all-confirmed');
    const prefixes = normalizePrefixList(flags.has('prefix') ? [flags.get('prefix')] : []);
    if (write && prefixes.length === 0 && !allConfirmed) {
      throw new Error('prune --write requires --prefix or --all-confirmed to avoid deleting broad dynamic i18n keys accidentally.');
    }
    const analysis = await analyzeProject(analyzeOptions);
    const result = await pruneUnusedKeys({ analysis, write, prefixes, allConfirmed });
    if (result.removedKeys.length === 0) {
      const scope = allConfirmed ? ' confirmed by audit' : prefixes.length > 0 ? ` under ${prefixes.join(', ')}` : '';
      console.log(`No high-confidence unused i18n keys to prune${scope}.`);
      return;
    }
    const scope = allConfirmed ? ' confirmed by audit' : prefixes.length > 0 ? ` under ${prefixes.join(', ')}` : '';
    console.log(`${write ? 'Removed' : 'Would remove'} ${result.removedKeys.length} high-confidence unused i18n key(s)${scope}:`);
    for (const key of result.removedKeys) {
      console.log(`- ${key}`);
    }
    if (!write) {
      console.log('\nRun with --prefix <key-prefix> --write or --all-confirmed --write to update locale files.');
    }
    return;
  }

  if (command === 'set-key') {
    const key = positional[1];
    if (!key) {
      throw new Error('set-key requires a key.');
    }

    const localeFiles = await readLocaleFiles(
      analyzeOptions.rootDirectory ?? projectRoot,
      analyzeOptions.localeFilePaths ?? DEFAULT_LOCALE_FILES,
    );
    const setKeyOptions = collectSetKeyCliOptions(
      localeFiles,
      flags,
      positional.slice(2),
    );

    const result = await setLocaleKey({
      ...analyzeOptions,
      key,
      valuesByLocale: setKeyOptions.valuesByLocale,
      write: setKeyOptions.write,
      allowOverwrite: setKeyOptions.allowOverwrite,
    });
    printSetKeyResult(result);
    return;
  }

  if (command === 'find-text') {
    const analysis = await analyzeProject(analyzeOptions);
    const query = positional.slice(1).join(' ');
    if (!query) {
      throw new Error('find-text requires a text query.');
    }
    printFindTextResults(findKeysByText(analysis, query, { locale: flags.get('locale') }));
    return;
  }

  if (command === 'find-key') {
    const analysis = await analyzeProject(analyzeOptions);
    const query = positional[1];
    if (!query) {
      throw new Error('find-key requires a key or prefix.');
    }
    printFindKeyResults(findKeysByPrefix(analysis, query));
    return;
  }

  throw new Error(`Unknown command: ${command}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === currentFilePath) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
