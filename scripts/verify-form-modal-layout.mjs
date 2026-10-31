/**
 * Guards that every coding provider / model form dialog uses ONE layout.
 *
 * ## The rule
 *
 * Every one of these dialogs puts labels above their fields
 * (`layout="vertical"`, from `shared/modelFormLayout.ts`). That shape is what
 * gives a JSON editor or a capability-chip row the dialog's full width; a fixed
 * label column squeezed every control into the same narrow right-hand strip.
 *
 * ## What the check is really about
 *
 * **Uniformity, not the direction.** The original bug was one dialog using
 * vertical while five used horizontal — the ZCode page read as two products'
 * chrome. A single dialog switching *back* to horizontal recreates exactly
 * that, so this script does not care which layout the files use as a group; it
 * cares that they cannot disagree, and that none of them smuggles a `labelCol`
 * in alongside `layout="vertical"` (a column wins over the layout, producing a
 * label that sits above the field *and* is indented by a phantom column).
 *
 * ## Why this is a source scan
 *
 * The failure is invisible at runtime: `layout="horizontal"` still compiles,
 * still renders, still works. During the ZCode migration the same mistake was
 * made twice (the provider form, then the model form), each time discovered
 * only by looking at the screen.
 *
 * Scope is deliberately narrow: only files named `*ProviderFormModal.tsx` and
 * `*ModelFormModal.tsx`, plus the shared `ModelFormModal` itself. Elsewhere in
 * the app — single-field config modals, settings forms — any layout is fine.
 */
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const webRoot = fileURLToPath(new URL('../web', import.meta.url));
const codingRoot = path.join(webRoot, 'features', 'coding');
const sharedModelFormModal = path.join(
  webRoot,
  'components',
  'common',
  'ModelFormModal',
  'index.tsx',
);

/** Directory names skipped while walking; none currently hold form modals. */
const SKIPPED_DIRECTORIES = new Set(['node_modules', '__tests__', 'fixtures']);

const isGovernedFile = (filename) =>
  filename.endsWith('ProviderFormModal.tsx')
  || filename.endsWith('ModelFormModal.tsx')
  // `ModelsJsonModelModal` / `ModelsJsonProviderFormModal` differ only in the
  // middle word; they are the WorkBuddy / CodeBuddy dialogs and must follow the
  // same rule. Matching on the suffix alone silently skipped them.
  || filename.endsWith('ModelModal.tsx')
  || filename.endsWith('ProviderModal.tsx');

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

const files = [
  ...(await collectGovernedFiles(codingRoot)),
  sharedModelFormModal,
].sort();

/**
 * Every layout a governed file states, with where it was found.
 *
 * Comments are stripped first. The previous version tested
 * `source.includes('MODEL_FORM_LAYOUT')`, so a comment that merely *named* the
 * constant — every explanatory note added after the vertical migration does —
 * counted as a second layout and the file failed with "states more than one
 * layout". The interesting part is the constant actually being passed.
 */
const layoutOf = (source, relativePath) => {
  const code = source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
  const layouts = new Set();
  for (const match of code.matchAll(/layout=["'{](horizontal|vertical)/g)) {
    layouts.add(match[1]);
  }
  // The shared constant also counts: a file can only be judged by what it
  // renders, and `layout={MODEL_FORM_LAYOUT}` renders `vertical`.
  if (/layout=\{MODEL_FORM_LAYOUT\}/.test(code)) {
    layouts.add('via-constant');
  }
  void relativePath;
  return layouts;
};

/**
 * Field-name pairs that are one fact split in two, and therefore must sit
 * inside one `MODEL_FORM_PAIR_STYLE` grid.
 *
 * Kept as literal field names rather than a semantic rule because the set is
 * small, stable and already decided (user-directed, 2026-11-01: 「支持的思考档位
 * 默认思考档位 一组 输入模态 输出模态 一组」). The point is that the list is
 * written down somewhere the next dialog cannot quietly ignore; a pair nobody
 * has adjudicated falls through and stays a judgement call.
 *
 * `ModelsJsonModelModal` uses `MODEL_FORM_TRIPLE_STYLE` for its limits (both
 * plus temperature), which the check accepts.
 */
const PAIRED_FIELD_NAMES = [
  ['reasoningLevels', 'defaultReasoningLevel'],
  ['reasoningEfforts', 'reasoningEffort'],
  ['supportEfforts', 'defaultEffort'],
  ['inputModalities', 'outputModalities'],
  ['contextWindow', 'maxOutputTokensMax'],
  ['contextWindow', 'maxTokens'],
  ['maxInputTokens', 'maxOutputTokens'],
  // Codex pairs the context window with its speed tiers (user-directed,
  // 2026-11-01: 「上下文窗口 和 速度等级 放到一行吧」) — both size the model's
  // output envelope.
  ['contextWindow', 'serviceTiers'],
  // Kimi's three payload sizes: how much the model holds, how much may go in,
  // how much may come back (triple row).
  ['maxContextSize', 'maxInputSize'],
];

/**
 * Start of the opening tag that encloses `position`, by counting JSX tag depth
 * backwards from that point.
 *
 * `MODEL_FORM_PAIR_STYLE` is written on the wrapper `<div>` before the first
 * field, so a naive "is the constant between the two `name=` attributes" test
 * fails on every correctly-paired dialog. Walking back to the enclosing element
 * asks the question the rule actually means: are both fields inside one grid?
 *
 * Only JSX open/close tags are counted (`<Tag`, `</Tag`, `<Tag … />`); braces,
 * strings and comments are not — a cheap scan is enough for these files, and a
 * miss falls through to "no violation" rather than a false positive.
 */
function precedingUnclosedTagStart(source, position) {
  const before = source.slice(0, position);
  const tags = [...before.matchAll(/<\/?([A-Za-z][\w.]*)([^<>]*?)(\/?)>/g)];
  let depth = 0;
  for (let index = tags.length - 1; index >= 0; index -= 1) {
    const [full, , , selfClosing] = tags[index];
    if (selfClosing) continue;
    if (full.startsWith('</')) {
      depth += 1;
    } else if (depth === 0) {
      return tags[index].index;
    } else {
      depth -= 1;
    }
  }
  return -1;
}

/** Index of `needle` in `source`, ignoring matches that fall inside comments. */
function codeIndexOf(source, needle) {
  let from = 0;
  for (;;) {
    const at = source.indexOf(needle, from);
    if (at === -1) return -1;
    const before = source.slice(0, at);
    const lastBlockOpen = before.lastIndexOf('/*');
    const lastBlockClose = before.lastIndexOf('*/');
    const lineStart = before.lastIndexOf('\n');
    const linePrefix = before.slice(lineStart + 1);
    const inBlockComment = lastBlockOpen > lastBlockClose;
    const inLineComment = /(^|[^:])\/\//.test(linePrefix);
    if (!inBlockComment && !inLineComment) return at;
    from = at + needle.length;
  }
}

/**
 * Files whose name matches the scan but which do not own a `<Form>`.
 *
 * Each entry must say why. A forwarder that renders another dialog has no
 * layout of its own to be wrong about.
 */
const EXEMPT_FILES = new Map([
  [
    'features/coding/shared/providerShare/ShareProviderModal.tsx',
    'Thin forwarder: builds a request and renders `ProviderTransferModal` (features/shared/deepLink). It declares no Form, so there is no layout here to be inconsistent about.',
  ],
]);

const violations = [];
const perFileLayout = new Map();

for (const file of files) {
  const source = await readFile(file, 'utf8');
  // `path.relative` yields backslashes on Windows; the exemption keys are
  // written with the forward slashes the rest of the repo uses.
  const relative = path.relative(webRoot, file).split(path.sep).join('/');
  if (EXEMPT_FILES.has(relative)) {
    continue;
  }
  const layouts = layoutOf(source, relative);

  if (layouts.size === 0) {
    violations.push({
      file: relative,
      problem: 'states no `layout` at all — it would inherit whatever its parent Form used',
    });
    continue;
  }
  if (layouts.size > 1) {
    violations.push({
      file: relative,
      problem: `states more than one layout (${[...layouts].join(', ')}) — one dialog, one shape`,
    });
    continue;
  }

  const onlyLayout = [...layouts][0];
  perFileLayout.set(relative, onlyLayout);

  // A `labelCol` wins over `layout="vertical"`: the label ends up above the
  // field but still indented by a column that no longer exists. `wrapperCol`
  // has the same effect on the control — Grok's backend-search checkbox kept
  // `wrapperCol={{ offset: 5, span: 19 }}` long after the dialog went vertical,
  // which indented it and squeezed it to 19/24 of the width for no reason.
  if (onlyLayout !== 'horizontal') {
    for (const [index, line] of source.split('\n').entries()) {
      const match = /\b(labelCol|wrapperCol)=/.exec(line);
      if (!match) continue;
      violations.push({
        file: relative,
        line: index + 1,
        problem: `passes \`${match[1]}\` alongside a non-horizontal layout (the column wins)`,
      });
    }
  }

  // Fields that are one fact split in two must share a row
  // (`MODEL_FORM_PAIR_STYLE`). This is a source scan because the defect is
  // purely spatial: a stacked pair renders, validates and saves exactly like a
  // paired one — it just costs a label line plus a control line per field, and
  // a dialog of eight short selects outgrows the viewport.
  //
  // The check is name-based on purpose: it names the fields that are known to
  // pair, so adding a new dialog with `reasoningLevels` stacked is caught
  // without an AST. A field not in the list (a name, an id) is left alone —
  // those genuinely belong on their own rows.
  for (const [left, right] of PAIRED_FIELD_NAMES) {
    const leftAt = codeIndexOf(source, `name="${left}"`);
    const rightAt = codeIndexOf(source, `name="${right}"`);
    if (leftAt === -1 || rightAt === -1) continue;
    const firstAt = Math.min(leftAt, rightAt);
    const lastAt = Math.max(leftAt, rightAt);
    // The grid is not *between* the two `name=` attributes — it is the element
    // enclosing both, so it appears before the first one and (as a closing tag)
    // after the second. Look at the enclosing element: walk back to the nearest
    // unclosed opening tag before `firstAt`.
    const enclosingStart = precedingUnclosedTagStart(source, firstAt);
    if (enclosingStart === -1) continue;
    const enclosing = source.slice(enclosingStart, lastAt);
    if (!enclosing.includes('MODEL_FORM_PAIR_STYLE') && !enclosing.includes('MODEL_FORM_TRIPLE_STYLE')) {
      violations.push({
        file: relative,
        line: source.slice(0, lastAt).split('\n').length,
        problem: `\`${left}\` and \`${right}\` are one fact in two halves but are not inside a \`MODEL_FORM_PAIR_STYLE\` grid`,
      });
    }
  }
}

/**
 * Cross-file uniformity, **within each family**.
 *
 * Model dialogs and provider dialogs are separate families with different field
 * shapes: a model dialog carries JSON editors and a capability-chip row, a
 * provider dialog is a handful of one-line inputs. They were migrated on
 * different dates, so comparing one family against the other would report a
 * violation that nothing can fix without migrating both in the same commit.
 *
 * Within a family the rule still holds — that is the ZCode bug: one model
 * dialog vertical, five horizontal.
 */
const familyOf = (relative) => (
  // Model forms first: the shared `components/common/ModelFormModal/index.tsx`
  // ends in `index.tsx`, not `ModelFormModal.tsx`, so the suffix test alone
  // classified it as a provider dialog.
  /ModelFormModal/.test(relative) || /ModelModal\.tsx$/.test(relative)
    ? 'model'
    : 'provider'
);

const layoutsByFamily = new Map();
for (const [file, layout] of perFileLayout) {
  const family = familyOf(file);
  const existing = layoutsByFamily.get(family) ?? new Set();
  existing.add(layout);
  layoutsByFamily.set(family, existing);
}

const distinctLayouts = new Set(perFileLayout.values());
for (const [family, layouts] of layoutsByFamily) {
  if (layouts.size <= 1) {
    continue;
  }
  const byLayout = new Map();
  for (const [file, layout] of perFileLayout) {
    if (familyOf(file) !== family) {
      continue;
    }
    byLayout.set(layout, [...(byLayout.get(layout) ?? []), file]);
  }
  for (const [layout, group] of byLayout) {
    violations.push({
      file: group.join(', '),
      problem: `${family} dialog uses layout "${layout}" while its siblings use `
        + `${[...layouts].filter((item) => item !== layout).join(' / ')}`,
    });
  }
}

if (files.length === 0) {
  console.error('No provider/model form modals found — the scan path is wrong.');
  process.exit(1);
}

if (violations.length > 0) {
  console.error('Every coding provider/model dialog must use the same form layout.\n');
  for (const violation of violations) {
    console.error(`  ${violation.file}${violation.line ? `:${violation.line}` : ''}`);
    console.error(`    ${violation.problem}\n`);
  }
  console.error(
    'Fix: import MODEL_FORM_LAYOUT from `features/coding/shared/modelFormLayout` and pass\n'
      + '`layout={MODEL_FORM_LAYOUT}`. Never combine it with `labelCol` / `wrapperCol`.\n'
      + 'See docs/new-cli-onboarding-sop.md §4.0.2-A.',
  );
  process.exit(1);
}

console.log(
  `Form modal layout check passed (${files.length} files, all `
  + `${[...distinctLayouts].join('/')}).`,
);
