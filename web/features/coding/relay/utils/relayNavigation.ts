export interface Api2ApiTabConfig {
  key: string;
  labelKey: string;
  path: string;
}

/**
 * 2API's internal tabs.
 *
 * Routing drives the selection (the same shape Gateway uses) so a tab survives
 * a reload, and KeepAlive keeps each one's scroll position while it is away.
 */
export const RELAY_TABS: Api2ApiTabConfig[] = [
  {
    key: 'channels',
    labelKey: 'relay.page.tabs.channels',
    path: '/2api/channels',
  },
  {
    key: 'plugins',
    labelKey: 'relay.page.tabs.plugins',
    path: '/2api/plugins',
  },
  {
    key: 'checkin',
    labelKey: 'relay.page.tabs.checkin',
    path: '/2api/checkin',
  },
  {
    key: 'settings',
    labelKey: 'relay.page.tabs.settings',
    path: '/2api/settings',
  },
];

export const DEFAULT_RELAY_PATH = RELAY_TABS[0].path;

export const isRelayPath = (pathname: string) =>
  pathname === '/2api' || pathname.startsWith('/2api/');

export function resolveRelayTabFromPath(pathname: string): string {
  const match = RELAY_TABS.find((tab) => pathname.startsWith(tab.path));
  return match?.key ?? RELAY_TABS[0].key;
}

export function getRelayPathForTab(tabKey: string): string {
  return RELAY_TABS.find((tab) => tab.key === tabKey)?.path ?? DEFAULT_RELAY_PATH;
}
