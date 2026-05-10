import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildMcpGroups,
  filterMcpServersBySearch,
  getMcpDisplayNote,
  getMcpGroupOptions,
  normalizeMcpMetadataText,
} from '../../../../../features/coding/mcp/utils/mcpGrouping.ts';
import type { McpServer } from '../../../../../features/coding/mcp/types/index.ts';

function makeServer(overrides: Partial<McpServer>): McpServer {
  return {
    id: 'server-1',
    name: 'default-server',
    server_type: 'stdio',
    server_config: { command: 'node', args: [] },
    enabled_tools: [],
    sync_details: [],
    description: null,
    user_group: null,
    user_note: null,
    tags: [],
    timeout: null,
    sort_index: 0,
    created_at: 1,
    updated_at: 1,
    ...overrides,
  };
}

test('normalizeMcpMetadataText trims empty values to null', () => {
  assert.equal(normalizeMcpMetadataText('  Reverse  '), 'Reverse');
  assert.equal(normalizeMcpMetadataText('   '), null);
  assert.equal(normalizeMcpMetadataText(null), null);
});

test('getMcpGroupOptions returns sorted non-empty custom groups', () => {
  const servers = [
    makeServer({ id: 'a', user_group: 'Reverse' }),
    makeServer({ id: 'b', user_group: 'Database' }),
    makeServer({ id: 'c', user_group: ' Reverse ' }),
    makeServer({ id: 'd', user_group: '' }),
  ];

  assert.deepEqual(getMcpGroupOptions(servers), ['Database', 'Reverse']);
});

test('getMcpDisplayNote prefers user note and falls back to description', () => {
  assert.equal(
    getMcpDisplayNote(makeServer({ user_note: 'Use for browser debugging', description: 'Browser tools' })),
    'Use for browser debugging',
  );
  assert.equal(
    getMcpDisplayNote(makeServer({ user_note: null, description: 'Browser tools' })),
    'Browser tools',
  );
});

test('filterMcpServersBySearch matches config summary, custom group, and note', () => {
  const servers = [
    makeServer({ id: 'reverse', name: 'apk-helper', user_group: 'Reverse' }),
    makeServer({ id: 'note', name: 'memory', user_note: 'Use with project notes' }),
    makeServer({ id: 'config', name: 'fetch', server_config: { command: 'uvx', args: [] } }),
  ];

  const getSummary = (server: McpServer) => (
    server.server_config as { command?: string }
  ).command ?? '';

  assert.deepEqual(
    filterMcpServersBySearch(servers, 'reverse', getSummary).map((server) => server.id),
    ['reverse'],
  );
  assert.deepEqual(
    filterMcpServersBySearch(servers, 'project notes', getSummary).map((server) => server.id),
    ['note'],
  );
  assert.deepEqual(
    filterMcpServersBySearch(servers, 'uvx', getSummary).map((server) => server.id),
    ['config'],
  );
});

test('buildMcpGroups groups by custom group and keeps ungrouped servers', () => {
  const servers = [
    makeServer({ id: 'a', user_group: 'Reverse' }),
    makeServer({ id: 'b', user_group: null }),
    makeServer({ id: 'c', user_group: 'Reverse' }),
  ];

  const groups = buildMcpGroups(servers, { groupUngrouped: 'Ungrouped' });

  assert.deepEqual(groups.map((group) => [group.label, group.servers.map((server) => server.id)]), [
    ['Reverse', ['a', 'c']],
    ['Ungrouped', ['b']],
  ]);
});
