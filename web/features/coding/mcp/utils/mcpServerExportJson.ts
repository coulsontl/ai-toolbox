import type { HttpConfig, McpServer, StdioConfig } from '../types';
import { preservedServerConfigFields } from './mcpServerConfigFields.ts';

/**
 * Convert one managed server into the standard `mcpServers` entry shape that
 * `parseMcpServersFromJsonValue` (mcpJsonImport.ts) understands, so an export
 * can be pasted straight back into "Import from JSON".
 *
 * Only server config is exported. Management metadata (group/note/tags) is
 * intentionally omitted: the importer ignores it, and the import dialog picks
 * sync targets itself.
 */
function buildServerConfigEntry(server: McpServer): Record<string, unknown> {
  const config = server.server_config as StdioConfig | HttpConfig;

  // Tool-specific keys (OmO's `connectTimeoutMs` / `requestTimeoutMs`, or an
  // imported `lifecycle`) are not modelled by the form, but an export must
  // carry them through so a paste-back preserves the original entry.
  const extraFields = preservedServerConfigFields(config, ['name', 'id']);

  if (server.server_type === 'stdio') {
    const stdioConfig = config as StdioConfig;
    const entry: Record<string, unknown> = {
      ...extraFields,
      type: 'stdio',
      command: stdioConfig.command,
      args: stdioConfig.args ?? [],
    };
    if (stdioConfig.env && Object.keys(stdioConfig.env).length > 0) {
      entry.env = stdioConfig.env;
    }
    if (stdioConfig.startup_timeout_sec !== undefined) {
      entry.startup_timeout_sec = stdioConfig.startup_timeout_sec;
    }
    if (stdioConfig.tool_timeout_sec !== undefined) {
      entry.tool_timeout_sec = stdioConfig.tool_timeout_sec;
    }
    return entry;
  }

  const httpConfig = config as HttpConfig;
  const entry: Record<string, unknown> = {
    ...extraFields,
    type: server.server_type,
    url: httpConfig.url,
  };
  if (httpConfig.headers && Object.keys(httpConfig.headers).length > 0) {
    entry.headers = httpConfig.headers;
  }
  if (httpConfig.startup_timeout_sec !== undefined) {
    entry.startup_timeout_sec = httpConfig.startup_timeout_sec;
  }
  if (httpConfig.tool_timeout_sec !== undefined) {
    entry.tool_timeout_sec = httpConfig.tool_timeout_sec;
  }
  return entry;
}

/** Build the clipboard-ready `{ mcpServers: { name: config } }` payload. */
export function buildMcpServerExportJson(server: McpServer): Record<string, unknown> {
  return {
    mcpServers: {
      [server.name]: buildServerConfigEntry(server),
    },
  };
}

/** Pretty-printed export text, ready for the clipboard. */
export function formatMcpServerExportJson(server: McpServer): string {
  return JSON.stringify(buildMcpServerExportJson(server), null, 2);
}
