import React from 'react';
import { useTranslation } from 'react-i18next';

/** The theme shape a plugin page receives from the bridge's `hello`. */
export interface PluginBridgeTheme {
  dark: boolean;
  lang: string;
  tokens: Record<string, string>;
}

/**
 * Magpie's page bridge paints each token as `--magpie-<key>`, so the keys must
 * keep its kebab-case names. Values come from AI Toolbox's semantic CSS
 * variables and are read at handshake time to follow the live theme.
 */
const TOKEN_VARIABLES: Record<string, string> = {
  bg: '--color-bg-layout',
  card: '--color-bg-container',
  fg: '--color-text-primary',
  'fg-2': '--color-text-secondary',
  muted: '--color-text-tertiary',
  faint: '--color-text-tertiary',
  line: '--color-border',
  accent: '--color-primary',
  'accent-soft': '--color-bg-selected',
  green: '--color-status-success',
  red: '--color-status-error',
  amber: '--color-status-warning',
};

const FALLBACK_FONT_STACK =
  'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';
const CODE_FONT_STACK =
  'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace';

export const readPluginBridgeTheme = (language: string): PluginBridgeTheme => {
  const computed = getComputedStyle(document.documentElement);
  const tokens: Record<string, string> = {};
  for (const [name, variable] of Object.entries(TOKEN_VARIABLES)) {
    tokens[name] = computed.getPropertyValue(variable).trim();
  }
  const themeAttribute = document.documentElement.getAttribute('data-theme');
  const dark = themeAttribute === 'dark'
    || (themeAttribute !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  // Text that sits on the accent colour: white in light, near-white in dark.
  tokens['accent-fg'] = computed
    .getPropertyValue(dark ? '--color-text-primary' : '--color-bg-container')
    .trim();
  const bodyFont = document.body
    ? getComputedStyle(document.body).fontFamily.trim()
    : '';
  tokens.font = bodyFont || FALLBACK_FONT_STACK;
  tokens.code = CODE_FONT_STACK;
  return { dark, lang: language, tokens };
};

/** Track the live app theme and language for plugin page handshakes. */
export const usePluginBridgeTheme = (): PluginBridgeTheme => {
  const { i18n } = useTranslation();
  const [theme, setTheme] = React.useState(() => readPluginBridgeTheme(i18n.language));
  const themeRef = React.useRef(theme);

  React.useEffect(() => {
    const update = () => {
      const next = readPluginBridgeTheme(i18n.language);
      if (JSON.stringify(next) !== JSON.stringify(themeRef.current)) {
        themeRef.current = next;
        setTheme(next);
      }
    };

    update();
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme', 'style', 'class'],
    });
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    media.addEventListener('change', update);

    return () => {
      observer.disconnect();
      media.removeEventListener('change', update);
    };
  }, [i18n.language]);

  return theme;
};
