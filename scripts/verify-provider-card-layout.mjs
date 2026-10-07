/**
 * Guards the "provider card is a thin mapping layer" rule (AGENTS.md Hard Rule 14).
 *
 * Every CLI's provider list must render one of the three fixed styles in
 * `web/features/coding/shared/providerCardVariants/`, and the CLI's own
 * `*ProviderCard.tsx` must only map its storage shape onto `ProviderCardVariantProps`.
 * The layout itself — the drag registration, the selection checkbox, the card
 * chrome — lives in `CardShell` and the three style components.
 *
 * Why this needs a guard: the rule existed since the variants were introduced,
 * but two of the three styles (`ClaudeStyleCard`, `CodexStyleCard`) had **zero
 * consumers** while their natural owners — Claude Code and Codex — kept 776- and
 * 1134-line bespoke cards that re-implemented the same chrome. Nothing failed:
 * it compiles, renders, and works. The only symptom is that changing a shared
 * behaviour (card spacing, drag handle, selection border) needs nine edits
 * instead of one, and any of the nine can be forgotten.
 *
 * The check is structural: a mapping layer does not import `useSortable`, does
 * not render its own `<Card>`, and does not hand-roll the selection checkbox.
 * All three are `CardShell`'s job.
 *
 * The migration is not finished, so the check runs as a **ratchet**: the four
 * cards still holding their own chrome are listed in `PENDING_MIGRATION`. The
 * set may only shrink — migrating one without removing it from the list is an
 * error, so the list cannot silently go stale.
 *
 * Scope: `web/features/coding/<cli>/components/*ProviderCard.tsx`. Files whose
 * name matches but which are not list cards are listed in `EXEMPT_FILES`.
 */
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const webRoot = fileURLToPath(new URL('../web', import.meta.url));
const codingRoot = path.join(webRoot, 'features', 'coding');

/** Directory names skipped while walking. */
const SKIPPED_DIRECTORIES = new Set(['node_modules', '__tests__', 'fixtures']);

/**
 * Files that match `*ProviderCard.tsx` but are not provider-list cards, so the
 * rule does not apply. Each entry must state why.
 */
const EXEMPT_FILES = new Map([
  [
    'zcode/components/ZcodeOfficialAccountCard.tsx',
    'Official-account row card, not a provider-list card; it is not sortable and has no provider chrome.',
  ],
  [
    'antigravity/components/AntigravityProviderCard.tsx',
    'Despite the name, this is the official-account panel for a single official provider (rendered in its own Collapse section, returns null for non-official providers). It has no provider-list chrome. The misleading name is recorded in the SOP as a rename candidate.',
  ],
]);

/**
 * Cards that still hold their own chrome. **This list may only shrink.**
 *
 * Remove an entry in the same commit that migrates the file to a
 * `providerCardVariants` style; the check fails if an entry no longer violates.
 */
const PENDING_MIGRATION = new Set([
  'claudedesktop/components/ClaudeDesktopProviderCard.tsx',
  'geminicli/components/GeminiCliProviderCard.tsx',
  'grok/components/GrokProviderCard.tsx',
]);

/**
 * What a thin mapping layer must not contain, and the fix for each.
 *
 * The message names the component that already does the job, so the reader does
 * not have to find it.
 */
const FORBIDDEN = [
  {
    pattern: /\buseSortable\s*\(/,
    what: 'registers its own sortable node',
    fix: 'pass `modelSection.sortableId` / `modelSection.draggable` (or `dragDisabled`) and let CardShell register it',
  },
  {
    // `\b` rather than `[\s>]`: the tag may end the line, and `<CardShell` must
    // not match.
    pattern: /<Card\b/,
    what: 'renders its own <Card>',
    fix: 'render a style component (ClaudeStyleCard / CodexStyleCard / OpenCodeStyleCard); CardShell owns the card chrome',
  },
  {
    pattern: /\bManagementCheckbox\b/,
    what: 'hand-rolls the batch-selection checkbox',
    fix: 'pass `providerState.selectable` / `selected` / `onSelectChange`; CardShell renders the checkbox',
  },
];

const isGovernedFile = (filename) => filename.endsWith('ProviderCard.tsx');

/**
 * Every prop declared on `ProviderCardVariantProps` (and the three sub-shapes)
 * must have a render point in at least one style card.
 *
 * Why: the prop shapes are one flat structure shared by all three styles, and a
 * style reads only what it renders. A prop that **no** style renders is
 * invisible to the type checker — callers pass it, nothing happens, and the
 * next reader assumes the capability exists. This is the same defect family as
 * lesson #70 (`extraActions` declared but never rendered) and #97; `isApplied`
 * sat dead from the day it was introduced until an audit removed it in favour
 * of `accent`.
 */
const PROP_SOURCE_FILES = [
  'web/features/coding/shared/providerCardVariants/types.ts',
  'web/features/coding/shared/providerCardVariants/CardShell.tsx',
];
const STYLE_CARD_FILES = [
  'web/features/coding/shared/providerCardVariants/ClaudeStyleCard.tsx',
  'web/features/coding/shared/providerCardVariants/CodexStyleCard.tsx',
  'web/features/coding/shared/providerCardVariants/OpenCodeStyleCard.tsx',
];

/**
 * Props that intentionally have no render point, each with the reason.
 *
 * Keep this empty if at all possible: a prop here is a prop callers can pass
 * with no effect.
 */
const PROPS_WITHOUT_RENDER_POINT = new Map();

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
    if (isGovernedFile(entry.name)) {
      found.push(path.join(directory, entry.name));
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
  if (EXEMPT_FILES.has(relative)) {
    continue;
  }

  const source = await readFile(file, 'utf8');
  const lines = source.split('\n');

  // One finding per rule per file: the import line and the call site are the
  // same defect, and repeating it buries the rest.
  const findings = [];
  for (const rule of FORBIDDEN) {
    for (const [index, line] of lines.entries()) {
      // Comments may legitimately mention these names; only code counts.
      const code = line.replace(/\/\/.*$/, '');
      if (rule.pattern.test(code)) {
        findings.push({ line: index + 1, text: line.trim(), what: rule.what, fix: rule.fix });
        break;
      }
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
  console.error('No provider cards found — the scan path is wrong.');
  process.exit(1);
}

let failed = false;

/**
 * Prop names declared across the variant prop shapes.
 *
 * Read from the source rather than hard-coded so the check follows the types;
 * an interface added later is covered without touching this script.
 */
async function collectDeclaredProps() {
  const declared = new Set();
  for (const relative of PROP_SOURCE_FILES) {
    const source = await readFile(path.join(webRoot, '..', relative), 'utf8');
    // Only the variant prop shapes; `CardShellProps` is a component-internal API
    // whose every field is rendered by definition.
    const blocks = source.matchAll(
      /export interface (ProviderCardState|ProviderCardActions|ProviderCardModels|ProviderCardVariantProps|ProviderCardMetaEntry|ProviderCardModel)\b[^{]*\{([\s\S]*?)\n\}/g,
    );
    for (const [, , body] of blocks) {
      for (const line of body.split('\n')) {
        const match = /^\s{2}([a-zA-Z][a-zA-Z0-9]*)\??:/.exec(line);
        if (match) {
          declared.add(match[1]);
        }
      }
    }
  }
  return declared;
}

const styleCardSources = await Promise.all(
  STYLE_CARD_FILES.map(async (relative) => ({
    relative,
    source: await readFile(path.join(webRoot, '..', relative), 'utf8'),
  })),
);

// `modelSection.*` props are forwarded straight into `ModelListSection`, so its
// source is the render point for them.
const modelSectionSource = await readFile(
  path.join(webRoot, 'features', 'coding', 'shared', 'ModelListSection.tsx'),
  'utf8',
);

const declaredProps = await collectDeclaredProps();
const unrendered = [];
for (const prop of declaredProps) {
  if (PROPS_WITHOUT_RENDER_POINT.has(prop)) {
    continue;
  }
  // A prop counts as rendered if any style card or the shared model section
  // mentions it: `modelSection.*` is forwarded straight into `ModelListSection`.
  const mentioned = styleCardSources.some(({ source }) => source.includes(prop));
  const inModelSection = modelSectionSource.includes(prop);
  if (!mentioned && !inModelSection) {
    unrendered.push(prop);
  }
}

if (unrendered.length > 0) {
  failed = true;
  console.error(
    '\nThese props are declared on the variant prop shapes but no style card renders them.',
  );
  console.error('A caller can pass them and nothing happens. Either render them in a style card');
  console.error('(and in every style that should honour them), or delete them from types.ts:\n');
  for (const prop of unrendered) {
    console.error(`  ${prop}`);
  }
}

if (violations.length > 0) {
  failed = true;
  console.error(
    'Provider cards must be thin mapping layers over providerCardVariants (AGENTS.md Hard Rule 14).\n',
  );
  for (const violation of violations) {
    console.error(`  ${violation.file}:${violation.line} — ${violation.what}`);
    console.error(`    ${violation.text}`);
    console.error(`    Fix: ${violation.fix}\n`);
  }
  console.error(
    'See web/features/coding/shared/providerCardVariants/AGENTS.md ("迁移一个 CLI 的步骤")\n' +
      'and docs/new-cli-onboarding-sop.md §4.2.1.',
  );
}

if (unexpectedlyClean.length > 0) {
  failed = true;
  console.error(
    '\nThese files are listed in PENDING_MIGRATION but no longer violate the rule.',
  );
  console.error('Migration done — delete their entries from scripts/verify-provider-card-layout.mjs:\n');
  for (const file of unexpectedlyClean) {
    console.error(`  ${file}`);
  }
}

if (failed) {
  process.exit(1);
}

console.log(
  `Provider card layout check passed (${files.length} files, ` +
    `${PENDING_MIGRATION.size} still pending migration).`,
);
