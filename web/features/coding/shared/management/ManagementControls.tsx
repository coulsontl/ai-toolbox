import React from 'react';
import { createPortal } from 'react-dom';
import {
  Check,
  Circle,
  CircleDot,
  Search,
  Square,
  SquareCheck,
  X,
} from 'lucide-react';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import styles from './ManagementControls.module.less';

type ButtonVariant = 'default' | 'subtle' | 'ghost' | 'primary' | 'danger';
type ControlSize = 'default' | 'compact';

const MENU_VIEWPORT_MARGIN = 12;
const MENU_TRIGGER_GAP = 6;
const MENU_FALLBACK_WIDTH = 168;

const clampMenuCoordinate = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), Math.max(min, max));

interface ManagementButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  icon?: React.ReactNode;
  variant?: ButtonVariant;
  controlSize?: ControlSize;
}

const buttonVariantClass: Record<ButtonVariant, string> = {
  default: styles.buttonDefault,
  subtle: styles.buttonSubtle,
  ghost: styles.buttonGhost,
  primary: styles.buttonPrimary,
  danger: styles.buttonDanger,
};

export const ManagementButton: React.FC<ManagementButtonProps> = ({
  icon,
  variant = 'default',
  controlSize = 'default',
  className,
  children,
  ...buttonProps
}) => (
  <button
    {...buttonProps}
    type={buttonProps.type ?? 'button'}
    className={[
      styles.button,
      buttonVariantClass[variant],
      controlSize === 'compact' ? styles.buttonCompact : '',
      className ?? '',
    ].filter(Boolean).join(' ')}
  >
    {icon}
    {children}
  </button>
);

interface ManagementIconButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  icon: React.ReactNode;
  danger?: boolean;
  controlSize?: ControlSize;
}

export const ManagementIconButton: React.FC<ManagementIconButtonProps> = ({
  icon,
  danger,
  controlSize = 'default',
  className,
  ...buttonProps
}) => (
  <button
    {...buttonProps}
    type={buttonProps.type ?? 'button'}
    aria-label={buttonProps['aria-label'] || buttonProps.title}
    className={[
      styles.iconButton,
      controlSize === 'compact' ? styles.iconButtonCompact : '',
      danger ? styles.iconButtonDanger : '',
      className ?? '',
    ].filter(Boolean).join(' ')}
  >
    {icon}
  </button>
);

interface ManagementSearchInputProps {
  value: string;
  placeholder: string;
  clearLabel: string;
  onChange: (value: string) => void;
  className?: string;
  ariaLabel?: string;
}

export const ManagementSearchInput: React.FC<ManagementSearchInputProps> = ({
  value,
  placeholder,
  clearLabel,
  onChange,
  className,
  ariaLabel,
}) => (
  <label
    className={[
      styles.searchShell,
      value ? styles.searchShellWithClear : '',
      className ?? '',
    ].filter(Boolean).join(' ')}
  >
    <Search size={15} aria-hidden="true" />
    <input
      className={styles.searchInput}
      value={value}
      placeholder={placeholder}
      aria-label={ariaLabel ?? placeholder}
      onChange={(event) => onChange(event.target.value)}
    />
    {value ? (
      <button
        type="button"
        className={styles.searchClearButton}
        aria-label={clearLabel}
        onClick={() => onChange('')}
      >
        <X size={14} aria-hidden="true" />
      </button>
    ) : null}
  </label>
);

export interface ManagementSegmentedOption<TValue extends string> {
  value: TValue;
  label: React.ReactNode;
  icon?: React.ReactNode;
  disabled?: boolean;
  title?: string;
}

interface ManagementSegmentedProps<TValue extends string> {
  value: TValue;
  options: ManagementSegmentedOption<TValue>[];
  onChange: (value: TValue) => void;
  ariaLabel: string;
  className?: string;
  disabled?: boolean;
  title?: string;
}

export function ManagementSegmented<TValue extends string>({
  value,
  options,
  onChange,
  ariaLabel,
  className,
  disabled,
  title,
}: ManagementSegmentedProps<TValue>) {
  return (
    <div
      className={[styles.segmented, className ?? ''].filter(Boolean).join(' ')}
      role="radiogroup"
      aria-label={ariaLabel}
      aria-disabled={disabled || undefined}
      title={title}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={value === option.value}
          title={option.title}
          disabled={disabled || option.disabled}
          className={[
            styles.segmentedButton,
            value === option.value ? styles.segmentedButtonActive : '',
          ].filter(Boolean).join(' ')}
          onClick={() => {
            if (value !== option.value) {
              onChange(option.value);
            }
          }}
        >
          {option.icon}
          {option.label}
        </button>
      ))}
    </div>
  );
}

type ManagementMenuItemKind = 'checkbox' | 'radio';

interface ManagementMenuSectionItem {
  key: string;
  type: 'section';
  label: React.ReactNode;
}

interface ManagementMenuActionItem {
  key: string;
  type?: 'item';
  label: React.ReactNode;
  tooltip?: React.ReactNode;
  icon?: React.ReactNode;
  active?: boolean;
  kind?: ManagementMenuItemKind;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

export type ManagementMenuItem = ManagementMenuActionItem | ManagementMenuSectionItem;

interface ManagementMenuProps {
  items: ManagementMenuItem[];
  children: React.ReactNode;
  title?: string;
  disabled?: boolean;
  align?: 'start' | 'end';
  controlSize?: ControlSize;
  triggerClassName?: string;
}

export const ManagementMenu: React.FC<ManagementMenuProps> = ({
  items,
  children,
  title,
  disabled,
  align = 'end',
  controlSize = 'default',
  triggerClassName,
}) => {
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);
  const menuRef = React.useRef<HTMLDivElement | null>(null);
  const tooltipIdPrefix = React.useId();
  const [open, setOpen] = React.useState(false);
  const [position, setPosition] = React.useState({ top: 0, left: 0 });

  const closeMenu = React.useCallback(() => setOpen(false), []);

  const updatePosition = React.useCallback(() => {
    const triggerElement = triggerRef.current;
    if (!triggerElement) {
      return;
    }
    const rect = triggerElement.getBoundingClientRect();
    const menuElement = menuRef.current;
    const menuWidth = menuElement?.offsetWidth ?? MENU_FALLBACK_WIDTH;
    const menuHeight = menuElement?.offsetHeight ?? 0;
    const preferredLeft = align === 'end' ? rect.right - menuWidth : rect.left;
    const nextPosition = {
      top: clampMenuCoordinate(
        rect.bottom + MENU_TRIGGER_GAP,
        MENU_VIEWPORT_MARGIN,
        window.innerHeight - menuHeight - MENU_VIEWPORT_MARGIN,
      ),
      left: clampMenuCoordinate(
        preferredLeft,
        MENU_VIEWPORT_MARGIN,
        window.innerWidth - menuWidth - MENU_VIEWPORT_MARGIN,
      ),
    };
    setPosition((previousPosition) => {
      if (
        previousPosition.top === nextPosition.top &&
        previousPosition.left === nextPosition.left
      ) {
        return previousPosition;
      }
      return nextPosition;
    });
  }, [align]);

  React.useLayoutEffect(() => {
    if (!open) {
      return;
    }
    updatePosition();
  }, [items.length, open, updatePosition]);

  React.useEffect(() => {
    if (!open) {
      return undefined;
    }

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) {
        return;
      }
      closeMenu();
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        closeMenu();
      }
    };

    window.addEventListener('pointerdown', handlePointerDown, true);
    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('resize', updatePosition);
    window.addEventListener('scroll', updatePosition, true);

    return () => {
      window.removeEventListener('pointerdown', handlePointerDown, true);
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('resize', updatePosition);
      window.removeEventListener('scroll', updatePosition, true);
    };
  }, [closeMenu, open, updatePosition]);

  const renderMenuIndicator = (item: ManagementMenuActionItem) => {
    if (item.kind === 'checkbox') {
      return item.active
        ? <SquareCheck size={14} aria-hidden="true" />
        : <Square size={14} aria-hidden="true" />;
    }

    if (item.kind === 'radio') {
      return item.active
        ? <CircleDot size={14} aria-hidden="true" />
        : <Circle size={14} aria-hidden="true" />;
    }

    if (item.active) {
      return <Check size={14} aria-hidden="true" />;
    }

    return item.icon ?? null;
  };

  return (
    <span className={styles.menuHost}>
      <button
        ref={triggerRef}
        type="button"
        title={title}
        aria-label={title}
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled || items.length === 0}
        className={[
          styles.menuTrigger,
          controlSize === 'compact' ? styles.menuTriggerCompact : '',
          triggerClassName ?? '',
        ].filter(Boolean).join(' ')}
        onClick={() => {
          updatePosition();
          setOpen((previousOpen) => !previousOpen);
        }}
      >
        {children}
      </button>
      {open && createPortal(
        <div
          ref={menuRef}
          className={[
            styles.menu,
            align === 'end' ? styles.menuAlignEnd : '',
          ].filter(Boolean).join(' ')}
          role="menu"
          style={{ top: position.top, left: position.left }}
        >
          {items.map((item) => {
            if (item.type === 'section') {
              return (
                <div key={item.key} className={styles.menuSection} role="presentation">
                  {item.label}
                </div>
              );
            }

            const indicator = renderMenuIndicator(item);
            const tooltipId = item.tooltip ? `${tooltipIdPrefix}-${item.key}-tooltip` : undefined;
            const menuItemRole = item.kind === 'checkbox'
              ? 'menuitemcheckbox'
              : item.kind === 'radio'
                ? 'menuitemradio'
                : 'menuitem';

            return (
              <button
                key={item.key}
                type="button"
                role={menuItemRole}
                aria-checked={item.kind === 'checkbox' || item.kind === 'radio' ? item.active : undefined}
                aria-describedby={tooltipId}
                disabled={item.disabled}
                className={[
                  styles.menuItem,
                  item.active ? styles.menuItemActive : '',
                  item.danger ? styles.menuItemDanger : '',
                ].filter(Boolean).join(' ')}
                onClick={() => {
                  if (item.disabled) {
                    return;
                  }
                  closeMenu();
                  item.onSelect();
                }}
              >
                {indicator && (
                  <span className={styles.menuItemIcon}>
                    {indicator}
                  </span>
                )}
                <span>{item.label}</span>
                {item.tooltip && (
                  <span id={tooltipId} className={styles.menuItemTooltip} role="tooltip">
                    {item.tooltip}
                  </span>
                )}
              </button>
            );
          })}
        </div>,
        document.body,
      )}
    </span>
  );
};

interface ManagementCheckboxProps {
  checked: boolean;
  indeterminate?: boolean;
  disabled?: boolean;
  ariaLabel: string;
  onChange: (checked: boolean) => void;
  onClick?: (event: React.MouseEvent<HTMLInputElement>) => void;
}

export const ManagementCheckbox: React.FC<ManagementCheckboxProps> = ({
  checked,
  indeterminate,
  disabled,
  ariaLabel,
  onChange,
  onClick,
}) => {
  const inputRef = React.useRef<HTMLInputElement | null>(null);

  React.useEffect(() => {
    if (inputRef.current) {
      inputRef.current.indeterminate = !!indeterminate;
    }
  }, [indeterminate]);

  return (
    <input
      ref={inputRef}
      type="checkbox"
      className={styles.checkbox}
      checked={checked}
      disabled={disabled}
      aria-label={ariaLabel}
      onClick={onClick}
      onChange={(event) => onChange(event.target.checked)}
    />
  );
};

export const ManagementEmpty: React.FC<{ description: React.ReactNode }> = ({ description }) => (
  <div className={styles.empty}>{description}</div>
);

export const ManagementLoading: React.FC<{ label?: React.ReactNode }> = ({ label }) => (
  <div className={styles.loading}>
    <span className={styles.spinner} aria-hidden="true" />
    {label}
  </div>
);

interface VirtualGridProps<TItem> {
  items: TItem[];
  getKey: (item: TItem) => React.Key;
  renderItem: (item: TItem) => React.ReactNode;
  columns?: number;
  virtualize?: boolean;
  minColumnWidth?: number;
  maxColumns?: number;
  rowGap?: number;
  defaultRowHeight?: number;
  overscanRows?: number;
  className?: string;
}

const DEFAULT_ROW_HEIGHT = 126;
const DEFAULT_ROW_GAP = 10;
const DEFAULT_OVERSCAN_ROWS = 4;
const clampVirtualGridColumns = (columns: number) => Math.min(5, Math.max(1, columns));

export function VirtualGrid<TItem>({
  items,
  getKey,
  renderItem,
  columns,
  virtualize = true,
  minColumnWidth = 430,
  maxColumns = 2,
  rowGap = DEFAULT_ROW_GAP,
  defaultRowHeight = DEFAULT_ROW_HEIGHT,
  overscanRows = DEFAULT_OVERSCAN_ROWS,
  className,
}: VirtualGridProps<TItem>) {
  const { isActive } = useKeepAlive();
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const rowObserverMapRef = React.useRef<Map<number, ResizeObserver>>(new Map());
  const [viewportHeight, setViewportHeight] = React.useState(720);
  const [scrollTop, setScrollTop] = React.useState(0);
  const [listOffsetTop, setListOffsetTop] = React.useState(0);
  const [columnCount, setColumnCount] = React.useState(() =>
    clampVirtualGridColumns(columns ?? 1),
  );
  const [rowHeights, setRowHeights] = React.useState<Record<number, number>>({});

  const gridStyle = React.useMemo(() => ({
    '--management-grid-columns': `repeat(${columnCount}, minmax(0, 1fr))`,
    '--management-grid-gap': `${rowGap}px`,
  }) as React.CSSProperties, [columnCount, rowGap]);

  React.useLayoutEffect(() => {
    if (!isActive) {
      return undefined;
    }

    const containerElement = containerRef.current;
    if (!(containerElement instanceof HTMLElement)) {
      return undefined;
    }

    const scrollElement = containerElement.closest('main');
    let frameId = 0;
    const updateMetrics = () => {
      window.cancelAnimationFrame(frameId);
      frameId = window.requestAnimationFrame(() => {
        const nextColumnCount = Math.max(
          1,
          columns === undefined
            ? Math.min(maxColumns, Math.floor((containerElement.clientWidth + rowGap) / minColumnWidth))
            : clampVirtualGridColumns(columns),
        );
        setColumnCount(nextColumnCount);

        if (!virtualize || !(scrollElement instanceof HTMLElement)) {
          return;
        }

        const containerRect = containerElement.getBoundingClientRect();
        const scrollRect = scrollElement.getBoundingClientRect();
        setViewportHeight(scrollElement.clientHeight);
        setScrollTop(scrollElement.scrollTop);
        setListOffsetTop(containerRect.top - scrollRect.top + scrollElement.scrollTop);
      });
    };

    updateMetrics();
    const resizeObserver = new ResizeObserver(updateMetrics);
    resizeObserver.observe(containerElement);
    if (virtualize && scrollElement instanceof HTMLElement) {
      scrollElement.addEventListener('scroll', updateMetrics, { passive: true });
    }
    window.addEventListener('resize', updateMetrics);

    return () => {
      window.cancelAnimationFrame(frameId);
      resizeObserver.disconnect();
      if (virtualize && scrollElement instanceof HTMLElement) {
        scrollElement.removeEventListener('scroll', updateMetrics);
      }
      window.removeEventListener('resize', updateMetrics);
    };
  }, [columns, isActive, maxColumns, minColumnWidth, rowGap, virtualize, items.length]);

  React.useEffect(() => {
    setRowHeights({});
    for (const observer of rowObserverMapRef.current.values()) {
      observer.disconnect();
    }
    rowObserverMapRef.current.clear();
  }, [columnCount, items]);

  React.useEffect(() => () => {
    for (const observer of rowObserverMapRef.current.values()) {
      observer.disconnect();
    }
    rowObserverMapRef.current.clear();
  }, []);

  const updateMeasuredRowHeight = React.useCallback((rowIndex: number, rowHeight: number) => {
    setRowHeights((previousHeights) => {
      if (previousHeights[rowIndex] === rowHeight) {
        return previousHeights;
      }
      return { ...previousHeights, [rowIndex]: rowHeight };
    });
  }, []);

  const bindVirtualRowRef = React.useCallback(
    (rowIndex: number) => (node: HTMLDivElement | null) => {
      const previousObserver = rowObserverMapRef.current.get(rowIndex);
      if (previousObserver) {
        previousObserver.disconnect();
        rowObserverMapRef.current.delete(rowIndex);
      }

      if (!node) {
        return;
      }

      const measureRowHeight = () => updateMeasuredRowHeight(rowIndex, node.offsetHeight);
      measureRowHeight();

      const resizeObserver = new ResizeObserver(measureRowHeight);
      resizeObserver.observe(node);
      rowObserverMapRef.current.set(rowIndex, resizeObserver);
    },
    [updateMeasuredRowHeight],
  );

  const virtualizedRows = React.useMemo(() => {
    const safeColumnCount = Math.max(1, columnCount);
    const totalRows = Math.ceil(items.length / safeColumnCount);
    const estimatedRowHeight = defaultRowHeight + rowGap;
    const rowOffsets: number[] = [];
    let totalHeight = 0;

    for (let rowIndex = 0; rowIndex < totalRows; rowIndex += 1) {
      rowOffsets[rowIndex] = totalHeight;
      totalHeight += (rowHeights[rowIndex] ?? defaultRowHeight) + rowGap;
    }

    const viewportStart = Math.max(0, scrollTop - estimatedRowHeight * overscanRows);
    const viewportEnd = scrollTop + viewportHeight + estimatedRowHeight * overscanRows;
    const localViewportStart = Math.max(0, viewportStart - listOffsetTop);
    const localViewportEnd = Math.max(0, viewportEnd - listOffsetTop);

    let startRow = 0;
    while (startRow < totalRows) {
      const rowBottom = rowOffsets[startRow] + (rowHeights[startRow] ?? defaultRowHeight);
      if (rowBottom >= localViewportStart) {
        break;
      }
      startRow += 1;
    }

    let endRow = startRow;
    while (endRow < totalRows && rowOffsets[endRow] <= localViewportEnd) {
      endRow += 1;
    }

    const rows = [];
    for (let rowIndex = startRow; rowIndex < endRow; rowIndex += 1) {
      const rowStartIndex = rowIndex * safeColumnCount;
      rows.push({
        rowIndex,
        top: rowOffsets[rowIndex] ?? 0,
        items: items.slice(rowStartIndex, rowStartIndex + safeColumnCount),
      });
    }

    return {
      rows,
      totalHeight: Math.max(0, totalHeight - rowGap),
      safeColumnCount,
    };
  }, [
    columnCount,
    defaultRowHeight,
    items,
    listOffsetTop,
    overscanRows,
    rowGap,
    rowHeights,
    scrollTop,
    viewportHeight,
  ]);

  if (!virtualize) {
    return (
      <div
        ref={containerRef}
        className={[styles.grid, className ?? ''].filter(Boolean).join(' ')}
        style={gridStyle}
      >
        {items.map((item) => (
          <React.Fragment key={getKey(item)}>{renderItem(item)}</React.Fragment>
        ))}
      </div>
    );
  }

  return (
    <div ref={containerRef} className={styles.virtualGridShell}>
      <div
        className={styles.virtualViewport}
        style={{ height: virtualizedRows.totalHeight }}
      >
        {virtualizedRows.rows.map((row) => (
          <div
            key={`row-${row.rowIndex}`}
            ref={bindVirtualRowRef(row.rowIndex)}
            className={[styles.virtualRow, className ?? ''].filter(Boolean).join(' ')}
            style={{ ...gridStyle, top: row.top }}
          >
            {row.items.map((item) => (
              <React.Fragment key={getKey(item)}>{renderItem(item)}</React.Fragment>
            ))}
            {row.items.length < virtualizedRows.safeColumnCount
              ? Array.from({ length: virtualizedRows.safeColumnCount - row.items.length }).map((_, fillerIndex) => (
                  <div key={`row-${row.rowIndex}-filler-${fillerIndex}`} className={styles.virtualFiller} />
                ))
              : null}
          </div>
        ))}
      </div>
    </div>
  );
}
