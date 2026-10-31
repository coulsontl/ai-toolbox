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

/** Every layout literal a governed file states, with where it was found. */
const layoutOf = (source, relativePath) => {
  const layouts = new Set();
  for (const match of source.matchAll(/layout=["'{](horizontal|vertical)/g)) {
    layouts.add(match[1]);
  }
  // The shared constant also counts: a file can only be judged by what it
  // renders, and `layout={MODEL_FORM_LAYOUT}` renders `vertical`.
  if (source.includes('MODEL_FORM_LAYOUT')) {
    layouts.add('via-constant');
  }
  void relativePath;
  return layouts;
};

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
  // field but still indented by a column that no longer exists.
  if (onlyLayout !== 'horizontal' && /\blabelCol=/.test(source)) {
    for (const [index, line] of source.split('\n').entries()) {
      if (/\blabelCol=/.test(line)) {
        violations.push({
          file: relative,
          line: index + 1,
          problem: 'passes `labelCol` alongside a non-horizontal layout (the column wins)',
        });
      }
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
