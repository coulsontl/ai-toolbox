import React from 'react';
import {
  AppstoreOutlined,
  ClockCircleOutlined,
  CopyOutlined,
  FolderOpenOutlined,
  MessageOutlined,
  ReloadOutlined,
  SearchOutlined,
  UnorderedListOutlined,
} from '@ant-design/icons';
import {
  Button,
  Collapse,
  Drawer,
  Empty,
  Input,
  Modal,
  Space,
  Spin,
  Tag,
  Typography,
  message,
} from 'antd';
import { useTranslation } from 'react-i18next';

import { getToolSessionDetail, listToolSessions } from './sessionManagerApi';
import type {
  SessionDetail,
  SessionMessage,
  SessionMeta,
  SessionTocItem,
  SessionTool,
} from './types';
import {
  buildSessionTocItems,
  formatRelativeTime,
  formatSessionTitle,
  formatTimestamp,
  getRoleLabel,
  getToolLabel,
  shortSessionId,
  shouldCollapseMessage,
} from './utils';
import styles from './SessionManagerPanel.module.less';

const { Text } = Typography;

interface SessionManagerPanelProps {
  tool: SessionTool;
  translationKey?: string;
}

const PAGE_SIZE = 10;

const SessionManagerPanel: React.FC<SessionManagerPanelProps> = ({
  tool,
  translationKey = 'sessionManager.title',
}) => {
  const { t } = useTranslation();
  const sentinelRef = React.useRef<HTMLDivElement | null>(null);
  const [expanded, setExpanded] = React.useState(false);
  const [query, setQuery] = React.useState('');
  const [debouncedQuery, setDebouncedQuery] = React.useState('');
  const [loading, setLoading] = React.useState(false);
  const [loadingMore, setLoadingMore] = React.useState(false);
  const [items, setItems] = React.useState<SessionMeta[]>([]);
  const [page, setPage] = React.useState(1);
  const [hasMore, setHasMore] = React.useState(false);
  const [total, setTotal] = React.useState(0);
  const [detailOpen, setDetailOpen] = React.useState(false);
  const [detailLoading, setDetailLoading] = React.useState(false);
  const [detail, setDetail] = React.useState<SessionDetail | null>(null);
  const [detailQuery, setDetailQuery] = React.useState('');
  const [mobileTocOpen, setMobileTocOpen] = React.useState(false);
  const [activeMessageIndex, setActiveMessageIndex] = React.useState<number | null>(null);
  const messageRefs = React.useRef<Map<number, HTMLDivElement>>(new Map());
  const [expandedMessages, setExpandedMessages] = React.useState<Record<number, boolean>>({});
  const listContextIdRef = React.useRef(0);
  const listReplaceRequestIdRef = React.useRef(0);
  const listAppendRequestIdRef = React.useRef(0);
  const detailRequestIdRef = React.useRef(0);

  React.useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedQuery(query.trim()), 250);
    return () => window.clearTimeout(timer);
  }, [query]);

  React.useEffect(() => {
    if (expanded) {
      return;
    }

    listContextIdRef.current += 1;
    listReplaceRequestIdRef.current += 1;
    listAppendRequestIdRef.current += 1;
    setLoading(false);
    setLoadingMore(false);
  }, [expanded]);

  const loadSessions = React.useCallback(async (nextPage: number, append: boolean) => {
    if (!expanded) {
      return;
    }

    const requestContextId = append ? listContextIdRef.current : listContextIdRef.current + 1;
    const requestId = append
      ? listAppendRequestIdRef.current + 1
      : listReplaceRequestIdRef.current + 1;

    const isCurrentRequest = () => {
      if (requestContextId !== listContextIdRef.current) {
        return false;
      }
      return append
        ? requestId === listAppendRequestIdRef.current
        : requestId === listReplaceRequestIdRef.current;
    };

    if (append) {
      listAppendRequestIdRef.current = requestId;
      setLoadingMore(true);
    } else {
      listContextIdRef.current = requestContextId;
      listReplaceRequestIdRef.current = requestId;
      listAppendRequestIdRef.current += 1;
      setLoading(true);
      setLoadingMore(false);
      setHasMore(false);
    }

    try {
      const result = await listToolSessions({
        tool,
        query: debouncedQuery || undefined,
        page: nextPage,
        pageSize: PAGE_SIZE,
      });

      if (!isCurrentRequest()) {
        return;
      }

      setItems((current) => (append ? [...current, ...result.items] : result.items));
      setPage(result.page);
      setHasMore(result.hasMore);
      setTotal(result.total);
    } catch (error) {
      if (!isCurrentRequest()) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    } finally {
      if (!isCurrentRequest()) {
        return;
      }
      if (append) {
        setLoadingMore(false);
      } else {
        setLoading(false);
      }
    }
  }, [debouncedQuery, expanded, t, tool]);

  React.useEffect(() => {
    if (!expanded) {
      return;
    }
    void loadSessions(1, false);
  }, [expanded, debouncedQuery, loadSessions]);

  React.useEffect(() => {
    if (!expanded || !hasMore || loading || loadingMore) {
      return;
    }

    const sentinel = sentinelRef.current;
    if (!sentinel) {
      return;
    }

    const observer = new IntersectionObserver((entries) => {
      const target = entries[0];
      if (target?.isIntersecting) {
        void loadSessions(page + 1, true);
      }
    }, {
      rootMargin: '120px',
    });

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [expanded, hasMore, loadSessions, loading, loadingMore, page]);

  const tocItems = React.useMemo<SessionTocItem[]>(() => {
    return buildSessionTocItems(detail?.messages ?? []);
  }, [detail?.messages]);

  const filteredMessages = React.useMemo(() => {
    const messages = detail?.messages ?? [];
    const normalizedQuery = detailQuery.trim().toLowerCase();
    if (!normalizedQuery) {
      return messages.map((message, index) => ({ message, index }));
    }

    return messages
      .map((message, index) => ({ message, index }))
      .filter(({ message }) => {
        return (
          message.content.toLowerCase().includes(normalizedQuery)
          || message.role.toLowerCase().includes(normalizedQuery)
        );
      });
  }, [detail?.messages, detailQuery]);

  const handleRefresh = async () => {
    await loadSessions(1, false);
  };

  const resetDetailState = React.useCallback(() => {
    detailRequestIdRef.current += 1;
    setDetail(null);
    setDetailLoading(false);
    setDetailQuery('');
    setExpandedMessages({});
    setMobileTocOpen(false);
    setActiveMessageIndex(null);
    messageRefs.current.clear();
  }, []);

  const handleOpenDetail = async (session: SessionMeta) => {
    const requestId = detailRequestIdRef.current + 1;
    detailRequestIdRef.current = requestId;
    setDetailOpen(true);
    setDetail(null);
    setDetailLoading(true);
    setDetailQuery('');
    setExpandedMessages({});
    setMobileTocOpen(false);
    setActiveMessageIndex(null);
    messageRefs.current.clear();

    try {
      const result = await getToolSessionDetail(tool, session.sourcePath);
      if (requestId !== detailRequestIdRef.current) {
        return;
      }
      setDetail(result);
    } catch (error) {
      if (requestId !== detailRequestIdRef.current) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    } finally {
      if (requestId !== detailRequestIdRef.current) {
        return;
      }
      setDetailLoading(false);
    }
  };

  const handleCopyText = async (text: string, successText: string) => {
    try {
      await navigator.clipboard.writeText(text);
      message.success(successText);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    }
  };

  const scrollToMessage = (index: number) => {
    const target = messageRefs.current.get(index);
    if (!target) {
      return;
    }

    target.scrollIntoView({ behavior: 'smooth', block: 'center' });
    setActiveMessageIndex(index);
    setMobileTocOpen(false);
    window.setTimeout(() => {
      setActiveMessageIndex((current) => (current === index ? null : current));
    }, 1800);
  };

  const renderMessage = (messageItem: SessionMessage, index: number) => {
    const isCollapsible = shouldCollapseMessage(messageItem.content);
    const isExpanded = expandedMessages[index] ?? false;

    return (
      <div
        key={`${index}-${messageItem.ts ?? 'no-ts'}`}
        ref={(node) => {
          if (node) {
            messageRefs.current.set(index, node);
          } else {
            messageRefs.current.delete(index);
          }
        }}
        className={`${styles.messageCard}${activeMessageIndex === index ? ` ${styles.messageCardActive}` : ''}`}
      >
        <div className={styles.messageHeader}>
          <div className={styles.messageHeaderLeft}>
            <Tag>{getRoleLabel(messageItem.role, t)}</Tag>
            {messageItem.ts ? <Text type="secondary">{formatTimestamp(messageItem.ts)}</Text> : null}
          </div>
          <Button
            size="small"
            type="text"
            icon={<CopyOutlined />}
            onClick={() => void handleCopyText(messageItem.content, t('sessionManager.copyMessageSuccess'))}
          >
            {t('common.copy')}
          </Button>
        </div>
        <div className={`${styles.messageContent}${isCollapsible && !isExpanded ? ` ${styles.messageCollapsed}` : ''}`}>
          {messageItem.content}
        </div>
        <div className={styles.messageFooter}>
          <span />
          {isCollapsible ? (
            <Button
              type="link"
              size="small"
              onClick={() => {
                setExpandedMessages((current) => ({
                  ...current,
                  [index]: !isExpanded,
                }));
              }}
            >
              {isExpanded ? t('sessionManager.collapseMessage') : t('sessionManager.expandMessage')}
            </Button>
          ) : null}
        </div>
      </div>
    );
  };

  return (
    <>
      <Collapse
        className={styles.collapseCard}
        activeKey={expanded ? ['session-manager'] : []}
        onChange={(keys) => setExpanded(keys.includes('session-manager'))}
        items={[
          {
            key: 'session-manager',
            label: (
              <Text strong>
                <MessageOutlined style={{ marginRight: 8 }} />
                {t(translationKey)}
              </Text>
            ),
            children: (
              <div>
                <div className={styles.toolbar}>
                  <div className={styles.toolbarLeft}>
                    <Input
                      allowClear
                      className={styles.searchInput}
                      prefix={<SearchOutlined />}
                      placeholder={t('sessionManager.searchPlaceholder')}
                      value={query}
                      onChange={(event) => setQuery(event.target.value)}
                    />
                    <Text className={styles.summaryText}>
                      {t('sessionManager.totalSessions', { count: total })}
                    </Text>
                  </div>
                  <Button icon={<ReloadOutlined />} onClick={() => void handleRefresh()}>
                    {t('common.refresh')}
                  </Button>
                </div>

                <Spin spinning={loading}>
                  {items.length === 0 ? (
                    <Empty
                      className={styles.emptyState}
                      description={t('sessionManager.empty')}
                    />
                  ) : (
                    <div className={styles.list}>
                      {items.map((session) => {
                        const displayTime = session.lastActiveAt || session.createdAt;
                        return (
                          <div
                            key={`${session.providerId}-${session.sessionId}-${session.sourcePath}`}
                            className={styles.sessionCard}
                            onClick={() => void handleOpenDetail(session)}
                          >
                            <div className={styles.sessionHeader}>
                              <div className={styles.sessionHeaderMain}>
                                <div className={styles.sessionTitleRow}>
                                  <span className={styles.sessionTitle}>
                                    {formatSessionTitle(session, t)}
                                  </span>
                                  <Tag icon={<AppstoreOutlined />}>
                                    {getToolLabel(session.providerId, t)}
                                  </Tag>
                                </div>
                                <div className={styles.sessionMetaRow}>
                                  <span><ClockCircleOutlined style={{ marginRight: 4 }} />{formatRelativeTime(displayTime, t)}</span>
                                  <span>{shortSessionId(session.sessionId)}</span>
                                  {session.projectDir ? (
                                    <span><FolderOpenOutlined style={{ marginRight: 4 }} />{session.projectDir}</span>
                                  ) : null}
                                </div>
                              </div>
                              <div className={styles.sessionActions} onClick={(event) => event.stopPropagation()}>
                                <Button
                                  size="small"
                                  icon={<CopyOutlined />}
                                  onClick={() => void handleCopyText(session.sessionId, t('sessionManager.copySessionIdSuccess'))}
                                >
                                  {t('sessionManager.copySessionId')}
                                </Button>
                                <Button
                                  size="small"
                                  icon={<CopyOutlined />}
                                  disabled={!session.resumeCommand}
                                  onClick={() => {
                                    if (!session.resumeCommand) {
                                      return;
                                    }
                                    void handleCopyText(session.resumeCommand, t('sessionManager.copyResumeSuccess'));
                                  }}
                                >
                                  {t('sessionManager.copyResume')}
                                </Button>
                              </div>
                            </div>
                            <div className={styles.sessionSummary}>
                              {session.summary?.trim() || t('sessionManager.noSummary')}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </Spin>

                <div ref={sentinelRef} className={styles.sentinel} />
                {(hasMore || loadingMore) ? (
                  <div className={styles.loadMore}>
                    <Button
                      loading={loadingMore}
                      disabled={loading || loadingMore}
                      onClick={() => void loadSessions(page + 1, true)}
                    >
                      {t('sessionManager.loadMore')}
                    </Button>
                  </div>
                ) : null}
              </div>
            ),
          },
        ]}
      />

      <Modal
        open={detailOpen}
        onCancel={() => {
          resetDetailState();
          setDetailOpen(false);
        }}
        width={1200}
        footer={null}
        destroyOnHidden
        title={detail ? formatSessionTitle(detail.meta, t) : t('sessionManager.detailTitle')}
      >
        <Spin spinning={detailLoading}>
          {detail ? (
            <div className={styles.detailLayout}>
              <div className={styles.tocPane}>
                <div className={styles.tocHeader}>
                  <Text strong>{t('sessionManager.tocTitle')}</Text>
                  <Text type="secondary">{tocItems.length}</Text>
                </div>
                <div className={styles.tocList}>
                  {tocItems.length === 0 ? (
                    <Text type="secondary">{t('sessionManager.tocEmpty')}</Text>
                  ) : tocItems.map((item, tocIndex) => (
                    <button
                      key={`${item.index}-${tocIndex}`}
                      type="button"
                      className={styles.tocButton}
                      onClick={() => scrollToMessage(item.index)}
                    >
                      <div><Text strong>{tocIndex + 1}</Text></div>
                      <div className={styles.tocPreview}>{item.preview}</div>
                    </button>
                  ))}
                </div>
              </div>

              <div className={styles.detailMain}>
                <div className={styles.detailMeta}>
                  <Text>{t('sessionManager.sessionId')}: {detail.meta.sessionId}</Text>
                  <Text>{t('sessionManager.provider')}: {getToolLabel(detail.meta.providerId, t)}</Text>
                  {detail.meta.projectDir ? (
                    <Text>{t('sessionManager.projectDir')}: {detail.meta.projectDir}</Text>
                  ) : null}
                  {detail.meta.createdAt ? (
                    <Text>{t('sessionManager.createdAt')}: {formatTimestamp(detail.meta.createdAt)}</Text>
                  ) : null}
                  {detail.meta.lastActiveAt ? (
                    <Text>{t('sessionManager.lastActiveAt')}: {formatTimestamp(detail.meta.lastActiveAt)}</Text>
                  ) : null}
                </div>

                <div className={styles.detailToolbar}>
                  <div className={styles.detailToolbarLeft}>
                    <Input
                      allowClear
                      prefix={<SearchOutlined />}
                      placeholder={t('sessionManager.searchInSession')}
                      value={detailQuery}
                      onChange={(event) => setDetailQuery(event.target.value)}
                    />
                    <Button
                      className={styles.mobileTocButton}
                      icon={<UnorderedListOutlined />}
                      onClick={() => setMobileTocOpen(true)}
                    >
                      {t('sessionManager.tocTitle')}
                    </Button>
                  </div>
                  <Space>
                    <Button
                      icon={<CopyOutlined />}
                      onClick={() => void handleCopyText(
                        detail.messages.map((messageItem) => `[${messageItem.role}] ${messageItem.content}`).join('\n\n'),
                        t('sessionManager.copyConversationSuccess'),
                      )}
                    >
                      {t('sessionManager.copyConversation')}
                    </Button>
                    <Button
                      icon={<CopyOutlined />}
                      disabled={!detail.meta.resumeCommand}
                      onClick={() => {
                        if (!detail.meta.resumeCommand) {
                          return;
                        }
                        void handleCopyText(detail.meta.resumeCommand, t('sessionManager.copyResumeSuccess'));
                      }}
                    >
                      {t('sessionManager.copyResume')}
                    </Button>
                  </Space>
                </div>

                <div className={styles.messagesList}>
                  {filteredMessages.length === 0 ? (
                    <Empty description={t('sessionManager.noMessagesMatched')} />
                  ) : filteredMessages.map(({ message: messageItem, index }) => renderMessage(messageItem, index))}
                </div>
              </div>
            </div>
          ) : (
            <Empty description={t('sessionManager.emptyDetail')} />
          )}
        </Spin>
      </Modal>

      <Drawer
        open={mobileTocOpen}
        onClose={() => setMobileTocOpen(false)}
        title={t('sessionManager.tocTitle')}
        placement="right"
      >
        <div className={styles.tocList}>
          {tocItems.length === 0 ? (
            <Text type="secondary">{t('sessionManager.tocEmpty')}</Text>
          ) : tocItems.map((item, tocIndex) => (
            <button
              key={`${item.index}-${tocIndex}-drawer`}
              type="button"
              className={styles.tocButton}
              onClick={() => scrollToMessage(item.index)}
            >
              <div><Text strong>{tocIndex + 1}</Text></div>
              <div className={styles.tocPreview}>{item.preview}</div>
            </button>
          ))}
        </div>
      </Drawer>
    </>
  );
};

export default SessionManagerPanel;
