import React from 'react';
import { Button, Menu, Typography } from 'antd';
import { MenuFoldOutlined, MenuUnfoldOutlined } from '@ant-design/icons';
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
  getIcon,
  onSectionSelect,
  markerAttr = DEFAULT_MARKER_ATTR,
}) => {
  const { Text } = Typography;
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = React.useState(false);
  const [sidebarSections, setSidebarSections] = React.useState<SidebarSectionMarker[]>([]);
  const [activeSectionId, setActiveSectionId] = React.useState<string>('');

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

    setSidebarSections(sorted);
  }, [markerAttr]);

  const scrollToSection = React.useCallback((id: string) => {
    const el = document.getElementById(id);
    if (!el) return;
    el.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }, []);

  React.useEffect(() => {
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
    };
  }, [scanSidebarSections]);

  React.useEffect(() => {
    if (!sidebarSections.length) return;
    if (sidebarSections.some((s) => s.id === activeSectionId)) return;
    setActiveSectionId(sidebarSections[0].id);
  }, [activeSectionId, sidebarSections]);

  React.useEffect(() => {
    if (!sidebarSections.length) return;

    const scrollRoot = document.querySelector('main') as HTMLElement | null;
    const targets = sidebarSections.map((s) => document.getElementById(s.id)).filter(Boolean) as HTMLElement[];
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
  }, [sidebarSections]);

  const menuItems = React.useMemo(() => {
    return sidebarSections.map((s) => {
      const icon = getIcon?.(s.id);
      const label = s.title;
      return {
        key: s.id,
        icon,
        label,
      };
    });
  }, [sidebarSections, getIcon]);

  const handleMenuSelect: MenuProps['onClick'] = ({ key }) => {
    const id = String(key);
    onSectionSelect?.(id);
    setActiveSectionId(id);
    // Let state updates for Collapse happen before scrolling.
    requestAnimationFrame(() => scrollToSection(id));
  };

  return (
    <div className={styles.pageWithSidebar}>
      <aside className={`${styles.sidebar} ${sidebarCollapsed ? styles.sidebarCollapsed : ''}`}>
        <div className="sidebarHeaderWrapper">
          <div className={styles.sidebarHeader}>
            {!sidebarCollapsed ? <Text strong>{sidebarTitle}</Text> : <span />}
            <Button
              type="text"
              size="small"
              icon={sidebarCollapsed ? <MenuUnfoldOutlined /> : <MenuFoldOutlined />}
              onClick={() => setSidebarCollapsed((v) => !v)}
            />
          </div>
        </div>

        <Menu
          mode="inline"
          inlineCollapsed={sidebarCollapsed}
          selectedKeys={activeSectionId ? [activeSectionId] : []}
          items={menuItems}
          onClick={handleMenuSelect}
        />
      </aside>

      <div className={styles.content} ref={contentRef}>
        {children}
      </div>
    </div>
  );
};

export default SectionSidebarLayout;

