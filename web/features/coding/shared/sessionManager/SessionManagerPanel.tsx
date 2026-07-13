import React from 'react';
import {
  CheckOutlined,
  CloseOutlined,
  ClockCircleOutlined,
  CopyOutlined,
  DeleteOutlined,
  ExclamationCircleOutlined,
  ExportOutlined,
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
  Input,
  Modal,
  Radio,
  Select,
  Spin,
  Tag,
  Tooltip,
  Typography,
  message,
} from 'antd';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { open } from '@tauri-apps/plugin-dialog';

import {
  deleteToolSessions,
  deleteToolSession,
  exportToolSessions,
  importToolSession,
  listToolSessions,
} from './sessionManagerApi';
import type {
  DeleteToolSessionsResult,
  ExportToolSessionsResult,
  SessionListCacheState,
  SessionListLoadMode,
  SessionExportFormat,
  SessionMeta,
  SessionPathOption,
  SessionSourceMode,
  SessionSourceOption,
  SessionTool,
} from './types';
import {
  buildSessionDetailPath,
  SESSION_MANAGER_REFRESH_EVENT,
  type SessionManagerRefreshEventDetail,
} from './sessionDetailNavigation';
import {
  advanceVisibleContextId,
  formatRelativeTime,
  formatSessionTitle,
  resolveEffectiveSessionSourceMode,
  shortSessionId,
  shouldShowVisibleFeedback as shouldShowVisibleFeedbackForContext,
} from './utils';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import styles from './SessionManagerPanel.module.less';

const { Text } = Typography;

interface SessionManagerPanelProps {
  tool: SessionTool;
  translationKey?: string;
  expandNonce?: number;
  refreshNonce?: number;
  extra?: React.ReactNode;
  sourceMode?: SessionSourceMode;
  onSourceModeChange?: (sourceMode: SessionSourceMode) => void;
}

const PAGE_SIZE = 10;
const ALL_PATHS_VALUE = '__all_paths__';
const SESSION_MANAGER_DEBUG_STORAGE_KEY = 'ai-toolbox.sessionManager.debug';
let rememberedSessionSourceMode: SessionSourceMode = 'all';

type MetadataRefreshReason = 'manual-refresh' | null;

interface LoadSessionsOptions {
  forceRefresh?: boolean;
  loadMode?: SessionListLoadMode;
  background?: boolean;
  refreshReason?: MetadataRefreshReason;
  showFullListLoading?: boolean;
  trigger?: string;
}

interface CompleteAllSessionsSnapshot {
  key: string;
  items: SessionMeta[];
  availableSources: SessionSourceOption[];
}

const isSessionManagerDebugEnabled = () => {
  try {
    return window.localStorage.getItem(SESSION_MANAGER_DEBUG_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
};

const debugSessionManager = (event: string, payload: Record<string, unknown>) => {
  if (!isSessionManagerDebugEnabled()) {
    return;
  }

  console.info(`[SessionManagerPanel] ${event}`, payload);
};

const buildCompleteAllSessionsKey = (
  tool: SessionTool,
  query: string,
  pathFilter: string,
  refreshNonce: number,
) => [
  tool,
  query,
  pathFilter,
  refreshNonce,
].join('\u001f');

const sessionMatchesSourceMode = (session: SessionMeta, sourceMode: SessionSourceMode) => {
  return sourceMode === 'all' || session.runtimeSource === sourceMode;
};

const buildSessionPathOptions = (
  sessions: SessionMeta[],
  allPathsLabel: string,
): SessionPathOption[] => {
  const paths: SessionPathOption[] = [{
    label: allPathsLabel,
    value: ALL_PATHS_VALUE,
  }];
  const seenPaths = new Set<string>();

  sessions.forEach((session) => {
    const projectDir = session.projectDir?.trim();
    if (!projectDir) {
      return;
    }

    const dedupeKey = projectDir.toLowerCase();
    if (seenPaths.has(dedupeKey)) {
      return;
    }

    seenPaths.add(dedupeKey);
    paths.push({
      label: projectDir,
      value: projectDir,
    });
  });

  return paths;
};

const buildSessionPathOptionsFromValues = (
  availablePaths: string[],
  allPathsLabel: string,
): SessionPathOption[] => [
  {
    label: allPathsLabel,
    value: ALL_PATHS_VALUE,
  },
  ...availablePaths.map((item) => ({
    label: item,
    value: item,
  })),
];

interface SessionManagerContentProps {
  tool: SessionTool;
  expanded: boolean;
  refreshNonce?: number;
  manualRefreshNonce?: number;
  sourceMode: SessionSourceMode;
  showRuntimeSourceTag: boolean;
  onAvailableSourcesChange: (sources: SessionSourceOption[]) => void;
  onMetadataRefreshStateChange: (reason: MetadataRefreshReason) => void;
}

const SessionManagerContent: React.FC<SessionManagerContentProps> = ({
  tool,
  expanded,
  refreshNonce = 0,
  manualRefreshNonce = 0,
  sourceMode,
  showRuntimeSourceTag,
  onAvailableSourcesChange,
  onMetadataRefreshStateChange,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { isActive, rememberScrollPosition } = useKeepAlive();
  const [query, setQuery] = React.useState('');
  const [debouncedQuery, setDebouncedQuery] = React.useState('');
  const [pathFilter, setPathFilter] = React.useState('');
  const [loading, setLoading] = React.useState(false);
  const [loadingFullList, setLoadingFullList] = React.useState(false);
  const [pathOptions, setPathOptions] = React.useState<SessionPathOption[]>([]);
  const [pathOptionsLoading, setPathOptionsLoading] = React.useState(false);
  const [items, setItems] = React.useState<SessionMeta[]>([]);
  const [total, setTotal] = React.useState(0);
  const [partial, setPartial] = React.useState(false);
  const [cacheState, setCacheState] = React.useState<SessionListCacheState>('none');
  const [metaComplete, setMetaComplete] = React.useState(false);
  const [initialListLoaded, setInitialListLoaded] = React.useState(false);
  const [messageSearchRunning, setMessageSearchRunning] = React.useState(false);
  const [metadataRefreshReason, setMetadataRefreshReason] = React.useState<MetadataRefreshReason>(null);
  const [importing, setImporting] = React.useState(false);
  const [selectionMode, setSelectionMode] = React.useState(false);
  const [selectedSourcePaths, setSelectedSourcePaths] = React.useState<string[]>([]);
  const [bulkExporting, setBulkExporting] = React.useState(false);
  const [bulkDeleting, setBulkDeleting] = React.useState(false);
  const listContextIdRef = React.useRef(0);
  const listReplaceRequestIdRef = React.useRef(0);
  const activePageRef = React.useRef(isActive);
  const visibleContextIdRef = React.useRef(0);
  const previousSourceModeRef = React.useRef(sourceMode);
  const latestBackgroundRefreshKeyRef = React.useRef<string | null>(null);
  const latestInitialLoadKeyRef = React.useRef<string | null>(null);
  const handledManualRefreshNonceRef = React.useRef(0);
  const completeAllSessionsSnapshotRef = React.useRef<CompleteAllSessionsSnapshot | null>(null);
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
    onMetadataRefreshStateChange(metadataRefreshReason);
  }, [metadataRefreshReason, onMetadataRefreshStateChange]);

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
    setLoading(false);
    setLoadingFullList(false);
    setPathOptions([]);
    setPathOptionsLoading(false);
    setSelectionMode(false);
    setSelectedSourcePaths([]);
    setBulkExporting(false);
    setPartial(false);
    setCacheState('none');
    setMetaComplete(false);
    setInitialListLoaded(false);
    setMessageSearchRunning(false);
    setMetadataRefreshReason(null);
    latestBackgroundRefreshKeyRef.current = null;
    latestInitialLoadKeyRef.current = null;
    completeAllSessionsSnapshotRef.current = null;
  }, [expanded]);

  React.useEffect(() => {
    if (previousSourceModeRef.current === sourceMode) {
      return;
    }

    previousSourceModeRef.current = sourceMode;
    setPathFilter('');
  }, [sourceMode]);

  const loadSessions = React.useCallback(async (
    options: LoadSessionsOptions = {},
  ) => {
    if (!expanded) {
      return;
    }

    const {
      forceRefresh = false,
      loadMode = 'cache-first',
      background = false,
      refreshReason = null,
      showFullListLoading = false,
      trigger = 'unknown',
    } = options;
    const snapshotKey = buildCompleteAllSessionsKey(tool, debouncedQuery, pathFilter, refreshNonce);
    const visibleContextId = captureVisibleContextId();
    const requestContextId = background ? listContextIdRef.current : listContextIdRef.current + 1;
    const requestId = listReplaceRequestIdRef.current + 1;

    if (forceRefresh || loadMode === 'refresh') {
      completeAllSessionsSnapshotRef.current = null;
    }

    const isCurrentRequest = () => {
      if (requestContextId !== listContextIdRef.current) {
        return false;
      }
      return requestId === listReplaceRequestIdRef.current;
    };
    const finishLoadingState = () => {
      if (background) {
        if (isCurrentRequest()) {
          setMetadataRefreshReason((current) => (current === refreshReason ? null : current));
          setMessageSearchRunning(false);
          setLoadingFullList(false);
        }
        return;
      }

      if (requestId === listReplaceRequestIdRef.current) {
        setLoading(false);
        setPathOptionsLoading(false);
        setMetadataRefreshReason((current) => (current === refreshReason ? null : current));
      }
    };

    if (background) {
      listReplaceRequestIdRef.current = requestId;
      debugSessionManager('request:start', {
        trigger,
        tool,
        sourceMode,
        loadMode,
        background,
        forceRefresh,
        refreshReason,
        showFullListLoading,
        query: debouncedQuery,
        pathFilter,
        requestContextId,
        requestId,
      });
      if (refreshReason) {
        setMetadataRefreshReason(refreshReason);
      }
      if (showFullListLoading) {
        setLoadingFullList(true);
      }
      if (debouncedQuery) {
        setMessageSearchRunning(true);
      }
    } else {
      listContextIdRef.current = requestContextId;
      listReplaceRequestIdRef.current = requestId;
      debugSessionManager('request:start', {
        trigger,
        tool,
        sourceMode,
        loadMode,
        background,
        forceRefresh,
        refreshReason,
        showFullListLoading,
        query: debouncedQuery,
        pathFilter,
        requestContextId,
        requestId,
      });
      setLoading(true);
      setPathOptionsLoading(true);
      setLoadingFullList(false);
      setPartial(false);
      setCacheState('none');
      setMetaComplete(false);
      setInitialListLoaded(false);
      setMessageSearchRunning(false);
      setMetadataRefreshReason(refreshReason);
      latestBackgroundRefreshKeyRef.current = null;
    }

    try {
      const result = await listToolSessions({
        tool,
        query: debouncedQuery || undefined,
        pathFilter: pathFilter || undefined,
        page: 1,
        pageSize: PAGE_SIZE,
        forceRefresh,
        sourceMode,
        loadMode,
      });

      if (!isCurrentRequest()) {
        debugSessionManager('request:ignore-stale-result', {
          trigger,
          tool,
          sourceMode,
          loadMode,
          background,
          requestContextId,
          requestId,
          currentContextId: listContextIdRef.current,
          currentRequestId: listReplaceRequestIdRef.current,
        });
        return;
      }

      if (!background) {
        clearSelection();
      }

      setItems(result.items);
      setTotal(result.total);
      setPartial(Boolean(result.partial));
      setCacheState(result.cacheState ?? 'none');
      setMetaComplete(Boolean(result.metaComplete));
      if (!background) {
        setInitialListLoaded(true);
      }
      setMessageSearchRunning(Boolean(debouncedQuery && !result.messageSearchComplete));
      if (
        sourceMode === 'all'
        && !result.partial
        && result.metaComplete
        && result.messageSearchComplete !== false
      ) {
        completeAllSessionsSnapshotRef.current = {
          key: snapshotKey,
          items: result.items,
          availableSources: result.availableSources ?? [],
        };
        debugSessionManager('snapshot:store-all', {
          tool,
          query: debouncedQuery,
          pathFilter,
          refreshNonce,
          itemCount: result.items.length,
          trigger,
          loadMode,
        });
      }
      debugSessionManager('request:result', {
        trigger,
        tool,
        sourceMode,
        loadMode,
        background,
        forceRefresh,
        requestContextId,
        requestId,
        itemCount: result.items.length,
        total: result.total,
        partial: Boolean(result.partial),
        cacheState: result.cacheState ?? 'none',
        metaComplete: Boolean(result.metaComplete),
        messageSearchComplete: Boolean(result.messageSearchComplete),
        hasMore: result.hasMore,
      });
      onAvailableSourcesChange(result.availableSources ?? []);
      setPathOptions(buildSessionPathOptionsFromValues(
        result.availablePaths ?? [],
        t('sessionManager.allPaths'),
      ));
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
      debugSessionManager('request:finish', {
        trigger,
        tool,
        sourceMode,
        loadMode,
        background,
        requestContextId,
        requestId,
        isCurrent: isCurrentRequest(),
      });
    }
  }, [
    captureVisibleContextId,
    clearSelection,
    debouncedQuery,
    expanded,
    pathFilter,
    refreshNonce,
    shouldShowVisibleFeedback,
    sourceMode,
    t,
    tool,
    onAvailableSourcesChange,
  ]);

  React.useEffect(() => {
    if (!expanded) {
      return;
    }

    const snapshotKey = buildCompleteAllSessionsKey(tool, debouncedQuery, pathFilter, refreshNonce);
    const initialLoadKey = [
      tool,
      sourceMode,
      debouncedQuery,
      pathFilter,
      refreshNonce,
    ].join('\u001f');
    if (latestInitialLoadKeyRef.current === initialLoadKey) {
      debugSessionManager('initial:skip-duplicate', {
        tool,
        sourceMode,
        query: debouncedQuery,
        pathFilter,
        refreshNonce,
        initialLoadKey,
      });
      return;
    }

    latestInitialLoadKeyRef.current = initialLoadKey;
    latestBackgroundRefreshKeyRef.current = null;
    const allSessionsSnapshot = completeAllSessionsSnapshotRef.current;
    if (allSessionsSnapshot?.key === snapshotKey) {
      const filteredItems = allSessionsSnapshot.items.filter((session) => (
        sessionMatchesSourceMode(session, sourceMode)
      ));
      debugSessionManager('initial:apply-all-snapshot', {
        tool,
        sourceMode,
        query: debouncedQuery,
        pathFilter,
        refreshNonce,
        initialLoadKey,
        snapshotKey,
        itemCount: filteredItems.length,
      });
      listContextIdRef.current += 1;
      listReplaceRequestIdRef.current += 1;
      clearSelection();
      setLoading(false);
      setLoadingFullList(false);
      setPathOptionsLoading(false);
      setItems(filteredItems);
      setTotal(filteredItems.length);
      setPartial(false);
      setCacheState('fresh');
      setMetaComplete(true);
      setInitialListLoaded(true);
      setMessageSearchRunning(false);
      setMetadataRefreshReason(null);
      onAvailableSourcesChange(allSessionsSnapshot.availableSources);
      setPathOptions(buildSessionPathOptions(filteredItems, t('sessionManager.allPaths')));
      return;
    }

    debugSessionManager('initial:run', {
      tool,
      sourceMode,
      query: debouncedQuery,
      pathFilter,
      refreshNonce,
      initialLoadKey,
    });
    void loadSessions({ loadMode: 'cache-first', trigger: 'initial-cache-first' });
  }, [
    clearSelection,
    debouncedQuery,
    expanded,
    loadSessions,
    onAvailableSourcesChange,
    pathFilter,
    refreshNonce,
    sourceMode,
    t,
    tool,
  ]);

  React.useEffect(() => {
    if (!expanded || !initialListLoaded || loading || loadingFullList || metadataRefreshReason) {
      return;
    }

    const needsFullMetadata = partial || cacheState === 'stale' || !metaComplete;
    const needsMessageSearch = Boolean(debouncedQuery && messageSearchRunning);
    if (!needsFullMetadata && !needsMessageSearch) {
      return;
    }

    const backgroundKey = [
      tool,
      sourceMode,
      debouncedQuery,
      pathFilter,
      needsFullMetadata ? 'meta' : 'search',
    ].join('|');
    if (latestBackgroundRefreshKeyRef.current === backgroundKey) {
      debugSessionManager('background:skip-duplicate', {
        tool,
        sourceMode,
        query: debouncedQuery,
        pathFilter,
        backgroundKey,
        needsFullMetadata,
        needsMessageSearch,
      });
      return;
    }
    latestBackgroundRefreshKeyRef.current = backgroundKey;

    debugSessionManager('background:run-full', {
      tool,
      sourceMode,
      query: debouncedQuery,
      pathFilter,
      backgroundKey,
      needsFullMetadata,
      needsMessageSearch,
      partial,
      cacheState,
      metaComplete,
      messageSearchRunning,
    });
    void loadSessions({
      loadMode: 'full',
      background: true,
      showFullListLoading: needsFullMetadata,
      trigger: 'background-full',
    });
  }, [
    cacheState,
    debouncedQuery,
    expanded,
    initialListLoaded,
    loadSessions,
    loading,
    loadingFullList,
    messageSearchRunning,
    metaComplete,
    metadataRefreshReason,
    partial,
    pathFilter,
    sourceMode,
    tool,
  ]);

  React.useEffect(() => {
    const handleRefreshEvent = (event: Event) => {
      const detail = (event as CustomEvent<SessionManagerRefreshEventDetail>).detail;
      if (detail?.tool !== tool || !expanded) {
        return;
      }
      void loadSessions({
        forceRefresh: true,
        loadMode: 'refresh',
        refreshReason: 'manual-refresh',
        trigger: 'refresh-event',
      });
    };

    window.addEventListener(SESSION_MANAGER_REFRESH_EVENT, handleRefreshEvent);
    return () => window.removeEventListener(SESSION_MANAGER_REFRESH_EVENT, handleRefreshEvent);
  }, [expanded, loadSessions, tool]);

  const handleRefresh = async () => {
    latestBackgroundRefreshKeyRef.current = null;
    await loadSessions({
      forceRefresh: true,
      loadMode: 'refresh',
      refreshReason: 'manual-refresh',
      trigger: 'manual-refresh',
    });
  };

  React.useEffect(() => {
    if (!expanded || manualRefreshNonce <= handledManualRefreshNonceRef.current) {
      return;
    }

    handledManualRefreshNonceRef.current = manualRefreshNonce;
    void handleRefresh();
  }, [expanded, manualRefreshNonce]);

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
      await loadSessions({
        forceRefresh: true,
        loadMode: 'refresh',
        background: true,
        refreshReason: 'manual-refresh',
        trigger: 'import-refresh',
      });
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

  const handleOpenDetail = (session: SessionMeta) => {
    const fromScrollTop = rememberScrollPosition();
    navigate(buildSessionDetailPath(tool, session.sourcePath), {
      state: {
        from: location.pathname + location.search,
        fromScrollTop,
      },
    });
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

  const performDeleteSession = async (session: SessionMeta, visibleContextId: number) => {
    await deleteToolSession(tool, session.sourcePath);

    await loadSessions({
      forceRefresh: true,
      loadMode: 'refresh',
      background: true,
      refreshReason: 'manual-refresh',
      trigger: 'delete-refresh',
    });
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

    await loadSessions({
      forceRefresh: true,
      loadMode: 'refresh',
      background: true,
      refreshReason: 'manual-refresh',
      trigger: 'bulk-delete-refresh',
    });

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

  const performBulkExportSessions = async (
    exportDir: string,
    visibleContextId: number,
    exportFormat: SessionExportFormat,
  ): Promise<ExportToolSessionsResult> => {
    const result = await exportToolSessions(tool, selectedSourcePaths, exportDir, exportFormat);
    const failedSourcePathSet = new Set(result.failedItems.map((item) => item.sourcePath));

    if (result.exportedCount > 0 && shouldShowVisibleFeedback(visibleContextId)) {
      message.success(t('sessionManager.bulkExportSuccess', { count: result.exportedCount }));
    }

    if (result.failedItems.length > 0 && shouldShowVisibleFeedback(visibleContextId)) {
      const firstFailure = result.failedItems[0];
      const errorSummary = result.failedItems.length === 1
        ? firstFailure.error
        : t('sessionManager.bulkExportPartialFailure', { count: result.failedItems.length, error: firstFailure.error });
      message.error(errorSummary || t('common.error'));
    }

    if (result.failedItems.length === 0) {
      exitSelectionMode();
      return result;
    }

    setSelectedSourcePaths((current) => current.filter((sourcePath) => failedSourcePathSet.has(sourcePath)));
    return result;
  };

  const handleBulkExportSessions = async () => {
    if (selectedSourcePaths.length === 0) {
      return;
    }

    let exportFormat: SessionExportFormat = 'ai_toolbox';
    if (tool === 'grok') {
      const selectedFormat = await new Promise<SessionExportFormat | null>((resolve) => {
        let currentFormat: SessionExportFormat = 'ai_toolbox';
        Modal.confirm({
          title: t('sessionManager.grokExportFormatTitle'),
          content: (
            <Radio.Group
              defaultValue={currentFormat}
              onChange={(event) => {
                currentFormat = event.target.value as SessionExportFormat;
              }}
            >
              <Radio value="ai_toolbox">{t('sessionManager.grokExportAiToolbox')}</Radio>
              <Radio value="grok_markdown">{t('sessionManager.grokExportMarkdown')}</Radio>
              <Radio value="grok_native">{t('sessionManager.grokExportNative')}</Radio>
            </Radio.Group>
          ),
          okText: t('common.confirm'),
          cancelText: t('common.cancel'),
          onOk: () => resolve(currentFormat),
          onCancel: () => resolve(null),
        });
      });
      if (!selectedFormat) {
        return;
      }
      exportFormat = selectedFormat;
    }

    let selectedExportDir: string | null = null;

    try {
      const selected = await open({
        multiple: false,
        directory: true,
        title: t('sessionManager.bulkExportDialogTitle'),
      });

      if (!selected || Array.isArray(selected)) {
        return;
      }

      selectedExportDir = selected;
    } catch (error) {
      if (!shouldShowVisibleFeedback()) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
      return;
    }

    const visibleContextId = captureVisibleContextId();

    try {
      setBulkExporting(true);
      await performBulkExportSessions(selectedExportDir, visibleContextId, exportFormat);
    } catch (error) {
      if (!shouldShowVisibleFeedback(visibleContextId)) {
        return;
      }
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    } finally {
      setBulkExporting(false);
    }
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

  const renderRuntimeSourceTag = (session: SessionMeta) => {
    if (!showRuntimeSourceTag || !session.runtimeSource) {
      return null;
    }

    const label = session.runtimeSource === 'wsl'
      ? session.runtimeDistro
        ? t('sessionManager.sourceMode.wslWithDistro', { distro: session.runtimeDistro })
        : t('sessionManager.sourceMode.wsl')
      : t('sessionManager.sourceMode.local');

    return (
      <Tag
        bordered={false}
        className={session.runtimeSource === 'wsl' ? styles.runtimeSourceTagWsl : styles.runtimeSourceTagLocal}
      >
        {label}
      </Tag>
    );
  };

  const showListOverlay = loading && (
    items.length === 0 || metadataRefreshReason === 'manual-refresh'
  );
  const statusHint = debouncedQuery
    ? !metaComplete
      ? t('sessionManager.searchWaitingForFullList')
      : messageSearchRunning
        ? t('sessionManager.searchingMessageContent')
        : null
    : cacheState === 'stale'
      ? t('sessionManager.usingCachedSessions')
      : null;

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
              {partial
                ? t('sessionManager.recentSessionsLoaded', { count: items.length })
                : t('sessionManager.totalSessions', { count: total })}
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
                className={styles.actionButton}
                icon={<ExportOutlined />}
                disabled={selectedSourcePaths.length === 0}
                loading={bulkExporting}
                onClick={() => void handleBulkExportSessions()}
              >
                {t('sessionManager.bulkExport', { count: selectedSourcePaths.length })}
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
          {!selectionMode ? (
            <>
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
            </>
          ) : null}
        </div>

        {statusHint ? (
          <Text className={styles.statusHint}>
            {statusHint}
          </Text>
        ) : null}

        <Spin spinning={showListOverlay}>
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

                      handleOpenDetail(session);
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
                          {renderRuntimeSourceTag(session)}
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

        {loadingFullList ? (
          <div className={styles.fullListLoading}>
            <Spin size="small" />
            <Text className={styles.fullListLoadingText}>
              {t('sessionManager.loadingFullList')}
            </Text>
          </div>
        ) : null}
      </div>

    </>
  );
};

const SessionManagerPanel: React.FC<SessionManagerPanelProps> = ({
  tool,
  translationKey = 'sessionManager.title',
  expandNonce = 0,
  refreshNonce = 0,
  extra,
  sourceMode: controlledSourceMode,
  onSourceModeChange,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = React.useState(false);
  const [uncontrolledSourceMode, setUncontrolledSourceMode] = React.useState<SessionSourceMode>(() => rememberedSessionSourceMode);
  const [availableSources, setAvailableSources] = React.useState<SessionSourceOption[]>([]);
  const [availableSourcesResolved, setAvailableSourcesResolved] = React.useState(false);
  const [metadataRefreshReason, setMetadataRefreshReason] = React.useState<MetadataRefreshReason>(null);
  const [manualRefreshNonce, setManualRefreshNonce] = React.useState(0);
  const sourceMode = controlledSourceMode ?? uncontrolledSourceMode;

  const handleAvailableSourcesChange = React.useCallback((sources: SessionSourceOption[]) => {
    setAvailableSources(sources);
    setAvailableSourcesResolved(true);
  }, []);

  const hasLocalSource = availableSources.some((item) => item.source === 'local');
  const hasWslSource = availableSources.some((item) => item.source === 'wsl');
  const showSourceSwitcher = hasLocalSource && hasWslSource;
  const effectiveSourceMode = resolveEffectiveSessionSourceMode(sourceMode, availableSources);

  const handleSourceModeChange = React.useCallback((value: string | number) => {
    const nextSourceMode = value as SessionSourceMode;
    rememberedSessionSourceMode = nextSourceMode;
    if (controlledSourceMode === undefined) {
      setUncontrolledSourceMode(nextSourceMode);
    }
    onSourceModeChange?.(nextSourceMode);
  }, [controlledSourceMode, onSourceModeChange]);

  React.useEffect(() => {
    if (!availableSourcesResolved || sourceMode === effectiveSourceMode) {
      return;
    }

    rememberedSessionSourceMode = effectiveSourceMode;
    if (controlledSourceMode === undefined) {
      setUncontrolledSourceMode(effectiveSourceMode);
    }
    onSourceModeChange?.(effectiveSourceMode);
  }, [
    availableSourcesResolved,
    controlledSourceMode,
    effectiveSourceMode,
    onSourceModeChange,
    sourceMode,
  ]);

  React.useEffect(() => {
    if (expandNonce <= 0) {
      return;
    }

    setExpanded(true);
  }, [expandNonce]);

  const sourceModeOptions = React.useMemo(() => [
    { label: t('sessionManager.sourceMode.all'), value: 'all' as const },
    { label: t('sessionManager.sourceMode.local'), value: 'local' as const },
    { label: t('sessionManager.sourceMode.wsl'), value: 'wsl' as const },
  ], [t]);

  const sourceSwitcher = showSourceSwitcher ? (
    <div className={styles.sourceSegmented} role="tablist" aria-label={t('sessionManager.title')}>
      {sourceModeOptions.map((option) => {
        const selected = sourceMode === option.value;
        return (
          <button
            key={option.value}
            type="button"
            role="tab"
            aria-selected={selected}
            className={`${styles.sourceSegmentButton}${selected ? ` ${styles.sourceSegmentButtonActive}` : ''}`}
            onClick={() => handleSourceModeChange(option.value)}
          >
            <span>{option.label}</span>
          </button>
        );
      })}
    </div>
  ) : null;

  const metadataRefreshText = metadataRefreshReason
    ? t('sessionManager.refreshingSessions')
    : null;
  const metadataRefreshing = metadataRefreshReason !== null;

  const refreshControl = (
    <div className={styles.headerRefreshControl}>
      {metadataRefreshText ? (
        <Text className={styles.headerRefreshText}>
          <Spin size="small" />
          <span>{metadataRefreshText}</span>
        </Text>
      ) : null}
      <Tooltip
        title={metadataRefreshing
          ? t('sessionManager.refreshingSessions')
          : t('sessionManager.refreshSessions')}
      >
        <Button
          type="text"
          size="small"
          className={styles.headerRefreshButton}
          icon={<ReloadOutlined />}
          loading={metadataRefreshing}
          disabled={metadataRefreshing}
          onClick={() => {
            setExpanded(true);
            setManualRefreshNonce((value) => value + 1);
          }}
        />
      </Tooltip>
    </div>
  );

  const headerExtra = sourceSwitcher || extra || refreshControl ? (
    <div
      className={styles.headerExtra}
      onClick={(event) => event.stopPropagation()}
      onMouseDown={(event) => event.stopPropagation()}
    >
      {extra}
      {sourceSwitcher}
      {refreshControl}
    </div>
  ) : null;

  return (
    <Collapse
      className={styles.collapseCard}
      destroyOnHidden
      activeKey={expanded ? ['session-manager'] : []}
      onChange={(keys) => {
        const nextExpanded = keys.includes('session-manager');
        setExpanded(nextExpanded);
        if (!nextExpanded) {
          setMetadataRefreshReason(null);
          setManualRefreshNonce(0);
        }
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
          extra: headerExtra,
          children: (
            <SessionManagerContent
              tool={tool}
              expanded={expanded}
              refreshNonce={refreshNonce}
              manualRefreshNonce={manualRefreshNonce}
              sourceMode={effectiveSourceMode}
              showRuntimeSourceTag={showSourceSwitcher && effectiveSourceMode === 'all'}
              onAvailableSourcesChange={handleAvailableSourcesChange}
              onMetadataRefreshStateChange={setMetadataRefreshReason}
            />
          ),
        },
      ]}
    />
  );
};

export default SessionManagerPanel;
