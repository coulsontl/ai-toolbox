import React from 'react';
import { Modal, Form, Input, Button, Typography, Select, Collapse, Space, Alert, message } from 'antd';
import { MoreOutlined, WarningOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { OH_MY_OPENCODE_AGENTS, type OhMyOpenCodeAgentConfig } from '@/types/ohMyOpenCode';
import { getAgentDisplayName, getAgentDescription } from '@/services/ohMyOpenCodeApi';
import JsonEditor from '@/components/common/JsonEditor';

const { Text } = Typography;

// 分割 agents：前面 8 个为基础 agent，后面为高级 agent
const BASIC_AGENT_COUNT = 8;
const getBasicAgentKeys = (): string[] => OH_MY_OPENCODE_AGENTS.slice(0, BASIC_AGENT_COUNT).map((a) => a.key);
const getAdvancedAgentKeys = (): string[] => OH_MY_OPENCODE_AGENTS.slice(BASIC_AGENT_COUNT).map((a) => a.key);

interface OhMyOpenCodeConfigModalProps {
  open: boolean;
  isEdit: boolean;
  initialValues?: {
    id?: string;
    name: string;
    agents?: Record<string, OhMyOpenCodeAgentConfig | undefined> | null;
    otherFields?: Record<string, unknown>;
  };
  modelOptions: { label: string; value: string }[];
  onCancel: () => void;
  onSuccess: (values: OhMyOpenCodeConfigFormValues) => void;
}

export interface OhMyOpenCodeConfigFormValues {
  id?: string;
  name: string;
  agents: Record<string, OhMyOpenCodeAgentConfig | undefined>;
  otherFields?: Record<string, unknown>;
}

const OhMyOpenCodeConfigModal: React.FC<OhMyOpenCodeConfigModalProps> = ({
  open,
  isEdit,
  initialValues,
  modelOptions,
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [advancedCollapsed, setAdvancedCollapsed] = React.useState(true);

  // Track which agents have advanced settings expanded
  const [expandedAgents, setExpandedAgents] = React.useState<Record<string, boolean>>({});

  // Store advanced settings values in refs to avoid re-renders
  const advancedSettingsRef = React.useRef<Record<string, Record<string, unknown>>>({});
  const advancedSettingsRawRef = React.useRef<Record<string, string>>({});
  const otherFieldsRef = React.useRef<Record<string, unknown>>({});
  const otherFieldsRawRef = React.useRef<string>('');

  // Track if modal has been initialized to avoid re-initialization on parent re-renders
  const initializedRef = React.useRef(false);
  const prevOpenRef = React.useRef(false);

  // Agent types from centralized constant
  const basicAgentKeys = React.useMemo(() => getBasicAgentKeys(), []);
  const advancedAgentKeys = React.useMemo(() => getAdvancedAgentKeys(), []);

  const labelCol = 6;
  const wrapperCol = 18;

  // Initialize form values - only when modal opens (not on every parent re-render)
  React.useEffect(() => {
    prevOpenRef.current = open;

    // Reset initialized flag when modal closes
    if (!open) {
      initializedRef.current = false;
      return;
    }

    // Skip if already initialized (prevents overwriting user input on parent re-renders)
    if (initializedRef.current) {
      return;
    }

    initializedRef.current = true;

    if (initialValues) {
      // Parse agent models and advanced settings from config
      const agentFields: Record<string, string | undefined> = {};
      const validityState: Record<string, boolean> = {};

      [...basicAgentKeys, ...advancedAgentKeys].forEach((agentType) => {
        const agent = initialValues.agents?.[agentType];
        if (agent) {
          // Extract model
          if (typeof agent.model === 'string' && agent.model) {
            agentFields[`agent_${agentType}`] = agent.model;
          }

          // Extract advanced fields (everything except model) and store in ref
          const advancedConfig: Record<string, unknown> = {};
          Object.keys(agent).forEach((key) => {
            if (key !== 'model' && agent[key as keyof OhMyOpenCodeAgentConfig] !== undefined) {
              advancedConfig[key] = agent[key as keyof OhMyOpenCodeAgentConfig];
            }
          });

          advancedSettingsRef.current[agentType] = advancedConfig;
        }

        // Initialize validity state
        validityState[agentType] = true;
      });

      form.setFieldsValue({
        id: initialValues.id,
        name: initialValues.name,
        ...agentFields,
        otherFields: initialValues.otherFields || {},
      });

      otherFieldsRef.current = initialValues.otherFields || {};
      otherFieldsRawRef.current = initialValues.otherFields && Object.keys(initialValues.otherFields).length > 0
        ? JSON.stringify(initialValues.otherFields, null, 2)
        : '';
    } else {
      form.resetFields();
      form.setFieldsValue({
        otherFields: {},
      });

      // Reset refs
      [...basicAgentKeys, ...advancedAgentKeys].forEach((agentType) => {
        advancedSettingsRef.current[agentType] = {};
        advancedSettingsRawRef.current[agentType] = '';
      });
      otherFieldsRef.current = {};
      otherFieldsRawRef.current = '';
    }
    setExpandedAgents({}); // Collapse all on open
    setAdvancedCollapsed(true); // Collapse advanced agents on open
  }, [open, initialValues, form, basicAgentKeys, advancedAgentKeys]);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      setLoading(true);

      // Validate otherFields JSON at submit time
      const otherFieldsRaw = otherFieldsRawRef.current.trim();
      let parsedOtherFields: Record<string, unknown> = {};
      if (otherFieldsRaw !== '') {
        try {
          parsedOtherFields = JSON.parse(otherFieldsRaw);
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

      // Validate and parse all advanced settings at submit time
      const parsedAdvancedSettings: Record<string, Record<string, unknown>> = {};
      for (const agentType of [...basicAgentKeys, ...advancedAgentKeys]) {
        const rawAdvanced = advancedSettingsRawRef.current[agentType]?.trim() || '';
        if (rawAdvanced !== '') {
          try {
            const parsed = JSON.parse(rawAdvanced);
            if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
              message.error(t('opencode.ohMyOpenCode.invalidJson'));
              setLoading(false);
              return;
            }
            parsedAdvancedSettings[agentType] = parsed;
          } catch {
            message.error(t('opencode.ohMyOpenCode.invalidJson'));
            setLoading(false);
            return;
          }
        }
      }

      // Build agents object with merged advanced settings
      const agents: Record<string, OhMyOpenCodeAgentConfig | undefined> = {};
      [...basicAgentKeys, ...advancedAgentKeys].forEach((agentType) => {
        const modelFieldName = `agent_${agentType}` as keyof typeof values;

        const modelValue = values[modelFieldName];
        const advancedValue = parsedAdvancedSettings[agentType];

        // Only create agent config if model is set OR advanced settings exist
        if (modelValue || (advancedValue && Object.keys(advancedValue).length > 0)) {
          agents[agentType] = {
            ...(modelValue ? { model: modelValue } : {}),
            ...(advancedValue || {}),
          } as OhMyOpenCodeAgentConfig;
        } else {
          agents[agentType] = undefined;
        }
      });

      const result: OhMyOpenCodeConfigFormValues = {
        name: values.name,
        agents,
        otherFields: Object.keys(parsedOtherFields).length > 0 ? parsedOtherFields : undefined,
      };

      // Include id when editing (read from form values which were set from initialValues)
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

  const toggleAdvancedSettings = (agentType: string) => {
    setExpandedAgents(prev => ({
      ...prev,
      [agentType]: !prev[agentType],
    }));
  };

  // Render agent item
  const renderAgentItem = (agentType: string) => (
    <div key={agentType}>
      <Form.Item
        label={getAgentDisplayName(agentType).split(' ')[0]}
        tooltip={getAgentDescription(agentType)}
        style={{ marginBottom: expandedAgents[agentType] ? 8 : 12 }}
      >
        <Space.Compact style={{ width: '100%' }}>
          <Form.Item name={`agent_${agentType}`} noStyle>
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
            icon={<MoreOutlined />}
            onClick={() => toggleAdvancedSettings(agentType)}
            type={expandedAgents[agentType] ? 'primary' : 'default'}
            title={t('opencode.ohMyOpenCode.advancedSettings')}
          />
        </Space.Compact>
      </Form.Item>

      {expandedAgents[agentType] && (
        <Form.Item
          help={t('opencode.ohMyOpenCode.advancedSettingsHint')}
          labelCol={{ span: 24 }}
          wrapperCol={{ span: 24 }}
          style={{ marginBottom: 16, marginLeft: labelCol * 4 + 8 }}
        >
          <JsonEditor
            value={advancedSettingsRef.current[agentType] && Object.keys(advancedSettingsRef.current[agentType]).length > 0 ? advancedSettingsRef.current[agentType] : undefined}
            onChange={(value) => {
              // Store raw string for submit-time validation
              if (value === null || value === undefined) {
                advancedSettingsRawRef.current[agentType] = '';
              } else if (typeof value === 'string') {
                advancedSettingsRawRef.current[agentType] = value;
              } else {
                advancedSettingsRawRef.current[agentType] = JSON.stringify(value, null, 2);
              }
            }}
            height={150}
            minHeight={100}
            maxHeight={300}
            resizable
            mode="text"
            placeholder={`{
    "temperature": 0.5
}`}
          />
        </Form.Item>
      )}
    </div>
  );

  return (
    <Modal
      title={isEdit
        ? t('opencode.ohMyOpenCode.editConfig')
        : t('opencode.ohMyOpenCode.addConfig')}
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
          <Input
            placeholder={t('opencode.ohMyOpenCode.configNamePlaceholder')}
          />
        </Form.Item>

        <div style={{ maxHeight: 500, overflowY: 'auto', paddingRight: 8, marginTop: 16 }}>
          {/* 基础 Agent（前面 7 个） */}
          <Collapse
            defaultActiveKey={['basic-agents']}
            ghost
            items={[
              {
                key: 'basic-agents',
                label: <Text strong>{t('opencode.ohMyOpenCode.agentModels')}</Text>,
                children: (
                  <>
                    <Text type="secondary" style={{ display: 'block', fontSize: 12, marginBottom: 12 }}>
                      {t('opencode.ohMyOpenCode.agentModelsHint')}
                    </Text>
                    {basicAgentKeys.map(renderAgentItem)}
                  </>
                ),
              },
            ]}
          />

          {/* 高级 Agent（后面 7 个） */}
          <Collapse
            defaultActiveKey={advancedCollapsed ? [] : ['advanced-agents']}
            onChange={(keys) => setAdvancedCollapsed(!keys.includes('advanced-agents'))}
            style={{ marginTop: 8 }}
            ghost
            items={[
              {
                key: 'advanced-agents',
                label: (
                  <Space>
                    <Text strong>{t('opencode.ohMyOpenCode.advancedAgents')}</Text>
                    <WarningOutlined style={{ color: '#faad14' }} />
                  </Space>
                ),
                children: (
                  <>
                    <Alert
                      type="warning"
                      message={t('opencode.ohMyOpenCode.advancedAgentsHint')}
                      style={{ marginBottom: 12 }}
                    />
                    {advancedAgentKeys.map(renderAgentItem)}
                  </>
                ),
              },
            ]}
          />

          {/* 其他配置 */}
          <Collapse
            defaultActiveKey={[]}
            style={{ marginTop: 8 }}
            ghost
            items={[
              {
                key: 'other',
                label: <Text strong>{t('opencode.ohMyOpenCode.otherFields')}</Text>,
                children: (
                  <Form.Item
                    help={t('opencode.ohMyOpenCode.otherFieldsHint')}
                    labelCol={{ span: 24 }}
                    wrapperCol={{ span: 24 }}
                  >
                    <JsonEditor
                      value={otherFieldsRef.current && Object.keys(otherFieldsRef.current).length > 0 ? otherFieldsRef.current : undefined}
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
    "background_task": {
        "defaultConcurrency": 5
    }
}`}
                    />
                  </Form.Item>
                ),
              },
            ]}
          />
        </div>
      </Form>
    </Modal>
  );
};

export default OhMyOpenCodeConfigModal;
