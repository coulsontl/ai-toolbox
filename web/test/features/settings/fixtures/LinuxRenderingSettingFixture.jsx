import React from 'react';
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider } from 'antd';
import LinuxRenderingSetting from '@/features/settings/components/LinuxRenderingSetting';
import i18n from '@/i18n';
import '@/App.css';

await i18n.changeLanguage('zh-CN');
const state = { mode: 'compatible', active: 'compatible', calls: [], failSave: false, failLoad: false, environmentOverride: false, deferSave: false, release: null };
const info = () => ({ supported: true, mode: state.mode, active_mode: state.active, workaround_level: 1, environment_override: state.environmentOverride, restart_required: state.mode !== state.active });
window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    state.calls.push({ command, args });
    if (command === 'get_linux_webview_rendering') {
      if(state.failLoad) { state.failLoad = false; throw Error('test load failure'); }
      return info();
    }
    if (command === 'set_linux_webview_rendering') {
      if (state.failSave) { state.failSave = false; throw Error('test save failure'); }
      if (state.deferSave) await new Promise(resolve => { state.release = resolve; });
      state.mode = args.mode;
      return info();
    }
    if (command === 'restart_app') { state.active = state.mode; return; }
    throw Error('Unexpected command: ' + command);
  },
};
const root = createRoot(document.getElementById('root'));
let key = 0;
const mount = () => root.render(<ConfigProvider><App><div style={{ padding: 24, maxWidth: 700 }}><LinuxRenderingSetting key={key++} /></div></App></ConfigProvider>);
window.renderingFixture = { state, mount, label: key => i18n.t(key).replace(/\s/g, '') };
mount();
