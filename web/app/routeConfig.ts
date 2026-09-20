import type { ComponentType } from 'react';
import { NotesPage } from '@/features/daily';
import { OpenCodePage, ClaudeCodePage, ClaudeDesktopPage, HermesPage, DshPage, CodexPage, GrokPage, KimiPage, GeminiCliPage, PiPage, OhMyPiPage } from '@/features/coding';
import { OpenClawPage } from '@/features/coding/openclaw';
import { SettingsPage } from '@/features/settings';
import { SkillsPage } from '@/features/coding/skills';
import { McpPage } from '@/features/coding/mcp';
import { ImagePage } from '@/features/coding/image';
import { GatewayPage } from '@/features/coding/gateway';
import { MiniBrowserPage } from '@/features/mini-browser';
import {
  ClaudeCodeSessionDetailPage,
  ClaudeDesktopSessionDetailPage,
  CodexSessionDetailPage,
  GrokSessionDetailPage,
  KimiSessionDetailPage,
  GeminiCliSessionDetailPage,
  OpenClawSessionDetailPage,
  OpenCodeSessionDetailPage,
  PiSessionDetailPage,
  OhMyPiSessionDetailPage,
  HermesSessionDetailPage,
  DshSessionDetailPage,
} from '@/features/coding/shared/sessionManager/detail/SessionDetailPage';

export interface RouteEntry {
  path: string;
  routePath?: string;
  component: ComponentType;
  chrome?: RouteChromeConfig;
}

export type RouteChromeMode = 'default' | 'secondary';
export type RouteContentPadding = 'default' | 'compact' | 'none';

export interface RouteChromeConfig {
  mode?: RouteChromeMode;
  contentPadding?: RouteContentPadding;
  ownerTabKey?: string;
  parentPath?: string;
}

/**
 * 统一路由配置，新增页面只需在此处添加一条记录。
 * routes.tsx 和 MainLayout 的 KeepAliveOutlet 共同消费此配置。
 *
 * KeepAlive 注意事项：
 * - 页面组件在 Tab 切走时不会卸载，通过 display:none 隐藏
 * - 避免在 loadConfig 等后台刷新函数中直接调用 message.error，应使用 silent 参数
 * - 避免使用 window.location.reload()，应改为调用数据刷新函数
 * - 可通过 useKeepAlive() hook 获取 isActive 状态，感知页面是否可见
 */
export const PAGE_ROUTES: RouteEntry[] = [
  { path: '/daily/notes', component: NotesPage },
  { path: '/coding/opencode', component: OpenCodePage },
  {
    path: '/coding/opencode/sessions/detail',
    component: OpenCodeSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'opencode',
      parentPath: '/coding/opencode',
    },
  },
  { path: '/coding/claudecode', component: ClaudeCodePage },
  {
    path: '/coding/claudecode/sessions/detail',
    component: ClaudeCodeSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'claudecode',
      parentPath: '/coding/claudecode',
    },
  },
  { path: '/coding/codex', component: CodexPage },
  {
    path: '/coding/codex/sessions/detail',
    component: CodexSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'codex',
      parentPath: '/coding/codex',
    },
  },
  { path: '/coding/grok', component: GrokPage },
  {
    path: '/coding/grok/sessions/detail',
    component: GrokSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'grok',
      parentPath: '/coding/grok',
    },
  },
  { path: '/coding/kimi', component: KimiPage },
  {
    path: '/coding/kimi/sessions/detail',
    component: KimiSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'kimi',
      parentPath: '/coding/kimi',
    },
  },
  { path: '/coding/openclaw', component: OpenClawPage },
  {
    path: '/coding/openclaw/sessions/detail',
    component: OpenClawSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'openclaw',
      parentPath: '/coding/openclaw',
    },
  },
  { path: '/coding/geminicli', component: GeminiCliPage },
  {
    path: '/coding/geminicli/sessions/detail',
    component: GeminiCliSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'geminicli',
      parentPath: '/coding/geminicli',
    },
  },
  { path: '/coding/pi', component: PiPage },
  {
    path: '/coding/pi/sessions/detail',
    component: PiSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'pi',
      parentPath: '/coding/pi',
    },
  },
  { path: '/coding/oh-my-pi', component: OhMyPiPage },
  {
    path: '/coding/oh-my-pi/sessions/detail',
    component: OhMyPiSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'oh_my_pi',
      parentPath: '/coding/oh-my-pi',
    },
  },
  { path: '/coding/claudedesktop', component: ClaudeDesktopPage },
  {
    path: '/coding/claudedesktop/sessions/detail',
    component: ClaudeDesktopSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'claudedesktop',
      parentPath: '/coding/claudedesktop',
    },
  },
  { path: '/coding/hermes', component: HermesPage },
  {
    path: '/coding/hermes/sessions/detail',
    component: HermesSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'hermes',
      parentPath: '/coding/hermes',
    },
  },
  { path: '/coding/dsh', component: DshPage },
  {
    path: '/coding/dsh/sessions/detail',
    component: DshSessionDetailPage,
    chrome: {
      mode: 'secondary',
      contentPadding: 'compact',
      ownerTabKey: 'dsh',
      parentPath: '/coding/dsh',
    },
  },
  { path: '/settings', component: SettingsPage },
  { path: '/skills', component: SkillsPage },
  { path: '/mcp', component: McpPage },
  { path: '/gateway', routePath: '/gateway/*', component: GatewayPage },
  { path: '/mini-browser', component: MiniBrowserPage },
  { path: '/images', component: ImagePage },
];
