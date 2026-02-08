import React from 'react';
import { Modal, Form, Input, Button, Typography, Collapse, Select, message, Divider, Space } from 'antd';
import { PlusOutlined, DeleteOutlined, SwapOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { SLIM_AGENT_TYPES, SLIM_AGENT_DISPLAY_NAMES, SLIM_AGENT_DESCRIPTIONS, type OhMyOpenCodeSlimAgents, type SlimAgentType } from '@/types/ohMyOpenCodeSlim';
import JsonEditor from '@/components/common/JsonEditor';
import styles from './OhMyOpenCodeSlimConfigModal.module.less';

const { Text } = Typography;

interface OhMyOpenCodeSlimConfigModalProps {
  open: boolean;
  isEdit: boolean;
  initialValues?: {
    id?: string;
    name: string;
    agents?: OhMyOpenCodeSlimAgents;
    otherFields?: Record<string, unknown>;
  };
  modelOptions: { label: string; value: string }[];
  /** Map of model ID to its variant keys */
  modelVariantsMap?: Record<string, string[]>;
  onCancel: () => void;
  onSuccess: (values: OhMyOpenCodeSlimConfigFormValues) => void;
}

export interface OhMyOpenCodeSlimConfigFormValues {
  id?: string;
  name: string;
  agents: OhMyOpenCodeSlimAgents;
  otherFields?: Record<string, unknown>;
}

const OhMyOpenCodeSlimConfigModal: React.FC<OhMyOpenCodeSlimConfigModalProps> = ({
  open,
  isEdit,
  initialValues,
  modelOptions,
  modelVariantsMap = {},
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);

  // Custom agents (user-defined)
  const [customAgents, setCustomAgents] = React.useState<string[]>([]);
  const [newAgentKey, setNewAgentKey] = React.useState('');
  const [showAddAgent, setShowAddAgent] = React.useState(false);

  // Batch replace model state
  const [batchReplaceFromModel, setBatchReplaceFromModel] = React.useState<string | undefined>(undefined);
  const [batchReplaceToModel, setBatchReplaceToModel] = React.useState<string | undefined>(undefined);
  const [batchReplaceFromVariant, setBatchReplaceFromVariant] = React.useState<string | undefined>(undefined);
  const [batchReplaceToVariant, setBatchReplaceToVariant] = React.useState<string | undefined>(undefined);

  const fromModelVariants = React.useMemo(
    () => (batchReplaceFromModel ? modelVariantsMap[batchReplaceFromModel] ?? [] : []),
    [batchReplaceFromModel, modelVariantsMap]
  );

  const toModelVariants = React.useMemo(
    () => (batchReplaceToModel ? modelVariantsMap[batchReplaceToModel] ?? [] : []),
    [batchReplaceToModel, modelVariantsMap]
  );

  // Store otherFields - keep both raw string and parsed value for submit-time validation
  const otherFieldsRef = React.useRef<Record<string, unknown>>({});
  const otherFieldsRawRef = React.useRef<string>('');

  // Track if modal has been initialized
  const initializedRef = React.useRef(false);

  const labelCol = 6;
  const wrapperCol = 18;

  // Built-in agent keys
  const builtInAgentKeys = React.useMemo(() => [...SLIM_AGENT_TYPES], []);

  // Initialize form values when modal opens
  React.useEffect(() => {
    if (!open) {
      initializedRef.current = false;
      return;
    }

    if (initializedRef.current) {
      return;
    }

    initializedRef.current = true;

    if (initialValues) {
      // Build form values with nested agent paths
      const formValues: Record<string, unknown> = {
        id: initialValues.id,
        name: initialValues.name,
      };

      const detectedCustomAgents: string[] = [];
      const builtInAgentKeySet = new Set<string>(builtInAgentKeys);

      // Set agent models (built-in + custom)
      if (initialValues.agents) {
        Object.entries(initialValues.agents).forEach(([agentType, agent]) => {
          if (agent?.model) {
            formValues[`agent_${agentType}_model`] = agent.model;
          }
          if (typeof agent?.variant === 'string' && agent.variant) {
            formValues[`agent_${agentType}_variant`] = agent.variant;
          }
          
          // Track custom agents
          if (!builtInAgentKeySet.has(agentType)) {
            detectedCustomAgents.push(agentType);
          }
        });
      }

      setCustomAgents(detectedCustomAgents);

      form.setFieldsValue(formValues);
      otherFieldsRef.current = initialValues.otherFields || {};
      otherFieldsRawRef.current = initialValues.otherFields && Object.keys(initialValues.otherFields).length > 0
        ? JSON.stringify(initialValues.otherFields, null, 2)
        : '';
    } else {
      form.resetFields();
      setCustomAgents([]);
      otherFieldsRef.current = {};
      otherFieldsRawRef.current = '';
    }
    
    setShowAddAgent(false);
    setNewAgentKey('');
    setBatchReplaceFromModel(undefined);
    setBatchReplaceToModel(undefined);
    setBatchReplaceFromVariant(undefined);
    setBatchReplaceToVariant(undefined);
  }, [open, initialValues, form, builtInAgentKeys]);

  React.useEffect(() => {
    if (!batchReplaceFromModel) {
      setBatchReplaceFromVariant(undefined);
      return;
    }
    if (batchReplaceFromVariant && !fromModelVariants.includes(batchReplaceFromVariant)) {
      setBatchReplaceFromVariant(undefined);
    }
  }, [batchReplaceFromModel, batchReplaceFromVariant, fromModelVariants]);

  React.useEffect(() => {
    if (!batchReplaceToModel) {
      setBatchReplaceToVariant(undefined);
      return;
    }
    if (batchReplaceToVariant && !toModelVariants.includes(batchReplaceToVariant)) {
      setBatchReplaceToVariant(undefined);
    }
  }, [batchReplaceToModel, batchReplaceToVariant, toModelVariants]);

  const handleBatchReplaceModel = () => {
    const fromModel = batchReplaceFromModel;
    const toModel = batchReplaceToModel;

    if (!fromModel || !toModel) {
      message.warning(t('opencode.ohMyOpenCode.batchReplaceRequired'));
      return;
    }

    const sourceVariants = modelVariantsMap[fromModel] ?? [];
    if (batchReplaceFromVariant && !sourceVariants.includes(batchReplaceFromVariant)) {
      message.warning(t('opencode.ohMyOpenCode.batchReplaceInvalidFromVariant'));
      return;
    }

    const targetVariants = modelVariantsMap[toModel] ?? [];
    if (batchReplaceToVariant && !targetVariants.includes(batchReplaceToVariant)) {
      message.warning(t('opencode.ohMyOpenCode.batchReplaceInvalidToVariant'));
      return;
    }

    if (fromModel === toModel) {
      message.warning(t('opencode.ohMyOpenCode.batchReplaceSameModel'));
      return;
    }

    const allAgentKeys = [...builtInAgentKeys, ...customAgents];
    const modelFieldNames = allAgentKeys.map((agentType) => `agent_${agentType}_model`);

    const values = form.getFieldsValue(true) as Record<string, unknown>;
    const updateValues: Record<string, unknown> = {};

    let replacedCount = 0;
    let clearedVariantCount = 0;
    const hasTargetVariants = targetVariants.length > 0;

    modelFieldNames.forEach((modelFieldName) => {
      if (values[modelFieldName] !== fromModel) {
        return;
      }

      const variantFieldName = modelFieldName.replace('_model', '_variant');
      const variantValue = values[variantFieldName];

      if (batchReplaceFromVariant) {
        if (typeof variantValue !== 'string' || variantValue !== batchReplaceFromVariant) {
          return;
        }
      }

      updateValues[modelFieldName] = toModel;
      replacedCount += 1;

      if (batchReplaceToVariant) {
        updateValues[variantFieldName] = batchReplaceToVariant;
        return;
      }

      if (typeof variantValue === 'string' && variantValue) {
        if (!hasTargetVariants || !targetVariants.includes(variantValue)) {
          updateValues[variantFieldName] = undefined;
          clearedVariantCount += 1;
        }
      }
    });

    if (replacedCount === 0) {
      message.warning(t('opencode.ohMyOpenCode.batchReplaceNoMatch'));
      return;
    }

    form.setFieldsValue(updateValues);

    if (clearedVariantCount > 0) {
      message.success(t('opencode.ohMyOpenCode.batchReplaceSuccessWithVariantReset', {
        count: replacedCount,
        variantCount: clearedVariantCount,
      }));
      return;
    }

    message.success(t('opencode.ohMyOpenCode.batchReplaceSuccess', { count: replacedCount }));
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      setLoading(true);

      // Validate otherFields JSON at submit time
      const rawContent = otherFieldsRawRef.current.trim();
      let parsedOtherFields: Record<string, unknown> = {};
      if (rawContent !== '') {
        try {
          parsedOtherFields = JSON.parse(rawContent);
          if (typeof parsedOtherFields !== 'object' || parsedOtherFields === null || Array.isArray(parsedOtherFields)) {
            message.error(t('opencode.ohMyOpenCode.invalidJson'));
            setLoading(false);
            return;
          }
        } catch {
          message.error(t('opencode.ohMyOpenCode.invalidJson'));
          setLoading(false);
          return;
        }
      }

      // Build agents object (built-in + custom)
      const allAgentKeys = [...builtInAgentKeys, ...customAgents];
      const agents: OhMyOpenCodeSlimAgents = {};
      allAgentKeys.forEach((agentType) => {
        const modelFieldName = `agent_${agentType}_model`;
        const modelValue = values[modelFieldName];

        if (modelValue) {
          agents[agentType] = {
            model: modelValue,
          };
        }
      });

      const result: OhMyOpenCodeSlimConfigFormValues = {
        name: values.name,
        agents,
        otherFields: Object.keys(parsedOtherFields).length > 0
          ? parsedOtherFields
          : undefined,
      };

      // Include id when editing
      if (isEdit && values.id) {
        result.id = values.id;
      }

      onSuccess(result);
      form.resetFields();
    } catch (error) {
      console.error('Form validation error:', error);
    } finally {
      setLoading(false);
    }
  };

  // Handle adding custom agent
  const handleAddCustomAgent = () => {
    const key = newAgentKey.trim();
    if (!key) {
      message.warning(t('opencode.ohMyOpenCode.customAgentKeyRequired'));
      return;
    }
    // Check for duplicates
    const allKeys = [...builtInAgentKeys, ...customAgents];
    if (allKeys.includes(key)) {
      message.warning(t('opencode.ohMyOpenCode.customAgentKeyDuplicate'));
      return;
    }
    setCustomAgents(prev => [...prev, key]);
    setNewAgentKey('');
    setShowAddAgent(false);
  };

  // Handle removing custom agent
  const handleRemoveCustomAgent = (agentKey: string) => {
    setCustomAgents(prev => prev.filter(k => k !== agentKey));
    // Clear form field
    form.setFieldValue(`agent_${agentKey}_model`, undefined);
  };

  // Render built-in agent item
  const renderBuiltInAgentItem = (agentType: SlimAgentType) => (
    <Form.Item
      key={agentType}
      label={SLIM_AGENT_DISPLAY_NAMES[agentType]}
      tooltip={SLIM_AGENT_DESCRIPTIONS[agentType]}
      name={`agent_${agentType}_model`}
    >
                <Select
                  placeholder={t('opencode.ohMyOpenCode.selectModel')}
                  options={modelOptions}
                  allowClear
                  showSearch
                  optionFilterProp="label"
      />
    </Form.Item>
  );

  // Render custom agent item (with delete button)
  const renderCustomAgentItem = (agentType: string) => (
    <Form.Item
      key={agentType}
      label={<span style={{ color: '#1890ff' }}>{agentType}</span>}
      tooltip={t('opencode.ohMyOpenCode.customAgentTooltip')}
    >
            <Space.Compact style={{ width: '100%' }}>
              <Form.Item name={`agent_${agentType}_model`} noStyle>
                <Select
                  placeholder={t('opencode.ohMyOpenCode.selectModel')}
                  options={modelOptions}
                  allowClear
                  showSearch
                  optionFilterProp="label"
            style={{ width: 'calc(100% - 32px)' }}
                  />
                </Form.Item>
              <Button
                icon={<DeleteOutlined />}
                onClick={() => handleRemoveCustomAgent(agentType)}
                danger
                title={t('common.delete')}
              />
            </Space.Compact>
    </Form.Item>
  );

  return (
    <Modal
      title={isEdit
        ? t('opencode.ohMyOpenCodeSlim.editConfig')
        : t('opencode.ohMyOpenCodeSlim.addConfig')}
      open={open}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="submit" type="primary" loading={loading} onClick={handleSubmit}>
          {t('common.save')}
        </Button>,
      ]}
      width={800}
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={{ span: labelCol }}
        wrapperCol={{ span: wrapperCol }}
        style={{ marginTop: 24 }}
      >
        {/* Hidden ID field for editing */}
        <Form.Item name="id" hidden>
          <Input />
        </Form.Item>

        <Form.Item
          label={t('opencode.ohMyOpenCode.configName')}
          name="name"
          rules={[{ required: true, message: t('opencode.ohMyOpenCode.configNamePlaceholder') }]}
        >
          <Input placeholder={t('opencode.ohMyOpenCode.configNamePlaceholder')} />
        </Form.Item>

        <div className={styles.scrollArea}>
          {/* 批量替换模型（编辑模式） */}
          {isEdit && (
            <Collapse
              defaultActiveKey={[]}
              className={styles.sectionCard}
              ghost
              items={[
                {
                  key: 'batch-replace-model',
                  label: <Text strong>{t('opencode.ohMyOpenCode.batchReplaceModel')}</Text>,
                  children: (
                    <div className={styles.batchPanel}>
                      <Text type="secondary" className={styles.helperText}>
                        {t('opencode.ohMyOpenCode.batchReplaceHint')}
                      </Text>
                      <div className={styles.batchFlow}>
                        <div className={`${styles.batchGroup} ${styles.batchGroupFrom}`}>
                          <div className={styles.batchGroupHeader}>
                            <Text className={`${styles.batchGroupTag} ${styles.batchGroupTagFrom}`}>
                              {t('opencode.ohMyOpenCode.batchReplaceFromTitle')}
                            </Text>
                            <Text type="secondary" className={styles.batchGroupHint}>
                              {t('opencode.ohMyOpenCode.batchReplaceFromHint')}
                            </Text>
                          </div>
                          <div className={styles.batchGroupFields}>
                            <div className={styles.batchField}>
                              <Text className={styles.batchLabel}>{t('opencode.ohMyOpenCode.batchReplaceFromPlaceholder')}</Text>
                              <Select
                                value={batchReplaceFromModel}
                                placeholder={t('opencode.ohMyOpenCode.batchReplaceFromPlaceholder')}
                                options={modelOptions}
                                allowClear
                                showSearch
                                optionFilterProp="label"
                                className={styles.batchSelect}
                                onChange={(value) => {
                                  setBatchReplaceFromModel(value);
                                  setBatchReplaceFromVariant(undefined);
                                }}
                              />
                            </div>
                            <div className={styles.batchField}>
                              <Text className={styles.batchLabel}>{t('opencode.ohMyOpenCode.batchReplaceFromVariantPlaceholder')}</Text>
                              <Select
                                value={batchReplaceFromVariant}
                                placeholder={t('opencode.ohMyOpenCode.batchReplaceFromVariantPlaceholder')}
                                options={fromModelVariants.map((v) => ({ label: v, value: v }))}
                                allowClear
                                disabled={!batchReplaceFromModel || fromModelVariants.length === 0}
                                className={styles.batchSelect}
                                onChange={(value) => setBatchReplaceFromVariant(value)}
                              />
                            </div>
                          </div>
                        </div>

                        <div className={styles.batchArrow}>
                          <span className={styles.batchArrowIcon}>
                            <SwapOutlined />
                          </span>
                        </div>

                        <div className={`${styles.batchGroup} ${styles.batchGroupTo}`}>
                          <div className={styles.batchGroupHeader}>
                            <Text className={`${styles.batchGroupTag} ${styles.batchGroupTagTo}`}>
                              {t('opencode.ohMyOpenCode.batchReplaceToTitle')}
                            </Text>
                            <Text type="secondary" className={styles.batchGroupHint}>
                              {t('opencode.ohMyOpenCode.batchReplaceToHint')}
                            </Text>
                          </div>
                          <div className={styles.batchGroupFields}>
                            <div className={styles.batchField}>
                              <Text className={styles.batchLabel}>{t('opencode.ohMyOpenCode.batchReplaceToPlaceholder')}</Text>
                              <Select
                                value={batchReplaceToModel}
                                placeholder={t('opencode.ohMyOpenCode.batchReplaceToPlaceholder')}
                                options={modelOptions}
                                allowClear
                                showSearch
                                optionFilterProp="label"
                                className={styles.batchSelect}
                                onChange={(value) => {
                                  setBatchReplaceToModel(value);
                                  setBatchReplaceToVariant(undefined);
                                }}
                              />
                            </div>
                            <div className={styles.batchField}>
                              <Text className={styles.batchLabel}>{t('opencode.ohMyOpenCode.batchReplaceToVariantPlaceholder')}</Text>
                              <Select
                                value={batchReplaceToVariant}
                                placeholder={t('opencode.ohMyOpenCode.batchReplaceToVariantPlaceholder')}
                                options={toModelVariants.map((v) => ({ label: v, value: v }))}
                                allowClear
                                disabled={!batchReplaceToModel || toModelVariants.length === 0}
                                className={styles.batchSelect}
                                onChange={(value) => setBatchReplaceToVariant(value)}
                              />
                            </div>
                          </div>
                        </div>
                      </div>
                      <div className={styles.batchActionRow}>
                        <Text type="secondary" className={styles.batchActionHint}>
                          {t('opencode.ohMyOpenCode.batchReplaceActionHint')}
                        </Text>
                        <Button
                          type="primary"
                          icon={<SwapOutlined />}
                          onClick={handleBatchReplaceModel}
                          className={styles.batchActionButton}
                        >
                          {t('opencode.ohMyOpenCode.batchReplaceAction')}
                        </Button>
                      </div>
                    </div>
                  ),
                },
              ]}
            />
          )}

          {/* Agent 模型配置 */}
          <Collapse
            defaultActiveKey={['agents']}
            ghost
            items={[
              {
                key: 'agents',
                label: <Text strong>{t('opencode.ohMyOpenCode.agentModels')}</Text>,
                children: (
                  <>
                    <Text type="secondary" style={{ display: 'block', fontSize: 12, marginBottom: 12 }}>
                      {t('opencode.ohMyOpenCode.agentModelsHint')}
                    </Text>

                    {SLIM_AGENT_TYPES.map(renderBuiltInAgentItem)}
                    
                    {/* Custom Agents */}
                    {customAgents.length > 0 && (
                      <>
                        <Divider style={{ margin: '12px 0', fontSize: 12 }}>
                          {t('opencode.ohMyOpenCode.customAgents')}
                        </Divider>
                        {customAgents.map(renderCustomAgentItem)}
                      </>
                    )}
                    
                    {/* Add Custom Agent */}
                    {showAddAgent ? (
                      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
                        <Input
                          placeholder={t('opencode.ohMyOpenCode.customAgentKeyPlaceholder')}
                          value={newAgentKey}
                          onChange={(e) => setNewAgentKey(e.target.value)}
                          onPressEnter={handleAddCustomAgent}
                          style={{ flex: 1 }}
                        />
                        <Button type="primary" onClick={handleAddCustomAgent}>
                          {t('common.confirm')}
                        </Button>
                        <Button onClick={() => { setShowAddAgent(false); setNewAgentKey(''); }}>
                          {t('common.cancel')}
                        </Button>
                      </div>
                    ) : (
                      <Button
                        type="dashed"
                        icon={<PlusOutlined />}
                        onClick={() => setShowAddAgent(true)}
                        style={{ width: '100%', marginTop: 12 }}
                      >
                        {t('opencode.ohMyOpenCode.addCustomAgent')}
                      </Button>
                    )}
                  </>
                ),
              },
            ]}
          />

          {/* 其他配置（JSON） */}
          <Collapse
            defaultActiveKey={[]}
            style={{ marginTop: 8 }}
            ghost
            items={[
              {
                key: 'other',
                label: <Text strong>{t('opencode.ohMyOpenCode.otherFields')}</Text>,
                children: (
                  <>
                    <div style={{ marginBottom: 12, fontSize: 12 }}>
                      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
                        <thead>
                          <tr style={{ backgroundColor: 'var(--color-bg-elevated)' }}>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid var(--color-border-secondary)' }}>{t('opencode.ohMyOpenCodeSlim.optionName')}</th>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid var(--color-border-secondary)' }}>{t('opencode.ohMyOpenCodeSlim.optionType')}</th>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid var(--color-border-secondary)' }}>{t('opencode.ohMyOpenCodeSlim.optionDefault')}</th>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid var(--color-border-secondary)' }}>{t('opencode.ohMyOpenCodeSlim.optionDesc')}</th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>tmux.enabled</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>boolean</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>false</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>{t('opencode.ohMyOpenCodeSlim.tmuxEnabledDesc')}</td>
                          </tr>
                          <tr style={{ backgroundColor: 'var(--color-bg-hover)' }}>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>tmux.layout</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>string</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>"main-vertical"</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>{t('opencode.ohMyOpenCodeSlim.tmuxLayoutDesc')}</td>
                          </tr>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>disabled_mcps</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>string[]</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>[]</td>
                            <td style={{ padding: '8px', border: '1px solid var(--color-border-secondary)' }}>{t('opencode.ohMyOpenCodeSlim.disabledMcpsDesc')}</td>
                          </tr>
                        </tbody>
                      </table>
                    </div>
                    <Form.Item
                      labelCol={{ span: 24 }}
                      wrapperCol={{ span: 24 }}
                    >
                      <JsonEditor
                        value={otherFieldsRef.current && Object.keys(otherFieldsRef.current).length > 0
                          ? otherFieldsRef.current
                          : undefined}
                        onChange={(value) => {
                          // Store raw string for submit-time validation
                          if (value === null || value === undefined) {
                            otherFieldsRawRef.current = '';
                          } else if (typeof value === 'string') {
                            otherFieldsRawRef.current = value;
                          } else {
                            otherFieldsRawRef.current = JSON.stringify(value, null, 2);
                          }
                        }}
                        height={200}
                        minHeight={150}
                        maxHeight={400}
                        resizable
                        mode="text"
                        placeholder={`{
  "tmux": {
    "enabled": true,
    "layout": "main-vertical",
    "main_pane_size": 60
  }
}`}
                      />
                    </Form.Item>
                  </>
                ),
              },
            ]}
          />
        </div>
      </Form>
    </Modal>
  );
};

export default OhMyOpenCodeSlimConfigModal;
