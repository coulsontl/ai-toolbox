import React from 'react';
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import CodexModelFormModal from '@/features/coding/codex/components/CodexModelFormModal';
import GrokModelFormModal from '@/features/coding/grok/components/GrokModelFormModal';
import KimiModelFormModal from '@/features/coding/kimi/components/KimiModelFormModal';
import OpenClawModelFormModal from '@/features/coding/openclaw/components/OpenClawModelFormModal';
import ZcodeModelFormModal from '@/features/coding/zcode/components/ZcodeModelFormModal';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const dark = parameters.get('theme') === 'dark';
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = dark ? 'dark' : 'light';

window.__TAURI_INTERNALS__ = {
  invoke: async (command) => {
    throw new Error('Unexpected fixture command: ' + command);
  },
};

const noop = () => {};
const DIALOGS = {
  codex: () => (
    <CodexModelFormModal open isEdit={false} onCancel={noop} onSubmit={noop} />
  ),
  grok: () => (
    <GrokModelFormModal open isEdit={false} onCancel={noop} onSubmit={noop} />
  ),
  kimi: () => (
    <KimiModelFormModal open isEdit={false} providerKey="default" onCancel={noop} onSubmit={noop} />
  ),
  openclaw: () => (
    <OpenClawModelFormModal open existingIds={[]} onCancel={noop} onSubmit={noop} />
  ),
  zcode: () => (
    <ZcodeModelFormModal open isEdit={false} onCancel={noop} onSubmit={noop} />
  ),
};

const which = parameters.get('dialog') || 'codex';
const renderDialog = DIALOGS[which];
if (!renderDialog) throw new Error('Unknown dialog: ' + which);

function Harness() {
  return (
    <ConfigProvider theme={{ algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
      <App>{renderDialog()}</App>
    </ConfigProvider>
  );
}

createRoot(document.getElementById('root')).render(<Harness />);
window.__dialogFixture = { dialog: which };
