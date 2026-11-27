export type PluginsPageTab = 'installed' | 'accounts' | 'market' | 'runtime';

/** The management tab or one plugin page the `/plugins` workspace shows. */
export type PluginsRoute =
  | { kind: 'tab'; tab: PluginsPageTab }
  | { kind: 'page'; pluginId: string; pageId: string | null };

export interface PluginsTabConfig {
  key: PluginsPageTab;
  labelKey: string;
  path: string;
}

export const PLUGIN_TABS: PluginsTabConfig[] = [
  {
    key: 'installed',
    labelKey: 'plugins.page.tabs.installed',
    path: '/plugins/installed',
  },
  {
    key: 'accounts',
    labelKey: 'plugins.page.tabs.accounts',
    path: '/plugins/accounts',
  },
  {
    key: 'market',
    labelKey: 'plugins.page.tabs.market',
    path: '/plugins/market',
  },
  {
    key: 'runtime',
    labelKey: 'plugins.page.tabs.runtime',
    path: '/plugins/runtime',
  },
];

export const DEFAULT_PLUGINS_PATH = PLUGIN_TABS[0].path;

export const isPluginsPath = (pathname: string) =>
  pathname === '/plugins' || pathname.startsWith('/plugins/');

export const resolvePluginsTabFromPath = (pathname: string): PluginsPageTab => {
  const matchedTab = PLUGIN_TABS.find(
    (tab) => pathname === tab.path || pathname.startsWith(`${tab.path}/`),
  );
  return matchedTab?.key ?? 'installed';
};

export const getPluginsPathForTab = (tabKey: PluginsPageTab) =>
  PLUGIN_TABS.find((tab) => tab.key === tabKey)?.path ?? DEFAULT_PLUGINS_PATH;

/** Deep-link to the accounts tab with one provider preselected. */
export const getPluginsAccountsPath = (providerId?: string) => {
  const basePath = getPluginsPathForTab('accounts');
  return providerId ? `${basePath}?provider=${encodeURIComponent(providerId)}` : basePath;
};

const PLUGINS_PAGE_PREFIX = '/plugins/page/';

/** Deep-link to a plugin page; the page id is optional for a plugin's first page. */
export const getPluginsCustomPagePath = (pluginId: string, pageId?: string | null) => {
  const basePath = `${PLUGINS_PAGE_PREFIX}${encodeURIComponent(pluginId)}`;
  return pageId ? `${basePath}/${encodeURIComponent(pageId)}` : basePath;
};

const decodePathPart = (value: string): string => {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
};

/** Resolve any `/plugins/**` pathname to a management tab or a plugin page. */
export const resolvePluginsRoute = (pathname: string): PluginsRoute => {
  if (pathname.startsWith(PLUGINS_PAGE_PREFIX)) {
    const rest = pathname.slice(PLUGINS_PAGE_PREFIX.length);
    const [rawPluginId = '', rawPageId = ''] = rest.split('/');
    if (rawPluginId) {
      return {
        kind: 'page',
        pluginId: decodePathPart(rawPluginId),
        pageId: rawPageId ? decodePathPart(rawPageId) : null,
      };
    }
  }
  return { kind: 'tab', tab: resolvePluginsTabFromPath(pathname) };
};
