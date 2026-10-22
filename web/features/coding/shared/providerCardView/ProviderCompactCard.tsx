import React from 'react';
import { Drawer, Dropdown, Tooltip } from 'antd';
import { HolderOutlined, MoreOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import { ManagementCheckbox } from '@/features/coding/shared/management';
import {
  resolveProviderAccentChrome,
} from '@/features/coding/shared/providerCardVariants/cardChrome';
import {
  renderProviderLaunchAction,
  renderProviderPrimaryAction,
  type ProviderCardLaunchAction,
} from '@/features/coding/shared/providerCardVariants/providerCardActions';
import {
  buildProviderCardMenuItems,
  dispatchProviderCardMenuClick,
} from '@/features/coding/shared/providerCardVariants/providerCardMenu';
import type {
  ProviderCardActions,
  ProviderCardModel,
  ProviderCardState,
} from '@/features/coding/shared/providerCardVariants/types';
import ProviderDetailPanel from './ProviderDetailPanel';
import type { ProviderDetailSection } from './ProviderDetailPanel';
import styles from './ProviderCompactCard.module.less';

/**
 * What a cell opens: the detail sidebar's own props, minus the way out.
 *
 * The shape of the sidebar — the drawer, its width, the panel — is *this*
 * component's job, so a caller hands in content and nothing else. That is why
 * `sections` is the only required part: what a provider has to say is the
 * CLI's business, that there is somewhere to say it is the grid's.
 */
export interface ProviderCellDetail {
  /** The panel's title — a `ProviderNameLink` for a provider, a heading for a member. */
  title: React.ReactNode;
  /** State tags on the title's line (applied / official / the account count). */
  statusTags?: React.ReactNode;
  /** The card's own action row, on the title's state line. */
  primaryActions?: React.ReactNode;
  /** The quieter row beneath it (test / edit / copy / share / delete). */
  secondaryActions?: React.ReactNode;
  sections: ProviderDetailSection[];
}

/**
 * The compact provider card used by the card-grid layout.
 *
 * ```
 * ⠿  Name                                            ⋮   ← ⋮ on hover
 *    Opus: claude-opus-4-8
 *    ─────────────────────────────────────────────────────
 *    CLI 启动                              网关代理  应用
 * ```
 *
 * Clicking the body — anywhere that is not itself a control — opens the detail
 * sidebar, which this component draws (`detail`); the grid cell states one model
 * and the two primary actions, and everything else about the provider lives in
 * there.
 *
 * ## Why this is not a fourth `providerCardVariants` style
 *
 * The three styles in `providerCardVariants/` are chosen by the **shape of a
 * CLI's provider model** (does it own a model catalog? does it have a single
 * active provider?). This card is chosen by **how dense the user wants the
 * list** — the same provider, the same CLI, rendered two ways. Mixing it into
 * the variants would make "which style" answerable by a display preference,
 * which is exactly the conflation that module exists to prevent.
 *
 * ## Why it reuses `ProviderCardState` / `ProviderCardActions`
 *
 * The mapping layer already computes both for the list card. Reusing them means
 * a provider's actions, tags and highlight cannot differ between the two
 * layouts — the caller passes one object to whichever card it renders. That is
 * the whole point: "the buttons behave the same in both views" is structural
 * here, not a thing to keep in sync by hand.
 *
 * ## What it owns
 *
 * Everything `CardShell` owns for the list styles: sortable registration, the
 * selection checkbox, `data-provider-id`, and the accent chrome. The guard in
 * `scripts/verify-provider-card-layout.mjs` scopes to `*ProviderCard.tsx` /
 * `*OfficialAccountCard.tsx` mapping layers; this file is a shared shell like
 * `CardShell`, which is why its name deliberately does not match that suffix.
 */
/**
 * Re-exported under the name the card view has always used, so callers and the
 * module barrel keep their import path while the shape itself lives beside the
 * renderer that draws it.
 */
export type ProviderCompactCardLaunchAction = ProviderCardLaunchAction;

interface ProviderCompactCardProps {
  /** Title line. */
  provider: ProviderCardModel;
  /**
   * The second line: every `Role: model` pair the CLI states, in order.
   *
   * A pair without a `label` renders as the bare value — for facts that name
   * themselves (`@ai-sdk/openai-compatible` is the npm specifier, not a value
   * of some role), where a `NPM 包:` prefix would only cost room on a one-line
   * row.
   *
   * It is one truncated line rather than a wrapped block — the card's height
   * has to stay uniform across the grid, and a provider with six bindings must
   * not make its row taller than its neighbours'. Omit it (or pass an empty
   * array) to render no second line at all.
   */
  subtitle?: { label?: string; value: string }[];
  /** Same object the list card receives. */
  providerState?: ProviderCardState;
  /**
   * Whether this cell is a locate target, i.e. `data-provider-id` is written.
   * Off for members that are not providers (ZCode's official-account cell
   * registers under a card id no locate query should ever answer to — with
   * the attribute present, "locate" on an applied account found a card the
   * page had deliberately excluded, then reported "nothing applied" while an
   * applied cell sat on screen).
   */
  locateAnchor?: boolean;
  /**
   * Stable test hook distinguishing non-provider members, since they carry no
   * `data-provider-id` to query. Rendered as `data-provider-kind`.
   */
  memberKind?: string;
  /** Same object the list card receives. */
  actions?: ProviderCardActions;
  /**
   * Rendered immediately **before** the title.
   *
   * `nameTags` covers everything after the name, but nothing can reach in front
   * of it, because the name comes from `provider.name`. Codex's official card
   * needs that spot: its title is the heading of an account block, and such a
   * heading reads `🔗 官方账号 (2)` — glyph before, count after.
   */
  namePrefix?: React.ReactNode;
  /**
   * State tags beside the title — 已应用 / 官方 / 代理.
   *
   * The same slot the list card takes, minus the endpoint: the mapping layer
   * builds the list from whichever facts its layout states, so "this provider
   * is applied" cannot be true in one layout and invisible in the other. The
   * border alone was doing that job here, and a coloured border does not say
   * *why* it is coloured.
   */
  nameTags?: React.ReactNode;
  /** Footer's left side. */
  launchAction?: ProviderCompactCardLaunchAction;
  /**
   * Where the card's own action row goes.
   *
   * - `'footer'` (default): its own bar below a hairline divider, the Skills
   *   card's shape. Suits a card that also carries a second line — Claude's
   *   role bindings, say — where the divider is what separates "what this
   *   provider is" from "what you can do to it".
   * - `'subtitle'`: on the second line, right-aligned. Suits a card that states
   *   a single model, where the footer bar was mostly empty space under one
   *   short line (user-directed, 2026-10-12).
   *
   * The actions themselves are the same nodes either way — the mapping layer
   * hands in one `actions` object, so the two placements cannot disagree about
   * what the card offers.
   */
  actionPlacement?: 'footer' | 'subtitle';
  /**
   * The cell's own control of this provider, on its action row, before the
   * primary action — the slot where the CLIs that *do* apply a provider show
   * 应用 / 网关代理.
   *
   * A CLI whose providers are never applied has no such action, and the row
   * would state nothing while every neighbouring cell states its control
   * (ZCode: `设为默认` lives on a model row, and the one per-provider control
   * is the enable switch the list card draws inline in its header). Passing it
   * here puts the same control in the same place on the cell — and takes it out
   * of the "more" menu, which is where it lives while a primary action is
   * beside it (a switch next to 应用 competes with it; that is the menu's whole
   * reason, see `providerCardMenu.tsx`).
   *
   * A slot rather than a flag: the card does not decide what a provider's state
   * control is, its CLI does.
   */
  cellControl?: React.ReactNode;
  /**
   * Extra entries for the "more" menu, inserted after 分享 and before the
   * divider. The card view folds the list card's inline 连通性测试 here.
   */
  extraMenuItems?: Parameters<typeof buildProviderCardMenuItems>[0]['extraItems'];
  /**
   * The cell's detail sidebar: what clicking the body opens, and what the cell
   * itself is too small to state.
   *
   * The cell states one model and carries the two primary actions; every other
   * fact about the provider (endpoint, all role bindings, notes, the remaining
   * actions) belongs in that sidebar. Passing this is therefore also what makes
   * the body clickable — the same "the card is the way in" convention the
   * Skills and MCP cards use.
   *
   * The caller supplies content only. The drawer, its width, the panel and the
   * open/close state are drawn here, because they were byte-identical in every
   * mapping layer that used to render them: four copies of one surface is how
   * four sidebars start drifting apart, and the width is the one thing a user
   * notices first (`min(60vw, 760px)` — the Skills and MCP drawers' own).
   */
  detail?: ProviderCellDetail;
}

const ProviderCompactCard: React.FC<ProviderCompactCardProps> = ({
  provider,
  subtitle,
  providerState,
  // Every provider cell is a locate target; only explicit non-provider
  // members (official-account managers) pass `false`.
  locateAnchor = true,
  memberKind,
  actions,
  namePrefix,
  nameTags,
  launchAction,
  actionPlacement = 'footer',
  cellControl,
  extraMenuItems,
  detail,
}) => {
  const { t } = useTranslation();
  const {
    isDisabled,
    onToggleDisabled,
    connectivityStatus,
    selectable = false,
    selected = false,
    onSelectChange,
    selectDisabledReason,
    dimmed = false,
    draggable = false,
    sortableId,
    accent,
  } = providerState ?? {};

  // `useSortable` must run on every render, so a card without an id still
  // registers — under a placeholder, disabled. Same rule as `CardShell`.
  //
  // `sortableId` doubles as the locate anchor below, which is why it is required
  // in practice: a cell registered under the placeholder would answer to an id
  // no list contains, and `handleDragEnd` drops what it cannot find.
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: sortableId ?? 'provider-compact-card',
    disabled: !draggable,
  });

  const isSelected = selectable && selected;
  const { borderColor, background } = resolveProviderAccentChrome(accent, isSelected);

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : dimmed ? 0.6 : 1,
    // Read by `ProviderListSection`'s locate ring, whose radius must follow the
    // card's own (10px here, 8px on the list cards) or the flash draws square
    // corners around a rounded card.
    '--locate-flash-radius': '10px',
  } as React.CSSProperties;

  // Selection mode takes over the left slot for every row, eligible or not —
  // see `CardShell` for why an ineligible row keeps a disabled checkbox.
  const inSelectionMode = selectable || Boolean(selectDisabledReason);
  const showHandle = draggable && !inSelectionMode;
  const primary = actions?.primaryAction;
  // Built once and placed by `actionPlacement`: the same nodes move between the
  // second line and the footer bar, so the two placements cannot end up offering
  // different actions.
  const actionRow = (
    <>
      {renderProviderLaunchAction(launchAction)}
      <span className={styles.footerActions}>
        {actions?.gatewayActions}
        {cellControl}
        {renderProviderPrimaryAction(primary)}
      </span>
    </>
  );
  const menuItems = buildProviderCardMenuItems({
    t,
    actions,
    isDisabled,
    // A cell that states the switch on its own row drops the menu's copy of it:
    // one control, one place on one card — the same rule that keeps the block's
    // 默认 mark and its 切换 button from both being on screen.
    onToggleDisabled: cellControl ? undefined : onToggleDisabled,
    extraItems: extraMenuItems,
  });

  const [detailOpen, setDetailOpen] = React.useState(false);
  const canOpenDetail = Boolean(detail) && !inSelectionMode;

  /**
   * The card body opens the detail sidebar; anything that is itself a control
   * handles its own click. Without this guard, dragging the grip, ticking the
   * checkbox or pressing a footer button would also open the sidebar.
   *
   * Batch-selection mode suppresses the body click entirely, the same way
   * `SkillCard` does: in that mode a click on the body means "select this
   * card", and covering the list with a drawer instead would make selecting a
   * run of cards impossible.
   */
  const handleCardClick: React.MouseEventHandler<HTMLDivElement> = (event) => {
    if (!canOpenDetail) {
      return;
    }
    const target = event.target as HTMLElement;
    // The drag handle is a control too, though it is a `<span>`: pressing it and
    // moving the card must not also open the sidebar it is sitting on.
    if (target.closest('button, a, input, [role="menuitem"], [role="checkbox"], [data-provider-card-handle]')) {
      return;
    }
    setDetailOpen(true);
  };

  return (
    // The ref is attached even when dragging is off: dnd-kit measures the node
    // to compute transforms, and an unregistered node cannot be dragged the
    // moment the parent re-enables it.
    <div
      ref={setNodeRef}
      data-provider-id={locateAnchor ? sortableId : undefined}
      data-provider-kind={memberKind}
      className={styles.cardOuter}
      style={style}
    >
      <div
        className={styles.card}
        style={{ borderColor, background }}
        data-provider-card-clickable={canOpenDetail ? '' : undefined}
        onClick={canOpenDetail ? handleCardClick : undefined}
      >
        <div className={styles.headerRow}>
          {inSelectionMode ? (
            // A `<span>` host: a disabled input does not emit the hover the
            // tooltip listens for.
            <Tooltip title={selectable ? undefined : selectDisabledReason}>
              <span style={{ display: 'inline-flex' }}>
                <ManagementCheckbox
                  checked={selected}
                  disabled={!selectable}
                  ariaLabel={selectable
                    ? t('common.batch.selectItem')
                    : (selectDisabledReason ?? t('common.batch.selectUnavailable'))}
                  onChange={onSelectChange ?? (() => {})}
                />
              </span>
            </Tooltip>
          ) : showHandle ? (
            <span
              {...attributes}
              {...listeners}
              className={styles.dragHandle}
              data-provider-card-handle
              title={t('common.provider.reorderHint')}
              aria-label={t('common.provider.reorderHint')}
            >
              <HolderOutlined style={{ fontSize: 14 }} />
            </span>
          ) : null}
          <ProviderConnectivityStatus item={connectivityStatus} />
          {namePrefix}
          <ProviderNameLink
            name={provider.name}
            baseUrl={provider.baseUrl}
            className={styles.name}
            style={{ fontSize: 14, fontWeight: 600 }}
          />
          {nameTags && <span className={styles.nameTags} data-provider-card-tags>{nameTags}</span>}
          {menuItems.length > 0 && (
            // Hover-revealed. Opacity only: the button keeps its box at all
            // times, so the title cannot reflow as the pointer crosses the card.
            <span className={styles.hoverActions} data-provider-card-actions>
              <Dropdown
                trigger={['click']}
                menu={{
                  items: menuItems,
                  onClick: ({ key }) => dispatchProviderCardMenuClick(key, actions),
                }}
              >
                <button type="button" className={styles.miniBtn} title={t('common.more')} aria-label={t('common.more')}>
                  <MoreOutlined />
                </button>
              </Dropdown>
            </span>
          )}
        </div>

        {/* The second line and, with `actionPlacement="subtitle"`, the action
            row on it. Both placements are `data-provider-card-footer`, because
            that is the anchor for "this card's own actions" rather than for the
            bar they sit in — a check should not have to know which placement the
            CLI chose. */}
        <div
          className={[
            styles.body,
            actionPlacement === 'subtitle' ? styles.bodyWithAction : undefined,
          ].filter(Boolean).join(' ')}
        >
          <div className={styles.subtitleRow}>
            {subtitle && subtitle.length > 0 && (
              // One line of `Label: value` pairs, the values monospaced — the
              // same shape the list card's binding line uses, so a role is
              // readable at a glance instead of being an unexplained model name.
              //
              // The pairs are inline spans, not flex items: a flex row would clip
              // at the last whole item and show no ellipsis, while inline content
              // lets the browser place one at the truncation point. The `title`
              // carries the full line, which is the only way to read what the
              // ellipsis hid.
              <div
                className={styles.subtitle}
                data-provider-subtitle
                title={subtitle
                  .map((entry) => (entry.label ? `${entry.label}: ${entry.value}` : entry.value))
                  .join('  ')}
              >
                {subtitle.map((entry, index) => (
                  <span key={`${entry.label ?? ''}:${entry.value}:${index}`} className={styles.subtitleEntry}>
                    {entry.label && (
                      <span className={styles.subtitleLabel}>{entry.label}:</span>
                    )}
                    <span className={styles.subtitleValue}>{entry.value}</span>
                  </span>
                ))}
              </div>
            )}
            {actionPlacement === 'subtitle' && (
              <div className={styles.subtitleActions} data-provider-card-footer>
                {actionRow}
              </div>
            )}
          </div>
        </div>

        {actionPlacement === 'footer' && (
          <div className={styles.footerRow} data-provider-card-footer>
            {actionRow}
          </div>
        )}
      </div>

      {/* The cell's own sidebar, drawn here rather than by each caller.
          `destroyOnHidden` unmounts it on close, which is what the fixtures'
          "wait for it to be gone" steps rely on; `closable={false}` because the
          panel carries its own close button on the title line. */}
      {detail && (
        <Drawer
          placement="right"
          // `size`, not the deprecated `width`/`height`: antd 6 replaced them
          // with one CSS length, and the Skills and MCP detail drawers' own
          // `min(60vw, 760px)` carries over verbatim — the provider sidebar is
          // the same kind of surface, and three of them at three widths read as
          // three different products.
          size="min(60vw, 760px)"
          open={detailOpen}
          onClose={() => setDetailOpen(false)}
          destroyOnHidden
          closable={false}
          styles={{ body: { padding: 0, display: 'flex', flexDirection: 'column' } }}
        >
          <ProviderDetailPanel
            {...detail}
            onClose={() => setDetailOpen(false)}
          />
        </Drawer>
      )}
    </div>
  );
};

export default ProviderCompactCard;
