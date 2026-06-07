import React from 'react';
import {
  ListTree,
  PanelRight,
  PanelRightClose,
  Search,
  User,
  Wrench,
  X,
} from 'lucide-react';
import type { TFunction } from 'i18next';

import type { SessionNavigatorEntry } from './domain/messageNavigator';
import styles from './SessionDetailWorkbench.module.less';

interface SessionMessageNavigatorProps {
  entries: SessionNavigatorEntry[];
  activeMessageIndex: number | null;
  collapsed?: boolean;
  t: TFunction;
  onSelect: (index: number) => void;
  onToggleCollapse?: () => void;
}

const SessionMessageNavigator: React.FC<SessionMessageNavigatorProps> = ({
  entries,
  activeMessageIndex,
  collapsed = false,
  t,
  onSelect,
  onToggleCollapse,
}) => {
  const [filterText, setFilterText] = React.useState('');
  const [userOnly, setUserOnly] = React.useState(false);

  const filteredEntries = React.useMemo(() => {
    const normalizedFilter = filterText.trim().toLowerCase();
    return entries.filter((entry) => {
      if (userOnly && entry.role.toLowerCase() !== 'user') {
        return false;
      }
      if (!normalizedFilter) {
        return true;
      }
      const haystack = [
        entry.label,
        entry.preview,
        entry.role,
        entry.kind,
        formatNavigatorTime(entry.ts),
      ].join('\n').toLowerCase();
      return haystack.includes(normalizedFilter);
    });
  }, [entries, filterText, userOnly]);

  if (collapsed) {
    return (
      <aside
        className={`${styles.navigatorPane} ${styles.navigatorPaneCollapsed}`}
        aria-label={t('sessionManager.messageNavigatorTitle')}
      >
        <div className={styles.navigatorCollapsedInner}>
          <button
            type="button"
            className={styles.navigatorIconButton}
            aria-label={t('sessionManager.navigatorExpand')}
            title={t('sessionManager.navigatorExpand')}
            onClick={onToggleCollapse}
          >
            <PanelRight size={16} aria-hidden="true" />
          </button>
          <div className={styles.navigatorCollapsedDivider} />
          <ListTree size={15} aria-hidden="true" />
          <span className={styles.navigatorCollapsedCount}>{entries.length}</span>
        </div>
      </aside>
    );
  }

  const emptyText = entries.length === 0
    ? t('sessionManager.navigatorNoMessages')
    : t('sessionManager.navigatorNoMatches');

  return (
    <aside className={styles.navigatorPane} aria-label={t('sessionManager.messageNavigatorTitle')}>
      <div className={styles.navigatorHeader}>
        <ListTree size={15} aria-hidden="true" />
        <span className={styles.navigatorTitle}>{t('sessionManager.messageNavigatorTitle')}</span>
        <span className={styles.navigatorCount}>{filteredEntries.length}</span>
        <button
          type="button"
          className={`${styles.navigatorIconButton}${userOnly ? ` ${styles.navigatorIconButtonActive}` : ''}`}
          aria-label={t('sessionManager.navigatorUserOnly')}
          aria-pressed={userOnly}
          title={t('sessionManager.navigatorUserOnly')}
          onClick={() => setUserOnly((current) => !current)}
        >
          <User size={15} aria-hidden="true" />
        </button>
        {onToggleCollapse ? (
          <button
            type="button"
            className={styles.navigatorIconButton}
            aria-label={t('sessionManager.navigatorCollapse')}
            title={t('sessionManager.navigatorCollapse')}
            onClick={onToggleCollapse}
          >
            <PanelRightClose size={15} aria-hidden="true" />
          </button>
        ) : null}
      </div>

      <div className={styles.navigatorFilterBar}>
        <div className={styles.navigatorFilterInputShell}>
          <Search size={15} aria-hidden="true" />
          <input
            type="text"
            value={filterText}
            placeholder={t('sessionManager.navigatorFilterPlaceholder')}
            aria-label={t('sessionManager.navigatorFilterPlaceholder')}
            onChange={(event) => setFilterText(event.target.value)}
          />
          {filterText ? (
            <button
              type="button"
              className={styles.navigatorClearButton}
              aria-label={t('common.clear')}
              title={t('common.clear')}
              onClick={() => setFilterText('')}
            >
              <X size={12} aria-hidden="true" />
            </button>
          ) : null}
        </div>
      </div>

      <div className={styles.navigatorList} role="listbox" aria-label={t('sessionManager.messageNavigatorTitle')}>
        {filteredEntries.length === 0 ? (
          <div className={styles.navigatorEmpty}>{emptyText}</div>
        ) : filteredEntries.map((entry) => (
          <NavigatorEntryButton
            key={entry.id}
            entry={entry}
            active={activeMessageIndex === entry.messageIndex}
            t={t}
            onSelect={onSelect}
          />
        ))}
      </div>
    </aside>
  );
};

interface NavigatorEntryButtonProps {
  entry: SessionNavigatorEntry;
  active: boolean;
  t: TFunction;
  onSelect: (index: number) => void;
}

const NavigatorEntryButton: React.FC<NavigatorEntryButtonProps> = ({
  entry,
  active,
  t,
  onSelect,
}) => {
  const preview = entry.preview || entry.label;
  const roleClassName = getNavigatorRoleClassName(entry);
  return (
    <button
      type="button"
      className={[
        styles.navigatorItem,
        roleClassName,
        active ? styles.navigatorItemActive : '',
      ].filter(Boolean).join(' ')}
      aria-label={t('sessionManager.navigatorEntryLabel', {
        turnIndex: entry.turnIndex,
        preview,
      })}
      aria-selected={active}
      role="option"
      onClick={() => onSelect(entry.messageIndex)}
    >
      <span className={styles.navigatorItemTop}>
        <span className={styles.navigatorItemIdentity}>
          <span className={styles.navigatorDot} />
          <span className={styles.navigatorTurnIndex}>#{entry.turnIndex}</span>
          {entry.hasToolUse ? <Wrench size={12} aria-hidden="true" /> : null}
          {entry.matchCount > 0 ? (
            <span className={styles.navigatorMatchBadge}>
              <Search size={11} aria-hidden="true" />
              {entry.matchCount}
            </span>
          ) : null}
        </span>
        {entry.ts ? <span className={styles.navigatorTime}>{formatNavigatorTime(entry.ts)}</span> : null}
      </span>
      <span className={styles.navigatorPreview}>{preview}</span>
    </button>
  );
};

function getNavigatorRoleClassName(entry: SessionNavigatorEntry): string {
  if (entry.kind === 'tool') {
    return styles.navigatorRoleTool;
  }
  const role = entry.role.toLowerCase();
  if (role === 'user') {
    return styles.navigatorRoleUser;
  }
  if (role === 'assistant') {
    return styles.navigatorRoleAssistant;
  }
  if (role === 'system') {
    return styles.navigatorRoleSystem;
  }
  if (role === 'summary') {
    return styles.navigatorRoleSummary;
  }
  return styles.navigatorRoleSystem;
}

function formatNavigatorTime(timestamp?: number): string {
  if (!timestamp) {
    return '';
  }
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp));
}

export default SessionMessageNavigator;
