import React from 'react';
import {
  Modal,
  Form,
  Input,
  Select,
  InputNumber,
  Checkbox,
  Typography,
  Tag,
  Divider,
} from 'antd';
import { useTranslation } from 'react-i18next';
import {
  PRESET_MODELS,
  getPresetModelsVersion,
  subscribePresetModels,
  type PresetModel,
} from '@/constants/presetModels';
import type { GrokApiFormat } from '@/types/grok';
import {
  extractReasoningEffortsFromPresetVariants,
  getGrokPrimaryPresetNpmTypes,
  GROK_MODEL_REASONING_EFFORT_OPTIONS,
  pickDefaultReasoningEffort,
  type GrokModelFormValues,
} from '../utils/grokProviderModels';

import {
  MODEL_FORM_CHECKBOX_STYLES,
  MODEL_FORM_EXTRA_STYLE,
  MODEL_FORM_LAYOUT,
  MODEL_FORM_PAIR_STYLE,
} from '@/features/coding/shared/modelFormLayout';
import modelFormStyles from '../../shared/modelFormLayout.module.less';

const { Text } = Typography;

interface GrokModelFormModalProps {
  open: boolean;
  isEdit: boolean;
  /** Channel API format used to pick primary preset groups (xAI + matching SDK). */
  apiFormat?: GrokApiFormat | string | null;
  initialValues?: Partial<GrokModelFormValues>;
  existingKeys?: string[];
  onCancel: () => void;
  onSubmit: (values: GrokModelFormValues) => void | Promise<void>;
}

/**
 * Grok model editor: reuses the OpenCode modal shell (preset strip + form layout)
 * but fields map to Grok Build catalog / config.toml semantics.
 *
 * Primary presets: xAI + the single OpenCode SDK group for the current channel
 * API type (Chat / Responses / Anthropic are distinct). Remaining SDKs stay
 * under "other presets". Selecting a preset parses its variants into
 * `reasoningEfforts` (all selected).
 */
const GrokModelFormModal: React.FC<GrokModelFormModalProps> = ({
  open,
  isEdit,
  apiFormat,
  initialValues,
  existingKeys = [],
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<GrokModelFormValues>();
  const [submitting, setSubmitting] = React.useState(false);
  const [presetsExpanded, setPresetsExpanded] = React.useState(false);
  const reasoningEfforts = Form.useWatch('reasoningEfforts', form) as string[] | undefined;
  const presetModelsVersion = React.useSyncExternalStore(
    subscribePresetModels,
    getPresetModelsVersion,
    getPresetModelsVersion,
  );

  // Primary strip: xAI + current channel protocol family. Rest go to "other".
  const { primaryPresets, otherPresets, allPresets } = React.useMemo(() => {
    const primaryNpmTypes = getGrokPrimaryPresetNpmTypes(apiFormat);
    const primaryNpmSet = new Set(primaryNpmTypes);
    const primary: PresetModel[] = [];
    const other: PresetModel[] = [];
    const seen = new Set<string>();

    const pushUnique = (target: PresetModel[], preset: PresetModel) => {
      const id = preset.id?.trim();
      if (!id || seen.has(id)) {
        return;
      }
      seen.add(id);
      target.push(preset);
    };

    // Keep declared primary group order (xAI first, then protocol-matched SDKs).
    primaryNpmTypes.forEach((npmType) => {
      (PRESET_MODELS[npmType] || []).forEach((preset) => {
        pushUnique(primary, preset);
      });
    });
    Object.entries(PRESET_MODELS).forEach(([npmType, models]) => {
      if (primaryNpmSet.has(npmType)) {
        return;
      }
      models.forEach((preset) => {
        pushUnique(other, preset);
      });
    });

    return {
      primaryPresets: primary,
      otherPresets: other,
      allPresets: [...primary, ...other],
    };
  }, [apiFormat, presetModelsVersion]);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    form.setFieldsValue({
      key: '',
      model: '',
      displayName: '',
      contextWindow: undefined,
      reasoningEfforts: [],
      reasoningEffort: undefined,
      supportsBackendSearch: false,
      ...initialValues,
    });
    setPresetsExpanded(false);
  }, [form, initialValues, open]);

  /**
   * Keep the default effort inside the selected list, but only on explicit user
   * edits. Cleaning this up from an effect races the open-time backfill: the
   * first render still sees an empty `reasoningEfforts` watch value and wipes
   * the restored default effort.
   */
  const handleReasoningEffortsChange = (efforts: string[] | undefined) => {
    const current = form.getFieldValue('reasoningEffort') as string | undefined;
    if (!efforts || efforts.length === 0) {
      if (current) {
        form.setFieldValue('reasoningEffort', undefined);
      }
      return;
    }
    if (current && !efforts.includes(current)) {
      form.setFieldValue('reasoningEffort', pickDefaultReasoningEffort(efforts));
    }
  };

  const applyPreset = (preset: PresetModel) => {
    // Parse every effort-like token from variants and select ALL of them.
    // Local key and upstream model id always stay the same value.
    const mergedEfforts = extractReasoningEffortsFromPresetVariants(preset.variants);
    const modelId = preset.id;
    const nextKey = isEdit
      ? (form.getFieldValue('model') as string) || modelId
      : modelId;

    form.setFieldsValue({
      key: nextKey,
      model: nextKey,
      displayName: preset.name || nextKey,
      contextWindow: preset.contextLimit,
      // Default: full multi-select of every parsed effort level.
      reasoningEfforts: mergedEfforts,
      reasoningEffort: pickDefaultReasoningEffort(mergedEfforts),
    });
    setPresetsExpanded(false);
  };

  const handleOk = async () => {
    try {
      const values = await form.validateFields();
      setSubmitting(true);
      const efforts = (values.reasoningEfforts || [])
        .map((effort) => effort.trim())
        .filter(Boolean);
      const effort = values.reasoningEffort?.trim();
      // Keep local catalog key identical to the upstream model id.
      const modelId = values.model.trim();
      await onSubmit({
        key: modelId,
        model: modelId,
        displayName: values.displayName?.trim() || modelId,
        contextWindow: values.contextWindow,
        reasoningEfforts: efforts,
        reasoningEffort: effort && efforts.includes(effort) ? effort : efforts[0],
        supportsBackendSearch: values.supportsBackendSearch === true,
      });
    } finally {
      setSubmitting(false);
    }
  };

  const previousModelId = (initialValues?.model || initialValues?.key || '').trim();

  return (
    <Modal
      open={open}
      title={isEdit ? t('grok.model.editTitle') : t('grok.model.addTitle')}
      onCancel={onCancel}
      onOk={() => void handleOk()}
      confirmLoading={submitting}
      destroyOnHidden
      width={640}
    >
      <Form
        form={form}
        layout={MODEL_FORM_LAYOUT}
        classNames={{ label: modelFormStyles.label }}
        styles={{ extra: MODEL_FORM_EXTRA_STYLE }}
        style={{ marginTop: 24 }}
      >
        <Form.Item label={t('grok.model.upstreamId')} required>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Form.Item
              name="model"
              noStyle
              rules={[
                { required: true, message: t('common.error') },
                {
                  validator: async (_, value: string) => {
                    const modelId = value?.trim();
                    if (!modelId) {
                      return;
                    }
                    if (modelId.includes('[') || modelId.includes(']')) {
                      throw new Error(t('grok.model.invalidKey'));
                    }
                    // New model, or rename: reject colliding ids in the channel catalog.
                    if (existingKeys.includes(modelId) && modelId !== previousModelId) {
                      throw new Error(t('grok.model.duplicateKey'));
                    }
                  },
                },
              ]}
            >
              <Input
                placeholder="grok-4.5"
                style={{ flex: 1 }}
                onChange={(event) => {
                  // Keep hidden key field in sync so legacy readers still see key===model.
                  const modelId = event.target.value.trim();
                  form.setFieldValue('key', modelId);
                }}
                onBlur={(event) => {
                  const modelId = event.target.value.trim();
                  if (!form.getFieldValue('displayName')) {
                    form.setFieldValue('displayName', modelId);
                  }
                }}
              />
            </Form.Item>
            {/* Hidden key always mirrors model id (Grok local key === upstream id). */}
            <Form.Item name="key" hidden>
              <Input />
            </Form.Item>
            {allPresets.length > 0 && (
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
                onClick={() => setPresetsExpanded((prev) => !prev)}
              >
                {t('grok.model.selectPreset')}
                {presetsExpanded ? ' ▴' : ' ▾'}
              </a>
            )}
          </div>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t('grok.model.upstreamIdSameAsKeyHint')}
          </Text>
        </Form.Item>

        {presetsExpanded && allPresets.length > 0 && (
          <Form.Item style={{ marginTop: -8 }}>
            {/* Primary: xAI + current channel API family. Other SDKs stay secondary. */}
            {primaryPresets.length > 0 && (
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, maxHeight: 220, overflowY: 'auto' }}>
                {primaryPresets.map((preset) => (
                  <Tag
                    key={`primary-${preset.id}`}
                    style={{ cursor: 'pointer' }}
                    onClick={() => applyPreset(preset)}
                  >
                    {preset.name}
                  </Tag>
                ))}
              </div>
            )}
            {otherPresets.length > 0 && (
              <>
                <Divider style={{ margin: '12px 0', fontSize: 12 }}>
                  {t('grok.model.otherPresets')}
                </Divider>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, maxHeight: 220, overflowY: 'auto' }}>
                  {otherPresets.map((preset) => (
                    <Tag
                      key={`other-${preset.id}-${preset.name}`}
                      style={{ cursor: 'pointer' }}
                      onClick={() => applyPreset(preset)}
                    >
                      {preset.name}
                    </Tag>
                  ))}
                </div>
              </>
            )}
          </Form.Item>
        )}

        <Form.Item name="displayName" label={t('grok.model.displayName')}>
          <Input placeholder={t('grok.model.displayNamePlaceholder')} />
        </Form.Item>

        <Form.Item name="contextWindow" label={t('grok.model.contextWindow')}>
          <InputNumber min={1} style={{ width: '100%' }} placeholder="256000" />
        </Form.Item>

        {/* The supported efforts and the default one are the same setting read
            two ways — the default is a member of the list — so they share a
            row (see `MODEL_FORM_PAIR_STYLE`). */}
        <div style={MODEL_FORM_PAIR_STYLE}>
          <Form.Item
            name="reasoningEfforts"
            label={t('grok.model.reasoningEfforts')}
            extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('grok.model.reasoningEffortsHint')}</Text>}
          >
            <Select
              mode="multiple"
              allowClear
              placeholder={t('grok.model.reasoningEffortsPlaceholder')}
              options={GROK_MODEL_REASONING_EFFORT_OPTIONS.map((value) => ({
                value,
                label: value,
              }))}
              onChange={handleReasoningEffortsChange}
            />
          </Form.Item>

          <Form.Item
            name="reasoningEffort"
            label={t('grok.model.reasoningEffortDefault')}
            extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('grok.model.reasoningEffortDefaultHint')}</Text>}
          >
            <Select
              allowClear
              placeholder={t('grok.model.reasoningEffortDefaultPlaceholder')}
              options={(reasoningEfforts || []).map((value) => ({
                value,
                label: value,
              }))}
              disabled={!reasoningEfforts || reasoningEfforts.length === 0}
            />
          </Form.Item>
        </div>

        {/* No `wrapperCol` offset: it was a horizontal-layout way of lining the
            checkbox up under the label column, and under `layout="vertical"`
            there is no column — it only indented and narrowed the control. */}
        <Form.Item name="supportsBackendSearch" valuePropName="checked">
          <Checkbox styles={MODEL_FORM_CHECKBOX_STYLES}>{t('grok.model.supportsBackendSearch')}</Checkbox>
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default GrokModelFormModal;
