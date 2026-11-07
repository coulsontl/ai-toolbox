import { parse as parseToml } from 'smol-toml';

/**
 * Codex 原生记忆模型（`[memories]`）的结构化读写。
 *
 * 这里只定向增删 `extract_model` / `consolidation_model` 两行，不重建整份
 * TOML：注释、空行、键顺序、行尾和 `[memories]` 的其他键都必须原样保留。
 * 内联表、点号键、多行字符串和非法 TOML 无法安全定向修改，交给原始 TOML
 * 编辑器处理。
 */

export type CodexMemoriesUnsupportedShape =
  | 'inline_table'
  | 'dotted_keys'
  | 'invalid_toml'
  | 'multiline_value';

export interface CodexMemoriesModels {
  extractModel?: string;
  consolidationModel?: string;
  unsupportedShape?: CodexMemoriesUnsupportedShape;
}

export interface PatchCodexMemoriesModelsResult {
  nextToml: string;
  changed: boolean;
  unsupportedShape?: CodexMemoriesUnsupportedShape;
}

// TOML allows a quoted table name and arbitrary whitespace inside the brackets;
// missing those spellings here would make the editor append a second
// `[memories]` table and turn valid TOML into a duplicate-table error.
const MEMORIES_KEY = '(?:"memories"|\'memories\'|memories)';
const MEMORIES_HEADER_RE = new RegExp(
  `^\\s*\\[\\s*${MEMORIES_KEY}\\s*\\]\\s*(?:#.*)?$`,
);
const MEMORIES_DOTTED_RE = new RegExp(`^${MEMORIES_KEY}\\s*\\.`);
const MEMORIES_INLINE_RE = new RegExp(`^${MEMORIES_KEY}\\s*=`);
const ANY_TABLE_HEADER_RE = /^\s*\[/;
const MULTILINE_VALUE_RE = /^(?:"""|''')/;
const STRING_TOKEN_RE = /^("(?:[^"\\]|\\.)*"|'[^']*')/;

function detectEol(text: string): string {
  return text.includes('\r\n') ? '\r\n' : '\n';
}

function detectTopLevelMemoriesShape(
  text: string,
): 'inline_table' | 'dotted_keys' | undefined {
  for (const line of text.split(/\r?\n/)) {
    if (ANY_TABLE_HEADER_RE.test(line)) {
      break;
    }
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) {
      continue;
    }
    if (MEMORIES_DOTTED_RE.test(trimmed)) {
      return 'dotted_keys';
    }
    if (MEMORIES_INLINE_RE.test(trimmed)) {
      return 'inline_table';
    }
  }
  return undefined;
}

interface MemoriesBlock {
  headerIndex: number;
  endIndex: number;
  lines: string[];
}

function findMemoriesBlock(text: string): MemoriesBlock | undefined {
  const lines = text.split(/\r?\n/);
  const headerIndex = lines.findIndex((line) => MEMORIES_HEADER_RE.test(line));
  if (headerIndex < 0) {
    return undefined;
  }

  let endIndex = lines.length;
  for (let index = headerIndex + 1; index < lines.length; index += 1) {
    if (ANY_TABLE_HEADER_RE.test(lines[index])) {
      endIndex = index;
      break;
    }
  }
  return { headerIndex, endIndex, lines };
}

interface MemoriesKeyLine {
  index: number;
  indent: string;
  keyToken: string;
  separator: string;
  value: string;
}

function findKeyLine(
  lines: string[],
  start: number,
  end: number,
  key: string,
): MemoriesKeyLine | undefined {
  const pattern = new RegExp(
    `^(\\s*)("${key}"|'${key}'|${key})(\\s*=\\s*)(.*)$`,
  );
  for (let index = start; index < end; index += 1) {
    const match = lines[index].match(pattern);
    if (match) {
      return {
        index,
        indent: match[1],
        keyToken: match[2],
        separator: match[3],
        value: match[4],
      };
    }
  }
  return undefined;
}

function formatTomlString(value: string, existingToken?: string): string {
  if (existingToken?.startsWith("'") && !value.includes("'")) {
    return `'${value}'`;
  }
  const escaped = value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  return `"${escaped}"`;
}

/**
 * 只替换值 token，保留行内注释。旧值不是字符串时丢弃旧值但保注释。
 */
function replaceValueToken(rawValue: string, value: string): string {
  const match = rawValue.match(STRING_TOKEN_RE);
  if (match) {
    return formatTomlString(value, match[1]) + rawValue.slice(match[0].length);
  }
  const comment = rawValue.match(/(#.*)$/);
  return formatTomlString(value) + (comment ? ` ${comment[1]}` : '');
}

function findBlockIndent(lines: string[], headerIndex: number, endIndex: number): string {
  for (let index = headerIndex + 1; index < endIndex; index += 1) {
    const trimmed = lines[index].trim();
    if (trimmed && !trimmed.startsWith('#')) {
      return lines[index].match(/^(\s*)/)?.[1] ?? '';
    }
  }
  return '';
}

function findBlockInsertIndex(lines: string[], headerIndex: number, endIndex: number): number {
  let insertIndex = headerIndex + 1;
  for (let index = headerIndex + 1; index < endIndex; index += 1) {
    const trimmed = lines[index].trim();
    if (trimmed && !trimmed.startsWith('#')) {
      insertIndex = index + 1;
    }
  }
  return insertIndex;
}

function readStringValue(
  memories: Record<string, unknown>,
  key: string,
): string | undefined {
  const value = memories[key];
  return typeof value === 'string' && value.trim() ? value : undefined;
}

export function readCodexMemoriesModels(configToml: string): CodexMemoriesModels {
  const text = configToml ?? '';
  if (!text.trim()) {
    return {};
  }

  let parsed: Record<string, unknown>;
  try {
    parsed = parseToml(text) as Record<string, unknown>;
  } catch {
    return { unsupportedShape: 'invalid_toml' };
  }

  const topLevelShape = detectTopLevelMemoriesShape(text);
  if (topLevelShape) {
    return { unsupportedShape: topLevelShape };
  }

  const block = findMemoriesBlock(text);
  if (!block) {
    return {};
  }

  const memories = parsed.memories;
  if (!memories || typeof memories !== 'object' || Array.isArray(memories)) {
    return {};
  }
  const memoriesTable = memories as Record<string, unknown>;

  const hasMultilineValue = (key: string): boolean => {
    const line = findKeyLine(block.lines, block.headerIndex + 1, block.endIndex, key);
    return !!line && MULTILINE_VALUE_RE.test(line.value.trim());
  };

  if (hasMultilineValue('extract_model') || hasMultilineValue('consolidation_model')) {
    return { unsupportedShape: 'multiline_value' };
  }

  return {
    extractModel: readStringValue(memoriesTable, 'extract_model'),
    consolidationModel: readStringValue(memoriesTable, 'consolidation_model'),
  };
}

function patchMemoriesKey(
  text: string,
  key: string,
  value: string,
): { nextToml: string; changed: boolean } {
  const eol = detectEol(text);
  const block = findMemoriesBlock(text);

  if (!block) {
    if (!value) {
      return { nextToml: text, changed: false };
    }
    let prefix = text;
    if (prefix.length > 0 && !prefix.endsWith('\n')) {
      prefix += eol;
    }
    if (prefix.length > 0 && !prefix.endsWith(`${eol}${eol}`)) {
      prefix += eol;
    }
    const line = `${key} = ${formatTomlString(value)}`;
    return { nextToml: `${prefix}[memories]${eol}${line}${eol}`, changed: true };
  }

  const lines = [...block.lines];
  const keyLine = findKeyLine(lines, block.headerIndex + 1, block.endIndex, key);

  if (value) {
    if (keyLine) {
      if (MULTILINE_VALUE_RE.test(keyLine.value.trim())) {
        return { nextToml: text, changed: false };
      }
      lines[keyLine.index] = `${keyLine.indent}${keyLine.keyToken}${keyLine.separator}${
        replaceValueToken(keyLine.value, value)
      }`;
      const nextToml = lines.join(eol);
      return { nextToml, changed: nextToml !== text };
    }

    const indent = findBlockIndent(lines, block.headerIndex, block.endIndex);
    const insertIndex = findBlockInsertIndex(lines, block.headerIndex, block.endIndex);
    lines.splice(insertIndex, 0, `${indent}${key} = ${formatTomlString(value)}`);
    return { nextToml: lines.join(eol), changed: true };
  }

  if (!keyLine) {
    return { nextToml: text, changed: false };
  }
  lines.splice(keyLine.index, 1);
  return { nextToml: lines.join(eol), changed: true };
}

export function patchCodexMemoriesModels(
  configToml: string,
  next: CodexMemoriesModels,
): PatchCodexMemoriesModelsResult {
  const current = readCodexMemoriesModels(configToml);
  if (current.unsupportedShape) {
    return {
      nextToml: configToml,
      changed: false,
      unsupportedShape: current.unsupportedShape,
    };
  }

  let text = configToml ?? '';
  let changed = false;
  const entries: Array<[string, string | undefined]> = [
    ['extract_model', next.extractModel],
    ['consolidation_model', next.consolidationModel],
  ];

  for (const [key, rawValue] of entries) {
    // Preserve the user's typing verbatim; only an all-whitespace value means
    // "remove this key".
    const value = rawValue ?? '';
    const result = patchMemoriesKey(text, key, value.trim() ? value : '');
    text = result.nextToml;
    changed = changed || result.changed;
  }

  return { nextToml: text, changed };
}
