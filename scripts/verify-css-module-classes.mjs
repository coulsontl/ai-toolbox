/**
 * Guards "every CSS Modules class a component references actually exists".
 *
 * Why this needs a guard: a CSS Modules import is a plain object, so a missing
 * key is not a type error, not a lint error, and not a runtime error either —
 * it evaluates to `undefined` and React simply renders no class. The element
 * keeps its children, its handlers and its text; it only loses its styling.
 *
 * That is exactly how `OhMyPiPage` lost the spacing and `scroll-margin-top` of
 * all six of its section wrappers: the page referenced `styles.ompSection`
 * while the stylesheet defined `.piSection`. Every static check passed, the
 * page rendered, and the defect surfaced only as "the gaps look wrong" after a
 * migration (2026-10-22).
 *
 * Scope: every `web/**\/*.tsx` that imports a `*.module.less`. Dynamic access
 * (`styles[`level${n}`]`) cannot be checked statically and is skipped.
 */
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const webRoot = fileURLToPath(new URL('../web', import.meta.url));

const SKIPPED_DIRECTORIES = new Set(['node_modules', '__tests__', 'fixtures']);

/**
 * Known offenders, with the reason each one is not fixed yet.
 *
 * **This list may only shrink.** Fixing a file without removing its entry is an
 * error, so the list cannot silently go stale.
 */
const KNOWN_MISSING = new Map([
  [
    'features/coding/codex/components/CodexMemoriesPanel/CodexMemoriesPanel.tsx',
    'modalEditorBody',
  ],
  [
    'features/coding/gateway/components/GatewayAggregateSettings.tsx',
    'listActions',
  ],
  [
    'features/coding/shared/providerModelRewrites/ModelRewritesCollapse.tsx',
    'toolbar',
  ],
]);

/**
 * Orphan counts that existed when the reverse check was added (2026-10-23).
 *
 * **This baseline may only shrink.** Each number is how many classes its
 * stylesheet defines without any importer referencing them. Deleting a class
 * without lowering its number is an error, so the set cannot silently go stale.
 *
 * **Empty since 2026-10-23** (user-directed: "迁移留下大量孤儿 CSS，全删了") —
 * the 28 stylesheets / 112 classes registered when the reverse check landed are
 * gone. Keep the map: a new CLI whose migration legitimately leaves a class
 * behind has somewhere to record it, and the "listed but clean" check below
 * only fires for entries.
 */
const KNOWN_ORPHAN_COUNTS = new Map();

/** Every `.tsx` under `directory`, skipping the usual noise. */
async function walk(directory) {
  return walkMatching(directory, name => name.endsWith('.tsx'));
}

/** Every `*.module.less` under `directory`. */
async function walkStylesheets(directory) {
  return walkMatching(directory, name => name.endsWith('.module.less'));
}

async function walkMatching(directory, matches) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (SKIPPED_DIRECTORIES.has(entry.name)) continue;
      files.push(...await walkMatching(path.join(directory, entry.name), matches));
    } else if (entry.isFile() && matches(entry.name)) {
      files.push(path.join(directory, entry.name));
    }
  }
  return files;
}

// `import styles from './X.module.less'` / `import cardStyles from './Y.module.less'`
const MODULE_IMPORT = /import\s+([A-Za-z_$][\w$]*)\s+from\s+['"]([^'"]+\.module\.less)['"]/g;

/**
 * Class names a Less file defines.
 *
 * Covers the three shapes this codebase uses: a top-level `.name`, a nested
 * `&.name`, and `.a.name` / `.a .name` inside a selector list. Pseudo-classes
 * and attribute selectors are ignored because they cannot be a module key.
 */
function definedClassNames(lessSource) {
  const names = new Set();
  // Comments first, and `//` as well as `/* */`. A comment that *mentions* a
  // class (`// was `.ompSection``) otherwise counts as a definition and hides
  // exactly the mismatch this guard exists to find — that is how the first
  // version of this script passed while `OhMyPiPage` was still broken. `//` is
  // only a comment when it does not follow `:` (URLs inside `url(...)`).
  const withoutComments = lessSource
    .replace(/\/\*[\s\S]*?\*\//g, '')
    // `@import '../styles/buttonSystem.less'` — the extension is not a class.
    .replace(/@import\s+[^;]+;/g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
  // Drop `:global` regions first — **both shapes**. Less accepts the call form
  // `:global(.ant-collapse) { … }` and the block form `:global { .ant-collapse
  // { … } }`; this repo uses both. Everything inside targets Ant Design's own
  // class names, which are emitted verbatim and are never importable as module
  // keys, so counting them reports every such rule as an orphan. Missing the
  // block form is what made the first version of this reverse check list
  // `.ant-form-item`, `.ant-card-head` and friends as deletable.
  const withoutGlobals = withoutComments.replace(
    /:global\((?:[^()]|\([^()]*\))*\)/g,
    '',
  ).replace(/:global\s*\{/g, ':global-block{');
  const withoutMixins = stripMixinSyntax(stripGlobalBlocks(withoutGlobals));
  for (const match of withoutMixins.matchAll(/\.([A-Za-z_][\w-]*)/g)) {
    names.add(match[1]);
  }
  return names;
}

/** Removes each `:global-block{ … }` region, brace-matched. */
function stripGlobalBlocks(source) {
  let result = '';
  let cursor = 0;
  for (;;) {
    const start = source.indexOf(':global-block{', cursor);
    if (start === -1) {
      result += source.slice(cursor);
      return result;
    }
    result += source.slice(cursor, start);
    let depth = 0;
    let index = start + ':global-block'.length;
    for (; index < source.length; index++) {
      const character = source[index];
      if (character === '{') depth++;
      else if (character === '}') {
        depth--;
        if (depth === 0) break;
      }
    }
    cursor = index + 1;
  }
}

/**
 * Removes Less **mixin** definitions and calls, which look like class names.
 *
 * `buttonSystem.less` defines `.buttonBase() { … }` and every consumer writes
 * `.buttonBase();` inside a rule. Both match a naive `\.(\w+)` scan, so the
 * first version of the reverse check offered 22 image-stylesheet "orphans" that
 * were mixins in active use — deleting them breaks the build. A mixin is told
 * apart by the empty parameter list: a class selector is `.name {` or `.name,`,
 * a definition is `.name()` or `.name(@x)`, a call is `.name();`.
 */
function stripMixinSyntax(source) {
  return source.replace(/\.[A-Za-z_][\w-]*\s*\([^()]*\)\s*(?=[{;,)])/g, '');
}

async function main() {
  const violations = [];
  const stale = new Set(KNOWN_MISSING.keys());

  /**
   * One stylesheet can be imported by several components — `styles.module.less`
   * is shared by a page's parts, `GatewayStatisticsView.module.less` by the
   * statistics view and both chart components. Orphan detection therefore has
   * to pool **every** importer's references per file; asking each importer on
   * its own reported 2655 "orphans", nearly all of them real classes used by a
   * sibling. The forward check stays per-importer, because a missing key is a
   * fact about one import.
   */
  const accessedByStylesheet = new Map();

  for (const filePath of await walk(webRoot)) {
    const source = await readFile(filePath, 'utf8');
    const imports = [...source.matchAll(MODULE_IMPORT)];
    if (imports.length === 0) continue;

    // Drop the import statements themselves: `'./styles.module.less'` contains
    // the literal `styles.module`, which would read as a member access.
    const body = source.replace(MODULE_IMPORT, '');

    for (const [, localName, lessRelativePath] of imports) {
      const lessPath = path.resolve(path.dirname(filePath), lessRelativePath);
      const defined = definedClassNames(await readFile(lessPath, 'utf8'));

      // `styles[`level${n}`]` is computed at runtime; only static member
      // accesses can be resolved.
      const accessed = new Set();
      for (const match of body.matchAll(new RegExp(`\\b${localName}\\.([A-Za-z_$][\\w$]*)`, 'g'))) {
        accessed.add(match[1]);
      }

      const relativeFile = path.relative(webRoot, filePath).split(path.sep).join('/');
      for (const className of accessed) {
        if (defined.has(className)) continue;
        violations.push({ file: relativeFile, className, lessPath });
      }

      // `styles[`level${n}`]` — capture the literal prefix before the template
      // hole so a computed key narrows the search instead of disabling it.
      // The prefix stops at the first `$`: `styles[`level${n}`]` is `level`,
      // and capturing the `$` too would make every prefix unmatchable.
      const dynamicPrefixPattern = new RegExp(
        String.raw`\b` + localName + String.raw`\[\x60([A-Za-z_][\w]*)`,
        'g',
      );
      const entry = accessedByStylesheet.get(lessPath) ?? {
        defined: definedClassNames(await readFile(lessPath, 'utf8')),
        accessed: new Set(),
        /**
         * Prefixes reached by a computed key: `styles[`level${n}`]` yields
         * `level`. A computed key does **not** opt the whole file out — that was
         * the first version's mistake, and it hid seven real orphans behind
         * `SessionDetailWorkbench`'s `messageRole${…}`. It only means "any class
         * starting with this prefix might be used"; everything else is still
         * checkable.
         */
        dynamicPrefixes: new Set(),
        importers: [],
      };
      for (const className of accessed) entry.accessed.add(className);
      for (const match of body.matchAll(dynamicPrefixPattern)) {
        entry.dynamicPrefixes.add(match[1]);
      }
      if (!entry.importers.includes(relativeFile)) entry.importers.push(relativeFile);
      accessedByStylesheet.set(lessPath, entry);
    }
  }

  const orphansByStylesheet = new Map();
  for (const [lessPath, entry] of accessedByStylesheet) {
    const orphans = [];
    for (const className of entry.defined) {
      if (entry.accessed.has(className)) continue;
      // A class a computed key could reach (`level0` under `level`) is not
      // provably unused.
      let reachedDynamically = false;
      for (const prefix of entry.dynamicPrefixes) {
        if (className.startsWith(prefix)) {
          reachedDynamically = true;
          break;
        }
      }
      if (reachedDynamically) continue;
      // The reverse direction: a class the stylesheet defines but no component
      // reads. That is what a migration leaves behind — the page drops its own
      // header, provider card and modal markup, and their classes stay in the
      // file looking like they still matter. A one-word typo shows up in both
      // directions at once: `styles.ompSection` against a `.piSection`
      // definition is one missing class **and** one orphan.
      orphans.push(className);
    }
    if (orphans.length > 0) {
      orphansByStylesheet.set(
        path.relative(webRoot, lessPath).split(path.sep).join('/'),
        { orphans, importers: entry.importers },
      );
    }
  }

  /**
   * A `*.module.less` that no module imports is entirely dead — every class in
   * it, not just the ones a reference happens to miss. `github-markdown-overrides
   * .module.less` sat unimported for months as a stale copy of
   * `MarkdownPreview/index.module.less`; the per-class check above never saw it
   * because it only looks at stylesheets that *have* an importer.
   */
  const importedStylesheets = new Set(accessedByStylesheet.keys());
  for (const stylesheetPath of await walkStylesheets(webRoot)) {
    if (importedStylesheets.has(stylesheetPath)) continue;
    const defined = [...definedClassNames(await readFile(stylesheetPath, 'utf8'))];
    if (defined.length === 0) continue;
    orphansByStylesheet.set(
      path.relative(webRoot, stylesheetPath).split(path.sep).join('/'),
      { orphans: defined, importers: [] },
    );
  }

  const unexpected = violations.filter(
    ({ file, className }) => KNOWN_MISSING.get(file) !== className,
  );
  for (const { file, className } of violations) {
    if (KNOWN_MISSING.get(file) === className) stale.delete(file);
  }

  if (unexpected.length > 0) {
    console.error('CSS Modules classes referenced but never defined:');
    for (const { file, className, lessPath } of unexpected) {
      console.error(`  web/${file}: styles.${className} (not in ${path.relative(webRoot, lessPath)})`);
    }
    console.error('\nA missing key evaluates to `undefined`: the element renders with no class at all.');
    console.error('Fix the reference or define the class — do not add it to KNOWN_MISSING.');
    process.exit(1);
  }

  if (stale.size > 0) {
    console.error('KNOWN_MISSING lists files that no longer violate — remove these entries:');
    for (const file of stale) console.error(`  web/${file}`);
    process.exit(1);
  }

  const grown = [];
  for (const [stylesheet, { orphans, importers }] of orphansByStylesheet) {
    const allowed = KNOWN_ORPHAN_COUNTS.get(stylesheet);
    if (allowed === undefined) {
      for (const className of orphans) grown.push({ stylesheet, className, importers, allowed: 0 });
      continue;
    }
    if (orphans.length > allowed) {
      // Name the excess, not the whole set: the baseline entries are expected.
      for (const className of orphans.slice(allowed)) {
        grown.push({ stylesheet, className, importers, allowed });
      }
    }
  }

  if (grown.length > 0) {
    console.error('CSS Modules classes defined but never referenced (new or above baseline):');
    for (const { stylesheet, className, importers, allowed } of grown) {
      console.error(`  ${stylesheet}: .${className}  [importers: ${importers.join(', ')}]  (baseline ${allowed})`);
    }
    console.error('\nA class nothing references is what a migration leaves behind: it still reads');
    console.error('like live styling, so the next change edits it and sees no effect. Delete it —');
    console.error('or, if the component was supposed to use it, that missing reference is the bug.');
    console.error('If this is legitimate pre-existing debt, raise that stylesheet\'s number in');
    console.error('KNOWN_ORPHAN_COUNTS and say why in the commit message.');
    process.exit(1);
  }

  const shrank = [];
  for (const [stylesheet, allowed] of KNOWN_ORPHAN_COUNTS) {
    const actual = orphansByStylesheet.get(stylesheet)?.orphans.length ?? 0;
    if (actual < allowed) shrank.push({ stylesheet, allowed, actual });
  }
  if (shrank.length > 0) {
    console.error('KNOWN_ORPHAN_COUNTS is stale — lower or remove these entries:');
    for (const { stylesheet, allowed, actual } of shrank) {
      console.error(`  ${stylesheet}: baseline ${allowed}, actual ${actual}`);
    }
    process.exit(1);
  }

  const totalOrphans = [...orphansByStylesheet.values()]
    .reduce((sum, entry) => sum + entry.orphans.length, 0);
  console.log(
    `CSS Modules class check passed (${violations.length} known missing, ${totalOrphans} orphans, all at baseline).`,
  );
}

await main();
