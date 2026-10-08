/**
 * Mounts the three shared card styles directly, with no page and no IPC, so
 * their *spacing* can be measured in a browser.
 *
 * Why not a page fixture: spacing is a property of the card component alone —
 * it has no handlers to exercise and reaches nothing outside React. A page
 * fixture would add dozens of stub commands (the Claude Code page alone calls
 * 40+) to answer a question the component already owns, and the day a page
 * gains a command the fixture breaks for a reason unrelated to what it guards.
 *
 * What it guards: each style carries a deliberate title-to-subtitle distance,
 * and the two differ on purpose. The Claude style stacks on 4px plus the row's
 * own 4px offset (8 total, as it always had); the Codex style stacks on 4. A
 * migration that flattened both onto one value halved the Claude gap without
 * anything failing, so the numbers are pinned here.
 */
import './ProviderCardSpacingStubs.js';
import { createRoot } from 'react-dom/client';
import { ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { updateGatewayProviderProfiles } from '@/features/coding/shared/gateway/providerProfiles';
import ClaudeStyleCard from '@/features/coding/shared/providerCardVariants/ClaudeStyleCard';
import CodexStyleCard from '@/features/coding/shared/providerCardVariants/CodexStyleCard';
import gatewayProfiles from '../../../../../../../tauri/resources/gateway_provider_profiles.json';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const themeMode = parameters.get('theme') || 'light';
const resolvedTheme = themeMode === 'system'
  ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
  : themeMode;
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = resolvedTheme;
updateGatewayProviderProfiles(gatewayProfiles);

const claudeCard = (
  <ClaudeStyleCard
    provider={{ id: 'claude-card', name: 'AxonHub-Claude', baseUrl: 'https://api.example.invalid/anthropic' }}
    metaEntries={[
      { kind: 'code', label: 'Haiku:', value: 'claude-haiku-4-5' },
      { kind: 'code', label: 'Sonnet:', value: 'claude-opus-4-8[1M]' },
    ]}
  />
);

const codexCard = (
  <CodexStyleCard
    provider={{ id: 'codex-card', name: 'AxonHub-Codex' }}
    metaEntries={[{ kind: 'code', value: 'https://api.example.invalid/v1' }]}
  />
);

window.providerCardSpacingFixture = {
  /** Distance from the bottom of the name row to the top of the meta row. */
  titleToMetaGap(cardLabel) {
    const card = [...document.querySelectorAll('.ant-card')]
      .find(node => node.textContent.includes(cardLabel));
    if (!card) throw new Error('No card rendered for: ' + cardLabel);
    const nameNode = [...card.querySelectorAll('*')]
      .find(node => node.children.length === 0 && node.textContent.trim().startsWith(cardLabel));
    const nameRow = nameNode?.closest('div');
    const metaRow = nameRow?.nextElementSibling;
    if (!nameRow || !metaRow) throw new Error('No meta row under: ' + cardLabel);
    return Math.round(metaRow.getBoundingClientRect().top - nameRow.getBoundingClientRect().bottom);
  },
};

createRoot(document.getElementById('root')).render(
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <div style={{ padding: 16 }}>
      {claudeCard}
      {codexCard}
    </div>
  </ConfigProvider>,
);