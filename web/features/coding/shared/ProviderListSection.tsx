import React from 'react';
import { App, Button, Collapse, Empty, Space, Spin, Tooltip, Typography } from 'antd';
import {
  AimOutlined,
  AppstoreOutlined,
  CheckSquareOutlined,
  DatabaseOutlined,
  LayoutOutlined,
  PlusOutlined,
  ThunderboltOutlined,
  UnorderedListOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderBatchToolbar from './providerList/ProviderBatchToolbar';
import ProviderSearchEmpty from './providerList/ProviderSearchEmpty';
import ProviderSearchInput from './providerList/ProviderSearchInput';
import ProviderSortDropdown from './providerList/ProviderSortDropdown';
import type { ProviderSortMode } from './providerList/sortProviders';
import type { ProviderBatchSelection } from './providerList/useProviderBatchSelection';
import type { ProviderViewMode } from './providerList/useProviderListViewPrefs';
import styles from './ProviderListSection.module.less';

const { Text } = Typography;

/** Held just past the `.locateFlash` animation's own 1.4s. */
const LOCATE_FLASH_DURATION_MS = 1500;

/** How long to wait for a smooth scroll to settle before flashing anyway. */
const SCROLL_SETTLE_DEADLINE_MS = 1200;

/** How long a locate waits for the section's expand motion to lay the card out. */
const CARD_LAYOUT_DEADLINE_MS = 1500;

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
 * Flashes a card once it has arrived and stopped moving.
 *
 * The flash must not start at click time: scrolling to a card further down the
 * list takes a few hundred ms, so a flash that ran during the scroll would be
 * spent almost entirely off screen and the user would arrive at an unmarked
 * card — the one thing the action exists to prevent.
 *
 * "Stopped moving" alone is not enough to mean "arrived": at the start of an
 * eased scroll — or while the main thread is busy — the card sits still for a
 * frame or two before the scroll commits, and two still frames would then read
 * as arrival while the card is still below the fold. The card must also be
 * fully in view. A card already on screen satisfies both at once, so the
 * no-scroll case still flashes immediately; a card taller than the viewport
 * never satisfies the second and falls back to the deadline.
 */
const flashWhenArrived = (card: HTMLElement) => {
  const startedAt = performance.now();
  let previousTop = card.getBoundingClientRect().top;
  let stillFrames = 0;
  const step = () => {
    const rect = card.getBoundingClientRect();
    // `height > 0` keeps a hidden card (a collapsed ancestor reports zero
    // rects) from counting as in-view: `top >= 0 && bottom <= innerHeight`
    // is trivially true for an all-zero rect, and the flash would be spent
    // on a card nobody can see.
    const inView = rect.height > 0 && rect.top >= 0 && rect.bottom <= window.innerHeight;
    stillFrames = inView && Math.abs(rect.top - previousTop) < 0.5 ? stillFrames + 1 : 0;
    previousTop = rect.top;
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
 * Waits until the card is laid out, then runs `onReady` with it.
 *
 * A collapsed panel keeps its children in the DOM at `display: none`, where
 * every rect is zero and `scrollIntoView` does nothing at all — scrolling at
 * that moment would spend the flash on an invisible card. Polling for a
 * measured box replaces guessing the expand motion's duration, which is a
 * themeable value with a 500ms fallback deadline of its own.
 *
 * Returns a cancel function; the caller owns when the run stops being wanted
 * (the user re-collapses the section, or the page unmounts).
 */
const waitForLaidOutCard = (
  root: HTMLElement,
  providerId: string,
  onReady: (card: HTMLElement) => void,
  onTimeout: () => void,
): (() => void) => {
  const startedAt = performance.now();
  let frameId = 0;
  let cancelled = false;
  const step = () => {
    if (cancelled) {
      return;
    }
    const card = root.querySelector<HTMLElement>(
      `[data-provider-id="${CSS.escape(providerId)}"]`,
    );
    if (card && card.getBoundingClientRect().height > 0) {
      onReady(card);
      return;
    }
    if (performance.now() - startedAt < CARD_LAYOUT_DEADLINE_MS) {
      frameId = window.requestAnimationFrame(step);
      return;
    }
    onTimeout();
  };
  frameId = window.requestAnimationFrame(step);
  return () => {
    cancelled = true;
    window.cancelAnimationFrame(frameId);
  };
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
  /** Set when no provider in the list can be tested; the button then explains itself. */
  batchTestDisabled?: boolean;
  batchTestDisabledReason?: string;
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
   * Current layout of the provider list, and its setter.
   *
   * Both must be supplied for the toolbar's layout switch to render — the
   * "pass it or it is absent" rule the section's other slots follow. Omit both
   * for a CLI whose cards have not been given a compact form yet: a switch
   * that flips to a layout nobody implemented is worse than no switch.
   */
  viewMode?: ProviderViewMode;
  onViewModeChange?: (mode: ProviderViewMode) => void;

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
  viewMode,
  onViewModeChange,
  dragDisabledBySort = false,
  onBatchTest,
  batchTesting = false,
  batchTestDisabled = false,
  batchTestDisabledReason,
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
  /** Cancels an in-flight locate whose panel was re-collapsed or unmounted. */
  const cancelPendingLocateRef = React.useRef<(() => void) | null>(null);

  const toolbarButtonStyle: React.CSSProperties = { fontSize: 12 };

  /**
   * The three "why nothing moved" answers, told to the user when the action
   * cannot point at a card.
   *
   * Guarded because `App.useApp()` does not throw outside an `<App>` provider:
   * antd's context default is `{ message: {} }`, so `message.info` is
   * `undefined` there — and a fixture or host that mounts the section directly
   * would crash on exactly the branches that exist to explain themselves.
   */
  const explainLocateFailure = (reason: string) => {
    message.info?.(reason);
  };

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
  const locateAppliedProvider = (providerId: string) => {
    const root = rootRef.current;
    if (!root) {
      return;
    }
    const card = root.querySelector<HTMLElement>(
      `[data-provider-id="${CSS.escape(providerId)}"]`,
    );
    if (!card) {
      explainLocateFailure(
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
    // The toolbar lives inside the collapse header, whose own keydown handler
    // toggles the panel on Enter. A native button fires its click *after* that
    // keydown, so without swallowing the event one keypress would both collapse
    // the section and run locate against the panel it just closed.
    event.stopPropagation();
    event.preventDefault();
    // Answered before expanding: with nothing applied there is no card to
    // scroll to, and opening the section to deliver that message would be an
    // edit to the page the user did not ask for.
    if (!locateProviderId) {
      explainLocateFailure(t('common.provider.locateNone'));
      return;
    }
    if (!collapsed) {
      locateAppliedProvider(locateProviderId);
      return;
    }
    // Expand first, then scroll once the panel has actually laid the card out.
    // Waiting on the measured box rather than on a fixed delay is deliberate:
    // a hidden panel reports zero rects, and the expand motion is a themeable
    // duration, so any constant here is a guess that fails silently (the scroll
    // becomes a no-op and the flash is spent on an invisible card).
    const root = rootRef.current;
    onCollapsedChange(false);
    cancelPendingLocateRef.current?.();
    cancelPendingLocateRef.current = waitForLaidOutCard(
      root ?? document.body,
      locateProviderId,
      (card) => {
        card.scrollIntoView({ behavior: 'smooth', block: 'center' });
        flashWhenArrived(card);
      },
      () => {
        explainLocateFailure(t('common.provider.locateMissing'));
      },
    );
  };

  // Re-collapsing ends a locate that is still waiting on the expand: the user
  // has said they do not want to see the list.
  React.useEffect(() => {
    if (collapsed) {
      cancelPendingLocateRef.current?.();
      cancelPendingLocateRef.current = null;
    }
  }, [collapsed]);

  React.useEffect(
    () => () => {
      cancelPendingLocateRef.current?.();
    },
    [],
  );

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
                      // rc-collapse's Panel handles keydown itself and toggles
                      // the section on Enter WITHOUT preventDefault, so the
                      // button's synthetic click still fires after it: one
                      // keypress would collapse the panel and then run locate
                      // against the panel it just closed. Ending the keydown
                      // here keeps the keyboard path equal to the pointer one.
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.stopPropagation();
                        }
                      }}
                    >
                      {t('common.provider.locate')}
                    </Button>
                  </Tooltip>
                )}
                {viewMode && onViewModeChange && (
                  // Sits with the other "the list itself" controls, ahead of
                  // 一键测试 / 通用配置 / 添加供应商: those do something *to* a
                  // provider, this one only changes how the providers are drawn.
                  //
                  // The glyph is antd's `LayoutOutlined` / `UnorderedListOutlined`
                  // pair, not lucide's `LayoutGrid`: a 2×2 of squares is exactly
                  // `AppstoreOutlined`, so the toggle read as a second 通用配置
                  // button. The pair also shares the toolbar's outlined stroke
                  // weight, which the lucide glyphs did not.
                  //
                  // It carries a word as well as a glyph. The label names the
                  // layout the click switches **to** — the icon and the tooltip
                  // say the same thing, because a lone layout glyph could as
                  // easily mean "you are in this layout" as "go to it".
                  //
                  // `data-provider-view-toggle` is the handle the browser
                  // fixture drives. It is kept even now that the button has
                  // text: the label is 卡片/列表, which the fixture would have to
                  // match against copy that is free to change.
                  <Tooltip
                    title={t(viewMode === 'list'
                      ? 'common.provider.viewMode.switchToCard'
                      : 'common.provider.viewMode.switchToList')}
                  >
                    <Button
                      type="link"
                      size="small"
                      style={toolbarButtonStyle}
                      data-provider-view-toggle={viewMode}
                      aria-label={t(viewMode === 'list'
                        ? 'common.provider.viewMode.switchToCard'
                        : 'common.provider.viewMode.switchToList')}
                      icon={viewMode === 'list' ? <LayoutOutlined /> : <UnorderedListOutlined />}
                      onClick={(event) => {
                        event.stopPropagation();
                        onViewModeChange(viewMode === 'list' ? 'card' : 'list');
                      }}
                    >
                      {t(viewMode === 'list'
                        ? 'common.provider.viewMode.card'
                        : 'common.provider.viewMode.list')}
                    </Button>
                  </Tooltip>
                )}
                {onBatchTest && (
                  // An empty target set still resolves: the click would flash a
                  // spinner and then clear every status badge without a result.
                  // The reason says why instead.
                  <Tooltip title={batchTestDisabled ? batchTestDisabledReason : undefined}>
                    <span>
                      <Button
                        type="link"
                        size="small"
                        style={toolbarButtonStyle}
                        icon={<ThunderboltOutlined />}
                        loading={batchTesting}
                        disabled={batchTestDisabled}
                        onClick={(event) => {
                          event.stopPropagation();
                          onBatchTest();
                        }}
                      >
                        {t('common.batchTest')}
                      </Button>
                    </span>
                  </Tooltip>
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
