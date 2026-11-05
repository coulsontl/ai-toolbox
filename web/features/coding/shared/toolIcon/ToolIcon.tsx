import React from 'react';
import {
  Amp,
  Antigravity,
  Cursor,
  DeepSeek,
  Gemini,
  GithubCopilot,
  Goose,
  Grok,
  HermesAgent,
  KiloCode,
  Kimi,
  OpenClaw,
  Qoder,
  Qwen,
  RooCode,
  Trae,
  Windsurf,
  Zhipu,
} from '@lobehub/icons';
import { Globe } from 'lucide-react';
import { useThemeStore } from '@/stores/themeStore';
// Marks for tools with no LobeHub brand and no tab asset, copied from the
// skills-manager reference project (see web/assets/agent-icons/NOTICE.md).
// SVGs are inlined via ?raw so rendering never depends on the asset pipeline.
import droidMarkRaw from '@/assets/agent-icons/droid.svg?raw';
import openclawFamilyMarkRaw from '@/assets/agent-icons/openclaw.svg?raw';
import workbuddyIconUrl from '@/assets/agent-icons/workbuddy.png';
// Tab-first marks: these tools render the exact assets their navigation tabs
// use in MainLayout (see TAB_ICONS there), ahead of any LobeHub brand icon.
import claudeMarkRaw from '@/assets/claude.svg?raw';
import chatgptMarkRaw from '@/assets/chatgpt.svg?raw';
import opencodeMarkRaw from '@/assets/opencode.svg?raw';
import omoNativeMarkRaw from '@/assets/omo-native.svg?raw';
import styles from './ToolIcon.module.less';

// Brand icons come from @lobehub/icons (see https://lobehub.com/icons). The
// `.Color` variant is preferred wherever the brand ships one; brands without
// it fall back to their mono icon, which renders in currentColor and adapts
// to light/dark themes automatically. Tool keys map 1:1 to the backend
// builtin tool keys in tauri/src/coding/tools/builtin.rs.
type ToolIconRenderer = React.ComponentType<{ size?: number | string }>;

// Local marks mirror the assets the navigation tabs use in MainLayout
// (web/assets/{claude,pi,omp}.svg). They are inlined as components instead of
// <img src> asset URLs so rendering never depends on the asset pipeline/CSP.
const ClaudeCodeMark: ToolIconRenderer = ({ size = 16 }) => (
  <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden="true" style={{ flex: 'none', lineHeight: 1 }}>
    <path
      fill="rgb(217, 119, 87)"
      d="M202.112 678.656l200.64-112.64 3.392-9.792-3.392-5.44h-9.792l-33.6-2.048-114.624-3.072-99.456-4.224-96.384-5.12-24.192-5.12-22.72-29.952 2.304-14.976 20.48-13.696 29.12 2.56 64.576 4.416 96.832 6.72 70.208 4.096 104.064 10.88h16.576l2.304-6.72-5.696-4.16-4.352-4.096-100.224-67.968-108.48-71.744-56.768-41.344-30.72-20.928-15.488-19.584-6.72-42.88 27.84-30.72 37.504 2.56 9.536 2.56 37.952 29.184 81.088 62.784 105.856 77.952 15.488 12.928 6.208-4.352 0.768-3.136L395.264 360l-57.6-104.064-61.44-105.92-27.392-43.904-7.168-26.304c-2.56-10.88-4.48-19.904-4.48-30.976l31.808-43.136L286.592 0l42.304 5.696 17.856 15.488 26.304 60.16 42.624 94.72 66.112 128.896 19.392 38.208 10.24 35.392 3.904 10.88h6.72v-6.208l5.44-72.576 10.048-89.088 9.856-114.688 3.328-32.256 16-38.72 31.808-20.928 24.768 11.904 20.416 29.184-2.88 18.816-12.16 78.72-23.68 123.52-15.552 82.56h9.088l10.304-10.24 41.856-55.552 70.208-87.808 30.976-34.88 36.16-38.464 23.232-18.368h43.904l32.32 48.064-14.464 49.6-45.184 57.28-37.44 48.576-53.76 72.32-33.536 57.856 3.072 4.608 8-0.768 121.408-25.792 65.6-11.904 78.208-13.44 35.392 16.512 3.84 16.832-13.952 34.304-83.648 20.672-98.112 19.648-146.176 34.56-1.792 1.28 2.048 2.56 65.92 6.272 28.096 1.536h68.928l128.384 9.6 33.536 22.144 20.16 27.136-3.392 20.672-51.648 26.304-69.696-16.512-162.688-38.72-55.744-13.952h-7.744v4.672l46.464 45.44 85.184 76.928 106.688 99.2 5.376 24.512-13.632 19.328-14.464-2.048-93.76-70.464-36.16-31.808-81.856-68.928h-5.44v7.232l18.88 27.648 99.648 149.76 5.184 45.952-7.232 14.976-25.856 9.024-28.352-5.12L673.408 856l-60.16-92.16-48.576-82.624-5.952 3.392-28.672 308.544-13.44 15.744-30.976 11.904-25.792-19.648-13.696-31.744 13.696-62.72 16.512-81.92 13.44-65.024 12.16-80.832 7.232-26.88-0.512-1.792-5.952 0.768-60.928 83.648-92.736 125.248-73.344 78.528-17.536 6.976-30.464-15.808 2.816-28.16 17.024-24.96 101.504-129.152 61.184-80 39.552-46.272-0.256-6.72h-2.368L177.6 789.44l-48 6.144-20.736-19.328 2.56-31.744 9.856-10.368 81.088-55.744-0.256 0.256z"
    />
  </svg>
);

// pi.svg mark: currentColor, so it follows the surrounding text color.
const PiMark: ToolIconRenderer = ({ size = 16 }) => (
  <svg width={size} height={size} viewBox="-47 -47 564 564" aria-hidden="true" style={{ flex: 'none', lineHeight: 1 }}>
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M0 0H352.07V234.71H234.71V352.07H117.36V469.43H0V0ZM117.36 117.36V234.71H234.71V117.36Z"
    />
    <path fill="currentColor" d="M352.07 234.71H469.43V469.43H352.07V234.71Z" />
  </svg>
);

// omp.svg mark (Oh My Pi): fixed brand gradient colors. The gradient id must
// be unique per instance: url(#id) resolves to the first matching element in
// the document, and KeepAlive keeps visited pages mounted under display:none —
// a hidden OMP mark there would otherwise blank every visible OMP mark.
const OmpMark: ToolIconRenderer = ({ size = 16 }) => {
  const gradientId = `omp-mark-grad-${React.useId().replace(/[^a-zA-Z0-9_-]/g, '')}`;
  return (
    <svg width={size} height={size} viewBox="5.5 8.5 53 53" aria-hidden="true" style={{ flex: 'none', lineHeight: 1 }}>
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#F84FCC" />
          <stop offset=".5" stopColor="#9362F4" />
          <stop offset="1" stopColor="#00DBE4" />
        </linearGradient>
      </defs>
      <path fill={`url(#${gradientId})`} d="M10 14h44v9H43v33h-9V23h-9v22h-9V23H10z" />
    </svg>
  );
};

const TOOL_ICON_RENDERERS: Record<string, ToolIconRenderer> = {
  grok: Grok,
  // Kimi.Color is a white glyph meant for a brand-blue badge background and is
  // invisible on light surfaces; the mono mark follows currentColor instead.
  kimi: Kimi,
  // Matches the geminicli tab, which uses Gemini.Color (not the CLI mark).
  gemini_cli: Gemini.Color,
  qwen_code: Qwen.Color,
  cursor: Cursor,
  antigravity: Antigravity.Color,
  antigravity_cli: Antigravity.Color,
  // ZCode ships no LobeHub mark of its own; it is Zhipu's coding CLI, so the
  // Zhipu brand mark is the closest truthful match.
  zcode: Zhipu.Color,
  zcode_cli: Zhipu.Color,
  amp: Amp.Color,
  kilo_code: KiloCode,
  roo_code: RooCode,
  goose: Goose,
  github_copilot: GithubCopilot,
  github_copilot_intellij: GithubCopilot,
  qoder: Qoder.Color,
  qoder_work: Qoder.Color,
  trae: Trae.Color,
  trae_cn: Trae.Color,
  windsurf: Windsurf,
  hermes: HermesAgent,
  dsh: DeepSeek.Color,
  // Public shared-skills directory (agentskills.io): no brand mark exists, so
  // it uses the reference project's convention for network sources - a
  // currentColor Globe glyph that adapts to light/dark themes.
  shared_agents: Globe,
};

// Raw SVG marks, rendered inline. The inner svg is sized by the wrapper via
// CSS (see .rawIcon in ToolIcon.module.less). Entries map 1:1 to the assets
// MainLayout uses for the same tool's navigation tab — `omo_native` reuses the
// tab asset so the Skills/MCP lists show the same mark as the sidebar.
const RAW_SVG_MARKS: Record<string, string> = {
  claude_desktop: claudeMarkRaw,
  codex: chatgptMarkRaw,
  opencode: opencodeMarkRaw,
  omo_native: omoNativeMarkRaw,
  droid: droidMarkRaw,
  qclaw: openclawFamilyMarkRaw,
  easyclaw: openclawFamilyMarkRaw,
  autoclaw: openclawFamilyMarkRaw,
};

// Raw marks drawn with `currentColor` (they follow .rawIcon's theme-safe
// color). Everything else has hardcoded fills and needs the tab-style dark
// filter (.rawIconFixedColor) — inverting a currentColor mark would make it
// invisible again.
const CURRENT_COLOR_RAW_MARKS = new Set(['droid']);

// PNG marks cannot be inlined as SVG; render as <img>.
const PNG_ICON_URLS: Record<string, string> = {
  workbuddy: workbuddyIconUrl,
  workbuddy_ai: workbuddyIconUrl,
};

// Shared Git source glyph (lucide omits a GitHub mark from the default set
// used here). Sized via the `size` prop; color inherits from `currentColor`.
export const GitHubSourceIcon: React.FC<{ size?: number; className?: string }> = ({
  size = 16,
  className,
}) => (
  <svg
    className={className}
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.4 5.4 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65S8.93 17.38 9 18v4" />
    <path d="M9 18c-4.51 2-5-2-7-2" />
  </svg>
);

// Short two-character fallback label for tools without a brand icon,
// such as user-defined tools or OpenClaw-family derivatives.
export function getToolAbbreviation(label: string, toolKey: string): string {
  const words = label.trim().split(/\s+/).filter(Boolean);
  if (words.length >= 2) {
    return `${words[0][0]}${words[1][0]}`.toUpperCase();
  }
  const word = words[0] || toolKey;
  return word.slice(0, 2).toUpperCase();
}

interface ToolIconProps {
  toolKey: string;
  label: string;
  size?: number;
  /** Custom-tool brand icon as an http(s) image URL. */
  iconUrl?: string;
}

export const ToolIcon: React.FC<ToolIconProps> = ({ toolKey, label, size = 16, iconUrl }) => {
  const { resolvedTheme } = useThemeStore();

  // Local marks matching the navigation tab assets (inline SVG components).
  if (toolKey === 'claude_code') {
    return <ClaudeCodeMark size={size} />;
  }
  if (toolKey === 'pi') {
    return <PiMark size={size} />;
  }
  if (toolKey === 'oh_my_pi') {
    return <OmpMark size={size} />;
  }
  // OpenClaw follows the tab treatment: color mark on light, mono on dark.
  if (toolKey === 'openclaw') {
    const OpenClawVariant = resolvedTheme === 'dark' ? OpenClaw : OpenClaw.Color;
    return <OpenClawVariant size={size} />;
  }
  // Reference-project SVG marks, rendered inline (never via asset URL <img>).
  const rawMark = RAW_SVG_MARKS[toolKey];
  if (rawMark) {
    return (
      <span
        className={`${styles.rawIcon}${CURRENT_COLOR_RAW_MARKS.has(toolKey) ? '' : ` ${styles.rawIconFixedColor}`}`}
        style={{ width: size, height: size }}
        dangerouslySetInnerHTML={{ __html: rawMark }}
      />
    );
  }
  // Reference-project PNG marks.
  const pngIconUrl = PNG_ICON_URLS[toolKey];
  if (pngIconUrl) {
    return (
      <img
        src={pngIconUrl}
        alt=""
        title={label}
        draggable={false}
        width={size}
        height={size}
        className={styles.iconImage}
      />
    );
  }
  // Custom tools may carry a user-provided http(s) image URL.
  if (iconUrl && /^https?:\/\//.test(iconUrl)) {
    return (
      <img
        src={iconUrl}
        alt=""
        title={label}
        draggable={false}
        loading="lazy"
        width={size}
        height={size}
        className={styles.iconImage}
      />
    );
  }
  const IconRenderer = TOOL_ICON_RENDERERS[toolKey];
  if (IconRenderer) {
    // Host span anchors `currentColor` (lucide Globe, LobeHub mono variants)
    // to a theme-safe color — buttons and menu items do not inherit the page
    // text color, so a bare renderer draws black-on-dark in the dark theme.
    return (
      <span className={styles.iconHost}>
        <IconRenderer size={size} />
      </span>
    );
  }
  return (
    <span
      className={styles.fallbackBadge}
      title={label}
      style={{ width: size, height: size, fontSize: size * 0.5 }}
    >
      {getToolAbbreviation(label, toolKey)}
    </span>
  );
};
