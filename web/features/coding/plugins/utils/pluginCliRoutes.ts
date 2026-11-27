/**
 * Where each plugin CLI target lives in the app.
 *
 * The backend reports the coding sub-tab keys (`claudecode`, `openclaw`,
 * `oh_my_pi`, …), while the routes use the product names. The gateway-key
 * aliases (`claude`, `workbuddy_desktop`, …) are accepted as well so a caller
 * from the gateway settings pages can reuse the map. A key we do not know
 * simply yields no link instead of a wrong destination.
 */
const PLUGIN_CLI_PATHS: Record<string, string> = {
  claude: '/coding/claudecode',
  claudecode: '/coding/claudecode',
  claude_desktop: '/coding/claudedesktop',
  claudedesktop: '/coding/claudedesktop',
  codex: '/coding/codex',
  grok: '/coding/grok',
  kimi: '/coding/kimi',
  gemini: '/coding/geminicli',
  geminicli: '/coding/geminicli',
  antigravity: '/coding/antigravity',
  opencode: '/coding/opencode',
  openclaw: '/coding/openclaw',
  pi: '/coding/pi',
  oh_my_pi: '/coding/oh-my-pi',
  omp: '/coding/oh-my-pi',
  omo_native: '/coding/omo-native',
  omo: '/coding/omo-native',
  hermes: '/coding/hermes',
  dsh: '/coding/dsh',
  zcode: '/coding/zcode',
  workbuddy_desktop: '/coding/workbuddy',
  workbuddy: '/coding/workbuddy',
  codebuddy_cli: '/coding/codebuddy',
  codebuddy: '/coding/codebuddy',
};

export const getPluginCliPath = (cliKey: string): string | null =>
  PLUGIN_CLI_PATHS[cliKey.trim().toLowerCase()] ?? null;
