/**
 * Pins the **page** rhythm of the four `CodingPageHeader` pages that also own a
 * config *directory* (pi / oh_my_pi / dsh / hermes).
 *
 * Why this exists: every one of those pages was migrated onto `CodingPageHeader`
 * while keeping a `display: flex; column; gap: 16px` page container from its
 * bespoke-header days. `CodingPageHeader` brings its **own** 16px bottom margin,
 * so the header-to-first-section gap became 32px — the same defect OpenCode
 * recorded on 2026-10-19 (SOP §4.3) and the same one the user reported again on
 * 2026-10-22 for the three freshly migrated pages. It is invisible to
 * `tsc`, to the i18n check and to every component-level fixture: only the page
 * container's computed layout decides it.
 *
 * Why not mount the real pages: each reaches 30+ Tauri commands, and the
 * question here is purely "what gap does this stylesheet produce". So the
 * fixture mounts the **real CSS modules** inside the same `main` scroller the
 * app uses (`App.css` locks `html/body/#root` to `overflow: hidden`; the
 * scrolling element is `MainLayout`'s `main`), and measures the geometry.
 *
 * Screenshot: the artifact proves the blocks were on screen, not just in the DOM.
 */
import { createRoot } from 'react-dom/client';
import { ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import piStyles from '@/features/coding/pi/pages/PiPage.module.less';
import ompStyles from '@/features/coding/oh_my_pi/pages/OhMyPiPage.module.less';
import dshStyles from '@/features/coding/dsh/pages/DshPage.module.less';
import hermesStyles from '@/features/coding/hermes/pages/HermesPage.module.less';
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

/** One page shell per migrated page, using that page's own stylesheet. */
const PAGES = [
  { key: 'pi', label: 'Pi', styles: piStyles, sectionClass: 'piSection' },
  { key: 'omp', label: 'Oh My Pi', styles: ompStyles, sectionClass: 'ompSection' },
  { key: 'dsh', label: 'DeepSeek Harness', styles: dshStyles, sectionClass: 'dshSection' },
  { key: 'hermes', label: 'Hermes', styles: hermesStyles, sectionClass: 'hermesSection' },
];

const PageShell = ({ page }) => (
  <div className={page.styles.pageContent} data-page-shell={page.key}>
    <CodingPageHeader
      title={page.label}
      configPath={`C:\\Users\\test\\.${page.key}`}
      configPathKind="directory"
      onPreviewConfig={() => {}}
      onOpenFolder={() => {}}
      onRefresh={() => {}}
    />
    <div className={page.styles[page.sectionClass]} data-section="first">
      <div data-section-body style={{ height: 40, background: 'var(--color-bg-container)' }} />
    </div>
    <div className={page.styles[page.sectionClass]} data-section="second">
      <div data-section-body style={{ height: 40, background: 'var(--color-bg-container)' }} />
    </div>
  </div>
);

const app = (
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <main style={{ height: '100vh', overflow: 'auto', padding: 16 }}>
      {PAGES.map((page) => <PageShell key={page.key} page={page} />)}
    </main>
  </ConfigProvider>
);

window.pageRhythmFixture = {
  /**
   * Vertical distance between two sibling boxes, measured through their rects.
   * Positive means a visible gap; negative would mean overlap.
   */
  gapBetween(pageKey, fromSelector, toSelector) {
    const shell = document.querySelector(`[data-page-shell="${pageKey}"]`);
    if (!shell) return null;
    const from = shell.querySelector(fromSelector);
    const to = shell.querySelector(toSelector);
    if (!from || !to) return null;
    return Math.round(to.getBoundingClientRect().top - from.getBoundingClientRect().bottom);
  },
  /** The header is the shell's first element; it is never a `.…Section`. */
  headerGapToFirstSection(pageKey) {
    const shell = document.querySelector(`[data-page-shell="${pageKey}"]`);
    if (!shell) return null;
    const header = shell.firstElementChild;
    const firstSection = shell.querySelector('[data-section="first"]');
    return Math.round(firstSection.getBoundingClientRect().top - header.getBoundingClientRect().bottom);
  },
  /**
   * A section whose body carries no margin must space itself from the next
   * sibling by its own `margin-bottom`, not by borrowing the neighbour's.
   * The second section is the last child, so the first section's bottom margin
   * is what separates them.
   */
  sectionGap(pageKey) {
    return this.gapBetween(pageKey, '[data-section="first"]', '[data-section="second"]');
  },
};

createRoot(document.getElementById('root')).render(app);
