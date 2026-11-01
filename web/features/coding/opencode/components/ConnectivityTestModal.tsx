import React from 'react';
import { Modal, Form, Input, InputNumber, Button, Collapse, Table, Tag, Space, Tooltip, message, Switch, Typography, Checkbox, Popconfirm, Select, type TableProps } from 'antd';
import { CaretRightOutlined, SettingOutlined, InfoCircleOutlined, DeleteOutlined } from '@ant-design/icons';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import {
  testProviderModelConnectivity,
  type ConnectivityTestRequest,
  type ConnectivityTestResult,
  type OpenCodeDiagnosticsConfig,
} from '@/services/opencodeApi';
import {
  testGatewayProviderModelConnectivity,
  type GatewayConnectivityTestRequest,
} from '@/services/proxyGatewayApi';
import type { OpenCodeProvider } from '@/types/opencode';
import {
  buildTokenCapFields,
  resolveModelConnection,
  type ProviderModelConnections,
} from '@/features/coding/shared/providerConnectivity/modelConnection';
import {
  declaredVisionCapability,
  visionProbeMismatchesDeclaration,
  type VisionProbeStatus,
} from '@/features/coding/shared/providerConnectivity/visionProbe';
import {
  canOfferModelRemoval,
  selectRemovableModelIds,
} from '@/features/coding/shared/providerConnectivity/removableModels';
import styles from './ConnectivityTestModal.module.less';


interface ConnectivityTestModalProps {
  open: boolean;
  onCancel: () => void;
  providerId: string;
  providerName: string;
  providerConfig: OpenCodeProvider;
  apiFormat?: ConnectivityTestRequest['apiFormat'];
  configValueMode?: ConnectivityTestRequest['configValueMode'];
  /** Per-model connection overrides (OMP `models.yml` allows per-model api/baseUrl). */
  modelConnections?: ProviderModelConnections;
  modelIds: string[];
  /**
   * Models the caller is willing to delete. Omit it and the modal offers no
   * removal at all — being testable is not the same as being removable (the
   * probe also covers config-only ids and official read-only entries).
   */
  removableModelIds?: string[];
  diagnostics?: OpenCodeDiagnosticsConfig;
  onSaveDiagnostics: (diagnostics: OpenCodeDiagnosticsConfig) => Promise<void>;
  onRemoveModels?: (modelIds: string[]) => Promise<void>;
  gatewayRequest?: Pick<GatewayConnectivityTestRequest, 'cliKey' | 'providerId'>;
}

interface TestResult extends Partial<ConnectivityTestResult> {
  key: string;
  modelId: string;
  status: string;
  loading?: boolean;
}

const SUPPORTED_NPMS = [
  '@ai-sdk/openai',
  '@ai-sdk/openai-compatible',
  '@ai-sdk/google',
  '@ai-sdk/anthropic',
];

const ConnectivityTestModal: React.FC<ConnectivityTestModalProps> = ({
  open,
  onCancel,
  providerId,
  providerName,
  providerConfig,
  apiFormat,
  configValueMode,
  modelConnections,
  modelIds,
  removableModelIds,
  diagnostics,
  onSaveDiagnostics,
  onRemoveModels,
  gatewayRequest,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [testing, setTesting] = React.useState(false);
  const [results, setResults] = React.useState<TestResult[]>([]);
  const [advancedActive, setAdvancedActive] = React.useState<string | string[]>([]);
  // 统一的选择状态：测试前用于选择要测试的模型，测试后用于选择要删除的模型
  const [selectedModelIds, setSelectedModelIds] = React.useState<string[]>([]);
  const [removing, setRemoving] = React.useState(false);
  // Removal is opt-in per model: a caller that does not say which ids are
  // deletable gets no removal button, instead of one that deletes everything
  // it just probed.
  const removableModelIdSet = React.useMemo(
    () => new Set(removableModelIds ?? []),
    [removableModelIds],
  );
  const canRemoveModels = canOfferModelRemoval(onRemoveModels, removableModelIds);
  
  // JSON editor states
  const [headersJson, setHeadersJson] = React.useState<unknown>({});
  const [headersValid, setHeadersValid] = React.useState(true);
  const [bodyJson, setBodyJson] = React.useState<unknown>({});
  const [bodyValid, setBodyValid] = React.useState(true);
  const [visionProbe, setVisionProbe] = React.useState(false);

  // Details modal state
  const [detailsModalOpen, setDetailsModalOpen] = React.useState(false);
  const [selectedResult, setSelectedResult] = React.useState<TestResult | null>(null);
  const [defaultTestModelId, setDefaultTestModelId] = React.useState<string | undefined>(undefined);
  // The config section collapses by default so the model list is visible on open.
  const [configExpanded, setConfigExpanded] = React.useState(false);
  const promptValue = Form.useWatch('prompt', form) as string | undefined;

  // Track if modal was just opened (to avoid re-initializing on diagnostics change)
  const prevOpenRef = React.useRef(false);

  const resolvedDefaultTestModelId = React.useMemo(() => {
    if (diagnostics?.defaultTestModelId && modelIds.includes(diagnostics.defaultTestModelId)) {
      return diagnostics.defaultTestModelId;
    }

    return undefined;
  }, [diagnostics?.defaultTestModelId, modelIds]);

  const persistDefaultTestModel = React.useCallback(async (nextDefaultTestModelId?: string) => {
    await onSaveDiagnostics({
      prompt: diagnostics?.prompt ?? form.getFieldValue('prompt') ?? 'say hi!',
      defaultTestModelId: nextDefaultTestModelId,
      ...(diagnostics?.temperature !== undefined ? { temperature: diagnostics.temperature } : {}),
      ...(diagnostics?.maxTokens !== undefined ? { maxTokens: diagnostics.maxTokens } : {}),
      ...(diagnostics?.maxOutputTokens !== undefined ? { maxOutputTokens: diagnostics.maxOutputTokens } : {}),
      ...(diagnostics?.stream !== undefined ? { stream: diagnostics.stream } : {}),
      ...(diagnostics?.headers ? { headers: diagnostics.headers } : {}),
      ...(diagnostics?.body ? { body: diagnostics.body } : {}),
      visionProbe: diagnostics?.visionProbe ?? visionProbe,
    });
  }, [onSaveDiagnostics, form, diagnostics, visionProbe]);

  // Initialize form with diagnostics prop - only when modal opens
  React.useEffect(() => {
    // Only initialize when modal transitions from closed to open
    if (open && !prevOpenRef.current) {
      form.setFieldsValue({
        prompt: diagnostics?.prompt || 'say hi!',
        temperature: apiFormat ? undefined : diagnostics?.temperature,
        maxTokens: apiFormat ? undefined : diagnostics?.maxTokens ?? diagnostics?.maxOutputTokens,
        stream: apiFormat ? true : diagnostics?.stream ?? true,
      });

      setHeadersJson(diagnostics?.headers || {});
      setBodyJson(diagnostics?.body || {});
      setVisionProbe(diagnostics?.visionProbe ?? false);
      setDefaultTestModelId(resolvedDefaultTestModelId);
      setSelectedModelIds(resolvedDefaultTestModelId ? [resolvedDefaultTestModelId] : []);
      // The modal stays mounted between opens, so reset the collapsed config.
      setConfigExpanded(false);

      setResults(modelIds.map(id => ({
        key: id,
        modelId: id,
        status: 'pending',
        requestUrl: '',
        requestHeaders: {},
        requestBody: {},
        responseHeaders: undefined,
        responseBody: undefined,
      })));
    }
    prevOpenRef.current = open;
  }, [open, diagnostics, modelIds, form, resolvedDefaultTestModelId, apiFormat]);

  const handleDefaultTestModelChange = React.useCallback(async (modelId?: string) => {
    setDefaultTestModelId(modelId);
    setSelectedModelIds(modelId ? [modelId] : []);

    try {
      await persistDefaultTestModel(modelId);
    } catch (error) {
      console.error('Failed to save default test model:', error);
      message.error(t('common.error'));
    }
  }, [persistDefaultTestModel, t]);

  const toggleConfigExpanded = React.useCallback(() => {
    setConfigExpanded((expanded) => !expanded);
  }, []);

  const handleConfigHeaderKeyDown = React.useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    // Only react to keys on the header itself, not on controls inside it.
    if (event.currentTarget !== event.target) {
      return;
    }

    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      toggleConfigExpanded();
    }
  }, [toggleConfigExpanded]);

  const handleRunTest = async () => {
    // 检查是否选中了要测试的模型
    if (selectedModelIds.length === 0) {
      message.warning(t('opencode.connectivity.noModelSelected'));
      return;
    }

    // 保存本次要测试的模型列表
    const modelsToTest = [...selectedModelIds];

    try {
      const values = await form.validateFields();
      
      if (!headersValid || !bodyValid) {
        message.error(t('opencode.connectivity.invalidJson'));
        return;
      }

      const headersHasValue = headersJson !== undefined && headersJson !== null && (
        typeof headersJson !== 'string' || headersJson.trim() !== ''
      );
      const headersIsObject = headersJson && typeof headersJson === 'object' && !Array.isArray(headersJson);
      if (headersHasValue && !headersIsObject) {
        message.error(t('opencode.connectivity.invalidJson'));
        return;
      }

      const bodyHasValue = bodyJson !== undefined && bodyJson !== null && (
        typeof bodyJson !== 'string' || bodyJson.trim() !== ''
      );
      const bodyIsObject = bodyJson && typeof bodyJson === 'object' && !Array.isArray(bodyJson);
      if (bodyHasValue && !bodyIsObject) {
        message.error(t('opencode.connectivity.invalidJson'));
        return;
      }

      setTesting(true);

      // 立即更新选中模型的状态为 running
      setResults(prev => prev.map(r => {
        if (modelsToTest.includes(r.modelId)) {
          return { ...r, status: 'running', loading: true, firstByteMs: undefined, totalMs: undefined, errorMessage: undefined };
        }
        return r;
      }));

      // 1. Save diagnostics configuration
      const npm = providerConfig.npm || '@ai-sdk/openai-compatible';

      const headersObject = (headersJson && typeof headersJson === 'object' && !Array.isArray(headersJson))
        ? (headersJson as Record<string, unknown>)
        : undefined;
      const bodyObject = (bodyJson && typeof bodyJson === 'object' && !Array.isArray(bodyJson))
        ? (bodyJson as Record<string, unknown>)
        : undefined;

      const newDiagnostics: OpenCodeDiagnosticsConfig = {
        prompt: values.prompt,
        defaultTestModelId,
        stream: values.stream,
        ...(values.temperature !== undefined ? { temperature: values.temperature } : {}),
        ...buildTokenCapFields(npm, values.maxTokens),
        ...(headersObject ? { headers: headersObject } : {}),
        ...(bodyObject ? { body: bodyObject } : {}),
        visionProbe,
      };

      await onSaveDiagnostics(newDiagnostics);

      const providerHeaders = (providerConfig.options?.headers as Record<string, unknown>) || {};
      const mergedHeaders = { ...providerHeaders, ...(headersObject || {}) };

      const baseRequest: ConnectivityTestRequest = {
        npm,
        ...(apiFormat ? { apiFormat } : {}),
        ...(configValueMode ? { configValueMode } : {}),
        providerId,
        baseUrl: providerConfig.options?.baseURL || '',
        apiKey: providerConfig.options?.apiKey,
        ...(providerConfig.options?.reasoningEffort
          ? { reasoningEffort: String(providerConfig.options.reasoningEffort) }
          : {}),
        prompt: values.prompt,
        stream: values.stream,
        ...(values.temperature !== undefined ? { temperature: values.temperature } : {}),
        // The token cap is added per model below: the field name depends on the
        // SDK that actually serves the request, which a model may override.
        ...(Object.keys(mergedHeaders).length > 0 ? { headers: mergedHeaders } : {}),
        ...(bodyObject ? { body: bodyObject } : {}),
        modelIds: [],
        timeoutSecs: 30,
        visionProbe,
      };

      // Run tests in parallel (streaming effect) - only for selected models
      const failedModelIds: string[] = [];
      const promises = modelsToTest.map(async (modelId) => {
        try {
          // A model may override the provider's connection (OMP `models.yml`), so
          // each test follows the model it is probing instead of the provider.
          const modelRequest = resolveModelConnection(baseRequest, modelId, modelConnections);
          const request = { ...modelRequest, ...buildTokenCapFields(modelRequest.npm, values.maxTokens) };
          const response = gatewayRequest
            ? await testGatewayProviderModelConnectivity({
                ...gatewayRequest,
                prompt: values.prompt,
                stream: values.stream,
                modelIds: [modelId],
                timeoutSecs: 30,
                visionProbe,
              })
            : await testProviderModelConnectivity({
                ...request,
                modelIds: [modelId],
              });

          const result = response.results[0];

          if (result.status !== 'success') {
            failedModelIds.push(modelId);
          }

          setResults(prev => prev.map(r => {
            if (r.modelId === modelId) {
              return { ...result, key: modelId, loading: false };
            }
            return r;
          }));
        } catch (error: unknown) {
          failedModelIds.push(modelId);
          // Tauri rejects with the backend error string, so `error.message`
          // alone would hide messages such as an unresolvable provider API key.
          const errorMessage = error instanceof Error ? error.message : String(error);
          setResults(prev => prev.map(r => {
            if (r.modelId === modelId) {
              return {
                key: modelId,
                modelId,
                status: 'error',
                errorMessage: errorMessage || 'Unknown error',
                loading: false,
                requestUrl: '',
                requestHeaders: {},
                requestBody: {},
                responseHeaders: undefined,
                responseBody: undefined,
              };
            }
            return r;
          }));
        }
      });

      await Promise.all(promises);

      // 测试完成后处理勾选状态
      if (failedModelIds.length > 0) {
        // 有失败的：自动选中可删除的失败模型（用于删除）
        setSelectedModelIds(
          canRemoveModels
            ? selectRemovableModelIds(failedModelIds, removableModelIds)
            : failedModelIds,
        );
      }
      // 没有失败的：保持用户的勾选状态不变

    } catch (error) {
      // A validation rejection can only come from the collapsed config section,
      // so expand it — otherwise the invalid field stays invisible.
      if (error && typeof error === 'object' && 'errorFields' in error) {
        setConfigExpanded(true);
        return;
      }

      console.error('Test failed:', error);
      message.error(t('common.error'));
    } finally {
      setTesting(false);
    }
  };

  const handleShowDetails = (record: TestResult) => {
    setSelectedResult(record);
    setDetailsModalOpen(true);
  };

  const handleRemoveModels = async () => {
    if (!onRemoveModels || selectedModelIds.length === 0) return;

    const readonlyIds = selectedModelIds.filter((modelId) => !removableModelIdSet.has(modelId));
    if (readonlyIds.length > 0) {
      message.warning(t('opencode.connectivity.removeReadonlyHint', { count: readonlyIds.length }));
      return;
    }

    const removedIds = selectedModelIds.filter((modelId) => removableModelIdSet.has(modelId));
    if (removedIds.length === 0) return;
    setRemoving(true);
    try {
      await onRemoveModels(removedIds);
      // 从 results 中移除已删除的模型
      setResults(prev => prev.filter(r => !removedIds.includes(r.modelId)));
      setSelectedModelIds([]);
      message.success(t('opencode.connectivity.removeSuccess', { count: removedIds.length }));
      // Removal completed the cleanup task — close the modal so users don't need an extra click.
      onCancel();
    } catch {
      message.error(t('common.error'));
    } finally {
      setRemoving(false);
    }
  };

  const handleSelectModel = (modelId: string, checked: boolean) => {
    if (checked) {
      setSelectedModelIds(prev => [...prev, modelId]);
    } else {
      setSelectedModelIds(prev => prev.filter(id => id !== modelId));
    }
  };

  const handleSelectAll = (checked: boolean) => {
    if (checked) {
      setSelectedModelIds([...modelIds]);
    } else {
      setSelectedModelIds([]);
    }
  };

  const hasFailedModels = results.some(r => r.status === 'error' || r.status === 'timeout');
  // 判断测试是否完成：有结果，且所有非 pending 状态的模型都不在 loading 状态
  const testedResults = results.filter(r => r.status !== 'pending');
  const isTestCompleted = testedResults.length > 0 && testedResults.every(r => !r.loading);
  const isAllSelected = modelIds.length > 0 && selectedModelIds.length === modelIds.length;
  const isIndeterminate = selectedModelIds.length > 0 && selectedModelIds.length < modelIds.length;
  const runningCount = results.filter((result) => result.loading || result.status === 'running').length;
  const successCount = results.filter((result) => result.status === 'success').length;
  const failedCount = results.filter((result) => result.status === 'error' || result.status === 'timeout').length;
  const pendingCount = results.filter((result) => result.status === 'pending').length;
  const defaultModelOptions = modelIds.map((modelId) => ({
    label: modelId,
    value: modelId,
  }));
  const summaryItems = [
    {
      label: t('opencode.connectivity.selectedCount'),
      value: selectedModelIds.length,
    },
    {
      label: t('opencode.connectivity.running'),
      value: runningCount,
    },
    {
      label: t('opencode.connectivity.success'),
      value: successCount,
    },
    {
      label: t('opencode.connectivity.failed'),
      value: failedCount,
    },
    {
      label: t('opencode.connectivity.pending'),
      value: pendingCount,
    },
  ];

  const columns: TableProps<TestResult>['columns'] = [
    // 统一的复选框列
    {
      title: (
        <Checkbox
          checked={isAllSelected}
          indeterminate={isIndeterminate}
          onChange={(e) => handleSelectAll(e.target.checked)}
          disabled={testing}
        />
      ),
      key: 'select',
      width: 40,
      render: (_: unknown, record: TestResult) => (
        <Checkbox
          checked={selectedModelIds.includes(record.modelId)}
          onChange={(e) => handleSelectModel(record.modelId, e.target.checked)}
          disabled={testing}
        />
      ),
    },
    {
      title: t('opencode.connectivity.modelId'),
      dataIndex: 'modelId',
      key: 'modelId',
      ellipsis: true,
    },
    {
      title: t('opencode.connectivity.status'),
      dataIndex: 'status',
      key: 'status',
      width: 100,
      align: 'center',
      render: (status: string, record: TestResult) => {
        if (record.loading || status === 'running') return <Tag color="processing">{t('opencode.connectivity.running')}</Tag>;
        if (status === 'success') return <Tag color="success">{t('opencode.connectivity.success')}</Tag>;
        if (status === 'error' || status === 'timeout') return <Tooltip title={record.errorMessage}><Tag color="error">{status}</Tag></Tooltip>;
        if (status === 'pending') return <Tag color="default">{t('opencode.connectivity.pending')}</Tag>;
        return <Tag>{status}</Tag>;
      },
    },
    {
      title: t('opencode.connectivity.vision'),
      dataIndex: 'visionStatus',
      key: 'visionStatus',
      width: 100,
      align: 'center',
      render: (visionStatus: VisionProbeStatus | undefined, record: TestResult) => {
        if (record.loading || record.status === 'running') return '-';
        if (visionStatus === 'passed' || visionStatus === 'failed') {
          const declared = declaredVisionCapability(providerConfig.models?.[record.modelId]);
          const mismatched = visionProbeMismatchesDeclaration(declared, visionStatus);
          const tag = visionStatus === 'passed'
            ? <Tag color={mismatched ? 'warning' : 'success'}>{t('opencode.connectivity.visionPassed')}</Tag>
            : <Tag color="error">{t('opencode.connectivity.visionFailed')}</Tag>;
          const tooltip = [
            mismatched ? t('opencode.connectivity.visionDeclaredMismatch') : undefined,
            visionStatus === 'failed' ? record.visionError : undefined,
          ].filter(Boolean).join('\n');
          return tooltip ? <Tooltip title={tooltip}>{tag}</Tooltip> : tag;
        }
        if (visionStatus === 'unavailable') return <Tag color="default">{t('opencode.connectivity.visionUnavailable')}</Tag>;
        return '-';
      },
    },
    {
      title: t('opencode.connectivity.firstByte'),
      dataIndex: 'firstByteMs',
      key: 'firstByteMs',
      width: 120,
      align: 'right',
      render: (value: number | null | undefined) => (value === undefined || value === null ? '-' : `${value}ms`),
    },
    {
      title: t('opencode.connectivity.totalTime'),
      dataIndex: 'totalMs',
      key: 'totalMs',
      width: 160,
      align: 'right',
      render: (value: number | null | undefined, record: TestResult) => {
        if (value === undefined || value === null) return '-';
        const canShowDetails = Boolean(record.requestUrl) && !record.loading;

        return (
          <Space>
            <span>{value}ms</span>
            {canShowDetails && (
              <Button
                type="link"
                size="small"
                onClick={() => handleShowDetails(record)}
                className={styles.detailsLink}
              >
                {t('opencode.connectivity.requestDetails')}
              </Button>
            )}
          </Space>
        );
      },
    },
  ];

  const npm = providerConfig.npm || '@ai-sdk/openai-compatible';
  const isSupported = SUPPORTED_NPMS.includes(npm);

  return (
    <Modal
      className={styles.modal}
      title={t('opencode.connectivity.title', { name: providerName })}
      open={open}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.close')}
        </Button>,
        isTestCompleted && hasFailedModels && canRemoveModels && (
          <Popconfirm
            key="remove"
            title={t('opencode.connectivity.removeConfirmTitle')}
            description={t('opencode.connectivity.removeConfirmDesc', { count: selectedModelIds.length })}
            onConfirm={handleRemoveModels}
            okText={t('common.confirm')}
            cancelText={t('common.cancel')}
            disabled={selectedModelIds.length === 0}
          >
            <Button
              danger
              icon={<DeleteOutlined />}
              loading={removing}
              disabled={selectedModelIds.length === 0}
            >
              {t('opencode.connectivity.removeSelected', { count: selectedModelIds.length })}
            </Button>
          </Popconfirm>
        ),
        <Tooltip key="submit" title={!isSupported ? t('opencode.connectivity.unsupportedNpm', { npm }) : ''}>
          <Button
            type="primary"
            icon={<CaretRightOutlined />}
            loading={testing}
            onClick={handleRunTest}
            disabled={!isSupported}
          >
            {t('opencode.connectivity.startTest')}
          </Button>
        </Tooltip>
      ].filter(Boolean)}
      width={800}
      styles={{ body: { paddingBottom: 16 } }}
    >
      <div className={styles.content}>
        <section className={`${styles.sectionCard} ${configExpanded ? '' : styles.configSectionCollapsed}`}>
          <div
            className={`${styles.sectionHeader} ${styles.configHeaderToggle} ${configExpanded ? styles.configHeaderExpanded : ''}`}
            role="button"
            tabIndex={0}
            aria-expanded={configExpanded}
            onClick={toggleConfigExpanded}
            onKeyDown={handleConfigHeaderKeyDown}
          >
            <div className={styles.configHeaderMain}>
              <div className={styles.sectionTitle}>
                {configExpanded ? (
                  <ChevronDown className={styles.configCaret} aria-hidden="true" />
                ) : (
                  <ChevronRight className={styles.configCaret} aria-hidden="true" />
                )}
                {t('opencode.connectivity.configSection')}
              </div>
              {configExpanded ? (
                <Typography.Text className={styles.sectionHint}>
                  {t('opencode.connectivity.configSectionHint')}
                </Typography.Text>
              ) : (
                <Typography.Text className={styles.configSummary}>
                  <span className={styles.configSummaryPrompt}>
                    {promptValue?.replace(/\s+/g, ' ').trim() || t('common.notSet')}
                  </span>
                  <span className={styles.configSummarySeparator}>·</span>
                  <span>
                    {visionProbe
                      ? t('opencode.connectivity.summaryVisionOn')
                      : t('opencode.connectivity.summaryVisionOff')}
                  </span>
                  <span className={styles.configSummarySeparator}>·</span>
                  <span className={styles.configSummaryModel}>
                    {t('opencode.connectivity.summaryDefaultModel', {
                      model: defaultTestModelId || t('common.notSet'),
                    })}
                  </span>
                </Typography.Text>
              )}
            </div>
          </div>

          <div hidden={!configExpanded} aria-hidden={!configExpanded}>
            <Form
              className={styles.form}
              form={form}
            >
              <div className={`${styles.promptItem} ${styles.formFieldRow}`}>
                <div className={styles.fieldLabel}>
                  {t('opencode.connectivity.prompt')}
                </div>
                <div className={styles.fieldContent}>
                  <Form.Item
                    style={{ marginBottom: 0 }}
                    name="prompt"
                    rules={[{ required: true }]}
                  >
                    <Input.TextArea className={styles.promptInput} rows={3} />
                  </Form.Item>
                </div>
              </div>

              <div className={styles.defaultModelPanel}>
                <div className={styles.formFieldRow}>
                  <div className={styles.fieldLabel}>
                    {t('opencode.connectivity.visionProbe')}
                  </div>
                  <div className={styles.fieldContent}>
                    <Switch checked={visionProbe} onChange={setVisionProbe} />
                    <Typography.Text className={styles.defaultModelHelp}>
                      {t('opencode.connectivity.visionProbeHint')}
                    </Typography.Text>
                  </div>
                </div>
              </div>

              <div className={styles.defaultModelPanel}>
                <div className={styles.formFieldRow}>
                  <div className={styles.fieldLabel}>
                    {t('opencode.connectivity.defaultTestModel')}
                  </div>
                  <div className={styles.fieldContent}>
                    <Form.Item
                      className={styles.defaultModelItem}
                      required
                    >
                      <Select
                        value={defaultTestModelId}
                        options={defaultModelOptions}
                        onChange={handleDefaultTestModelChange}
                        allowClear
                        placeholder={t('opencode.connectivity.defaultTestModelPlaceholder')}
                      />
                    </Form.Item>
                    <Typography.Text className={styles.defaultModelHelp}>
                      {t('opencode.connectivity.defaultTestModelHelp')}
                    </Typography.Text>
                  </div>
                </div>
              </div>

              {!gatewayRequest ? <Collapse
                className={styles.advancedCollapse}
                ghost
                activeKey={advancedActive}
                onChange={setAdvancedActive}
                items={[
                  {
                    key: 'advanced',
                    label: (
                      <div className={styles.collapseLabel}>
                        <div className={styles.collapseTitleRow}>
                          <SettingOutlined className={styles.collapseIcon} />
                          <div className={styles.collapseTitle}>{t('opencode.connectivity.moreParams')}</div>
                          <Typography.Text className={styles.collapseHint}>
                            {t('opencode.connectivity.moreParamsHint')}
                          </Typography.Text>
                        </div>
                      </div>
                    ),
                    children: (
                      <div className={styles.advancedContent}>
                        <div className={styles.metricsRow}>
                          <div className={styles.metricsGrid}>
                            <div className={styles.metricCard}>
                              <Typography.Text className={styles.metricLabel}>
                                {t('opencode.connectivity.temperature')}
                              </Typography.Text>
                              <Form.Item
                                name="temperature"
                                style={{ marginBottom: 0 }}
                              >
                                <InputNumber disabled={!!apiFormat} min={0} max={2} step={0.1} style={{ width: '100%' }} />
                              </Form.Item>
                            </div>
                            <div className={styles.metricCard}>
                              <Typography.Text className={styles.metricLabel}>
                                {t('opencode.connectivity.maxTokens')}
                              </Typography.Text>
                              <Form.Item
                                name="maxTokens"
                                style={{ marginBottom: 0 }}
                              >
                                <InputNumber disabled={!!apiFormat} min={1} step={100} style={{ width: '100%' }} />
                              </Form.Item>
                            </div>
                            <div className={styles.metricCard}>
                              <Typography.Text className={styles.metricLabel}>
                                {t('opencode.connectivity.stream')}
                              </Typography.Text>
                              <Form.Item
                                name="stream"
                                valuePropName="checked"
                                style={{ marginBottom: 0 }}
                              >
                                <Switch disabled={!!apiFormat} />
                              </Form.Item>
                            </div>
                          </div>
                        </div>

                        <div className={`${styles.jsonField} ${styles.jsonFieldRow}`}>
                          <div className={styles.jsonLabel}>
                            {t('opencode.connectivity.customHeaders')}
                            <Tooltip title={t('opencode.connectivity.customHeadersHint')}>
                              <InfoCircleOutlined className={styles.jsonLabelHint} />
                            </Tooltip>
                          </div>
                          <div className={styles.jsonEditorWrap}>
                            <Form.Item
                              style={{ marginBottom: 0 }}
                            >
                              <JsonEditor
                                value={headersJson}
                                onChange={(val, valid) => { setHeadersJson(val); setHeadersValid(valid); }}
                                mode="text"
                                height={150}
                                placeholder="{}"
                              />
                            </Form.Item>
                          </div>
                        </div>

                        <div className={`${styles.jsonField} ${styles.jsonFieldRow}`}>
                          <div className={styles.jsonLabel}>
                            {t('opencode.connectivity.customBody')}
                            <Tooltip title={t('opencode.connectivity.customBodyHint')}>
                              <InfoCircleOutlined className={styles.jsonLabelHint} />
                            </Tooltip>
                          </div>
                          <div className={styles.jsonEditorWrap}>
                            <Form.Item
                              style={{ marginBottom: 0 }}
                            >
                              <JsonEditor
                                value={bodyJson}
                                onChange={(val, valid) => { setBodyJson(val); setBodyValid(valid); }}
                                mode="text"
                                height={150}
                                placeholder="{}"
                              />
                            </Form.Item>
                          </div>
                        </div>
                      </div>
                    )
                  }
                ]}
              /> : null}
            </Form>
          </div>
        </section>

        <section className={styles.sectionCard}>
          <div className={styles.sectionHeader}>
            <div className={styles.sectionTitle}>{t('opencode.connectivity.results')}</div>
            <Typography.Text className={styles.sectionHint}>
              {t('opencode.connectivity.resultsHint')}
            </Typography.Text>
          </div>

          <div className={styles.summaryGrid}>
            {summaryItems.map((item) => (
              <div key={item.label} className={styles.summaryCard}>
                <Typography.Text className={styles.summaryLabel}>{item.label}</Typography.Text>
                <Typography.Text className={styles.summaryValue}>{item.value}</Typography.Text>
              </div>
            ))}
          </div>

          <div className={styles.tableWrap}>
            <Table
              dataSource={results}
              columns={columns}
              pagination={false}
              size="small"
              scroll={{ y: 300 }}
            />
          </div>
          <Typography.Text type="secondary" className={styles.disclaimer}>
            {t('opencode.connectivity.disclaimer')}
          </Typography.Text>
        </section>
      </div>

      <Modal
        className={styles.detailsModal}
        title={t('opencode.connectivity.detailsTitle', { modelId: selectedResult?.modelId || '' })}
        open={detailsModalOpen}
        onCancel={() => setDetailsModalOpen(false)}
        footer={[
          <Button key="close" onClick={() => setDetailsModalOpen(false)}>
            {t('common.close')}
          </Button>
        ]}
        width={800}
      >
        {selectedResult && (
          <div className={styles.detailsContent}>
            <JsonEditor
              value={{
                request: {
                  url: selectedResult.requestUrl,
                  headers: selectedResult.requestHeaders,
                  body: selectedResult.requestBody,
                },
                response: {
                  // Gateway tests report the client status and, when the gateway
                  // forwarded the request, the real upstream status/URL. Without them
                  // a failed test with an empty body gave no clue where it broke.
                  status: selectedResult.statusCode === undefined
                    ? undefined
                    : `${selectedResult.statusCode} ${selectedResult.statusText ?? ''}`.trim(),
                  upstreamStatusCode: selectedResult.upstreamStatusCode,
                  upstreamUrl: selectedResult.upstreamUrl,
                  headers: selectedResult.responseHeaders,
                  body: selectedResult.responseBody,
                }
              }}
              readOnly={true}
              height={500}
              mode="text"
            />
          </div>
        )}
      </Modal>
    </Modal>
  );
};

export default ConnectivityTestModal;
