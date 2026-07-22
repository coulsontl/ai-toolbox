import type { HttpConfig, StdioConfig } from '../types';
import { isJsonObject } from '../../../../utils/json.ts';

export interface ParsedMcpJsonServer {
  name: string;
  server_type: 'stdio' | 'http' | 'sse';
  server_config: StdioConfig | HttpConfig;
}

const DEFAULT_SINGLE_SERVER_NAME = 'imported-mcp-server';

function hasServerConfigShape(config: Record<string, unknown>): boolean {
  return (
    typeof config.command === 'string'
    || Array.isArray(config.command)
    || typeof config.url === 'string'
    || typeof config.httpUrl === 'string'
    || typeof config.serverUrl === 'string'
  );
}

function normalizeServerName(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

export function detectMcpServerType(config: Record<string, unknown>): 'stdio' | 'http' | 'sse' {
  if (config.type === 'stdio' || config.type === 'local') return 'stdio';
  if (config.type === 'http') return 'http';
  if (config.type === 'sse' || config.type === 'remote') return 'sse';
  if (config.command) return 'stdio';
  if (config.serverUrl) return 'http';
  if (config.httpUrl) return 'http';
  if (config.url) return 'http';
  return 'stdio';
}

function optionalPositiveNumber(value: unknown): number | undefined {
  if (value === null || value === undefined || value === '') {
    return undefined;
  }
  const numeric = Number(value);
  return Number.isFinite(numeric) && numeric > 0 ? numeric : undefined;
}

function optionalTimeoutFields(config: Record<string, unknown>): {
  startup_timeout_sec?: number;
  tool_timeout_sec?: number;
} {
  const startupTimeoutSec = optionalPositiveNumber(config.startup_timeout_sec);
  const toolTimeoutSec = optionalPositiveNumber(config.tool_timeout_sec);
  return {
    startup_timeout_sec: startupTimeoutSec,
    tool_timeout_sec: toolTimeoutSec,
  };
}

export function parseMcpServerConfig(config: Record<string, unknown>): StdioConfig | HttpConfig {
  const serverType = detectMcpServerType(config);
  const timeoutFields = optionalTimeoutFields(config);

  if (serverType === 'stdio') {
    let command = '';
    let args: string[] = [];

    if (Array.isArray(config.command)) {
      command = String(config.command[0] || '');
      args = config.command.slice(1).map(String);
    } else {
      command = String(config.command || '');
      args = Array.isArray(config.args) ? config.args.map(String) : [];
    }

    const env = isJsonObject(config.env)
      ? config.env as Record<string, string>
      : isJsonObject(config.environment)
        ? config.environment as Record<string, string>
        : undefined;

    return {
      command,
      args,
      env: env && Object.keys(env).length > 0 ? env : undefined,
      ...timeoutFields,
    };
  }

  const remoteUrl = serverType === 'http'
    ? config.serverUrl || config.httpUrl || config.url
    : config.url;

  return {
    url: String(remoteUrl || ''),
    headers: isJsonObject(config.headers) ? config.headers as Record<string, string> : undefined,
    ...timeoutFields,
  };
}

function extractServersObject(parsed: Record<string, unknown>): Record<string, unknown> {
  const wrappedMcpServers = parsed.mcpServers;
  if (isJsonObject(wrappedMcpServers)) {
    return wrappedMcpServers;
  }

  const wrappedServers = parsed.servers;
  if (isJsonObject(wrappedServers)) {
    return wrappedServers;
  }

  const mcpConfig = parsed.mcp;
  if (isJsonObject(mcpConfig) && isJsonObject(mcpConfig.servers)) {
    return mcpConfig.servers;
  }

  return parsed;
}

function buildParsedServer(name: string, config: Record<string, unknown>): ParsedMcpJsonServer | null {
  if (!hasServerConfigShape(config)) {
    return null;
  }

  const serverType = detectMcpServerType(config);
  const serverConfig = parseMcpServerConfig(config);

  if (serverType === 'stdio' && !(serverConfig as StdioConfig).command) {
    return null;
  }
  if ((serverType === 'http' || serverType === 'sse') && !(serverConfig as HttpConfig).url) {
    return null;
  }

  return {
    name,
    server_type: serverType,
    server_config: serverConfig,
  };
}

export function parseMcpServersFromJsonValue(value: unknown): ParsedMcpJsonServer[] {
  if (!isJsonObject(value)) {
    return [];
  }

  if (hasServerConfigShape(value)) {
    const name = normalizeServerName(value.name)
      ?? normalizeServerName(value.id)
      ?? DEFAULT_SINGLE_SERVER_NAME;
    const server = buildParsedServer(name, value);
    return server ? [server] : [];
  }

  const serversObject = extractServersObject(value);
  const parsedServers: ParsedMcpJsonServer[] = [];

  for (const [name, config] of Object.entries(serversObject)) {
    if (!isJsonObject(config)) {
      continue;
    }

    const server = buildParsedServer(name, config);
    if (server) {
      parsedServers.push(server);
    }
  }

  return parsedServers;
}
