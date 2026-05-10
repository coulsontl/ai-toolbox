import type { McpGroup, McpServer } from '../types';

export function normalizeMcpMetadataText(value: string | null | undefined): string | null {
  const trimmed = value?.trim() ?? '';
  return trimmed ? trimmed : null;
}

export function getMcpGroupOptions(servers: McpServer[]): string[] {
  const groups = new Set<string>();
  for (const server of servers) {
    const group = normalizeMcpMetadataText(server.user_group);
    if (group) {
      groups.add(group);
    }
  }
  return [...groups].sort((left, right) => left.localeCompare(right));
}

export function getMcpDisplayNote(server: McpServer): string | null {
  return normalizeMcpMetadataText(server.user_note)
    ?? normalizeMcpMetadataText(server.description);
}

export function filterMcpServersBySearch(
  servers: McpServer[],
  searchText: string,
  getConfigSummary: (server: McpServer) => string,
): McpServer[] {
  const keyword = searchText.trim().toLowerCase();
  if (!keyword) {
    return servers;
  }

  return servers.filter((server) => {
    const searchableValues = [
      server.name,
      server.server_type,
      getConfigSummary(server),
      server.description,
      server.user_group,
      server.user_note,
    ];

    return searchableValues.some((value) => value?.toLowerCase().includes(keyword));
  });
}

export function buildMcpGroups(
  servers: McpServer[],
  labels: { groupUngrouped: string },
): McpGroup[] {
  const groupMap = new Map<string, McpGroup>();

  for (const server of servers) {
    const userGroup = normalizeMcpMetadataText(server.user_group);
    const key = userGroup ? `custom:${userGroup}` : 'custom:__ungrouped__';
    const label = userGroup ?? labels.groupUngrouped;
    const existing = groupMap.get(key);

    if (existing) {
      existing.servers.push(server);
    } else {
      groupMap.set(key, {
        key,
        label,
        servers: [server],
      });
    }
  }

  return Array.from(groupMap.values());
}
