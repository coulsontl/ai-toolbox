/**
 * Guards the "control API and aitb are adapters, never a second
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
 * 1. `cli_dispatch.rs` / `routes_*.rs` must not write files directly; config
 *    writes belong to the owning CLI module (backups, WSL sync, events).
 * 2. The six gateway-managed CLIs must not be applied through bare
 *    `apply_*_provider` commands. They go through `provider_switch`, which
 *    restores and re-engages the takeover.
 * 3. No family channel hash may be reimplemented here (`Sha256` or a local
 *    `fn channel_id`); call `workbuddy_family::channel::channel_id`.
 * 4. Every `aitb <command> <subcommand>` documented in the built-in Skill must
 *    exist in `cli/src/args.rs`, so the Skill cannot describe ghost commands.
 *
 * Scope note: `commands.rs` and `discovery.rs` are intentionally out of the
 * file-write check — the first maintains the user PATH during `aitb` install,
 * the second writes the discovery file.
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

const failures = [];

/** Route and dispatch files that must stay orchestration-only. */
async function collectGuardedFiles() {
  const entries = await readdir(controlRoot, { withFileTypes: true });
  return entries
    .filter(entry => entry.isFile() && entry.name.endsWith('.rs'))
    .map(entry => entry.name)
    .filter(name => name === 'cli_dispatch.rs' || name.startsWith('routes'))
    .sort();
}

const guardedFiles = await collectGuardedFiles();
if (guardedFiles.length === 0) {
  console.error('No control API route/dispatch files found — the scan path is wrong.');
  process.exit(1);
}

const RULES = [
  {
    pattern: /\b(?:std|tokio)::fs::write\s*\(/,
    what: 'writes a file directly',
    fix: 'call the owning CLI module command/save helper (or provider_switch) instead',
  },
  {
    pattern: /\bOpenOptions\b/,
    what: 'opens a writable file handle',
    fix: 'call the owning CLI module command/save helper instead',
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
    pattern: /\bproxy_gateway_(?:restore_cli_direct|engage_single|engage_failover|engage_aggregate)\s*\(/,
    // `/v1/gateway/*` is the takeover API itself; every other route/dispatch
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

for (const name of guardedFiles) {
  const source = await readFile(path.join(controlRoot, name), 'utf8');
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
}

// The wrappers that make the control plane reuse the backend must stay in
// place. Deleting them is the quiet way to reintroduce duplicate flows: the
// app still builds, and only a real takeover reveals the regression.
const dispatchSource = await readFile(path.join(controlRoot, 'cli_dispatch.rs'), 'utf8');
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

// Doc -> code: every documented command must exist in the CLI parser.
const argsSource = await readFile(argsPath, 'utf8');
const skillSource = await readFile(skillPath, 'utf8');
const variantNames = new Set(
  [...argsSource.matchAll(/^\s{4}([A-Z][A-Za-z0-9]*)\b/gm)].map(match => match[1]),
);
const pascalCase = token =>
  token
    .split('-')
    .filter(Boolean)
    .map(part => part[0].toUpperCase() + part.slice(1))
    .join('');

const documented = new Map();
// `[ \t]`, not `\s`: prose that ends one line with `aitb status` and starts
// the next with `aitb ...` must not read as `aitb status aitb`.
for (const match of skillSource.matchAll(/\baitb[ \t]+([a-z][a-z-]*)(?:[ \t]+([a-z][a-z-]*))?/g)) {
  const [, command, subcommand] = match;
  documented.set(`${command} ${subcommand ?? ''}`.trim(), { command, subcommand });
}

for (const { command, subcommand } of documented.values()) {
  if (!variantNames.has(pascalCase(command))) {
    failures.push({
      file: 'tauri/resources/builtin_skills/ai-toolbox/references/control-cli.md',
      line: 1,
      text: `aitb ${command}`,
      what: 'documents a top-level command that args.rs does not define',
      fix: 'add the command to cli/src/args.rs or fix the Skill',
    });
    continue;
  }
  if (subcommand && !variantNames.has(pascalCase(subcommand))) {
    failures.push({
      file: 'tauri/resources/builtin_skills/ai-toolbox/references/control-cli.md',
      line: 1,
      text: `aitb ${command} ${subcommand}`,
      what: 'documents a subcommand that args.rs does not define',
      fix: 'add the subcommand to cli/src/args.rs or fix the Skill',
    });
  }
}

if (failures.length > 0) {
  console.error(
    'Control API / aitb must reuse existing flows (AGENTS.md Hard Rule 15).\n',
  );
  for (const failure of failures) {
    console.error(`  ${failure.file}:${failure.line} — ${failure.what}`);
    console.error(`    ${failure.text}`);
    console.error(`    Fix: ${failure.fix}\n`);
  }
  process.exit(1);
}

console.log(
  `Control API reuse check passed (${guardedFiles.length} route/dispatch files, ` +
    `${documented.size} documented commands).`,
);
