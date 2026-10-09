import React from 'react';
import { App, Button, Collapse, Empty, Space, Spin, Tooltip, Typography } from 'antd';
import {
  AimOutlined,
  AppstoreOutlined,
  CheckSquareOutlined,
  DatabaseOutlined,
  PlusOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderBatchToolbar from './providerList/ProviderBatchToolbar';
import ProviderSearchEmpty from './providerList/ProviderSearchEmpty';
import ProviderSearchInput from './providerList/ProviderSearchInput';
import ProviderSortDropdown from './providerList/ProviderSortDropdown';
import type { ProviderSortMode } from './providerList/sortProviders';
import type { ProviderBatchSelection } from './providerList/useProviderBatchSelection';
import styles from './ProviderListSection.module.less';

const { Text } = Typography;

/** Held just past the `.locateFlash` animation's own 1.4s. */
const LOCATE_FLASH_DURATION_MS = 1500;

/** The panel's expand animation; scrolling before it settles lands short. */
const COLLAPSE_SETTLE_DELAY_MS = 260;

/** How long to wait for a smooth scroll to settle before flashing anyway. */
const SCROLL_SETTLE_DEADLINE_MS = 1200;

/**
 * One pending flash-removal per card.
 *
 * A second locate on the same card has to *extend* the flash: without this, the
 * first click's removal timer fires mid-way through the second flash and cuts it
 * short — the same "arrived at a card with no highlight" outcome the arrival
 * delay exists to prevent, and re-clicking is exactly what a user does when a
 * long list takes a moment to move.
 */
const pendingFlashTimers = new WeakMap<HTMLElement, number>();

/**
 * Flashes a card once it has stopped moving.
 *
 * The flash must not start at click time: scrolling to a card further down the
 * list takes a few hundred ms, so a flash that ran during the scroll would be
 * spent almost entirely off screen and the user would arrive at an unmarked
 * card — the one thing the action exists to prevent. Waiting for the card's
 * position to settle also covers "the card was already in view" (no scroll
 * happens, so it settles immediately) and falls back to a deadline, because the
 * page cannot tell a finished scroll from a slow one.
 */
const flashWhenArrived = (card: HTMLElement) => {
  const startedAt = performance.now();
  let previousTop = card.getBoundingClientRect().top;
  let stillFrames = 0;
  const step = () => {
    const top = card.getBoundingClientRect().top;
    stillFrames = Math.abs(top - previousTop) < 0.5 ? stillFrames + 1 : 0;
    previousTop = top;
    if (stillFrames < 2 && performance.now() - startedAt < SCROLL_SETTLE_DEADLINE_MS) {
      window.requestAnimationFrame(step);
      return;
    }
    const previousTimer = pendingFlashTimers.get(card);
    if (previousTimer !== undefined) {
      window.clearTimeout(previousTimer);
    }
    card.classList.add(styles.locateFlash);
    pendingFlashTimers.set(card, window.setTimeout(() => {
      card.classList.remove(styles.locateFlash);
      pendingFlashTimers.delete(card);
    }, LOCATE_FLASH_DURATION_MS));
  };
  window.requestAnimationFrame(step);
};

/**
 * The hint block's visual contract: small, secondary-coloured text behind a
 * thin left rule.
 *
 * This lives here rather than in each caller because every caller used to
 * hand-copy the same style object — and the one that forgot silently rendered
 * its hint as full-size body text. The caller still owns the *wording*;
 * only the styling is the component's.
 */
const HINT_BLOCK_STYLE: React.CSSProperties = {
  fontSize: 12,
  color: 'var(--color-text-secondary)',
  borderLeft: '2px solid var(--color-border)',
  paddingLeft: 8,
  marginBottom: 12,
};

export interface ProviderListSectionProps {
  /** Sidebar section anchor id, e.g. `codex-providers`. */
  sectionId: string;

  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  loading?: boolean;

  /** Provider count; drives the empty state. */
  providerCount: number;
  /** Count after the search filter; drives the search-empty state. */
  visibleCount: number;

  /** Provider-level batch selection state (see `useProviderBatchSelection`). */
  batch: ProviderBatchSelection<string>;
  /** Ids eligible for batch selection (already filtered by the caller). */
  batchSelectableIds: string[];

  keyword: string;
  onKeywordChange: (keyword: string) => void;
  sortMode: ProviderSortMode;
  sortModes: readonly ProviderSortMode[];
  onSortModeChange: (mode: ProviderSortMode) => void;

  /**
   * Id of the provider currently in effect — the card wearing the "applied"
   * tag. Passing it (even as `''`, meaning "nothing is applied right now")
   * renders the toolbar's locate action; omitting it hides the action, for
   * tools that have no such thing as one applied provider (OmO Native runs
   * several at once).
   */
  locateProviderId?: string;
  /**
   * Whether the current sort mode is the one that disables dragging. Callers
   * already compute this to gate the drag grips; passing it on lets the sort
   * control say *why* they are gone.
   */
  dragDisabledBySort?: boolean;

  onBatchTest?: () => void;
  batchTesting?: boolean;
  onOpenCommonConfig?: () => void;
  onAddProvider: () => void;

  /** Extra nodes rendered in the section header label, next to the title (e.g. Gateway chips). */
  headerExtra?: React.ReactNode;

  /**
   * Extra empty-state sentence appended to `common.provider.emptyText`, for
   * tools that can import providers from somewhere specific ("…or import from
   * OpenCode"). Most tools need nothing here.
   */
  emptyTextHint?: React.ReactNode;

  /**
   * Hint block under the toolbar. Callers pass their own **wording only** —
   * the block's styling (small secondary text behind a left rule) is applied
   * by this component, so pass bare `<div>`s rather than a styled wrapper.
   */
  hint?: React.ReactNode;

  /**
   * Rendered in place of nothing when the list is empty or a search matches
   * nothing.
   *
   * The empty and search-empty branches replace `children` wholesale, which is
   * right for provider cards and wrong for anything that is merely *listed*
   * alongside them: a section the user can only reach by having at least one
   * provider, or by clearing their search, has silently gone missing.
   */
  alwaysVisible?: React.ReactNode;
  /** Rendered above the list (below the hint), e.g. the provider cards. */
  children: React.ReactNode;
  /** Import buttons rendered below the list (CC Switch / All API Hub / favorites). */
  footer?: React.ReactNode;
}

/**
 * Shared shell for the "provider list" section on every coding tab.
 *
 * Locks the skeleton the pages had each re-implemented: the Collapse section
 * header with the batch/search/sort/batch-test/common-config/add-provider
 * toolbar, the hint block, the empty and search-empty states, and the import
 * footer. Tool-specific wording and actions are passed in as props or slots so
 * a new CLI inherits the layout instead of copying it.
 */
const ProviderListSection: React.FC<ProviderListSectionProps> = ({
  sectionId,
  collapsed,
  onCollapsedChange,
  loading = false,
  providerCount,
  visibleCount,
  batch,
  batchSelectableIds,
  keyword,
  onKeywordChange,
  sortMode,
  sortModes,
  onSortModeChange,
  locateProviderId,
  dragDisabledBySort = false,
  onBatchTest,
  batchTesting = false,
  onOpenCommonConfig,
  onAddProvider,
  headerExtra,
  emptyTextHint,
  hint,
  alwaysVisible,
  children,
  footer,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const rootRef = React.useRef<HTMLDivElement | null>(null);

  const toolbarButtonStyle: React.CSSProperties = { fontSize: 12 };

  /**
   * Scrolls to the applied provider's card and flashes it.
   *
   * The card is found in the DOM by the `data-provider-id` that
   * `providerCardVariants/CardShell` puts on every card, rather than by state
   * threaded down from the owning page: the page already renders the cards
   * itself, and the answer here depends on what is *currently visible* (the
   * search filter, the sort mode) — which is exactly what the DOM knows and a
   * prop would have to be kept in sync with.
   */
  const locateAppliedProvider = () => {
    const root = rootRef.current;
    // The click handler has already explained the empty case to the user.
    if (!root || !locateProviderId) {
      return;
    }
    const card = root.querySelector<HTMLElement>(
      `[data-provider-id="${CSS.escape(locateProviderId)}"]`,
    );
    if (!card) {
      message.info(
        keyword.trim()
          ? t('common.provider.locateFiltered')
          : t('common.provider.locateMissing'),
      );
      return;
    }
    card.scrollIntoView({ behavior: 'smooth', block: 'center' });
    flashWhenArrived(card);
  };

  const handleLocateClick = (event: React.MouseEvent) => {
    event.stopPropagation();
    // Answered before expanding: with nothing applied there is no card to
    // scroll to, and opening the section to deliver that message would be an
    // edit to the page the user did not ask for.
    if (!locateProviderId) {
      message.info(t('common.provider.locateNone'));
      return;
    }
    if (!collapsed) {
      locateAppliedProvider();
      return;
    }
    // Expand first and scroll once the panel has settled — its content is
    // inside the collapse animation before that (`SectionSidebarLayout` waits
    // for the same reason).
    onCollapsedChange(false);
    window.setTimeout(locateAppliedProvider, COLLAPSE_SETTLE_DELAY_MS);
  };

  return (
    <div
      ref={rootRef}
      id={sectionId}
      data-sidebar-section="true"
      data-sidebar-title={t('common.provider.title')}
    >
      <Collapse
        style={{ marginBottom: 16 }}
        activeKey={collapsed ? [] : ['providers']}
        onChange={(keys) => onCollapsedChange(!keys.includes('providers'))}
        items={[
          {
            key: 'providers',
            label: (
              <Space size={8} wrap>
                <Text strong>
                  <DatabaseOutlined style={{ marginRight: 8 }} />
                  {t('common.provider.title')}
                </Text>
                {headerExtra}
              </Space>
            ),
            extra: (
              <Space size={4} wrap>
                <Button
                  type="link"
                  size="small"
                  style={{ ...toolbarButtonStyle, display: 'inline-flex', alignItems: 'center', gap: 4 }}
                  onClick={(event) => {
                    event.stopPropagation();
                    if (batch.selectionMode) {
                      batch.exitSelection();
                    } else {
                      batch.enterSelection();
                    }
                  }}
                >
                  <CheckSquareOutlined style={{ fontSize: 13, lineHeight: 1 }} />
                  <span>
                    {batch.selectionMode ? t('common.batch.exit') : t('common.batch.manage')}
                  </span>
                </Button>
                {batch.selectionMode && (
                  <ProviderBatchToolbar
                    hasSelection={batch.hasSelection}
                    visibleCount={batchSelectableIds.length}
                    isAllSelected={batch.isAllSelected}
                    indeterminate={batch.indeterminate}
                    onSelectAll={batch.selectAllFiltered}
                    onBatchDelete={batch.batchDelete}
                    disabled={loading}
                  />
                )}
                <ProviderSearchInput value={keyword} onChange={onKeywordChange} />
                <ProviderSortDropdown
                  mode={sortMode}
                  modes={sortModes}
                  onChange={onSortModeChange}
                  dragDisabledBySort={dragDisabledBySort}
                />
                {locateProviderId !== undefined && (
                  <Tooltip title={t('common.provider.locateTooltip')}>
                    <Button
                      type="link"
                      size="small"
                      style={toolbarButtonStyle}
                      icon={<AimOutlined />}
                      onClick={handleLocateClick}
                    >
                      {t('common.provider.locate')}
                    </Button>
                  </Tooltip>
                )}
                {onBatchTest && (
                  <Button
                    type="link"
                    size="small"
                    style={toolbarButtonStyle}
                    icon={<ThunderboltOutlined />}
                    loading={batchTesting}
                    onClick={(event) => {
                      event.stopPropagation();
                      onBatchTest();
                    }}
                  >
                    {t('common.batchTest')}
                  </Button>
                )}
                {onOpenCommonConfig && (
                  <Button
                    type="link"
                    size="small"
                    style={toolbarButtonStyle}
                    icon={<AppstoreOutlined />}
                    onClick={(event) => {
                      event.stopPropagation();
                      onOpenCommonConfig();
                    }}
                  >
                    {t('common.provider.commonConfig')}
                  </Button>
                )}
                <Button
                  type="link"
                  size="small"
                  style={toolbarButtonStyle}
                  icon={<PlusOutlined />}
                  onClick={(event) => {
                    event.stopPropagation();
                    onAddProvider();
                  }}
                >
                  {t('common.provider.add')}
                </Button>
              </Space>
            ),
            children: (
              <Spin spinning={loading}>
                {hint && <div style={HINT_BLOCK_STYLE}>{hint}</div>}
                {(providerCount === 0 || visibleCount === 0) && alwaysVisible}
                {providerCount === 0 ? (
                  <Empty
                    description={
                      emptyTextHint ? (
                        <Space orientation="vertical" size={2}>
                          <span>{t('common.provider.emptyText')}</span>
                          <span>{emptyTextHint}</span>
                        </Space>
                      ) : (
                        t('common.provider.emptyText')
                      )
                    }
                    style={{ marginTop: 40 }}
                  />
                ) : visibleCount === 0 ? (
                  <ProviderSearchEmpty />
                ) : (
                  children
                )}
                {footer && <div style={{ marginTop: 12 }}>{footer}</div>}
              </Spin>
            ),
          },
        ]}
      />
    </div>
  );
};

export default ProviderListSection;
