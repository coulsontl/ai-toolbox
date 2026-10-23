/**
 * Mounts the real `DshMigrationNotice` so its rendered shape can be measured.
 *
 * Why a fixture: the defect this guards was invisible to `tsc`, to the i18n
 * check and to every unit test. Ant Design 6 treats an object-form `closable`
 * as closable only when it also carries `closeIcon`/`closeText`, so
 * `closable={{ onClose }}` renders **no × at all** while typing perfectly —
 * the user reported exactly that ("没看到关闭按钮啊"). The notice cannot be
 * mounted through `DshPage` (30+ Tauri commands), so the check mounts the
 * component the page actually renders.
 */
import { createRoot } from 'react-dom/client';
import { ConfigProvider, theme } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import enUS from 'antd/locale/en_US';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import DshMigrationNotice from '@/features/coding/dsh/components/DshMigrationNotice';
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

/** The two paths the notice announces. Any string works; these read like real ones. */
const PATCH_PATH = 'C:\\Users\\test\\.dsh\\profiles\\web\\cordis.patch.yml';
const IMPORTED_PATH = 'C:\\Users\\test\\.dsh\\settings.yaml.imported';

window.dshMigrationNoticeFixture = {
  reset() {
    try { localStorage.clear(); } catch { /* storage disabled */ }
    render();
  },
  /** The notice element itself, identified by its own data attribute. */
  exists: () => Boolean(document.querySelector('[data-dsh-migration-notice]')),
  noticeText: () => document.querySelector('[data-dsh-migration-notice]')?.textContent ?? '',
  /**
   * Ant Design renders the close affordance inside `.ant-alert-close-icon`.
   * Screen-reader-wise it is a `<button>`, so require that too — an icon that
   * cannot be clicked is not a close button.
   */
  closeButtonIsClickable() {
    const closeIcon = document.querySelector('.ant-alert-close-icon');
    if (!closeIcon) return false;
    const target = closeIcon.querySelector('button') ?? closeIcon;
    const rect = target.getBoundingClientRect();
    if (rect.width < 4 || rect.height < 4) return false;
    if (getComputedStyle(target).pointerEvents === 'none') return false;
    target.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
    return true;
  },
  /** One text line: the notice must not render as a title + description block. */
  isSingleLine() {
    const notice = document.querySelector('[data-dsh-migration-notice]');
    if (!notice) return false;
    const style = getComputedStyle(notice);
    const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.5;
    return notice.getBoundingClientRect().height <= lineHeight * 1.6;
  },
  hasDescriptionBlock: () => Boolean(document.querySelector('.ant-alert-description')),
  dismissedPathInStorage() {
    try { return localStorage.getItem('dsh.migrationNoticeDismissedPath'); } catch { return null; }
  },
};

const root = createRoot(document.getElementById('root'));

function render() {
  root.render(
    <ConfigProvider
      locale={language === 'en-US' ? enUS : zhCN}
      theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}
    >
      <DshMigrationNotice patchPath={PATCH_PATH} importedPath={IMPORTED_PATH} />
    </ConfigProvider>,
  );
}

window.dshMigrationNoticeFixture.expectedPatchPath = PATCH_PATH;
render();
