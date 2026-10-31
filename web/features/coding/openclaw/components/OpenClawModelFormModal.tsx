import React from 'react';
import { Modal, Form, Input, Button, InputNumber, Tag, Divider, Row, Col, Typography, Checkbox } from 'antd';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';
import { RightOutlined, DownOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  PRESET_MODELS,
  getPresetModelsVersion,
  subscribePresetModels,
  type PresetModel,
} from '@/constants/presetModels';
import JsonEditor from '@/components/common/JsonEditor';
import type { OpenClawModel } from '@/types/openclaw';

import {
  CONTEXT_LIMIT_PRESETS,
  OUTPUT_LIMIT_PRESETS,
} from '@/features/coding/shared/modelLimitPresets';
import {
  MODEL_FORM_CHECKBOX_STYLES,
  MODEL_FORM_EXTRA_STYLE,
  MODEL_FORM_LAYOUT,
  MODEL_FORM_PAIR_STYLE,
} from '@/features/coding/shared/modelFormLayout';
import modelFormStyles from '../../shared/modelFormLayout.module.less';
const { Text } = Typography;



/** Map OpenClaw API protocol to npm SDK type for preset models lookup */
const API_TO_NPM: Record<string, string> = {
  'openai-completions': '@ai-sdk/openai-compatible',
  'openai-responses': '@ai-sdk/openai',
  'anthropic-messages': '@ai-sdk/anthropic',
  'google-generative-ai': '@ai-sdk/google',
};

/** Known model fields that are handled by dedicated form controls */
const KNOWN_MODEL_FIELDS = new Set([
  'id', 'name', 'alias', 'contextWindow', 'maxTokens', 'reasoning', 'input', 'cost',
]);

/** Extract extra (unknown) fields from an OpenClawModel */
function extractExtraFields(model: OpenClawModel): Record<string, unknown> | undefined {
  const extra: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(model)) {
    if (!KNOWN_MODEL_FIELDS.has(key) && value !== undefined) {
      extra[key] = value;
    }
  }
  return Object.keys(extra).length > 0 ? extra : undefined;
}

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
  extraParams?: Record<string, unknown>;
}

interface Props {
  open: boolean;
  editingModel?: OpenClawModel | null;
  /**
   * 表单预填 `editingModel` 但保存为**新**记录（复制）。模型 id 加 `_copy`
   * 后缀且保持可编辑——`models` 是数组、按 id 索引，不能造出重复 id。
   */
  isCopy?: boolean;
  existingIds: string[];
  apiProtocol?: string;
  onCancel: () => void;
  onSubmit: (values: ModelFormValues) => void;
}

const OpenClawModelFormModal: React.FC<Props> = ({
  open: modalOpen,
  editingModel,
  isCopy = false,
  existingIds,
  apiProtocol,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  // 复制走的是「新增」路径（新 id），只是预填了来源模型的值。
  const isEdit = !!editingModel && !isCopy;
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);
  const [presetsExpanded, setPresetsExpanded] = React.useState(false);
  const [extraParamsValue, setExtraParamsValue] = React.useState<unknown>(undefined);
  const [extraParamsValid, setExtraParamsValid] = React.useState(true);
  const presetModelsVersion = React.useSyncExternalStore(
    subscribePresetModels,
    getPresetModelsVersion,
    getPresetModelsVersion,
  );


  // Get preset models for current provider type
  const npmType = apiProtocol ? API_TO_NPM[apiProtocol] : undefined;

  const presetModels = React.useMemo(() => {
    if (!npmType) return [];
    return PRESET_MODELS[npmType] || [];
  }, [npmType, presetModelsVersion]);

  const otherPresetModels = React.useMemo(() => {
    if (!npmType) return [];
    return Object.entries(PRESET_MODELS)
      .filter(([type]) => type !== npmType)
      .flatMap(([, models]) => models);
  }, [npmType, presetModelsVersion]);

  // If no npmType, show all presets as a flat list
  const allPresetModels = React.useMemo(() => {
    if (npmType) return [];
    return Object.values(PRESET_MODELS).flat();
  }, [npmType, presetModelsVersion]);

  const handlePresetSelect = (preset: PresetModel) => {
    form.setFieldsValue({
      // Fill model ID only when adding a new model, not when editing
      ...(isEdit ? {} : { id: preset.id }),
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

  // Check if cost fields or extra params have content
  const hasAdvancedContent = React.useMemo(() => {
    const hasCost = editingModel?.cost && (editingModel.cost.input !== undefined || editingModel.cost.output !== undefined);
    const hasExtra = editingModel && Object.keys(extractExtraFields(editingModel) || {}).length > 0;
    return hasCost || hasExtra;
  }, [editingModel]);

  React.useEffect(() => {
    if (modalOpen) {
      if (editingModel) {
        form.setFieldsValue({
          id: isCopy ? `${editingModel.id}_copy` : editingModel.id,
          name: isCopy
            ? `${editingModel.name || editingModel.id} copy`
            : editingModel.name || '',
          contextWindow: editingModel.contextWindow,
          maxTokens: editingModel.maxTokens,
          reasoning: editingModel.reasoning || false,
          costInput: editingModel.cost?.input,
          costOutput: editingModel.cost?.output,
          costCacheRead: editingModel.cost?.cacheRead,
          costCacheWrite: editingModel.cost?.cacheWrite,
        });
        const extra = extractExtraFields(editingModel);
        setExtraParamsValue(extra);
        setExtraParamsValid(true);
        setAdvancedExpanded(!!hasAdvancedContent);
      } else {
        form.resetFields();
        setExtraParamsValue(undefined);
        setExtraParamsValid(true);
        setAdvancedExpanded(true);
      }
      setPresetsExpanded(false);
    }
  }, [modalOpen, editingModel, isCopy, form, hasAdvancedContent]);

  const handleExtraParamsChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setExtraParamsValue(value);
    }
    setExtraParamsValid(isValid);
  };

  const handleOk = async () => {
    try {
      const values = await form.validateFields();

      const result: ModelFormValues = { ...values };
      // Include extra params if valid and non-empty
      if (extraParamsValid && typeof extraParamsValue === 'object' && extraParamsValue !== null && Object.keys(extraParamsValue).length > 0) {
        result.extraParams = extraParamsValue as Record<string, unknown>;
      }
      onSubmit(result);
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
      destroyOnHidden
    >
      <Form
        form={form}
        layout={MODEL_FORM_LAYOUT}
        classNames={{ label: modelFormStyles.label }}
        styles={{ extra: MODEL_FORM_EXTRA_STYLE }}
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
          <Form.Item style={{ marginTop: -8 }}>
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

        {/* Both are lengths of the same payload, so they share a row (see
            `MODEL_FORM_PAIR_STYLE`). */}
        <div style={MODEL_FORM_PAIR_STYLE}>
          <Form.Item
            name="contextWindow"
            label={t('openclaw.providers.contextLimit')}
            getValueFromEvent={(val) => {
              const num = parseInt(val, 10);
              return isNaN(num) ? undefined : num;
            }}
          >
            <ImeSafeAutoComplete
              options={CONTEXT_LIMIT_PRESETS}
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
            <ImeSafeAutoComplete
              options={OUTPUT_LIMIT_PRESETS}
              placeholder={t('openclaw.providers.outputLimitPlaceholder')}
              style={{ width: '100%' }}
              filterOption={(inputValue, option) =>
                (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
                  option?.value.includes(inputValue)) ?? false
              }
            />
          </Form.Item>
        </div>

        <Form.Item name="reasoning" label={t('openclaw.providers.modelCapabilities')} valuePropName="checked">
          <Checkbox styles={MODEL_FORM_CHECKBOX_STYLES}>{t('openclaw.providers.reasoning')}</Checkbox>
        </Form.Item>

        {/* Advanced: Cost fields + Extra params */}
        <div style={{ marginBottom: advancedExpanded ? 16 : 0 }}>
          <Button
            type="link"
            onClick={() => setAdvancedExpanded(!advancedExpanded)}
            style={{ padding: 0, height: 'auto' }}
          >
            {advancedExpanded ? <DownOutlined /> : <RightOutlined />}
            <span style={{ marginLeft: 4 }}>
              {t('common.advancedSettings')}
              {hasAdvancedContent && !advancedExpanded && (
                <span style={{ marginLeft: 4, color: '#1890ff' }}>*</span>
              )}
            </span>
          </Button>
        </div>
        {advancedExpanded && (
          <>
            {/* Cost settings — dashed border fieldset */}
            <fieldset
              style={{
                border: '1px dashed var(--ant-color-border)',
                borderRadius: 8,
                padding: '20px 16px 4px',
                margin: '0 0 16px',
                position: 'relative',
              }}
            >
              <legend
                style={{
                  width: 'auto',
                  padding: '0 8px',
                  margin: '0 auto',
                  fontSize: 12,
                  color: 'var(--ant-color-text-secondary)',
                  textAlign: 'center',
                  lineHeight: 1,
                }}
              >
                {t('openclaw.providers.costSettings')} ($/M tokens)
              </legend>
              <Row gutter={16}>
                <Col span={12}>
                  <Form.Item name="costInput" label={t('openclaw.providers.costInput')}>
                    <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                  </Form.Item>
                </Col>
                <Col span={12}>
                  <Form.Item name="costOutput" label={t('openclaw.providers.costOutput')}>
                    <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                  </Form.Item>
                </Col>
              </Row>
              <Row gutter={16}>
                <Col span={12}>
                  <Form.Item name="costCacheRead" label={t('openclaw.providers.costCacheRead')}>
                    <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                  </Form.Item>
                </Col>
                <Col span={12}>
                  <Form.Item name="costCacheWrite" label={t('openclaw.providers.costCacheWrite')}>
                    <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
                  </Form.Item>
                </Col>
              </Row>
            </fieldset>

            {/* Extra params — JSON editor */}
            <Form.Item
              label={t('openclaw.providers.extraParams')}
              extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('openclaw.providers.extraParamsHint')}</Text>}
            >
              <JsonEditor
                value={extraParamsValue}
                onChange={handleExtraParamsChange}
                mode="text"
                height={150}
                minHeight={100}
                maxHeight={300}
                resizable
                placeholder={`{
    "input": ["text", "image"],
    "compat": { "streaming": true }
}`}
              />
            </Form.Item>
          </>
        )}
      </Form>
    </Modal>
  );
};

export default OpenClawModelFormModal;
