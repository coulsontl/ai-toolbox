import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildMcpServerExportJson,
  formatMcpServerExportJson,
} from '../../../../../features/coding/mcp/utils/mcpServerExportJson.ts';
import { parseMcpServersFromJsonValue } from '../../../../../features/coding/mcp/utils/mcpJsonImport.ts';
import type { McpServer } from '../../../../../features/coding/mcp/types/index.ts';

function makeServer(overrides: Partial<McpServer>): McpServer {
  return {
    id: 'server-1',
    name: 'context7',
    server_type: 'stdio',
    server_config: { command: 'npx', args: ['-y', '@upstash/context7-mcp'] },
    enabled_tools: ['claude_code'],
    sync_details: [],
    description: 'docs server',
    user_group: '开发辅助',
    user_note: 'note',
    tags: ['docs'],
    timeout: null,
    sort_index: 0,
    management_enabled: true,
    disabled_previous_tools: [],
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

test('buildMcpServerExportJson emits a stdio config the importer can read back', () => {
  const payload = buildMcpServerExportJson(makeServer({
    server_config: {
      command: 'C:\\Program Files\\node.exe',
      args: ['-y', 'pkg'],
      env: { FOO: 'bar' },
      startup_timeout_sec: 60,
      tool_timeout_sec: 120,
    },
  }));

  const parsed = parseMcpServersFromJsonValue(payload);
  assert.equal(parsed.length, 1);
  assert.equal(parsed[0].name, 'context7');
  assert.equal(parsed[0].server_type, 'stdio');

  const config = parsed[0].server_config as {
    command: string;
    args: string[];
    env?: Record<string, string>;
    startup_timeout_sec?: number;
    tool_timeout_sec?: number;
  };
  assert.equal(config.command, 'C:\\Program Files\\node.exe');
  assert.deepEqual(config.args, ['-y', 'pkg']);
  assert.deepEqual(config.env, { FOO: 'bar' });
  assert.equal(config.startup_timeout_sec, 60);
  assert.equal(config.tool_timeout_sec, 120);
});

test('buildMcpServerExportJson round-trips http servers with headers', () => {
  const payload = buildMcpServerExportJson(makeServer({
    name: 'remote',
    server_type: 'http',
    server_config: {
      url: 'https://example.com/mcp',
      headers: { Authorization: 'Bearer token' },
      tool_timeout_sec: 300,
    },
  }));

  const parsed = parseMcpServersFromJsonValue(payload);
  assert.equal(parsed.length, 1);
  assert.equal(parsed[0].server_type, 'http');

  const config = parsed[0].server_config as {
    url: string;
    headers?: Record<string, string>;
    tool_timeout_sec?: number;
  };
  assert.equal(config.url, 'https://example.com/mcp');
  assert.deepEqual(config.headers, { Authorization: 'Bearer token' });
  assert.equal(config.tool_timeout_sec, 300);
});

test('buildMcpServerExportJson omits empty optional sections', () => {
  const payload = buildMcpServerExportJson(makeServer({
    server_config: { command: 'npx', args: [] },
  })) as { mcpServers: Record<string, Record<string, unknown>> };

  const entry = payload.mcpServers.context7;
  assert.deepEqual(entry, { type: 'stdio', command: 'npx', args: [] });
  assert.equal('env' in entry, false);
  assert.equal('startup_timeout_sec' in entry, false);
});

test('buildMcpServerExportJson exports config only, never management metadata', () => {
  const payload = buildMcpServerExportJson(makeServer({}));
  assert.deepEqual(Object.keys(payload), ['mcpServers']);

  const text = formatMcpServerExportJson(makeServer({}));
  assert.equal(text.includes('开发辅助'), false);
  assert.equal(text.includes('docs server'), false);
  assert.equal(text.includes('"docs"'), false);
});

test('formatMcpServerExportJson produces indented parseable JSON', () => {
  const text = formatMcpServerExportJson(makeServer({}));
  assert.ok(text.includes('\n  "mcpServers": {'));
  assert.deepEqual(JSON.parse(text), buildMcpServerExportJson(makeServer({})));
});
