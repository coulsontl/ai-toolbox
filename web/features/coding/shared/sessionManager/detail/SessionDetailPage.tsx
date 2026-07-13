import React from 'react';
import {
  Empty,
  Form,
  Input,
  Modal,
  Radio,
  message,
} from 'antd';
import { AlertCircle, Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import { save } from '@tauri-apps/plugin-dialog';

import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import SecondaryPageShell from '@/components/layout/SecondaryPageShell';
import {
  deleteToolSession,
  exportToolSession,
  getToolSessionDetail,
  getToolSubagentSessionDetail,
  listToolSessionSubagents,
  renameToolSession,
} from '../sessionManagerApi';
import {
  buildSessionDetailPath,
  dispatchSessionManagerRefresh,
  getSessionToolBasePath,
  parseSessionDetailSearchParams,
} from '../sessionDetailNavigation';
import type {
  SessionDetail,
  SessionExportFormat,
  SessionMeta,
  SessionSubagentMeta,
  SessionTool,
} from '../types';
import {
  advanceVisibleContextId,
  formatSessionTitle,
  shouldShowVisibleFeedback as shouldShowVisibleFeedbackForContext,
} from '../utils';
import SessionDetailWorkbench from './SessionDetailWorkbench';
import styles from './SessionDetailPage.module.less';

interface SessionDetailPageProps {
  tool: SessionTool;
}

interface SessionDetailLocationState {
  from?: string;
  fromScrollTop?: number;
}

const canRenameSession = (tool: SessionTool) => (
  tool === 'opencode' || tool === 'codex' || tool === 'pi'
);

const buildSessionExportFileName = (
  tool: SessionTool,
  session: SessionMeta,
  exportFormat: SessionExportFormat,
) => `${tool}-session-${session.sessionId}.${exportFormat === 'grok_markdown' ? 'md' : 'json'}`;

const SessionDetailPage: React.FC<SessionDetailPageProps> = ({ tool }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const [searchParams] = useSearchParams();
  const { isActive } = useKeepAlive();
  const [detail, setDetail] = React.useState<SessionDetail | null>(null);
  const [subagentSessions, setSubagentSessions] = React.useState<SessionSubagentMeta[]>([]);
  const [loading, setLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [exporting, setExporting] = React.useState(false);
  const [renameModalOpen, setRenameModalOpen] = React.useState(false);
  const [renaming, setRenaming] = React.useState(false);
  const [reloadNonce, setReloadNonce] = React.useState(0);
  const requestIdRef = React.useRef(0);
  const activePageRef = React.useRef(isActive);
  const visibleContextIdRef = React.useRef(0);
  const [renameForm] = Form.useForm<{ title: string }>();

  visibleContextIdRef.current = advanceVisibleContextId(
    visibleContextIdRef.current,
    activePageRef.current,
    isActive,
  );
  activePageRef.current = isActive;

  const routeParams = React.useMemo(
    () => parseSessionDetailSearchParams(searchParams),
    [searchParams],
  );
  const detailLocationState = React.useMemo(
    () => getSessionDetailLocationState(location.state),
    [location.state],
  );
  const isSubagentDetail = Boolean(routeParams?.subagentSourcePath);
  const basePath = getSessionToolBasePath(tool);
  const sourcePath = routeParams?.sourcePath;
  const pageTitle = detail ? formatSessionTitle(detail.meta) : t('sessionManager.detailTitle');

  const shouldShowVisibleFeedback = React.useCallback((visibleContextId?: number) => {
    return shouldShowVisibleFeedbackForContext(
      activePageRef.current,
      visibleContextId,
      visibleContextIdRef.current,
    );
  }, []);

  const captureVisibleContextId = React.useCallback(() => visibleContextIdRef.current, []);

  const handleBackToList = React.useCallback(() => {
    navigate(detailLocationState?.from || basePath, {
      replace: false,
      state: buildRestoreLocationState(detailLocationState?.fromScrollTop),
    });
  }, [basePath, detailLocationState, navigate]);

  React.useEffect(() => {
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoadError(null);

    if (!routeParams) {
      setDetail(null);
      setSubagentSessions([]);
      setLoading(false);
      return undefined;
    }

    let disposed = false;
    const visibleContextId = captureVisibleContextId();

    const loadDetail = async () => {
      setLoading(true);
      setDetail(null);
      setSubagentSessions([]);
      try {
        if (routeParams.subagentSourcePath) {
          const result = await getToolSubagentSessionDetail(
            tool,
            routeParams.sourcePath,
            routeParams.subagentSourcePath,
          );
          if (disposed || requestId !== requestIdRef.current) {
            return;
          }
          setDetail(result);
          setSubagentSessions([]);
          return;
        }

        const [result, subagents] = await Promise.all([
          getToolSessionDetail(tool, routeParams.sourcePath),
          listToolSessionSubagents(tool, routeParams.sourcePath),
        ]);
        if (disposed || requestId !== requestIdRef.current) {
          return;
        }
        setDetail(result);
        setSubagentSessions(subagents);
      } catch (error) {
        if (disposed || requestId !== requestIdRef.current) {
          return;
        }
        const errorMessage = error instanceof Error ? error.message : String(error);
        setLoadError(errorMessage || t('common.error'));
        if (shouldShowVisibleFeedback(visibleContextId)) {
          message.error(errorMessage || t('common.error'));
        }
      } finally {
        if (!disposed && requestId === requestIdRef.current) {
          setLoading(false);
        }
      }
    };

    void loadDetail();

    return () => {
      disposed = true;
    };
  }, [
    captureVisibleContextId,
    reloadNonce,
    routeParams,
    shouldShowVisibleFeedback,
    t,
    tool,
  ]);

  const handleCopyText = async (text: string, successText: string) => {
    try {
      await navigator.clipboard.writeText(text);
      message.success(successText);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    }
  };

  const handleOpenSubagentDetail = React.useCallback((subagent: SessionSubagentMeta) => {
    if (!sourcePath) {
      return;
    }
    navigate(buildSessionDetailPath(tool, sourcePath, subagent.sourcePath), {
      state: buildSessionDetailLocationState(
        detailLocationState?.from || basePath,
        detailLocationState?.fromScrollTop,
      ),
    });
  }, [basePath, detailLocationState, navigate, sourcePath, tool]);

  const handleBackToParentDetail = React.useCallback(() => {
    if (!sourcePath) {
      handleBackToList();
      return;
    }
    navigate(buildSessionDetailPath(tool, sourcePath), {
      replace: false,
      state: buildSessionDetailLocationState(
        detailLocationState?.from || basePath,
        detailLocationState?.fromScrollTop,
      ),
    });
  }, [basePath, detailLocationState, handleBackToList, navigate, sourcePath, tool]);

  const handleExportSession = async (sessionDetail: SessionDetail) => {
    const exportMessageKey = `session-export-${tool}`;
    const visibleContextId = captureVisibleContextId();
    try {
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
      const exportPath = await save({
        title: t('sessionManager.exportDialogTitle'),
        defaultPath: buildSessionExportFileName(tool, sessionDetail.meta, exportFormat),
        filters: [
          {
            name: exportFormat === 'grok_markdown' ? 'Markdown' : 'JSON',
            extensions: [exportFormat === 'grok_markdown' ? 'md' : 'json'],
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
      await exportToolSession(tool, sessionDetail.meta.sourcePath, exportPath, exportFormat);
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

  const openRenameModal = () => {
    if (!detail || !canRenameSession(tool) || isSubagentDetail) {
      return;
    }
    renameForm.setFieldsValue({
      title: detail.meta.title?.trim() || '',
    });
    setRenameModalOpen(true);
  };

  const handleRenameSession = async () => {
    if (!detail || !canRenameSession(tool) || isSubagentDetail) {
      return;
    }

    const visibleContextId = captureVisibleContextId();
    try {
      const values = await renameForm.validateFields();
      setRenaming(true);
      await renameToolSession(tool, detail.meta.sourcePath, values.title);
      dispatchSessionManagerRefresh(tool);
      if (shouldShowVisibleFeedback(visibleContextId)) {
        message.success(t('sessionManager.renameSuccess'));
      }
      setRenameModalOpen(false);
      setReloadNonce((current) => current + 1);
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
      okText: t('common.delete'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel'),
      onOk: async () => {
        const visibleContextId = captureVisibleContextId();
        try {
          await deleteToolSession(tool, session.sourcePath);
          dispatchSessionManagerRefresh(tool);
          if (shouldShowVisibleFeedback(visibleContextId)) {
            message.success(t('sessionManager.deleteSuccess'));
          }
          handleBackToList();
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
      <SecondaryPageShell
        title={pageTitle}
        subtitle={
          isSubagentDetail
            ? t('sessionManager.subagentDetailSubtitle')
            : t('sessionManager.detailPageSubtitle')
        }
        backLabel={t('sessionManager.backToSessionList')}
        onBack={handleBackToList}
      >
        {routeParams ? (
          <>
            {loadError ? (
              <div className={styles.errorState}>
                <AlertCircle size={24} aria-hidden="true" />
                <span>{loadError}</span>
              </div>
            ) : detail ? (
              <SessionDetailWorkbench
                detail={detail}
                subagents={subagentSessions}
                isSubagentDetail={isSubagentDetail}
                exporting={exporting}
                canRename={canRenameSession(tool) && !isSubagentDetail}
                canExport={!isSubagentDetail}
                canDelete={!isSubagentDetail}
                t={t}
                onRename={openRenameModal}
                onExport={() => void handleExportSession(detail)}
                onDelete={() => handleDeleteSession(detail.meta)}
                onOpenSubagent={handleOpenSubagentDetail}
                onBackToParent={handleBackToParentDetail}
                onCopyText={handleCopyText}
              />
            ) : loading ? (
              <div className={styles.loadingState}>
                <Loader2 size={22} className={styles.loadingIcon} aria-hidden="true" />
                <span>{t('common.loading')}</span>
              </div>
            ) : (
              <Empty description={t('sessionManager.emptyDetail')} />
            )}
          </>
        ) : (
          <Empty description={t('sessionManager.invalidDetailRoute')} />
        )}
      </SecondaryPageShell>

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

function getSessionDetailLocationState(state: unknown): SessionDetailLocationState | null {
  if (!state || typeof state !== 'object') {
    return null;
  }

  const candidate = state as { from?: unknown; fromScrollTop?: unknown };
  return {
    ...(typeof candidate.from === 'string' ? { from: candidate.from } : {}),
    ...(typeof candidate.fromScrollTop === 'number' && Number.isFinite(candidate.fromScrollTop)
      ? { fromScrollTop: candidate.fromScrollTop }
      : {}),
  };
}

function buildSessionDetailLocationState(
  from: string,
  fromScrollTop?: number,
): SessionDetailLocationState {
  return {
    from,
    ...(typeof fromScrollTop === 'number' && Number.isFinite(fromScrollTop) ? { fromScrollTop } : {}),
  };
}

function buildRestoreLocationState(fromScrollTop?: number): { restoreScrollTop: number } | undefined {
  return typeof fromScrollTop === 'number' && Number.isFinite(fromScrollTop)
    ? { restoreScrollTop: fromScrollTop }
    : undefined;
}

export const OpenCodeSessionDetailPage = () => <SessionDetailPage tool="opencode" />;
export const ClaudeCodeSessionDetailPage = () => <SessionDetailPage tool="claudecode" />;
export const CodexSessionDetailPage = () => <SessionDetailPage tool="codex" />;
export const GrokSessionDetailPage = () => <SessionDetailPage tool="grok" />;
export const OpenClawSessionDetailPage = () => <SessionDetailPage tool="openclaw" />;
export const GeminiCliSessionDetailPage = () => <SessionDetailPage tool="geminicli" />;
export const PiSessionDetailPage = () => <SessionDetailPage tool="pi" />;

export default SessionDetailPage;
