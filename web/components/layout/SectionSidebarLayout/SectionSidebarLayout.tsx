import React from 'react';
import { Menu, Tooltip, Typography } from 'antd';
import { PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { MenuProps } from 'antd';
import styles from './SectionSidebarLayout.module.less';

export type SidebarSectionMarker = {
  id: string;
  title: string;
  /**
   * Optional visual/sidebar order.
   * If not provided, sections fall back to DOM order.
   */
  order?: number;
};

interface SectionSidebarLayoutProps {
  children: React.ReactNode;
  sidebarTitle?: React.ReactNode;
  sidebarHidden?: boolean;
  defaultCollapsed?: boolean;
  sections?: SidebarSectionMarker[];
  /**
   * Return an icon for a section id.
   * If not provided, Menu items will show no icon (default antd behavior).
   */
  getIcon?: (id: string) => React.ReactNode;
  /**
   * Called before scrolling when a sidebar item is clicked.
   * Use this to expand the target Collapse panel(s).
   */
  onSectionSelect?: (id: string) => void;
  /**
   * Section marker attribute. Defaults to `data-sidebar-section="true"`.
   * The marker node should have:
   * - `id` for anchor target
   * - `data-sidebar-title` for display text
   */
  markerAttr?: string;
}

const DEFAULT_MARKER_ATTR = 'data-sidebar-section';

const SectionSidebarLayout: React.FC<SectionSidebarLayoutProps> = ({
  children,
  sidebarTitle,
  sidebarHidden = false,
  defaultCollapsed = true,
  sections,
  getIcon,
  onSectionSelect,
  markerAttr = DEFAULT_MARKER_ATTR,
}) => {
  const { Text } = Typography;
  const { t } = useTranslation();
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const scrollRetryRafRef = React.useRef<number | null>(null);
  const scrollRetryTimeoutIdsRef = React.useRef<number[]>([]);
  const [internalSidebarCollapsed, setInternalSidebarCollapsed] = React.useState(defaultCollapsed);
  const [scannedSidebarSections, setScannedSidebarSections] = React.useState<SidebarSectionMarker[]>([]);
  const [activeSectionId, setActiveSectionId] = React.useState<string>('');
  const effectiveSidebarSections = React.useMemo(() => {
    return (sections ?? scannedSidebarSections)
      .map((section, index) => ({ section, index }))
      .sort((left, right) => {
        const leftOrder = left.section.order ?? Number.POSITIVE_INFINITY;
        const rightOrder = right.section.order ?? Number.POSITIVE_INFINITY;
        if (leftOrder !== rightOrder) {
          return leftOrder - rightOrder;
        }
        return left.index - right.index;
      })
      .map(({ section }) => section);
  }, [scannedSidebarSections, sections]);

  const scanSidebarSections = React.useCallback(() => {
    const root = contentRef.current;
    if (!root) return;

    type SidebarSectionMarkerWithIndex = SidebarSectionMarker & { __domIndex: number };

    const nodes = Array.from(root.querySelectorAll<HTMLElement>(`[${markerAttr}="true"]`));

    const markersWithIndex = nodes
      .map((node, index): SidebarSectionMarkerWithIndex | null => {
        const id = node.id;
        const title = node.dataset.sidebarTitle;
        const orderRaw = node.dataset.sidebarOrder;
        if (!id || !title) return null;
        const order = orderRaw ? Number(orderRaw) : undefined;
        return {
          id,
          title,
          order: order !== undefined && Number.isFinite(order) ? order : undefined,
          // Keep DOM order as a stable fallback.
          __domIndex: index,
        };
      })
      .filter((v): v is SidebarSectionMarkerWithIndex => v !== null);

    // Stable sort:
    // - sections with smaller `order` come first
    // - missing `order` fall back to DOM order
    const sorted = markersWithIndex
      .sort((a, b) => {
        const aOrder = a.order ?? Number.POSITIVE_INFINITY;
        const bOrder = b.order ?? Number.POSITIVE_INFINITY;
        if (aOrder !== bOrder) return aOrder - bOrder;
        return a.__domIndex - b.__domIndex;
      })
      .map(({ __domIndex: _ignored, ...rest }) => rest);

    setScannedSidebarSections(sorted);
  }, [markerAttr]);

  // KeepAlive can retain another page with the same section IDs.
  const findSection = React.useCallback((id: string) => (
    contentRef.current?.querySelector<HTMLElement>(`#${CSS.escape(id)}`)
  ), []);

  const scrollToSection = React.useCallback((id: string, behavior: ScrollBehavior = 'smooth') => {
    const el = findSection(id);
    if (!el) return;
    el.scrollIntoView({ behavior, block: 'start' });
  }, [findSection]);

  const clearPendingScrollRetries = React.useCallback(() => {
    if (scrollRetryRafRef.current !== null) {
      cancelAnimationFrame(scrollRetryRafRef.current);
      scrollRetryRafRef.current = null;
    }

    scrollRetryTimeoutIdsRef.current.forEach((timeoutId) => {
      window.clearTimeout(timeoutId);
    });
    scrollRetryTimeoutIdsRef.current = [];
  }, []);

  const scheduleScrollToSection = React.useCallback((id: string) => {
    clearPendingScrollRetries();

    // Bottom sections may sit inside collapsed panels. Scroll once immediately,
    // then correct after the expand animation creates new scroll space.
    scrollRetryRafRef.current = requestAnimationFrame(() => {
      scrollToSection(id, 'smooth');
    });

    scrollRetryTimeoutIdsRef.current = [220, 420].map((delay) => (
      window.setTimeout(() => {
        scrollToSection(id, 'auto');
      }, delay)
    ));
  }, [clearPendingScrollRetries, scrollToSection]);

  React.useEffect(() => {
    if (sections) {
      return;
    }

    const root = contentRef.current;
    if (!root) return;

    let rafId = 0;
    const scheduleScan = () => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(() => scanSidebarSections());
    };

    // Handle dynamic/async sections (e.g., OpenCode's OMO blocks).
    const observer = new MutationObserver(() => {
      scheduleScan();
    });
    observer.observe(root, { childList: true, subtree: true });

    scanSidebarSections();

    return () => {
      cancelAnimationFrame(rafId);
      observer.disconnect();
      clearPendingScrollRetries();
    };
  }, [clearPendingScrollRetries, scanSidebarSections, sections]);

  React.useEffect(() => {
    if (!effectiveSidebarSections.length) return;
    if (effectiveSidebarSections.some((section) => section.id === activeSectionId)) return;
    setActiveSectionId(effectiveSidebarSections[0].id);
  }, [activeSectionId, effectiveSidebarSections]);

  React.useEffect(() => {
    if (!effectiveSidebarSections.length) return;

    const scrollRoot = contentRef.current?.closest('main');
    const targets = effectiveSidebarSections
      .map((section) => findSection(section.id))
      .filter((target): target is HTMLElement => Boolean(target));
    if (!targets.length) return;

    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.filter((e) => e.isIntersecting);
        if (!visible.length) return;

        visible.sort((a, b) => Math.abs(a.boundingClientRect.top) - Math.abs(b.boundingClientRect.top));
        const targetId = (visible[0].target as HTMLElement).id;
        if (targetId) setActiveSectionId(targetId);
      },
      {
        root: scrollRoot ?? undefined,
        threshold: [0, 0.1, 0.2, 0.35],
        rootMargin: '-84px 0px -60% 0px',
      }
    );

    targets.forEach((t) => {
      observer.observe(t);
    });

    return () => observer.disconnect();
  }, [effectiveSidebarSections, findSection]);

  const menuItems = React.useMemo(() => {
    return effectiveSidebarSections.map((section) => {
      const icon = getIcon?.(section.id);
      const label = section.title;
      return {
        key: section.id,
        icon,
        label,
      };
    });
  }, [effectiveSidebarSections, getIcon]);

  const handleMenuSelect: MenuProps['onClick'] = ({ key }) => {
    const id = String(key);
    onSectionSelect?.(id);
    setActiveSectionId(id);
    scheduleScrollToSection(id);
  };

  return (
    <div className={styles.pageWithSidebar}>
      {!sidebarHidden && (
        <aside className={`${styles.sidebar} ${internalSidebarCollapsed ? styles.sidebarCollapsed : ''}`}>
          <div className="sidebarHeaderWrapper">
            <div className={styles.sidebarHeader}>
              {!internalSidebarCollapsed ? <Text strong>{sidebarTitle}</Text> : <span />}
              <Tooltip
                title={internalSidebarCollapsed ? t('settings.nav.expand') : t('settings.nav.collapse')}
                placement="right"
              >
                <button
                  type="button"
                  className={styles.collapseButton}
                  aria-label={internalSidebarCollapsed ? t('settings.nav.expand') : t('settings.nav.collapse')}
                  aria-expanded={!internalSidebarCollapsed}
                  onClick={() => setInternalSidebarCollapsed((value) => !value)}
                >
                  {internalSidebarCollapsed
                    ? <PanelLeftOpen size={16} />
                    : <PanelLeftClose size={16} />}
                </button>
              </Tooltip>
            </div>
          </div>

          <Menu
            mode="inline"
            inlineCollapsed={internalSidebarCollapsed}
            selectedKeys={activeSectionId ? [activeSectionId] : []}
            items={menuItems}
            onClick={handleMenuSelect}
          />
        </aside>
      )}

      <div className={styles.content} ref={contentRef}>
        {children}
      </div>
    </div>
  );
};

export default SectionSidebarLayout;
