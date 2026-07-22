import assert from 'node:assert/strict';
import test from 'node:test';

import { parseMcpServersFromJsonValue } from '../../../../../features/coding/mcp/utils/mcpJsonImport.ts';

const phpStormCommand = 'C:\\Program Files\\JetBrains\\PhpStorm 2026.1.1\\jbr\\bin\\java';
const phpStormClasspath = [
  'C:\\Program Files\\JetBrains\\PhpStorm 2026.1.1\\plugins\\mcpserver\\lib\\mcpserver-frontend.jar',
  'C:\\Program Files\\JetBrains\\PhpStorm 2026.1.1\\lib\\util-8.jar',
].join(';');

test('parseMcpServersFromJsonValue preserves spaces in stdio command and args', () => {
  const servers = parseMcpServersFromJsonValue({
    mcpServers: {
      phpstorm: {
        type: 'stdio',
        env: {
          IJ_MCP_SERVER_PORT: '64342',
        },
        command: phpStormCommand,
        args: [
          '-classpath',
          phpStormClasspath,
          'com.intellij.mcpserver.stdio.McpStdioRunnerKt',
        ],
      },
    },
  });

  assert.equal(servers.length, 1);
  assert.equal(servers[0].name, 'phpstorm');
  assert.equal(servers[0].server_type, 'stdio');

  const serverConfig = servers[0].server_config as { command: string; args: string[]; env?: Record<string, string> };
  assert.equal(serverConfig.command, phpStormCommand);
  assert.deepEqual(serverConfig.args, [
    '-classpath',
    phpStormClasspath,
    'com.intellij.mcpserver.stdio.McpStdioRunnerKt',
  ]);
  assert.deepEqual(serverConfig.env, {
    IJ_MCP_SERVER_PORT: '64342',
  });
});

test('parseMcpServersFromJsonValue accepts a bare single server config object', () => {
  const servers = parseMcpServersFromJsonValue({
    type: 'stdio',
    env: {
      IJ_MCP_SERVER_PORT: '64342',
    },
    command: phpStormCommand,
    args: [
      '-classpath',
      phpStormClasspath,
      'com.intellij.mcpserver.stdio.McpStdioRunnerKt',
    ],
  });

  assert.equal(servers.length, 1);
  assert.equal(servers[0].name, 'imported-mcp-server');

  const serverConfig = servers[0].server_config as { command: string; args: string[] };
  assert.equal(serverConfig.command, phpStormCommand);
  assert.equal(serverConfig.args[1], phpStormClasspath);
});

test('parseMcpServersFromJsonValue ignores non-server nested objects in server maps', () => {
  const servers = parseMcpServersFromJsonValue({
    env: {
      IJ_MCP_SERVER_PORT: '64342',
    },
  });

  assert.deepEqual(servers, []);
});

test('parseMcpServersFromJsonValue treats command-keyed maps as named servers', () => {
  const servers = parseMcpServersFromJsonValue({
    command: {
      type: 'stdio',
      command: 'npx',
      args: ['-y', '@modelcontextprotocol/server-time'],
    },
    url: {
      type: 'http',
      url: 'https://example.com/mcp',
    },
  });

  assert.equal(servers.length, 2);
  assert.equal(servers[0].name, 'command');
  const commandServerConfig = servers[0].server_config as { command: string; args: string[] };
  assert.equal(commandServerConfig.command, 'npx');
  assert.deepEqual(commandServerConfig.args, ['-y', '@modelcontextprotocol/server-time']);

  assert.equal(servers[1].name, 'url');
  const urlServerConfig = servers[1].server_config as { url: string };
  assert.equal(urlServerConfig.url, 'https://example.com/mcp');
});

test('parseMcpServersFromJsonValue preserves Codex/Grok second-based timeouts', () => {
  const servers = parseMcpServersFromJsonValue({
    mcpServers: {
      local: {
        type: 'stdio',
        command: 'npx',
        args: ['-y', 'pkg'],
        startup_timeout_sec: 120,
        tool_timeout_sec: 300,
      },
      remote: {
        type: 'http',
        url: 'https://example.com/mcp',
        startup_timeout_sec: '25',
        tool_timeout_sec: 1800,
      },
    },
  });

  assert.equal(servers.length, 2);
  const local = servers.find((server) => server.name === 'local');
  const remote = servers.find((server) => server.name === 'remote');
  assert.ok(local);
  assert.ok(remote);
  assert.equal(local.server_config.startup_timeout_sec, 120);
  assert.equal(local.server_config.tool_timeout_sec, 300);
  assert.equal(remote.server_config.startup_timeout_sec, 25);
  assert.equal(remote.server_config.tool_timeout_sec, 1800);
});
