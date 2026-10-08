import React from 'react';
import { Card } from 'antd';
import { HolderOutlined } from '@ant-design/icons';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useTranslation } from 'react-i18next';
import { ManagementCheckbox } from '@/features/coding/shared/management';

interface CardShellProps {
  /** Sortable id; without one the handle is omitted and the card is pinned. */
  sortableId?: string;
  draggable?: boolean;
  /** Renders a checkbox instead of the drag handle. */
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (checked: boolean) => void;
  /** Renders the whole card at reduced opacity. */
  dimmed?: boolean;
  /**
   * Chrome accent, mirroring the four bespoke cards this shell replaced:
   *
   * - `applied` — this is the provider currently in effect (primary border,
   *   selected background).
   * - `gatewayPrimary` — this is the failover P0 provider (success border,
   *   success-tinted gradient). Wins over `applied`: a provider can be both,
   *   and the gateway role is the more specific fact.
   *
   * Batch selection wins over both — the user is acting on the selection, and a
   * selected card that does not look selected is the worse failure.
   */
  accent?: 'applied' | 'gatewayPrimary';
  /** The card body. */
  children: React.ReactNode;
}

/**
 * The frame every provider card style shares: drag registration, the handle or
 * selection checkbox, and the card chrome.
 *
 * Kept as one component because these three are the parts that must stay
 * identical across styles — a card that forgets `setNodeRef` cannot be dragged,
 * and one that reimplements the handle drifts in cursor and hit area. The
 * *content* layout is what the three styles differ on, and that is left to
 * them.
 */
/**
 * The drag handle's own affordances: the grip is a small, low-contrast target,
 * so it lights up and gains a fill only while the pointer is over it. Without
 * that feedback the handle is hard to find and gives no sign it is grabbed.
 *
 * Kimi's card carried this (`.dragHandle:hover`); the other bespoke cards did
 * not. It belongs here rather than in the caller: it describes how the handle
 * behaves, which is `CardShell`'s job, and every consumer gets it at once.
 */
const dragHandleStyle: React.CSSProperties = {
  cursor: 'grab',
  padding: '4px 2px',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  color: 'var(--color-text-tertiary)',
  borderRadius: 4,
  flexShrink: 0,
  touchAction: 'none',
  transition: 'color 0.2s ease, background-color 0.2s ease',
};

const CardShell: React.FC<CardShellProps> = ({
  sortableId,
  draggable = false,
  selectable = false,
  selected = false,
  onSelectChange,
  dimmed = false,
  accent,
  children,
}) => {
  const { t } = useTranslation();

  // `useSortable` must run on every render, so a card rendered without an id
  // still registers — under a placeholder, disabled.
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: sortableId ?? 'provider-card',
    disabled: !draggable,
  });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : dimmed ? 0.6 : 1,
  };

  const showHandle = draggable && !selectable;

  const isSelected = selectable && selected;
  const borderColor = isSelected || accent === 'applied'
    ? 'var(--ant-color-primary)'
    : accent === 'gatewayPrimary'
      ? 'var(--color-status-success)'
      : 'var(--color-border-card)';
  const background = isSelected
    ? undefined
    : accent === 'gatewayPrimary'
      ? 'linear-gradient(135deg, color-mix(in srgb, var(--color-status-success) 12%, var(--color-bg-container)), var(--color-bg-container))'
      : accent === 'applied'
        ? 'var(--color-bg-selected)'
        : undefined;

  // The ref is attached even when dragging is off: dnd-kit measures the node to
  // compute transforms, and an unregistered node cannot be dragged the moment
  // the parent re-enables it.
  return (
    <div ref={setNodeRef} style={style}>
      {/* Bottom margin on the Card, not the wrapper: each card spaces itself so
          the gap survives a reorder among its siblings. */}
      <Card
        style={{
          marginBottom: 12,
          borderColor,
          background,
          boxShadow: 'var(--shadow-card-sm)',
          transition: 'box-shadow 0.16s ease',
        }}
        /* 16, matching every provider card that has not migrated yet
           (claudedesktop / geminicli / grok) and what claudecode and codex used
           before they moved onto this shell. The shell had inherited 8/12 from
           `components/common/ProviderCard` — a different, older card — so every
           migrated tab lost 8px of vertical padding and 4px horizontal without
           anything saying so (2026-10-08 report: "the new shared provider
           card's vertical margins are smaller than before"). */
        styles={{ body: { padding: 16 } }}
        onMouseEnter={(event) => {
          event.currentTarget.style.boxShadow = 'var(--shadow-card-sm-hover)';
        }}
        onMouseLeave={(event) => {
          event.currentTarget.style.boxShadow = 'var(--shadow-card-sm)';
        }}
      >
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
          {selectable ? (
            <div style={{ display: 'flex', alignItems: 'center', padding: '4px 0' }}>
              <ManagementCheckbox
                checked={selected}
                ariaLabel={t('common.batch.selectItem')}
                onChange={onSelectChange ?? (() => {})}
              />
            </div>
          ) : showHandle ? (
            <div
              {...attributes}
              {...listeners}
              style={{
                ...dragHandleStyle,
                cursor: isDragging ? 'grabbing' : 'grab',
                color: isDragging ? 'var(--color-text)' : dragHandleStyle.color,
                background: isDragging ? 'var(--color-fill-secondary)' : undefined,
              }}
              onMouseEnter={(event) => {
                event.currentTarget.style.color = 'var(--color-text)';
                event.currentTarget.style.background = 'var(--color-fill-secondary)';
              }}
              onMouseLeave={(event) => {
                event.currentTarget.style.color = 'var(--color-text-tertiary)';
                event.currentTarget.style.background = '';
              }}
            >
              <HolderOutlined style={{ fontSize: 16 }} />
            </div>
          ) : null}

          <div style={{ flex: 1, minWidth: 0 }}>{children}</div>
        </div>
      </Card>
    </div>
  );
};

export default CardShell;
