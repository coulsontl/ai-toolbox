import React from 'react';
import { Modal, Form, Input, Button, Typography, Collapse, Select, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { SLIM_AGENT_TYPES, SLIM_AGENT_DISPLAY_NAMES, SLIM_AGENT_DESCRIPTIONS, type OhMyOpenCodeSlimAgents } from '@/types/ohMyOpenCodeSlim';
import JsonEditor from '@/components/common/JsonEditor';

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
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);

  // Store otherFields - keep both raw string and parsed value for submit-time validation
  const otherFieldsRef = React.useRef<Record<string, unknown>>({});
  const otherFieldsRawRef = React.useRef<string>('');

  // Track if modal has been initialized
  const initializedRef = React.useRef(false);

  const labelCol = 6;
  const wrapperCol = 18;

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
      const formValues: any = {
        id: initialValues.id,
        name: initialValues.name,
      };

      // Set agent models
      if (initialValues.agents) {
        SLIM_AGENT_TYPES.forEach((agentType) => {
          const agent = initialValues.agents?.[agentType];
          if (agent?.model) {
            formValues[`agent_${agentType}_model`] = agent.model;
          }
        });
      }

      form.setFieldsValue(formValues);
      otherFieldsRef.current = initialValues.otherFields || {};
      otherFieldsRawRef.current = initialValues.otherFields && Object.keys(initialValues.otherFields).length > 0
        ? JSON.stringify(initialValues.otherFields, null, 2)
        : '';
    } else {
      form.resetFields();
      otherFieldsRef.current = {};
      otherFieldsRawRef.current = '';
    }
  }, [open, initialValues, form]);

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

      // Build agents object
      const agents: OhMyOpenCodeSlimAgents = {};
      SLIM_AGENT_TYPES.forEach((agentType) => {
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

  return (
    <Modal
      title={isEdit
        ? 'Oh My OpenCode Slim - 编辑配置'
        : 'Oh My OpenCode Slim - 新建配置'}
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
          label="配置名称"
          name="name"
          rules={[{ required: true, message: '请输入配置名称' }]}
        >
          <Input placeholder="如：我的配置" />
        </Form.Item>

        <div style={{ maxHeight: 500, overflowY: 'auto', paddingRight: 8, marginTop: 16 }}>
          {/* Agent 模型配置 */}
          <Collapse
            defaultActiveKey={['agents']}
            ghost
            items={[
              {
                key: 'agents',
                label: <Text strong>Agent 模型配置</Text>,
                children: (
                  <>
                    <Text type="secondary" style={{ display: 'block', fontSize: 12, marginBottom: 12 }}>
                      配置各个 Agent 使用的模型（格式：provider/model）
                    </Text>

                    {SLIM_AGENT_TYPES.map((agentType) => (
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
                    ))}
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
                label: <Text strong>其他配置（JSON）</Text>,
                children: (
                  <>
                    <div style={{ marginBottom: 12, fontSize: 12, color: '#666' }}>
                      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
                        <thead>
                          <tr style={{ backgroundColor: '#f5f5f5' }}>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid #e8e8e8' }}>选项</th>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid #e8e8e8' }}>类型</th>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid #e8e8e8' }}>默认值</th>
                            <th style={{ padding: '8px', textAlign: 'left', border: '1px solid #e8e8e8' }}>描述</th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>tmux.enabled</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>boolean</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>false</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>是否启用子代理的 tmux 窗格</td>
                          </tr>
                          <tr style={{ backgroundColor: '#fafafa' }}>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>tmux.layout</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>string</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>"main-vertical"</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>布局预设：main-vertical、main-horizontal、tiled、even-horizontal、even-vertical</td>
                          </tr>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>tmux.main_pane_size</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>number</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>60</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>主窗格大小百分比（20-80）</td>
                          </tr>
                          <tr style={{ backgroundColor: '#fafafa' }}>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>disabled_mcps</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>string[]</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>[]</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>要禁用的 MCP 服务器 ID（如 "websearch"）</td>
                          </tr>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>agents.&lt;name&gt;.model</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>string</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>-</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>覆盖特定代理的模型</td>
                          </tr>
                          <tr style={{ backgroundColor: '#fafafa' }}>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>agents.&lt;name&gt;.variant</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>string</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>-</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>推理强度："low"、"medium"、"high"</td>
                          </tr>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>agents.&lt;name&gt;.skills</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>string[]</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>-</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>该代理可使用的技能（"*" 表示所有技能）</td>
                          </tr>
                          <tr style={{ backgroundColor: '#fafafa' }}>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>agents.&lt;name&gt;.temperature</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>number</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>-</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>该代理的温度 (0.0 到 2.0)</td>
                          </tr>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>agents.&lt;name&gt;.prompt</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>string</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>-</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>该代理的基础提示词覆盖</td>
                          </tr>
                          <tr style={{ backgroundColor: '#fafafa' }}>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>agents.&lt;name&gt;.prompt_append</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>string</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>-</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>追加到基础提示词后的文本</td>
                          </tr>
                          <tr>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>agents.&lt;name&gt;.disable</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>boolean</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>-</td>
                            <td style={{ padding: '8px', border: '1px solid #e8e8e8' }}>禁用该特定代理</td>
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
