import React from 'react';
import { Drawer } from 'antd';
import { ChevronDown, ChevronUp } from 'lucide-react';
import type { TFunction } from 'i18next';

import type { SessionDetail, SessionSubagentMeta } from '../types';
import {
  DEFAULT_SESSION_CONTENT_FILTER,
  DEFAULT_SESSION_ROLE_FILTER,
  filterSessionMessages,
  type SessionContentFilter,
  type SessionContentFilterKey,
  type SessionRoleFilter,
  type SessionRoleFilterKey,
} from './domain/messageFilters';
import { flattenMessagesWithDateDividers } from './domain/messageFlatten';
import { buildNavigatorEntries } from './domain/messageNavigator';
import { messageMatchesQuery, type SessionSearchScope } from './domain/messageSearch';
import SessionDetailCommandBar from './SessionDetailCommandBar';
import SessionDetailStatusBar from './SessionDetailStatusBar';
import SessionMessageNavigator from './SessionMessageNavigator';
import SessionMessageViewer from './SessionMessageViewer';
import SessionSubagentPanel from './SessionSubagentPanel';
import styles from './SessionDetailWorkbench.module.less';

interface SessionDetailWorkbenchProps {
  detail: SessionDetail;
  subagents: SessionSubagentMeta[];
  isSubagentDetail: boolean;
  exporting: boolean;
  canRename: boolean;
  canExport: boolean;
  canDelete: boolean;
  t: TFunction;
  onRename: () => void;
  onExport: () => void;
  onDelete: () => void;
  onOpenSubagent: (subagent: SessionSubagentMeta) => void;
  onBackToParent: () => void;
  onCopyText: (text: string, successText: string) => void | Promise<void>;
}

const SessionDetailWorkbench: React.FC<SessionDetailWorkbenchProps> = ({
  detail,
  subagents,
  isSubagentDetail,
  exporting,
  canRename,
  canExport,
  canDelete,
  t,
  onRename,
  onExport,
  onDelete,
  onOpenSubagent,
  onBackToParent,
  onCopyText,
}) => {
  const [query, setQuery] = React.useState('');
  const [roleFilter, setRoleFilter] = React.useState<SessionRoleFilter>(DEFAULT_SESSION_ROLE_FILTER);
  const [contentFilter, setContentFilter] = React.useState<SessionContentFilter>(DEFAULT_SESSION_CONTENT_FILTER);
  const [searchScope, setSearchScope] = React.useState<SessionSearchScope>('content');
  const [activeMessageIndex, setActiveMessageIndex] = React.useState<number | null>(null);
  const [activeMatchOffset, setActiveMatchOffset] = React.useState(0);
  const [navigatorDrawerOpen, setNavigatorDrawerOpen] = React.useState(false);
  const [navigatorCollapsed, setNavigatorCollapsed] = React.useState(false);
  const [scrollControls, setScrollControls] = React.useState({
    canScrollUp: false,
    canScrollDown: false,
  });
  const messageRefs = React.useRef<Map<number, HTMLElement>>(new Map());
  const viewerRef = React.useRef<HTMLDivElement | null>(null);
  const assistantLabel = getAssistantLabel(detail.meta.providerId);

  React.useEffect(() => {
    setQuery('');
    setRoleFilter(DEFAULT_SESSION_ROLE_FILTER);
    setContentFilter(DEFAULT_SESSION_CONTENT_FILTER);
    setSearchScope('content');
    setActiveMessageIndex(null);
    setActiveMatchOffset(0);
    setNavigatorDrawerOpen(false);
    setNavigatorCollapsed(false);
    messageRefs.current.clear();
    viewerRef.current?.scrollTo({ top: 0 });
  }, [detail.meta.sourcePath]);

  const visibleMessages = React.useMemo(() => {
    if (isSubagentDetail) {
      return detail.messages;
    }
    return detail.messages.filter((message) => !message.isSidechain);
  }, [detail.messages, isSubagentDetail]);

  const filteredItems = React.useMemo(() => filterSessionMessages(visibleMessages, {
    query,
    roleFilter,
    contentFilter,
    searchScope,
  }), [contentFilter, query, roleFilter, searchScope, visibleMessages]);

  const rows = React.useMemo(() => flattenMessagesWithDateDividers(filteredItems), [filteredItems]);
  const navigatorEntries = React.useMemo(() => buildNavigatorEntries(visibleMessages, query, searchScope), [query, searchScope, visibleMessages]);
  const matchedMessageIndexes = React.useMemo(() => visibleMessages
    .map((message, index) => ({ message, index }))
    .filter(({ message }) => query.trim() && messageMatchesQuery(message, query, searchScope))
    .map(({ index }) => index), [query, searchScope, visibleMessages]);

  React.useEffect(() => {
    setActiveMatchOffset(0);
  }, [query, roleFilter, contentFilter, searchScope]);

  const toggleRoleFilter = React.useCallback((key: SessionRoleFilterKey) => {
    setRoleFilter((current) => ({
      ...current,
      [key]: !current[key],
    }));
  }, []);

  const toggleContentFilter = React.useCallback((key: SessionContentFilterKey) => {
    setContentFilter((current) => ({
      ...current,
      [key]: !current[key],
    }));
  }, []);

  const updateScrollControls = React.useCallback(() => {
    const node = viewerRef.current;
    if (!node) {
      setScrollControls((current) => (
        current.canScrollUp || current.canScrollDown
          ? { canScrollUp: false, canScrollDown: false }
          : current
      ));
      return;
    }

    const scrollThreshold = 8;
    const maxScrollTop = Math.max(0, node.scrollHeight - node.clientHeight);
    const nextControls = {
      canScrollUp: node.scrollTop > scrollThreshold,
      canScrollDown: node.scrollTop < maxScrollTop - scrollThreshold,
    };

    setScrollControls((current) => (
      current.canScrollUp === nextControls.canScrollUp && current.canScrollDown === nextControls.canScrollDown
        ? current
        : nextControls
    ));
  }, []);

  React.useEffect(() => {
    const node = viewerRef.current;
    if (!node) {
      updateScrollControls();
      return undefined;
    }

    updateScrollControls();
    const handleScroll = () => updateScrollControls();
    node.addEventListener('scroll', handleScroll, { passive: true });

    const resizeObserver = typeof ResizeObserver === 'undefined'
      ? null
      : new ResizeObserver(updateScrollControls);
    resizeObserver?.observe(node);

    const mutationObserver = typeof MutationObserver === 'undefined'
      ? null
      : new MutationObserver(updateScrollControls);
    mutationObserver?.observe(node, {
      attributes: true,
      characterData: true,
      childList: true,
      subtree: true,
    });

    const animationFrame = window.requestAnimationFrame(updateScrollControls);
    return () => {
      node.removeEventListener('scroll', handleScroll);
      resizeObserver?.disconnect();
      mutationObserver?.disconnect();
      window.cancelAnimationFrame(animationFrame);
    };
  }, [navigatorCollapsed, rows, updateScrollControls]);

  const scrollToMessage = React.useCallback((index: number) => {
    const node = messageRefs.current.get(index);
    if (!node) {
      return;
    }
    node.scrollIntoView({ behavior: 'smooth', block: 'center' });
    setActiveMessageIndex(index);
    setNavigatorDrawerOpen(false);
  }, []);

  const handleNextMatch = () => {
    if (matchedMessageIndexes.length === 0) {
      return;
    }
    const nextOffset = (activeMatchOffset + 1) % matchedMessageIndexes.length;
    setActiveMatchOffset(nextOffset);
    scrollToMessage(matchedMessageIndexes[nextOffset]);
  };

  const handlePreviousMatch = () => {
    if (matchedMessageIndexes.length === 0) {
      return;
    }
    const nextOffset = (activeMatchOffset - 1 + matchedMessageIndexes.length) % matchedMessageIndexes.length;
    setActiveMatchOffset(nextOffset);
    scrollToMessage(matchedMessageIndexes[nextOffset]);
  };

  const setMessageRef = React.useCallback((index: number, node: HTMLElement | null) => {
    if (node) {
      messageRefs.current.set(index, node);
      return;
    }
    messageRefs.current.delete(index);
  }, []);

  const scrollViewerTo = (position: 'top' | 'bottom') => {
    const node = viewerRef.current;
    if (!node) {
      return;
    }
    node.scrollTo({
      top: position === 'top' ? 0 : node.scrollHeight,
      behavior: 'smooth',
    });
  };
  const showScrollControls = scrollControls.canScrollUp || scrollControls.canScrollDown;

  return (
    <div className={`${styles.workbench}${navigatorCollapsed ? ` ${styles.workbenchNavigatorCollapsed}` : ''}`}>
      <SessionDetailCommandBar
        query={query}
        roleFilter={roleFilter}
        contentFilter={contentFilter}
        searchScope={searchScope}
        totalCount={visibleMessages.length}
        visibleCount={filteredItems.length}
        matchCount={matchedMessageIndexes.length}
        activeMatchPosition={matchedMessageIndexes.length > 0 ? activeMatchOffset + 1 : 0}
        canRename={canRename}
        canExport={canExport}
        canDelete={canDelete}
        exporting={exporting}
        hasResumeCommand={Boolean(detail.meta.resumeCommand)}
        isSubagentDetail={isSubagentDetail}
        subagentTitle={detail.meta.title || detail.meta.summary || detail.meta.sessionId}
        t={t}
        onQueryChange={setQuery}
        onRoleFilterToggle={toggleRoleFilter}
        onContentFilterToggle={toggleContentFilter}
        onSearchScopeChange={setSearchScope}
        onPreviousMatch={handlePreviousMatch}
        onNextMatch={handleNextMatch}
        onRename={onRename}
        onExport={onExport}
        onCopyResume={() => {
          if (detail.meta.resumeCommand) {
            void onCopyText(detail.meta.resumeCommand, t('sessionManager.copyResumeSuccess'));
          }
        }}
        onDelete={onDelete}
        onBackToParent={onBackToParent}
        onShowNavigator={() => setNavigatorDrawerOpen(true)}
      />

      {!isSubagentDetail ? (
        <SessionSubagentPanel
          subagents={subagents}
          t={t}
          onSelect={onOpenSubagent}
        />
      ) : null}

      <main className={styles.workbenchMain}>
        <div className={styles.messageViewerShell}>
          <SessionMessageViewer
            rows={rows}
            activeMessageIndex={activeMessageIndex}
            query={query}
            contentFilter={contentFilter}
            assistantLabel={assistantLabel}
            t={t}
            viewerRef={viewerRef}
            onCopyText={onCopyText}
            setMessageRef={setMessageRef}
          />
          {showScrollControls ? (
            <div className={styles.scrollControls}>
              {scrollControls.canScrollUp ? (
                <button
                  type="button"
                  className={styles.scrollControlButton}
                  onClick={() => scrollViewerTo('top')}
                  title={t('sessionManager.scrollToTop')}
                  aria-label={t('sessionManager.scrollToTop')}
                >
                  <ChevronUp size={13} aria-hidden="true" />
                </button>
              ) : null}
              {scrollControls.canScrollDown ? (
                <button
                  type="button"
                  className={styles.scrollControlButton}
                  onClick={() => scrollViewerTo('bottom')}
                  title={t('sessionManager.scrollToBottom')}
                  aria-label={t('sessionManager.scrollToBottom')}
                >
                  <ChevronDown size={13} aria-hidden="true" />
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
        <SessionMessageNavigator
          entries={navigatorEntries}
          activeMessageIndex={activeMessageIndex}
          collapsed={navigatorCollapsed}
          t={t}
          onSelect={scrollToMessage}
          onToggleCollapse={() => setNavigatorCollapsed((current) => !current)}
        />
      </main>

      <SessionDetailStatusBar
        detail={detail}
        visibleCount={filteredItems.length}
        totalCount={visibleMessages.length}
        t={t}
      />

      <Drawer
        open={navigatorDrawerOpen}
        onClose={() => setNavigatorDrawerOpen(false)}
        title={null}
        placement="right"
        width={320}
        closable={false}
        className={styles.navigatorDrawer}
      >
        <SessionMessageNavigator
          entries={navigatorEntries}
          activeMessageIndex={activeMessageIndex}
          t={t}
          onSelect={scrollToMessage}
          onToggleCollapse={() => setNavigatorDrawerOpen(false)}
        />
      </Drawer>
    </div>
  );
};

function getAssistantLabel(providerId: SessionDetail['meta']['providerId']): string {
  switch (providerId) {
    case 'claudecode':
      return 'Claude';
    case 'geminicli':
      return 'Gemini';
    case 'opencode':
      return 'OpenCode';
    case 'openclaw':
      return 'OpenClaw';
    case 'codex':
    default:
      return 'Codex';
  }
}

export default SessionDetailWorkbench;
