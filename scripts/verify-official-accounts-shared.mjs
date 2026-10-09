/**
 * Guards the "official accounts are one shared section" rule.
 *
 * Every CLI that shows an official login renders
 * `web/features/coding/shared/officialAccounts/OfficialAccountsSection` and
 * supplies only a mapping from its own account records to
 * `OfficialAccountRowView`. The layout, the row actions and the empty state live
 * in the shared component.
 *
 * Why this needs a guard: the section existed three times — inside Codex's
 * provider card, inside Kimi's and ZCode's own cards — and the three had drifted
 * in ways nobody chose. The same action was labelled differently, the same
 * explanation was a subtitle in one and a footnote in another, and a switch
 * button was hidden in one and drawn-but-disabled in another. None of that
 * fails a build; it just means every wording change has to be made three times,
 * and any of the three can be missed.
 *
 * The check is structural: a module that renders an official-account list must
 * not draw the account rows itself (no per-row switch/delete/save buttons built
 * by hand) and must import the shared section. Modules listed in
 * `PENDING_MIGRATION` are exempt, and **the list may only shrink**: migrating one
 * without removing its entry is an error, so the list cannot silently go stale.
 *
 * Scope: `web/features/coding/<cli>/components/*OfficialAccount*.tsx` plus the
 * provider cards that embed the section.
 */
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const webRoot = fileURLToPath(new URL('../web', import.meta.url));
const codingRoot = path.join(webRoot, 'features', 'coding');

const SKIPPED_DIRECTORIES = new Set(['node_modules', '__tests__', 'fixtures']);

/** The one implementation every CLI must render. */
const SHARED_SECTION_IMPORT = 'shared/officialAccounts';

/**
 * Files that render official-account rows but are not the migrated section yet.
 *
 * **This list may only shrink.** Remove an entry in the same commit that moves
 * the file onto the shared section.
 *
 * These three were invisible until 2026-10-09 because the check looked for
 * `anticon-swap` / `anticon-check` — Ant Design's *rendered* class names, which
 * never appear in source. The rule now keys on the shared import instead.
 */
const PENDING_MIGRATION = new Set([
  'antigravity/components/AntigravityProviderCard.tsx',
  'geminicli/components/GeminiCliProviderCard.tsx',
  'grok/components/GrokProviderCard.tsx',
]);

/** The shared implementation itself; it is not a consumer. */
const SHARED_DIRECTORY = path.join('shared', 'officialAccounts');

/** What a violation means, and the fix. */
const VIOLATION_FIX =
  'map this CLI\'s records to `OfficialAccountRowView` and render ' +
  '`shared/officialAccounts`, instead of drawing the title, rows and their ' +
  'actions here';

const isGovernedFile = (filename) =>
  /OfficialAccount.*\.tsx$/.test(filename) || filename.endsWith('ProviderCard.tsx');

async function collectGovernedFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const found = [];
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (SKIPPED_DIRECTORIES.has(entry.name)) {
        continue;
      }
      found.push(...(await collectGovernedFiles(path.join(directory, entry.name))));
      continue;
    }
    if (entry.isFile() && isGovernedFile(entry.name)) {
      const absolute = path.join(directory, entry.name);
      // The shared implementation is the thing every consumer imports; it is
      // not itself a consumer, and it necessarily iterates the rows.
      if (absolute.includes(SHARED_DIRECTORY)) {
        continue;
      }
      found.push(absolute);
    }
  }
  return found;
}

const files = (await collectGovernedFiles(codingRoot)).sort();
const violations = [];
const unexpectedlyClean = [];

for (const file of files) {
  const relative = path.relative(codingRoot, file).split(path.sep).join('/');
  const fullRelative = path.relative(webRoot, file).split(path.sep).join('/');
  const source = await readFile(file, 'utf8');

  // A file that does not touch official accounts at all is out of scope: most
  // provider cards never render the section.
  const mentionsOfficialAccounts = /officialAccount/i.test(source);
  if (!mentionsOfficialAccounts) {
    if (PENDING_MIGRATION.has(relative)) {
      unexpectedlyClean.push(relative);
    }
    continue;
  }

  const usesSharedSection = source.includes(SHARED_SECTION_IMPORT);
  const findings = [];
  if (!usesSharedSection) {
    // Keyed on the source-level signal every hand-written section shares: it
    // iterates its own account records to draw the rows. Icon names are *not*
    // usable here — antd renders them as classes that never appear in source,
    // and not every implementation used one for the switch.
    const rowMap = source.split('\n').findIndex(line =>
      /\.map\(/.test(line.replace(/\/\/.*$/, ''))
      && /officialAccount|accountRows/.test(line),
    );
    if (rowMap !== -1) {
      findings.push({
        line: rowMap + 1,
        text: source.split('\n')[rowMap].trim(),
        what: 'draws its own official-account rows',
        fix: VIOLATION_FIX,
      });
    }
  }

  if (findings.length === 0) {
    if (PENDING_MIGRATION.has(relative)) {
      unexpectedlyClean.push(relative);
    }
    continue;
  }

  if (PENDING_MIGRATION.has(relative)) {
    continue;
  }

  for (const finding of findings) {
    violations.push({ file: fullRelative, ...finding });
  }
}

if (files.length === 0) {
  console.error('No official-account modules found — the scan path is wrong.');
  process.exit(1);
}

let failed = false;

if (violations.length > 0) {
  failed = true;
  console.error(
    'Official accounts must render the shared section (web/features/coding/shared/officialAccounts).\n',
  );
  for (const violation of violations) {
    console.error(`  ${violation.file}:${violation.line} — ${violation.what}`);
    console.error(`    ${violation.text}`);
    console.error(`    Fix: ${violation.fix}\n`);
  }
  console.error(
    'See web/features/coding/shared/officialAccounts/AGENTS.md and ' +
      'docs/new-cli-onboarding-sop.md (official-account section).',
  );
}

if (unexpectedlyClean.length > 0) {
  failed = true;
  console.error('\nThese files are listed in PENDING_MIGRATION but no longer violate the rule.');
  console.error(
    'Migration done — delete their entries from scripts/verify-official-accounts-shared.mjs:\n',
  );
  for (const file of unexpectedlyClean) {
    console.error(`  ${file}`);
  }
}

if (failed) {
  process.exit(1);
}

console.log(
  `Official-account section check passed (${files.length} modules scanned, ` +
    `${PENDING_MIGRATION.size} still pending migration).`,
);
