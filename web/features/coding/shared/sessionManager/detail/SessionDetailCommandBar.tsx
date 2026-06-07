import React from 'react';
import { Button, Input } from 'antd';
import {
  ArrowLeft,
  Bot,
  Brain,
  ChevronRight,
  Copy,
  Download,
  Edit3,
  ListFilter,
  MessageSquareText,
  PanelRightOpen,
  Search,
  Terminal,
  Trash2,
  User,
  Wrench,
} from 'lucide-react';
import type { TFunction } from 'i18next';

import type {
  SessionContentFilter,
  SessionContentFilterKey,
  SessionRoleFilter,
  SessionRoleFilterKey,
} from './domain/messageFilters';
import type { SessionSearchScope } from './domain/messageSearch';
import styles from './SessionDetailWorkbench.module.less';

interface SessionDetailCommandBarProps {
  query: string;
  roleFilter: SessionRoleFilter;
  contentFilter: SessionContentFilter;
  searchScope: SessionSearchScope;
  totalCount: number;
  visibleCount: number;
  matchCount: number;
  activeMatchPosition: number;
  canRename: boolean;
  canExport: boolean;
  canDelete: boolean;
  exporting: boolean;
  hasResumeCommand: boolean;
  isSubagentDetail: boolean;
  subagentTitle: string;
  t: TFunction;
  onQueryChange: (value: string) => void;
  onRoleFilterToggle: (value: SessionRoleFilterKey) => void;
  onContentFilterToggle: (value: SessionContentFilterKey) => void;
  onSearchScopeChange: (value: SessionSearchScope) => void;
  onPreviousMatch: () => void;
  onNextMatch: () => void;
  onRename: () => void;
  onExport: () => void;
  onCopyResume: () => void;
  onDelete: () => void;
  onBackToParent: () => void;
  onShowNavigator: () => void;
}

const roleOptions: Array<{ value: SessionRoleFilterKey; labelKey: string }> = [
  { value: 'user', labelKey: 'sessionManager.filterUser' },
  { value: 'assistant', labelKey: 'sessionManager.filterAssistant' },
];

const contentOptions: Array<{ value: SessionContentFilterKey; labelKey: string }> = [
  { value: 'text', labelKey: 'sessionManager.filterText' },
  { value: 'thinking', labelKey: 'sessionManager.filterThinking' },
  { value: 'tool_call', labelKey: 'sessionManager.filterToolCalls' },
  { value: 'command', labelKey: 'sessionManager.filterCommands' },
];

const SessionDetailCommandBar: React.FC<SessionDetailCommandBarProps> = ({
  query,
  roleFilter,
  contentFilter,
  searchScope,
  totalCount,
  visibleCount,
  matchCount,
  activeMatchPosition,
  canRename,
  canExport,
  canDelete,
  exporting,
  hasResumeCommand,
  isSubagentDetail,
  subagentTitle,
  t,
  onQueryChange,
  onRoleFilterToggle,
  onContentFilterToggle,
  onSearchScopeChange,
  onPreviousMatch,
  onNextMatch,
  onRename,
  onExport,
  onCopyResume,
  onDelete,
  onBackToParent,
  onShowNavigator,
}) => (
  <header className={styles.commandBar}>
    {isSubagentDetail ? (
      <div className={styles.subagentBreadcrumb}>
        <button type="button" className={styles.subagentBackButton} onClick={onBackToParent}>
          <ArrowLeft size={15} aria-hidden="true" />
          <span>{t('sessionManager.backToParentSession')}</span>
        </button>
        <ChevronRight size={14} aria-hidden="true" />
        <span className={styles.subagentBreadcrumbTitle}>
          <Bot size={14} aria-hidden="true" />
          {subagentTitle}
        </span>
      </div>
    ) : null}
    <div className={styles.commandTopRow}>
      <div className={styles.searchScopeToggle}>
        <button
          type="button"
          className={searchScope === 'content' ? styles.searchScopeActive : undefined}
          onClick={() => onSearchScopeChange('content')}
        >
          {t('sessionManager.searchScopeContent')}
        </button>
        <button
          type="button"
          className={searchScope === 'toolId' ? styles.searchScopeActive : undefined}
          onClick={() => onSearchScopeChange('toolId')}
        >
          {t('sessionManager.searchScopeToolId')}
        </button>
      </div>
      <div className={styles.detailSearchGroup}>
        <Input
          allowClear
          className={styles.detailSearchInput}
          prefix={<Search size={15} />}
          placeholder={t('sessionManager.searchInDetail')}
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />
        <span className={styles.matchCounter}>
          {matchCount > 0 ? `${activeMatchPosition}/${matchCount}` : '0/0'}
        </span>
        <Button type="text" size="small" onClick={onPreviousMatch} disabled={matchCount === 0}>
          {t('sessionManager.previousMatch')}
        </Button>
        <Button type="text" size="small" onClick={onNextMatch} disabled={matchCount === 0}>
          {t('sessionManager.nextMatch')}
        </Button>
      </div>
      <div className={styles.actionGroup}>
        {canRename ? <Button icon={<Edit3 size={15} />} onClick={onRename}>{t('sessionManager.rename')}</Button> : null}
        {canExport ? (
          <Button icon={<Download size={15} />} loading={exporting} disabled={exporting} onClick={onExport}>
            {t(exporting ? 'sessionManager.exporting' : 'sessionManager.export')}
          </Button>
        ) : null}
        {!isSubagentDetail ? (
          <Button icon={<Copy size={15} />} disabled={!hasResumeCommand} onClick={onCopyResume}>
            {t('sessionManager.copyResume')}
          </Button>
        ) : null}
        <Button className={styles.navigatorToggle} icon={<PanelRightOpen size={15} />} onClick={onShowNavigator}>
          {t('sessionManager.messageNavigator')}
        </Button>
        {canDelete ? (
          <Button danger icon={<Trash2 size={15} />} onClick={onDelete}>
            {t('common.delete')}
          </Button>
        ) : null}
      </div>
    </div>

    <div className={styles.commandFilterRow}>
      <div className={styles.filterCount}>
        <ListFilter size={15} aria-hidden="true" />
        <span>{visibleCount === totalCount ? totalCount : `${visibleCount}/${totalCount}`}</span>
      </div>
      <div className={styles.filterSeparator} />
      <div className={styles.filterChipGroup}>
        {roleOptions.map((option) => (
          <FilterChip
            key={option.value}
            active={roleFilter[option.value]}
            label={t(option.labelKey)}
            icon={roleIcon(option.value)}
            onClick={() => onRoleFilterToggle(option.value)}
          />
        ))}
      </div>
      <div className={styles.filterSeparator} />
      <div className={styles.filterChipGroup}>
        {contentOptions.map((option) => (
          <FilterChip
            key={option.value}
            active={contentFilter[option.value]}
            label={t(option.labelKey)}
            icon={contentIcon(option.value)}
            onClick={() => onContentFilterToggle(option.value)}
          />
        ))}
      </div>
    </div>
  </header>
);

interface FilterChipProps {
  active: boolean;
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
}

const FilterChip: React.FC<FilterChipProps> = ({ active, label, icon, onClick }) => (
  <button
    type="button"
    className={`${styles.filterChip}${active ? ` ${styles.filterChipActive}` : ''}`}
    aria-pressed={active}
    title={label}
    onClick={onClick}
  >
    {icon}
    <span>{label}</span>
  </button>
);

function roleIcon(role: SessionRoleFilterKey): React.ReactNode {
  if (role === 'user') {
    return <User size={14} aria-hidden="true" />;
  }
  if (role === 'assistant') {
    return <Bot size={14} aria-hidden="true" />;
  }
  return <Bot size={14} aria-hidden="true" />;
}

function contentIcon(content: SessionContentFilterKey): React.ReactNode {
  if (content === 'text') {
    return <MessageSquareText size={14} aria-hidden="true" />;
  }
  if (content === 'thinking') {
    return <Brain size={14} aria-hidden="true" />;
  }
  if (content === 'command') {
    return <Terminal size={14} aria-hidden="true" />;
  }
  return <Wrench size={14} aria-hidden="true" />;
}

export default SessionDetailCommandBar;
