import React from 'react';
import {
  CheckOutlined,
  CloseOutlined,
  ClockCircleOutlined,
  CopyOutlined,
  DeleteOutlined,
  ExclamationCircleOutlined,
  ImportOutlined,
  FolderOpenOutlined,
  MessageOutlined,
  ReloadOutlined,
  SearchOutlined,
} from '@ant-design/icons';
import {
  Button,
  Checkbox,
  Collapse,
  Empty,
  Form,
  Input,
  Modal,
  Select,
  Spin,
  Typography,
  message,
} from 'antd';
import { useTranslation } from 'react-i18next';
import { open, save } from '@tauri-apps/plugin-dialog';

import {
  deleteToolSessions,
  deleteToolSession,
  exportToolSession,
  getToolSessionDetail,
  getToolSubagentSessionDetail,
  importToolSession,
  listToolSessionSubagents,
  listToolSessions,
  renameToolSession,
} from './sessionManagerApi';
import type {
  DeleteToolSessionsResult,
  SessionDetail,
  SessionMeta,
  SessionPathOption,
  SessionSubagentMeta,
  SessionTool,
} from './types';
import {
  advanceVisibleContextId,
  formatRelativeTime,
  formatSessionTitle,
  shortSessionId,
  shouldShowVisibleFeedback as shouldShowVisibleFeedbackForContext,
} from './utils';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import SessionDetailWorkbench from './detail/SessionDetailWorkbench';
import styles from './SessionManagerPanel.module.less';

const { Text } = Typography;

interface SessionManagerPanelProps {
  tool: SessionTool;
  translationKey?: string;
  expandNonce?: number;
  refreshNonce?: number;
  extra?: React.ReactNode;
}

const PAGE_SIZE = 10;
const ALL_PATHS_VALUE = '__all_paths__';

interface SessionManagerContentProps {
  tool: SessionTool;
  expanded: boolean;
  refreshNonce?: number;
}

const SessionManagerContent: React.FC<SessionManagerContentProps> = ({
  tool,
  expanded,
  refreshNonce = 0,
}) => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const sentinelRef = React.useRef<HTMLDivElement | null>(null);
  const [query, setQuery] = React.useState('');
  const [debouncedQuery, setDebouncedQuery] = React.useState('');
  const [pathFilter, setPathFilter] = React.useState('');
  const [loading, setLoading] = React.useState(false);
  const [loadingMore, setLoadingMore] = React.useState(false);
  const [pathOptions, setPathOptions] = React.useState<SessionPathOption[]>([]);
  const [pathOptionsLoading, setPathOptionsLoading] = React.useState(false);
  const [items, setItems] = React.useState<SessionMeta[]>([]);
  const [page, setPage] = React.useState(1);
  const [hasMore, setHasMore] = React.useState(false);
  const [total, setTotal] = React.useState(0);
  const [detailOpen, setDetailOpen] = React.useState(false);
  const [detailLoading, setDetailLoading] = React.useState(false);
  const [exporting, setExporting] = React.useState(false);
  const [importing, setImporting] = React.useState(false);
  const [detail, setDetail] = React.useState<SessionDetail | null>(null);
  const [rootDetail, setRootDetail] = React.useState<SessionDetail | null>(null);
  const [subagentSessions, setSubagentSessions] = React.useState<SessionSubagentMeta[]>([]);
  const [parentDetailStack, setParentDetailStack] = React.useState<SessionDetail[]>([]);
  const [renameModalOpen, setRenameModalOpen] = React.useState(false);
  const [renaming, setRenaming] = React.useState(false);
  const [selectionMode, setSelectionMode] = React.useState(false);
  const [selectedSourcePaths, setSelectedSourcePaths] = React.useState<string[]>([]);
  const [bulkDeleting, setBulkDeleting] = React.useState(false);
  const listContextIdRef = React.useRef(0);
  const listReplaceRequestIdRef = React.useRef(0);
  const listAppendRequestIdRef = React.useRef(0);
  const detailRequestIdRef = React.useRef(0);
  const activePageRef = React.useRef(isActive);
  const visibleContextIdRef = React.useRef(0);
  const [renameForm] = Form.useForm<{ title: string }>();
  const clearSelection = React.useCallback(() => {
    setSelectedSourcePaths([]);
  }, []);

  // KeepAlive pages stay mounted when hidden, so refs must be synchronized
  // during render to avoid effect timing races with in-flight async callbacks.
  visibleContextIdRef.current = advanceVisibleContextId(
    visibleContextIdRef.current,
    activePageRef.current,
    isActive,
  );
  activePageRef.current = isActive;

  const captureVisibleContextId = React.useCallback(() => visibleContextIdRef.current, []);

  const shouldShowVisibleFeedback = React.useCallback((visibleContextId?: number) => {
    return shouldShowVisibleFeedbackForContext(
      activePageRef.current,
      visibleContextId,
      visibleContextIdRef.current,
    );
  }, []);

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
    setPathOptions([]);
    setPathOptionsLoading(false);
    setSelectionMode(false);
    setSelectedSourcePaths([]);
  }, [expanded]);

  const loadSessions = React.useCallback(async (
    nextPage: number,
    append: boolean,
    forceRefresh = false,
  ) => {
    if (!expanded) {
      return;
    }

    const visibleContextId = captureVisibleContextId();
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
    const finishLoadingState = () => {
      if (append) {
        if (requestId === listAppendRequestIdRef.current) {
          setLoadingMore(false);
        }
        return;
      }

      if (requestId === listReplaceRequestIdRef.current) {
        setLoading(false);
        setPathOptionsLoading(false);
      }
    };

    if (append) {
      listAppendRequestIdRef.current = requestId;
      setLoadingMore(true);
    } else {
      listContextIdRef.current = requestContextId;
      listReplaceRequestIdRef.current = requestId;
      listAppendRequestIdRef.current += 1;
      setLoading(true);
      setPathOptionsLoading(true);
      setLoadingMore(false);
      setHasMore(false);
    }

    try {
      const result = await listToolSessions({
        tool,
        query: debouncedQuery || undefined,
        pathFilter: pathFilter || undefined,
        page: nextPage,
        pageSize: PAGE_SIZE,
        forceRefresh,
      });

      if (!isCurrentRequest()) {
        return;
      }

      if (!append) {
        clearSelection();
      }

      setItems((current) => (append ? [...current, ...result.items] : result.items));
      setPage(result.page);
      setHasMore(result.hasMore);
      setTotal(result.total);
      if (!append) {
        setPathOptions([
          {
            label: t('sessionManager.allPaths'),
            value: ALL_PATHS_VALUE,
          },
          ...(result.availablePaths ?? []).map((item) => ({
            label: item,
            value: item,
          })),
        ]);
      }
    } catch (error) {
      if (!isCurrentRequest()) {
        return;
      }
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    } finally {
      finishLoadingState();
    }
  }, [
    captureVisibleContextId,
    clearSelection,
    debouncedQuery,
    expanded,
    pathFilter,
    shouldShowVisibleFeedback,
    t,
    tool,
  ]);

  React.useEffect(() => {
    if (!expanded) {
      return;
    }
    void loadSessions(1, false);
  }, [expanded, debouncedQuery, loadSessions, pathFilter, refreshNonce]);

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

  const handleRefresh = async () => {
    await loadSessions(1, false, true);
  };

  const exitSelectionMode = React.useCallback(() => {
    setSelectionMode(false);
    clearSelection();
  }, [clearSelection]);

  React.useEffect(() => {
    clearSelection();
  }, [clearSelection, debouncedQuery, pathFilter]);

  const handleImportSession = async () => {
    let selectedImportPath: string | null = null;

    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: t('sessionManager.importDialogTitle'),
        filters: [
          {
            name: 'JSON',
            extensions: ['json'],
          },
        ],
      });

      if (!selected || Array.isArray(selected)) {
        return;
      }

      selectedImportPath = selected;
    } catch (error) {
      if (!shouldShowVisibleFeedback()) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
      return;
    }

    const importPath = selectedImportPath;
    if (!importPath) {
      return;
    }

    const visibleContextId = captureVisibleContextId();

    try {
      setImporting(true);
      await importToolSession(tool, importPath);
      await loadSessions(1, false, true);
      if (shouldShowVisibleFeedback(visibleContextId)) {
        message.success(t('sessionManager.importSuccess'));
      }
    } catch (error) {
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    } finally {
      setImporting(false);
    }
  };

  const resetDetailState = React.useCallback(() => {
    detailRequestIdRef.current += 1;
    setDetail(null);
    setRootDetail(null);
    setSubagentSessions([]);
    setParentDetailStack([]);
    setDetailLoading(false);
    setRenameModalOpen(false);
    setRenaming(false);
    setImporting(false);
    setExporting(false);
    renameForm.resetFields();
  }, [renameForm]);

  const fetchSessionDetail = React.useCallback(async (session: SessionMeta) => {
    const visibleContextId = captureVisibleContextId();
    const requestId = detailRequestIdRef.current + 1;
    detailRequestIdRef.current = requestId;

    try {
      const [result, subagents] = await Promise.all([
        getToolSessionDetail(tool, session.sourcePath),
        listToolSessionSubagents(tool, session.sourcePath),
      ]);
      if (requestId !== detailRequestIdRef.current) {
        return;
      }
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      setDetail(result);
      setRootDetail(result);
      setSubagentSessions(subagents);
      setParentDetailStack([]);
    } catch (error) {
      if (requestId !== detailRequestIdRef.current) {
        return;
      }
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    }
  }, [captureVisibleContextId, shouldShowVisibleFeedback, t, tool]);

  const handleOpenSubagentDetail = React.useCallback(async (subagent: SessionSubagentMeta) => {
    const parentDetail = detail;
    const parentSourcePath = rootDetail?.meta.sourcePath;
    if (!parentDetail || !parentSourcePath) {
      return;
    }

    const visibleContextId = captureVisibleContextId();
    const requestId = detailRequestIdRef.current + 1;
    detailRequestIdRef.current = requestId;
    setDetailLoading(true);

    try {
      const result = await getToolSubagentSessionDetail(tool, parentSourcePath, subagent.sourcePath);
      if (requestId !== detailRequestIdRef.current) {
        return;
      }
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      setParentDetailStack((current) => [...current, parentDetail]);
      setDetail(result);
    } catch (error) {
      if (requestId !== detailRequestIdRef.current) {
        return;
      }
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    } finally {
      if (requestId === detailRequestIdRef.current) {
        setDetailLoading(false);
      }
    }
  }, [captureVisibleContextId, detail, rootDetail?.meta.sourcePath, shouldShowVisibleFeedback, t, tool]);

  const handleBackToParentDetail = React.useCallback(() => {
    setParentDetailStack((current) => {
      const nextParent = current[current.length - 1];
      if (!nextParent) {
        return current;
      }
      setDetail(nextParent);
      return current.slice(0, -1);
    });
  }, []);

  const handleOpenDetail = async (session: SessionMeta) => {
    setDetailOpen(true);
    setDetail(null);
    setDetailLoading(true);

    try {
      await fetchSessionDetail(session);
    } finally {
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

  const buildSessionExportFileName = (session: SessionMeta) => {
    return `${tool}-session-${session.sessionId}.json`;
  };

  const exportSessionDetail = async (sessionDetail: SessionDetail) => {
    const exportMessageKey = `session-export-${tool}`;
    const visibleContextId = captureVisibleContextId();
    try {
      const exportPath = await save({
        title: t('sessionManager.exportDialogTitle'),
        defaultPath: buildSessionExportFileName(sessionDetail.meta),
        filters: [
          {
            name: 'JSON',
            extensions: ['json'],
          },
        ],
      });

      if (!exportPath) {
        return;
      }

      setExporting(true);
      if (shouldShowVisibleFeedback(visibleContextId)) {
        message.open({
          key: exportMessageKey,
          type: 'loading',
          content: t('sessionManager.exporting'),
          duration: 0,
        });
      }
      await exportToolSession(tool, sessionDetail.meta.sourcePath, exportPath);
      if (shouldShowVisibleFeedback(visibleContextId)) {
        message.success({
          key: exportMessageKey,
          content: t('sessionManager.exportSuccess'),
        });
      } else {
        message.destroy(exportMessageKey);
      }
    } catch (error) {
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        message.destroy(exportMessageKey);
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error({
        key: exportMessageKey,
        content: errorMessage || t('common.error'),
      });
    } finally {
      setExporting(false);
    }
  };

  const performDeleteSession = async (session: SessionMeta, visibleContextId: number) => {
    await deleteToolSession(tool, session.sourcePath);

    if (detail?.meta.sourcePath === session.sourcePath) {
      resetDetailState();
      setDetailOpen(false);
    }

    await loadSessions(1, false, true);
    if (shouldShowVisibleFeedback(visibleContextId)) {
      message.success(t('sessionManager.deleteSuccess'));
    }
  };

  const handleSelectionModeToggle = () => {
    if (selectionMode) {
      exitSelectionMode();
      return;
    }

    setSelectionMode(true);
    clearSelection();
  };

  const toggleSessionSelection = (session: SessionMeta) => {
    setSelectedSourcePaths((current) => (
      current.includes(session.sourcePath)
        ? current.filter((path) => path !== session.sourcePath)
        : [...current, session.sourcePath]
    ));
  };

  const handleSelectAllCurrentPage = () => {
    const currentPagePaths = items.map((session) => session.sourcePath);
    const allSelected = currentPagePaths.length > 0
      && currentPagePaths.every((sourcePath) => selectedSourcePaths.includes(sourcePath));

    setSelectedSourcePaths((current) => {
      if (allSelected) {
        return current.filter((sourcePath) => !currentPagePaths.includes(sourcePath));
      }

      const nextSelected = new Set(current);
      currentPagePaths.forEach((sourcePath) => {
        nextSelected.add(sourcePath);
      });
      return Array.from(nextSelected);
    });
  };

  const performBulkDeleteSessions = async (
    visibleContextId: number,
  ): Promise<DeleteToolSessionsResult> => {
    const result = await deleteToolSessions(tool, selectedSourcePaths);
    const failedSourcePathSet = new Set(result.failedItems.map((item) => item.sourcePath));

    if (
      detail
      && selectedSourcePaths.includes(detail.meta.sourcePath)
      && !failedSourcePathSet.has(detail.meta.sourcePath)
    ) {
      resetDetailState();
      setDetailOpen(false);
    }

    await loadSessions(1, false, true);

    if (result.deletedCount > 0 && shouldShowVisibleFeedback(visibleContextId)) {
      message.success(t('sessionManager.bulkDeleteSuccess', { count: result.deletedCount }));
    }

    if (result.failedItems.length > 0 && shouldShowVisibleFeedback(visibleContextId)) {
      const firstFailure = result.failedItems[0];
      const errorSummary = result.failedItems.length === 1
        ? firstFailure.error
        : t('sessionManager.bulkDeletePartialFailure', { count: result.failedItems.length, error: firstFailure.error });
      message.error(errorSummary || t('common.error'));
    }

    if (result.failedItems.length === 0) {
      exitSelectionMode();
      return result;
    }

    setSelectedSourcePaths((current) => current.filter((sourcePath) => failedSourcePathSet.has(sourcePath)));
    return result;
  };

  const handleBulkDeleteSessions = () => {
    if (selectedSourcePaths.length === 0) {
      return;
    }

    const previewTitles = items
      .filter((session) => selectedSourcePaths.includes(session.sourcePath))
      .slice(0, 5)
      .map((session) => formatSessionTitle(session))
      .join('、');

    Modal.confirm({
      title: t('sessionManager.bulkDeleteConfirmTitle', { count: selectedSourcePaths.length }),
      content: previewTitles
        ? t('sessionManager.bulkDeleteConfirmContentWithPreview', {
          count: selectedSourcePaths.length,
          titles: previewTitles,
        })
        : t('sessionManager.bulkDeleteConfirmContent', { count: selectedSourcePaths.length }),
      icon: <ExclamationCircleOutlined />,
      okText: t('common.delete'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel'),
      onOk: async () => {
        const visibleContextId = captureVisibleContextId();
        try {
          setBulkDeleting(true);
          await performBulkDeleteSessions(visibleContextId);
        } catch (error) {
          if (!shouldShowVisibleFeedback(visibleContextId)) {
            return;
          }
          const errorMessage = error instanceof Error ? error.message : String(error);
          message.error(errorMessage || t('common.error'));
        } finally {
          setBulkDeleting(false);
        }
      },
    });
  };

  const canRenameSession = tool === 'opencode' || tool === 'codex';

  const openRenameModal = () => {
    if (!detail || !canRenameSession) {
      return;
    }
    renameForm.setFieldsValue({
      title: detail.meta.title?.trim() || '',
    });
    setRenameModalOpen(true);
  };

  const handleRenameSession = async () => {
    if (!detail || !canRenameSession) {
      return;
    }

    const visibleContextId = captureVisibleContextId();
    try {
      const values = await renameForm.validateFields();
      setRenaming(true);
      await renameToolSession(tool, detail.meta.sourcePath, values.title);
      if (shouldShowVisibleFeedback(visibleContextId)) {
        message.success(t('sessionManager.renameSuccess'));
      }
      setRenameModalOpen(false);
      await Promise.all([
        fetchSessionDetail(detail.meta),
        loadSessions(1, false, true),
      ]);
    } catch (error) {
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      if (error instanceof Error) {
        message.error(error.message || t('common.error'));
      } else if (!('errorFields' in (error as object))) {
        message.error(String(error) || t('common.error'));
      }
    } finally {
      setRenaming(false);
    }
  };

  const handleDeleteSession = (session: SessionMeta) => {
    Modal.confirm({
      title: t('sessionManager.deleteConfirmTitle', { title: formatSessionTitle(session) }),
      content: t('sessionManager.deleteConfirmContent'),
      icon: <ExclamationCircleOutlined />,
      okText: t('common.delete'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel'),
      onOk: async () => {
        const visibleContextId = captureVisibleContextId();
        try {
          await performDeleteSession(session, visibleContextId);
        } catch (error) {
          if (!shouldShowVisibleFeedback(visibleContextId)) {
            return;
          }
          const errorMessage = error instanceof Error ? error.message : String(error);
          message.error(errorMessage || t('common.error'));
        }
      },
    });
  };

  return (
    <>
      <div>
        <div className={styles.toolbar}>
          <div className={styles.toolbarMain}>
            <div className={styles.toolbarLeft}>
              <Input
                allowClear
                className={styles.searchInput}
                prefix={<SearchOutlined />}
                placeholder={t('sessionManager.searchPlaceholder')}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
              <Select
                allowClear
                showSearch={{ optionFilterProp: 'label' }}
                className={styles.pathFilterSelect}
                placeholder={t('sessionManager.pathFilterPlaceholder')}
                loading={pathOptionsLoading}
                value={pathFilter || (pathOptions.length > 0 ? ALL_PATHS_VALUE : undefined)}
                onChange={(value) => setPathFilter(value === ALL_PATHS_VALUE ? '' : (value ?? ''))}
                options={pathOptions}
              />
            </div>
            <Text className={styles.summaryText}>
              {t('sessionManager.totalSessions', { count: total })}
            </Text>
          </div>
          <Button
            type="link"
            size="small"
            className={styles.actionButton}
            icon={selectionMode ? <CloseOutlined /> : <CheckOutlined />}
            onClick={handleSelectionModeToggle}
          >
            {selectionMode ? t('sessionManager.cancelSelection') : t('sessionManager.select')}
          </Button>
          {selectionMode ? (
            <>
              <Button
                type="link"
                size="small"
                className={styles.actionButton}
                icon={<CheckOutlined />}
                onClick={handleSelectAllCurrentPage}
              >
                {t('sessionManager.selectLoaded')}
              </Button>
              <Button
                type="link"
                size="small"
                danger
                className={styles.actionButton}
                icon={<DeleteOutlined />}
                disabled={selectedSourcePaths.length === 0}
                loading={bulkDeleting}
                onClick={handleBulkDeleteSessions}
              >
                {t('sessionManager.bulkDelete', { count: selectedSourcePaths.length })}
              </Button>
            </>
          ) : null}
          <Button
            type="link"
            size="small"
            className={styles.actionButton}
            icon={<ReloadOutlined />}
            onClick={() => void handleRefresh()}
          >
            {t('common.refresh')}
          </Button>
          <Button
            type="link"
            size="small"
            className={styles.actionButton}
            icon={<ImportOutlined />}
            onClick={() => void handleImportSession()}
            loading={importing}
          >
            {t('sessionManager.import')}
          </Button>
        </div>

        <Spin spinning={loading}>
          {items.length === 0 ? (
            <div className={styles.emptyState}>
              <Empty description={t(debouncedQuery || pathFilter ? 'sessionManager.emptyFiltered' : 'sessionManager.empty')} />
              {(debouncedQuery || pathFilter) ? (
                <Text className={styles.emptyHint}>
                  {t('sessionManager.emptyFilteredHint')}
                </Text>
              ) : null}
            </div>
          ) : (
            <div className={styles.list}>
              {items.map((session) => {
                const displayTime = session.lastActiveAt || session.createdAt;
                const selected = selectedSourcePaths.includes(session.sourcePath);
                return (
                  <div
                    key={`${session.providerId}-${session.sessionId}-${session.sourcePath}`}
                    className={`${styles.sessionCard}${selected ? ` ${styles.sessionCardSelected}` : ''}`}
                    onClick={() => {
                      if (selectionMode) {
                        toggleSessionSelection(session);
                        return;
                      }

                      void handleOpenDetail(session);
                    }}
                  >
                    <div className={styles.sessionHeader}>
                      {selectionMode ? (
                        <Checkbox
                          className={styles.sessionCheckbox}
                          checked={selected}
                          onChange={() => toggleSessionSelection(session)}
                          onClick={(event) => event.stopPropagation()}
                        />
                      ) : null}
                      <div className={styles.sessionHeaderMain}>
                        <div className={styles.sessionTitleRow}>
                          <span className={styles.sessionTitle}>
                            {formatSessionTitle(session)}
                          </span>
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
                          type="link"
                          size="small"
                          className={styles.actionButton}
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
                        <Button
                          type="link"
                          size="small"
                          danger
                          className={styles.actionButton}
                          icon={<DeleteOutlined />}
                          disabled={selectionMode}
                          onClick={() => {
                            handleDeleteSession(session);
                          }}
                        >
                          {t('common.delete')}
                        </Button>
                      </div>
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

      <Modal
        open={detailOpen}
        onCancel={() => {
          resetDetailState();
          setDetailOpen(false);
        }}
        width={1280}
        className={styles.detailModal}
        footer={null}
        destroyOnHidden
        title={null}
      >
        <Spin spinning={detailLoading}>
          {detail ? (
            <SessionDetailWorkbench
              detail={detail}
              subagents={subagentSessions}
              isSubagentDetail={parentDetailStack.length > 0}
              exporting={exporting}
              canRename={canRenameSession && parentDetailStack.length === 0}
              canExport={parentDetailStack.length === 0}
              canDelete={parentDetailStack.length === 0}
              t={t}
              onRename={openRenameModal}
              onExport={() => void exportSessionDetail(detail)}
              onDelete={() => handleDeleteSession(detail.meta)}
              onOpenSubagent={handleOpenSubagentDetail}
              onBackToParent={handleBackToParentDetail}
              onCopyText={handleCopyText}
            />
          ) : (
            <Empty description={t('sessionManager.emptyDetail')} />
          )}
        </Spin>
      </Modal>

      <Modal
        open={renameModalOpen}
        title={t('sessionManager.renameTitle')}
        okText={t('common.save')}
        cancelText={t('common.cancel')}
        onOk={() => void handleRenameSession()}
        confirmLoading={renaming}
        onCancel={() => {
          setRenameModalOpen(false);
          renameForm.resetFields();
        }}
        destroyOnHidden
      >
        <Form form={renameForm} layout="horizontal" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <Form.Item
            label={t('sessionManager.renameField')}
            name="title"
            rules={[
              {
                required: true,
                whitespace: true,
                message: t('sessionManager.renameRequired'),
              },
            ]}
          >
            <Input maxLength={200} placeholder={t('sessionManager.renamePlaceholder')} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
};

const SessionManagerPanel: React.FC<SessionManagerPanelProps> = ({
  tool,
  translationKey = 'sessionManager.title',
  expandNonce = 0,
  refreshNonce = 0,
  extra,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = React.useState(false);

  React.useEffect(() => {
    if (expandNonce <= 0) {
      return;
    }

    setExpanded(true);
  }, [expandNonce]);

  return (
    <Collapse
      className={styles.collapseCard}
      destroyOnHidden
      activeKey={expanded ? ['session-manager'] : []}
      onChange={(keys) => {
        const nextExpanded = keys.includes('session-manager');
        setExpanded(nextExpanded);
      }}
      items={[
        {
          key: 'session-manager',
          label: (
            <Text strong>
              <MessageOutlined style={{ marginRight: 8 }} />
              {t(translationKey)}
            </Text>
          ),
          extra,
          children: (
            <SessionManagerContent
              tool={tool}
              expanded={expanded}
              refreshNonce={refreshNonce}
            />
          ),
        },
      ]}
    />
  );
};

export default SessionManagerPanel;
