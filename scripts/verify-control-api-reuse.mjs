/**
 * Guards the "control API and aitoolbox are adapters, never a second
 * implementation" rule (AGENTS.md Hard Rule 15).
 *
 * The control plane is the easiest place to grow a shadow implementation: a
 * route handler can parse one more config format, a dispatch branch can call
 * `apply_*` directly, and everything still compiles and passes its happy-path
 * test. The defects only show up later, as a gateway takeover that loses its
 * aggregate manifest or a `***` placeholder written into a real config.
 *
 * The checks are structural, deliberately:
 *
 * 1. No control-plane file may write files directly; config writes belong to
 *    the owning CLI module (backups, WSL sync, events). `commands.rs` (PATH
 *    install) and `discovery.rs` (discovery file) are the only intentional
 *    writers.
 * 2. No control-plane file may parse or merge TOML/YAML itself. Whole-file
 *    config semantics live in `coding::config_patch`; response-boundary
 *    redaction in `redact.rs` is the only legitimate format parser here.
 * 3. The six gateway-managed CLIs must not be applied through bare
 *    `apply_*_provider` commands. They go through `provider_switch`, which
 *    restores and re-engages the takeover.
 * 4. No family channel hash may be reimplemented here (`Sha256` or a local
 *    `fn channel_id`); call `workbuddy_family::channel::channel_id`.
 * 5. Every command in `cli/src/args.rs` must be documented in the built-in
 *    Skill, and every documented `aitoolbox <command> <subcommand>` must exist.
 *
 * The harness always runs `runSelfTests()` first: the scan is fed small
 * positive/negative fixtures, so a broken regex or a scan path that silently
 * stops matching fails the check instead of passing it.
 */
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = fileURLToPath(new URL('..', import.meta.url));
const controlRoot = path.join(repoRoot, 'tauri', 'src', 'control_api');
const argsPath = path.join(repoRoot, 'cli', 'src', 'args.rs');
const skillPath = path.join(
  repoRoot,
  'tauri',
  'resources',
  'builtin_skills',
  'ai-toolbox',
  'references',
  'control-cli.md',
);

/** `aitoolbox` install rewrites the user PATH; the discovery file is the server's own state. */
const INTENTIONAL_FILE_WRITERS = new Set(['commands.rs', 'discovery.rs']);
/** Redaction at the response boundary must understand TOML/YAML to mask values. */
const REDACTION_BOUNDARY = new Set(['redact.rs']);

// JSON object merging is deliberately not forbidden: `merge_json_config_text`
// applies the control API's own masked-PATCH contract to a JSON payload, not a
// CLI config-file format. Format-specific TOML/YAML/JSONC parsing is what must
// live in the owning module (`coding::config_patch` or the CLI module).

const RULES = [
  {
    pattern: /\bfs::(?:write|copy|remove_file|remove_dir_all|create_dir|create_dir_all)\s*\(/,
    what: 'writes or deletes a file directly',
    fix: 'call the owning CLI module command/save helper (or provider_switch) instead',
    skipFiles: INTENTIONAL_FILE_WRITERS,
  },
  {
    pattern: /\bFile::create\s*\(/,
    what: 'creates a writable file handle',
    fix: 'call the owning CLI module command/save helper instead',
    skipFiles: INTENTIONAL_FILE_WRITERS,
  },
  {
    pattern: /\bOpenOptions\b/,
    what: 'opens a writable file handle',
    fix: 'call the owning CLI module command/save helper instead',
    skipFiles: INTENTIONAL_FILE_WRITERS,
  },
  {
    pattern: /\btoml_edit::/,
    what: 'parses or writes TOML itself',
    fix: 'call coding::config_patch or the owning CLI module; only redact.rs may parse formats',
    skipFiles: REDACTION_BOUNDARY,
  },
  {
    pattern: /\bserde_yaml::/,
    what: 'parses or writes YAML itself',
    fix: 'call coding::config_patch; only redact.rs may parse formats',
    skipFiles: REDACTION_BOUNDARY,
  },
  {
    pattern: /\btoml::(?:from_str|to_string|Value)\b/,
    what: 'converts JSON into TOML itself',
    fix: 'call coding::config_patch or the owning CLI module',
    skipFiles: REDACTION_BOUNDARY,
  },
  {
    pattern:
      /\bfn\s+(?:merge_(?:toml|yaml|inline_table)\w*|parse_(?:existing_)?toml\w*|patch_\w*toml\w*)/,
    what: 'defines a second TOML/YAML merge or parser',
    fix: 'move it to coding::config_patch or the owning CLI module',
  },
  {
    pattern: /\bapply_claude_config\s*\(/,
    what: 'applies Claude Code through the bare command',
    fix: 'call provider_switch::apply_or_switch_provider',
  },
  {
    pattern: /\bapply_claude_desktop_provider\s*\(/,
    what: 'applies Claude Desktop through the bare command',
    fix: 'call provider_switch::apply_or_switch_provider',
  },
  {
    pattern: /\bapply_codex_config\s*\(/,
    what: 'applies Codex through the bare command',
    fix: 'call provider_switch::apply_or_switch_provider',
  },
  {
    pattern: /\bapply_grok_provider(?:_to_file)?\s*\(/,
    what: 'applies Grok through the bare command',
    fix: 'call provider_switch::apply_or_switch_provider',
  },
  {
    pattern: /\bapply_kimi_provider(?:_to_file)?\s*\(/,
    what: 'applies Kimi through the bare command',
    fix: 'call provider_switch::apply_or_switch_provider',
  },
  {
    pattern: /\bapply_gemini_cli_provider\s*\(/,
    what: 'applies Gemini CLI through the bare command',
    fix: 'call provider_switch::apply_or_switch_provider',
  },
  {
    pattern:
      /\bproxy_gateway_(?:restore_cli_direct|engage_single|engage_failover|engage_aggregate)\s*\(/,
    // `/v1/gateway/*` is the takeover API itself; every other control-plane
    // file must go through provider_switch.
    skipFiles: new Set(['routes_gateway.rs']),
    what: 'runs takeover restore/engage itself',
    fix: 'use provider_switch::{apply_or_switch_provider, with_gateway_reattach}; only /v1/gateway routes may engage',
  },
  {
    pattern: /\bSha256\b/,
    what: 'reimplements a provider-family hash',
    fix: 'call workbuddy_family::channel::channel_id',
  },
  {
    pattern: /\bfn\s+channel_id\s*\(/,
    what: 'defines a second channel-id function',
    fix: 'call workbuddy_family::channel::channel_id',
  },
];

/** All route/dispatch/support files in the control plane must follow the rules. */
async function collectControlFiles() {
  const entries = await readdir(controlRoot, { withFileTypes: true });
  return entries
    .filter(entry => entry.isFile() && entry.name.endsWith('.rs'))
    .map(entry => entry.name)
    .sort();
}

function scanSource(name, source) {
  const failures = [];
  const lines = source.split('\n');
  for (const rule of RULES) {
    if (rule.skipFiles?.has(name)) {
      continue;
    }
    for (const [index, line] of lines.entries()) {
      // Comments explain the rule and may name the forbidden call.
      const code = line.replace(/\/\/.*$/, '');
      if (!rule.pattern.test(code)) {
        continue;
      }
      failures.push({
        file: `tauri/src/control_api/${name}`,
        line: index + 1,
        text: line.trim(),
        what: rule.what,
        fix: rule.fix,
      });
      break;
    }
  }
  return failures;
}

/** `ClearLogs` -> `clear-logs`, `FetchModels` -> `fetch-models`. */
function kebabCase(name) {
  return name
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1-$2')
    .toLowerCase();
}

/** Maps `pub enum Name { ... }` to its 4-space-indented variants. */
function parseEnums(source) {
  const enums = new Map();
  for (const match of source.matchAll(/^pub enum (\w+) \{([\s\S]*?)^\}/gm)) {
    const variants = [...match[2].matchAll(/^ {4}([A-Z][A-Za-z0-9]*)\b/gm)].map(
      variant => variant[1],
    );
    enums.set(match[1], variants);
  }
  return enums;
}

/** Maps each `Command` variant to the enum named by its `command:` field. */
function commandSubEnums(source) {
  const commandBody = source.match(/^pub enum Command \{([\s\S]*?)^\}/m)?.[1];
  if (commandBody === undefined) {
    return new Map();
  }
  const variants = [...commandBody.matchAll(/^ {4}([A-Z][A-Za-z0-9]*)\b/gm)];
  const subEnums = new Map();
  variants.forEach((variant, index) => {
    const end = index + 1 < variants.length ? variants[index + 1].index : commandBody.length;
    const chunk = commandBody.slice(variant.index, end);
    const subEnum = chunk.match(/\bcommand:\s*([A-Z]\w*),/)?.[1] ?? null;
    subEnums.set(variant[1], subEnum);
  });
  return subEnums;
}

/** The exact `parent subcommand` set the clap enums define. */
function expectedCommands(source) {
  const enums = parseEnums(source);
  const subEnums = commandSubEnums(source);
  const expected = new Set();
  for (const variant of enums.get('Command') ?? []) {
    const parent = kebabCase(variant);
    const subEnum = subEnums.get(variant);
    const subs = subEnum ? enums.get(subEnum) : undefined;
    if (subs && subs.length > 0) {
      for (const sub of subs) {
        expected.add(`${parent} ${kebabCase(sub)}`);
      }
    } else {
      expected.add(parent);
    }
  }
  return expected;
}

/** `aitoolbox <command> [<subcommand>]` tokens documented in the Skill. */
function documentedCommands(source) {
  const documented = new Map();
  // `[ \t]`, not `\s`: prose that ends one line with `aitoolbox status` and starts
  // the next with `aitoolbox ...` must not read as `aitoolbox status aitoolbox`.
  for (const match of source.matchAll(
    /\baitoolbox[ \t]+([a-z][a-z-]*)(?:[ \t]+([a-z][a-z-]*))?/g,
  )) {
    const [, command, subcommand] = match;
    documented.set(`${command} ${subcommand ?? ''}`.trim(), { command, subcommand });
  }
  return documented;
}

/** Checks both directions: documented -> code and code -> documented. */
function checkDocTree(argsSource, skillSource) {
  const failures = [];
  const expected = expectedCommands(argsSource);
  const documented = documentedCommands(skillSource);
  const topLevel = new Set([...expected].map(key => key.split(' ')[0]));

  for (const { command, subcommand } of documented.values()) {
    if (!topLevel.has(command)) {
      failures.push({
        file: 'tauri/resources/builtin_skills/ai-toolbox/references/control-cli.md',
        line: 1,
        text: `aitoolbox ${command}`,
        what: 'documents a top-level command that args.rs does not define',
        fix: 'add the command to cli/src/args.rs or fix the Skill',
      });
      continue;
    }
    if (subcommand && !expected.has(`${command} ${subcommand}`)) {
      failures.push({
        file: 'tauri/resources/builtin_skills/ai-toolbox/references/control-cli.md',
        line: 1,
        text: `aitoolbox ${command} ${subcommand}`,
        what: 'documents a subcommand that args.rs does not define under that command',
        fix: 'add the subcommand to cli/src/args.rs or fix the Skill',
      });
    }
  }

  for (const command of expected) {
    if (!documented.has(command)) {
      failures.push({
        file: 'cli/src/args.rs',
        line: 1,
        text: `aitoolbox ${command}`,
        what: 'defines a command the built-in Skill does not document',
        fix: 'document it in references/control-cli.md (and SKILL.md if it changes the workflow)',
      });
    }
  }
  return failures;
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(`control-api reuse guard self-test failed: ${message}`);
  }
}

/** Positive/negative fixtures for every rule and both doc directions. */
function runSelfTests() {
  const writeFailure = scanSource('routes_fixture.rs', 'std::fs::write(path, body)?;');
  assert(writeFailure.length === 1, 'direct file write must be flagged');
  assert(
    scanSource('commands.rs', 'std::fs::write(path, body)?;').length === 0,
    'the intentional PATH writer must stay allowed',
  );

  assert(
    scanSource('routes_fixture.rs', 'let value: toml_edit::DocumentMut = text.parse()?;').length ===
      1,
    'TOML parsing in a route must be flagged',
  );
  assert(
    scanSource('redact.rs', 'let value: toml_edit::DocumentMut = text.parse()?;').length === 0,
    'response-boundary redaction must be allowed to parse TOML',
  );
  assert(
    scanSource('routes_fixture.rs', 'fn merge_toml_document(a: &mut A, b: &B) {}').length === 1,
    'a local TOML merge must be flagged',
  );
  assert(
    scanSource('routes_fixture.rs', 'apply_codex_config(path);').length === 1,
    'bare provider apply must be flagged',
  );
  assert(
    scanSource('routes_cli.rs', 'proxy_gateway_engage_single(...).await?;').length === 1,
    'takeover orchestration outside /v1/gateway must be flagged',
  );
  assert(
    scanSource('routes_gateway.rs', 'proxy_gateway_engage_single(...).await?;').length === 0,
    '/v1/gateway is the takeover API and must be allowed to engage',
  );
  assert(
    scanSource('routes_fixture.rs', 'use sha2::{Digest, Sha256};').length === 1,
    'a second family hash must be flagged',
  );

  const fixtureArgs = [
    'pub enum Command {',
    '    Status,',
    '    Demo {',
    '        command: DemoCommand,',
    '    },',
    '}',
    'pub enum DemoCommand {',
    '    List,',
    '    FetchModels,',
    '}',
  ].join('\n');
  assert(
    [...expectedCommands(fixtureArgs)].sort().join('|') ===
      ['demo fetch-models', 'demo list', 'status'].join('|'),
    'the args.rs tree parser must follow nested subcommand enums',
  );

  const completeSkill = 'aitoolbox status\naitoolbox demo list\naitoolbox demo fetch-models\n';
  assert(
    checkDocTree(fixtureArgs, completeSkill).length === 0,
    'a fully documented tree must pass both directions',
  );
  assert(
    checkDocTree(
      fixtureArgs,
      'aitoolbox status\naitoolbox demo list\naitoolbox demo fetch-models\naitoolbox demo delete\n',
    ).some(failure => failure.text === 'aitoolbox demo delete'),
    'a documented ghost subcommand must be flagged',
  );
  assert(
    checkDocTree(fixtureArgs, 'aitoolbox status\naitoolbox demo list\n').some(
      failure => failure.text === 'aitoolbox demo fetch-models',
    ),
    'an undocumented args.rs command must be flagged',
  );
  assert(
    checkDocTree(fixtureArgs, 'aitoolbox status\naitoolbox demo list\naitoolbox demo fetch-models\naitoolbox ghost\n').some(
      failure => failure.text === 'aitoolbox ghost',
    ),
    'a documented ghost top-level command must be flagged',
  );
}

runSelfTests();

const guardedFiles = await collectControlFiles();
if (guardedFiles.length === 0) {
  console.error('No control API source files found — the scan path is wrong.');
  process.exit(1);
}

const failures = [];
const sources = new Map();
for (const name of guardedFiles) {
  const source = await readFile(path.join(controlRoot, name), 'utf8');
  sources.set(name, source);
  failures.push(...scanSource(name, source));
}

// The wrappers that make the control plane reuse the backend must stay in
// place. Deleting them is the quiet way to reintroduce duplicate flows: the
// app still builds, and only a real takeover reveals the regression.
const dispatchSource = sources.get('cli_dispatch.rs') ?? '';
for (const symbol of ['apply_or_switch_provider', 'with_gateway_reattach']) {
  if (!dispatchSource.includes(symbol)) {
    failures.push({
      file: 'tauri/src/control_api/cli_dispatch.rs',
      line: 1,
      text: `missing ${symbol}`,
      what: 'no longer routes gateway-managed writes through provider_switch',
      fix: 'restore the provider_switch call; do not apply the CLI directly',
    });
  }
}

// create / update / add-model / delete-model / set-default all wrap their
// write, and put_config + extract_config wrap theirs. A count catches the
// regression where one wrapper call is deleted while the rest stay.
const expectedWrappers = [
  {
    file: 'cli_dispatch.rs',
    source: dispatchSource,
    minimum: 5,
    what: 'provider create/update/model writes',
  },
  {
    file: 'routes_cli.rs',
    source: sources.get('routes_cli.rs') ?? '',
    minimum: 2,
    what: 'global config set/extract',
  },
];
for (const { file, source, minimum, what } of expectedWrappers) {
  const count = (source.match(/\bwith_gateway_reattach_api\s*\(/g) ?? []).length;
  if (count < minimum) {
    failures.push({
      file: `tauri/src/control_api/${file}`,
      line: 1,
      text: `${count} with_gateway_reattach_api call(s), expected at least ${minimum}`,
      what: `no longer wraps every gateway-managed ${what} in the re-engage window`,
      fix: 'wrap the write with with_gateway_reattach_api; do not call the backend save directly',
    });
  }
}

// Doc <-> code: the Skill and the clap tree must describe the same surface.
const argsSource = await readFile(argsPath, 'utf8');
const skillSource = await readFile(skillPath, 'utf8');
failures.push(...checkDocTree(argsSource, skillSource));

if (failures.length > 0) {
  console.error(
    'Control API / aitoolbox must reuse existing flows (AGENTS.md Hard Rule 15).\n',
  );
  for (const failure of failures) {
    console.error(`  ${failure.file}:${failure.line} — ${failure.what}`);
    console.error(`    ${failure.text}`);
    console.error(`    Fix: ${failure.fix}\n`);
  }
  process.exit(1);
}

const documentedCount = documentedCommands(skillSource).size;
console.log(
  `Control API reuse check passed (${guardedFiles.length} control-plane files, ` +
    `${documentedCount} documented commands, self-tests passed).`,
);
