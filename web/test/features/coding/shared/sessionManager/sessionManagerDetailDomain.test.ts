import assert from 'node:assert/strict';
import test from 'node:test';

import {
  getNormalizedToolName,
  getToolVariant,
} from '../../../../../features/coding/shared/sessionManager/detail/domain/toolCatalog.ts';
import { pairToolBlocks } from '../../../../../features/coding/shared/sessionManager/detail/domain/toolPairing.ts';
import { getMessageSearchText } from '../../../../../features/coding/shared/sessionManager/detail/domain/messageSearch.ts';
import { getMessagePreview } from '../../../../../features/coding/shared/sessionManager/detail/domain/messageBlocks.ts';
import { filterSessionMessages } from '../../../../../features/coding/shared/sessionManager/detail/domain/messageFilters.ts';
import { buildNavigatorEntries } from '../../../../../features/coding/shared/sessionManager/detail/domain/messageNavigator.ts';
import { parseSessionCommandTags } from '../../../../../features/coding/shared/sessionManager/detail/domain/commandTags.ts';
import type { SessionMessage } from '../../../../../features/coding/shared/sessionManager/types.ts';

test('tool catalog normalizes common aliases', () => {
  assert.equal(getNormalizedToolName('Bash'), 'bash');
  assert.equal(getNormalizedToolName('execute_command'), 'bash');
  assert.equal(getNormalizedToolName('Read'), 'read');
  assert.equal(getNormalizedToolName('MultiEdit'), 'multi_edit');
  assert.equal(getNormalizedToolName('mcp__filesystem__read_file'), 'mcp');
  assert.equal(getToolVariant('web_search'), 'web');
});

test('pairToolBlocks merges matching call and result by tool id', () => {
  const blocks = pairToolBlocks([
    {
      kind: 'tool_call',
      toolId: 'tool-1',
      toolName: 'Bash',
      input: { command: 'echo hi' },
    },
    {
      kind: 'tool_result',
      toolId: 'tool-1',
      output: { stdout: 'hi' },
    },
  ]);

  assert.equal(blocks.length, 1);
  assert.equal(blocks[0].kind, 'tool_execution');
  assert.equal(blocks[0].normalizedToolName, 'bash');
  assert.equal(blocks[0].status, 'success');
});

test('pairToolBlocks infers normalized tool name from input when raw name is unknown', () => {
  const blocks = pairToolBlocks([
    {
      kind: 'tool_call',
      toolId: 'tool-1',
      toolName: 'unknown',
      input: { pattern: 'SessionManagerPanel' },
    },
  ]);

  assert.equal(blocks[0].normalizedToolName, 'grep');
  assert.equal(blocks[0].variant, 'search');
});

test('message search text includes tool input and output', () => {
  const message: SessionMessage = {
    role: 'assistant',
    content: '[Tool: Grep]',
    blocks: [{
      kind: 'tool_execution',
      toolId: 'tool-1',
      toolName: 'Grep',
      input: { pattern: 'SessionManagerPanel' },
      output: { stdout: 'matched line' },
    }],
  };

  const searchText = getMessageSearchText(message);
  assert.match(searchText, /SessionManagerPanel/);
  assert.match(searchText, /matched line/);
});

test('tool id search scope only searches tool identifiers', () => {
  const message: SessionMessage = {
    role: 'assistant',
    content: '[Tool: Grep]',
    blocks: [{
      kind: 'tool_execution',
      toolId: 'tool-abc',
      toolName: 'Grep',
      input: { pattern: 'SessionManagerPanel' },
      output: { stdout: 'matched line' },
    }],
  };

  assert.match(getMessageSearchText(message, 'toolId'), /tool-abc/);
  assert.doesNotMatch(getMessageSearchText(message, 'toolId'), /SessionManagerPanel/);
});

test('command tag parsing hides redundant command message in preview', () => {
  const text = '<command-name>/clear</command-name>\n<command-message>clear</command-message>\n<command-args></command-args>';
  const parsed = parseSessionCommandTags(text);
  const message: SessionMessage = {
    role: 'system',
    content: text,
    blocks: [{ kind: 'system', text }],
  };

  assert.equal(parsed.commandName, '/clear');
  assert.equal(parsed.commandMessage, undefined);
  assert.equal(getMessagePreview(message), '/clear');
});

test('filters include tool messages by block type', () => {
  const messages: SessionMessage[] = [
    { role: 'user', content: 'hello' },
    {
      role: 'assistant',
      content: '[Tool: Read]',
      blocks: [{ kind: 'tool_call', toolName: 'Read', input: { file_path: 'Cargo.toml' } }],
    },
  ];

  const filtered = filterSessionMessages(messages, {
    query: '',
    roleFilter: { user: true, assistant: true },
    contentFilter: {
      text: false,
      thinking: false,
      tool_call: true,
      command: false,
    },
    searchScope: 'content',
  });

  assert.equal(filtered.length, 1);
  assert.equal(filtered[0].index, 1);
});

test('filters include command-tagged system messages', () => {
  const messages: SessionMessage[] = [
    { role: 'user', content: 'hello' },
    {
      role: 'system',
      content: '<command-name>/clear</command-name><command-message>clear</command-message>',
      blocks: [{ kind: 'system', text: '<command-name>/clear</command-name><command-message>clear</command-message>' }],
    },
  ];

  const filtered = filterSessionMessages(messages, {
    query: '',
    roleFilter: { user: true, assistant: true },
    contentFilter: {
      text: false,
      thinking: false,
      tool_call: false,
      command: true,
    },
    searchScope: 'content',
  });

  assert.equal(filtered.length, 1);
  assert.equal(filtered[0].index, 1);
});

test('role filters toggle user and assistant independently', () => {
  const messages: SessionMessage[] = [
    { role: 'user', content: 'hello' },
    { role: 'assistant', content: 'answer' },
  ];

  const filtered = filterSessionMessages(messages, {
    query: '',
    roleFilter: { user: false, assistant: true },
    contentFilter: {
      text: true,
      thinking: true,
      tool_call: true,
      command: true,
    },
    searchScope: 'content',
  });

  assert.equal(filtered.length, 1);
  assert.equal(filtered[0].index, 1);
});

test('content filters keep visible blocks from mixed assistant messages', () => {
  const messages: SessionMessage[] = [{
    role: 'assistant',
    content: 'mixed response',
    blocks: [
      { kind: 'text', text: 'visible text' },
      { kind: 'thinking', text: 'hidden thinking' },
      { kind: 'tool_call', toolName: 'Read', input: { file_path: 'Cargo.toml' } },
      { kind: 'system', text: '<command-name>/clear</command-name>' },
    ],
  }];

  const textOnly = filterSessionMessages(messages, {
    query: '',
    roleFilter: { user: true, assistant: true },
    contentFilter: {
      text: true,
      thinking: false,
      tool_call: false,
      command: false,
    },
    searchScope: 'content',
  });
  const thinkingOnly = filterSessionMessages(messages, {
    query: '',
    roleFilter: { user: true, assistant: true },
    contentFilter: {
      text: false,
      thinking: true,
      tool_call: false,
      command: false,
    },
    searchScope: 'content',
  });

  assert.equal(textOnly.length, 1);
  assert.equal(thinkingOnly.length, 1);
});

test('navigator entries include user and tool entries', () => {
  const messages: SessionMessage[] = [
    { role: 'user', content: 'open the file' },
    {
      role: 'assistant',
      content: '[Tool: Read]',
      blocks: [{ kind: 'tool_call', toolId: 'tool-1', toolName: 'Read' }],
    },
  ];

  const entries = buildNavigatorEntries(messages, 'Read');
  assert.equal(entries.some((entry) => entry.role === 'user'), true);
  assert.equal(entries.some((entry) => entry.kind === 'tool' && entry.label === 'Read'), true);
  assert.deepEqual(entries.map((entry) => entry.turnIndex), [1, 2]);
  assert.equal(entries.find((entry) => entry.kind === 'tool')?.hasToolUse, true);
});

test('navigator entries hide placeholder assistant messages', () => {
  const messages: SessionMessage[] = [
    { role: 'assistant', content: '(assistant message)' },
    { role: 'assistant', content: '' },
    { role: 'user', content: 'real user message' },
  ];

  const entries = buildNavigatorEntries(messages, '');
  assert.equal(entries.length, 1);
  assert.equal(entries[0].preview, 'real user message');
  assert.equal(entries[0].turnIndex, 1);
});

test('navigator entries hide unknown-only tool messages', () => {
  const messages: SessionMessage[] = [
    {
      role: 'assistant',
      content: '[Tool: unknown]',
      blocks: [{
        kind: 'tool_call',
        toolId: 'tool-1',
        toolName: 'unknown',
        normalizedToolName: 'unknown',
      }],
    },
  ];

  const entries = buildNavigatorEntries(messages, '');
  assert.equal(entries.length, 0);
});

test('navigator entries use normalized tool display names', () => {
  const messages: SessionMessage[] = [
    {
      role: 'assistant',
      content: '[Tool: unknown]',
      blocks: [{
        kind: 'tool_call',
        toolId: 'tool-1',
        toolName: 'unknown',
        normalizedToolName: 'grep',
        input: { pattern: 'SessionManagerPanel' },
      }],
    },
  ];

  const entries = buildNavigatorEntries(messages, '');
  assert.equal(entries.length, 1);
  assert.equal(entries[0].label, 'Grep');
  assert.equal(entries[0].preview, 'Grep');
});
