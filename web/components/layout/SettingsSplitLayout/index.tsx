import React from 'react';
import { Tooltip } from 'antd';
import { PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import styles from './SettingsSplitLayout.module.less';

export interface SettingsSplitLayoutItem {
  key: string;
  label: string;
  icon: React.ReactNode;
}

interface SettingsSplitLayoutProps {
  items: SettingsSplitLayoutItem[];
  activeKey: string;
  onChange: (key: string) => void;
  banner?: React.ReactNode;
  /**
   * `page` fills the viewport below the app chrome and scrolls its detail
   * column on its own. `embedded` grows with the page so it can sit below an
   * existing sticky header without creating a second scrollbar.
   */
  variant?: 'page' | 'embedded';
  /** Shows a toggle that collapses the rail to icon-only width. */
  collapsible?: boolean;
  /** Rail title shown at the top, next to the collapse toggle. */
  title?: string;
  ariaLabel?: string;
  children: React.ReactNode;
}

const joinClassNames = (...classNames: Array<string | false | null | undefined>) =>
  classNames.filter(Boolean).join(' ');

const SettingsSplitLayout: React.FC<SettingsSplitLayoutProps> = ({
  items,
  activeKey,
  onChange,
  banner,
  variant = 'page',
  collapsible = false,
  title,
  ariaLabel,
  children,
}) => {
  const { t } = useTranslation();
  const idPrefix = React.useId();
  const navRef = React.useRef<HTMLElement | null>(null);
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const [collapsed, setCollapsed] = React.useState(false);

  // Switching categories in the fixed-height page variant should start the new
  // detail column at its top instead of keeping the previous scroll offset.
  React.useEffect(() => {
    if (variant === 'page') {
      contentRef.current?.scrollTo({ top: 0 });
    }
  }, [activeKey, variant]);

  const focusTab = (key: string) => {
    navRef.current
      ?.querySelector<HTMLButtonElement>(`[data-settings-tab-key="${CSS.escape(key)}"]`)
      ?.focus();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    const currentIndex = items.findIndex((item) => item.key === activeKey);
    if (currentIndex < 0) {
      return;
    }

    let nextIndex: number;
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowRight':
        nextIndex = (currentIndex + 1) % items.length;
        break;
      case 'ArrowUp':
      case 'ArrowLeft':
        nextIndex = (currentIndex - 1 + items.length) % items.length;
        break;
      case 'Home':
        nextIndex = 0;
        break;
      case 'End':
        nextIndex = items.length - 1;
        break;
      default:
        return;
    }

    event.preventDefault();
    const nextKey = items[nextIndex].key;
    onChange(nextKey);
    focusTab(nextKey);
  };

  const activeItem = items.find((item) => item.key === activeKey) ?? items[0];

  return (
    <div
      className={joinClassNames(
        styles.splitLayout,
        variant === 'page' ? styles.page : styles.embedded,
      )}
    >
      <aside className={joinClassNames(styles.sidebar, collapsed && styles.sidebarCollapsed)}>
        {title || collapsible ? (
          <div className={styles.navHeader}>
            {!collapsed && title ? (
              <span className={styles.brandTitle}>{title}</span>
            ) : null}
            {collapsible ? (
              <Tooltip
                title={collapsed ? t('settings.nav.expand') : t('settings.nav.collapse')}
                placement="right"
              >
                <button
                  type="button"
                  className={styles.navCollapse}
                  aria-label={collapsed ? t('settings.nav.expand') : t('settings.nav.collapse')}
                  aria-expanded={!collapsed}
                  onClick={() => setCollapsed((previous) => !previous)}
                >
                  {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
                </button>
              </Tooltip>
            ) : null}
          </div>
        ) : null}

        <nav
          ref={navRef}
          className={styles.nav}
          role="tablist"
          aria-orientation="vertical"
          aria-label={ariaLabel}
          onKeyDown={handleKeyDown}
        >
          {items.map((item) => {
            const isActive = item.key === activeKey;
            return (
              <Tooltip
                key={item.key}
                title={collapsed ? item.label : undefined}
                placement="right"
              >
                <button
                  type="button"
                  role="tab"
                  id={`${idPrefix}-tab-${item.key}`}
                  aria-selected={isActive}
                  aria-controls={`${idPrefix}-panel`}
                  aria-label={item.label}
                  tabIndex={isActive ? 0 : -1}
                  data-settings-tab-key={item.key}
                  className={joinClassNames(styles.navItem, isActive && styles.navItemActive)}
                  onClick={() => onChange(item.key)}
                >
                  <span className={styles.navIcon} aria-hidden="true">
                    {item.icon}
                  </span>
                  <span className={styles.navLabel}>{item.label}</span>
                </button>
              </Tooltip>
            );
          })}
        </nav>
      </aside>

      <div
        ref={contentRef}
        className={styles.content}
        role="tabpanel"
        id={`${idPrefix}-panel`}
        aria-labelledby={activeItem ? `${idPrefix}-tab-${activeItem.key}` : undefined}
        tabIndex={0}
      >
        {banner ? <div className={styles.banner}>{banner}</div> : null}
        <div className={styles.contentBody}>{children}</div>
      </div>
    </div>
  );
};

export default SettingsSplitLayout;
