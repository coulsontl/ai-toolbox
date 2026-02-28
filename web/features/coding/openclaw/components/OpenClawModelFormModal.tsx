import React from 'react';
import { Modal, Form, Input, AutoComplete, Button, Switch, InputNumber, Tag, Divider, Row, Col } from 'antd';
import { RightOutlined, DownOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useAppStore } from '@/stores';
import { PRESET_MODELS, type PresetModel } from '@/constants/presetModels';
import type { OpenClawModel } from '@/types/openclaw';

// Context limit options with display labels
const CONTEXT_LIMIT_OPTIONS = [
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
  { value: '128000', label: '128K' },
  { value: '200000', label: '200K' },
  { value: '256000', label: '256K' },
  { value: '1000000', label: '1M' },
  { value: '2000000', label: '2M' },
];

// Output limit options with display labels
const OUTPUT_LIMIT_OPTIONS = [
  { value: '2048', label: '2K' },
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
];

/** Map OpenClaw API protocol to npm SDK type for preset models lookup */
const API_TO_NPM: Record<string, string> = {
  'openai-completions': '@ai-sdk/openai-compatible',
  'openai-responses': '@ai-sdk/openai-compatible',
  'anthropic-messages': '@ai-sdk/anthropic',
  'google-generative-ai': '@ai-sdk/google',
};

export interface ModelFormValues {
  id: string;
  name?: string;
  contextWindow?: number;
  maxTokens?: number;
  reasoning?: boolean;
  costInput?: number;
  costOutput?: number;
  costCacheRead?: number;
  costCacheWrite?: number;
}

interface Props {
  open: boolean;
  editingModel?: OpenClawModel | null;
  existingIds: string[];
  apiProtocol?: string;
  onCancel: () => void;
  onSubmit: (values: ModelFormValues) => void;
}

const OpenClawModelFormModal: React.FC<Props> = ({
  open: modalOpen,
  editingModel,
  existingIds,
  apiProtocol,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const language = useAppStore((state) => state.language);
  const [form] = Form.useForm();
  const isEdit = !!editingModel;
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);
  const [presetsExpanded, setPresetsExpanded] = React.useState(false);

  const labelCol = { span: language === 'zh-CN' ? 5 : 7 };
  const wrapperCol = { span: 19 };

  // Get preset models for current provider type
  const npmType = apiProtocol ? API_TO_NPM[apiProtocol] : undefined;

  const presetModels = React.useMemo(() => {
    if (!npmType) return [];
    return PRESET_MODELS[npmType] || [];
  }, [npmType]);

  const otherPresetModels = React.useMemo(() => {
    if (!npmType) return [];
    return Object.entries(PRESET_MODELS)
      .filter(([type]) => type !== npmType)
      .flatMap(([, models]) => models);
  }, [npmType]);

  // If no npmType, show all presets as a flat list
  const allPresetModels = React.useMemo(() => {
    if (npmType) return [];
    return Object.values(PRESET_MODELS).flat();
  }, [npmType]);

  const handlePresetSelect = (preset: PresetModel) => {
    // Only keep the current id unchanged, overwrite everything else
    form.setFieldsValue({
      name: preset.name,
      contextWindow: preset.contextLimit,
      maxTokens: preset.outputLimit,
      reasoning: preset.reasoning ?? false,
      costInput: undefined,
      costOutput: undefined,
      costCacheRead: undefined,
      costCacheWrite: undefined,
    });
    setPresetsExpanded(false);
  };

  const hasPresets = presetModels.length > 0 || allPresetModels.length > 0;

  // Check if cost fields have content
  const hasAdvancedContent = React.useMemo(() => {
    if (!editingModel?.cost) return false;
    return editingModel.cost.input !== undefined || editingModel.cost.output !== undefined;
  }, [editingModel]);

  React.useEffect(() => {
    if (modalOpen) {
      if (editingModel) {
        form.setFieldsValue({
          id: editingModel.id,
          name: editingModel.name || '',
          contextWindow: editingModel.contextWindow,
          maxTokens: editingModel.maxTokens,
          reasoning: editingModel.reasoning || false,
          costInput: editingModel.cost?.input,
          costOutput: editingModel.cost?.output,
          costCacheRead: editingModel.cost?.cacheRead,
          costCacheWrite: editingModel.cost?.cacheWrite,
        });
        setAdvancedExpanded(hasAdvancedContent);
      } else {
        form.resetFields();
        setAdvancedExpanded(false);
      }
      setPresetsExpanded(false);
    }
  }, [modalOpen, editingModel, form, hasAdvancedContent]);

  const handleOk = async () => {
    try {
      const values = await form.validateFields();
      onSubmit(values);
    } catch {
      // validation error
    }
  };

  return (
    <Modal
      title={isEdit ? t('openclaw.providers.editModel') : t('openclaw.providers.addModel')}
      open={modalOpen}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="submit" type="primary" onClick={handleOk}>
          {t('common.save')}
        </Button>,
      ]}
      width={600}
      destroyOnClose
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={labelCol}
        wrapperCol={wrapperCol}
        style={{ marginTop: 24 }}
        autoComplete="off"
      >
        <Form.Item
          label={t('openclaw.providers.modelId')}
          required
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Form.Item
              name="id"
              noStyle
              rules={[
                { required: true, message: t('common.required') },
                {
                  validator: (_, value) => {
                    if (!isEdit && value && existingIds.includes(value)) {
                      return Promise.reject(new Error('Model ID already exists'));
                    }
                    return Promise.resolve();
                  },
                },
              ]}
            >
              <Input placeholder={t('openclaw.providers.modelIdPlaceholder')} disabled={isEdit} style={{ flex: 1 }} />
            </Form.Item>
            {hasPresets && (
              <a
                style={{
                  flexShrink: 0,
                  fontSize: 12,
                  fontWeight: 500,
                  color: 'var(--ant-color-text-secondary)',
                  cursor: 'pointer',
                  userSelect: 'none',
                  whiteSpace: 'nowrap',
                }}
                onClick={() => setPresetsExpanded(!presetsExpanded)}
              >
                {t('openclaw.providers.selectPreset')}
                {presetsExpanded ? ' ▴' : ' ▾'}
              </a>
            )}
          </div>
        </Form.Item>

        {presetsExpanded && (
          <Form.Item wrapperCol={{ offset: language === 'zh-CN' ? 5 : 7, span: 19 }} style={{ marginTop: -8 }}>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
              {(presetModels.length > 0 ? presetModels : allPresetModels).map((preset) => (
                <Tag
                  key={preset.id}
                  style={{ cursor: 'pointer', transition: 'all 0.2s' }}
                  onClick={() => handlePresetSelect(preset)}
                >
                  {preset.name}
                </Tag>
              ))}
            </div>
            {otherPresetModels.length > 0 && (
              <>
                <Divider style={{ margin: '12px 0', fontSize: 12, color: 'var(--color-text-tertiary)' }}>
                  {t('openclaw.providers.otherPresets')}
                </Divider>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
                  {otherPresetModels.map((preset) => (
                    <Tag
                      key={preset.id}
                      style={{ cursor: 'pointer', transition: 'all 0.2s' }}
                      onClick={() => handlePresetSelect(preset)}
                    >
                      {preset.name}
                    </Tag>
                  ))}
                </div>
              </>
            )}
          </Form.Item>
        )}

        <Form.Item name="name" label={t('openclaw.providers.modelName')}>
          <Input placeholder={t('openclaw.providers.modelNamePlaceholder')} />
        </Form.Item>

        <Form.Item
          name="contextWindow"
          label={t('openclaw.providers.contextLimit')}
          getValueFromEvent={(val) => {
            const num = parseInt(val, 10);
            return isNaN(num) ? undefined : num;
          }}
        >
          <AutoComplete
            options={CONTEXT_LIMIT_OPTIONS}
            placeholder={t('openclaw.providers.contextLimitPlaceholder')}
            style={{ width: '100%' }}
            filterOption={(inputValue, option) =>
              (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
              option?.value.includes(inputValue)) ?? false
            }
          />
        </Form.Item>

        <Form.Item
          name="maxTokens"
          label={t('openclaw.providers.outputLimit')}
          getValueFromEvent={(val) => {
            const num = parseInt(val, 10);
            return isNaN(num) ? undefined : num;
          }}
        >
          <AutoComplete
            options={OUTPUT_LIMIT_OPTIONS}
            placeholder={t('openclaw.providers.outputLimitPlaceholder')}
            style={{ width: '100%' }}
            filterOption={(inputValue, option) =>
              (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
              option?.value.includes(inputValue)) ?? false
            }
          />
        </Form.Item>

        <Form.Item name="reasoning" label={t('openclaw.providers.reasoning')} valuePropName="checked">
          <Switch />
        </Form.Item>

        {/* Advanced: Cost fields */}
        <div style={{ marginBottom: advancedExpanded ? 16 : 0 }}>
          <Button
            type="link"
            onClick={() => setAdvancedExpanded(!advancedExpanded)}
            style={{ padding: 0, height: 'auto' }}
          >
            {advancedExpanded ? <DownOutlined /> : <RightOutlined />}
            <span style={{ marginLeft: 4 }}>
              {t('openclaw.providers.costSettings')}
              <span style={{ fontSize: 11, color: 'var(--ant-color-text-secondary)', fontWeight: 'normal' }}> ($/M tokens)</span>
              {hasAdvancedContent && !advancedExpanded && (
                <span style={{ marginLeft: 4, color: '#1890ff' }}>*</span>
              )}
            </span>
          </Button>
        </div>
        {advancedExpanded && (
          <>
            <Row gutter={16}>
              <Col span={12}>
                <Form.Item name="costInput" label={t('openclaw.providers.costInput')} labelCol={{ span: language === 'zh-CN' ? 10 : 14 }} wrapperCol={{ span: language === 'zh-CN' ? 14 : 10 }}>
                  <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                </Form.Item>
              </Col>
              <Col span={12}>
                <Form.Item name="costOutput" label={t('openclaw.providers.costOutput')} labelCol={{ span: language === 'zh-CN' ? 10 : 14 }} wrapperCol={{ span: language === 'zh-CN' ? 14 : 10 }}>
                  <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                </Form.Item>
              </Col>
            </Row>
            <Row gutter={16}>
              <Col span={12}>
                <Form.Item name="costCacheRead" label={t('openclaw.providers.costCacheRead')} labelCol={{ span: language === 'zh-CN' ? 10 : 14 }} wrapperCol={{ span: language === 'zh-CN' ? 14 : 10 }}>
                  <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                </Form.Item>
              </Col>
              <Col span={12}>
                <Form.Item name="costCacheWrite" label={t('openclaw.providers.costCacheWrite')} labelCol={{ span: language === 'zh-CN' ? 10 : 14 }} wrapperCol={{ span: language === 'zh-CN' ? 14 : 10 }}>
                  <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                </Form.Item>
              </Col>
            </Row>
          </>
        )}
      </Form>
    </Modal>
  );
};

export default OpenClawModelFormModal;
