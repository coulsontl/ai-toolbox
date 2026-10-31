import React from 'react';
import { Checkbox, Form, Input, Modal, Select, Tooltip, Typography } from 'antd';
import { InfoCircleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { KimiCatalogModel } from '@/types/kimi';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';
import {
  CONTEXT_LIMIT_PRESETS,
  OUTPUT_LIMIT_PRESETS,
} from '@/features/coding/shared/modelLimitPresets';
import {
  KIMI_SUPPORTED_EFFORTS,
  normalizeKimiCapabilities,
  normalizeKimiStringArray,
} from '../utils/kimiCatalogModels';

import {
  MODEL_FORM_EXTRA_STYLE,
  MODEL_FORM_LAYOUT,
  MODEL_FORM_PAIR_STYLE,
  MODEL_FORM_TRIPLE_STYLE,
} from '@/features/coding/shared/modelFormLayout';
import modelFormStyles from '../../shared/modelFormLayout.module.less';

const { Text } = Typography;

/** `InputNumber`'s parse, kept as one object so the three size fields match. */
const numberFromInput = (value: string | number | undefined) => {
  const parsed = typeof value === 'number' ? value : parseInt(value ?? '', 10);
  return Number.isNaN(parsed) ? undefined : parsed;
};

/**
 * One switch per capability, in the order the CLI documents them.
 *
 * A `Select mode="multiple"` made the user open a dropdown, read six bare
 * identifiers (`image_in`, `always_thinking`, …) and pick from a list whose
 * meaning lived only in the hint underneath. Six booleans fit on one line, so
 * each is a checkbox with a readable caption and the identifier stays in the
 * stored value (user-directed, 2026-11-01: 「kimi 的能力声明 改成勾选」).
 */
const CAPABILITY_FIELDS: { value: string; labelKey: string }[] = [
  { value: 'image_in', labelKey: 'kimi.model.capabilityImageIn' },
  { value: 'video_in', labelKey: 'kimi.model.capabilityVideoIn' },
  { value: 'audio_in', labelKey: 'kimi.model.capabilityAudioIn' },
  { value: 'thinking', labelKey: 'kimi.model.capabilityThinking' },
  { value: 'always_thinking', labelKey: 'kimi.model.capabilityAlwaysThinking' },
  { value: 'tool_use', labelKey: 'kimi.model.capabilityToolUse' },
];

const EFFORT_OPTIONS = KIMI_SUPPORTED_EFFORTS.map((value) => ({
  value,
  label: value,
}));

interface KimiModelFormModalProps {
  open: boolean;
  isEdit: boolean;
  /** Row being edited; omitted when adding a new model. */
  initialValues?: KimiCatalogModel;
  /** Provider key used as the default for new rows. */
  providerKey: string;
  /** Alias keys already taken by the other rows, used to reject collisions. */
  existingKeys?: string[];
  onCancel: () => void;
  onSubmit: (model: KimiCatalogModel) => void | Promise<void>;
}

interface KimiModelFormValues {
  key: string;
  model: string;
  displayName?: string;
  maxContextSize?: number;
  maxInputSize?: number;
  maxOutputSize?: number;
  reasoningKey?: string;
  capabilities?: string[];
  supportEfforts?: string[];
  defaultEffort?: string;
}

function toFormValues(item: KimiCatalogModel): KimiModelFormValues {
  return {
    key: item.key,
    model: item.model,
    displayName: item.displayName,
    maxContextSize: item.maxContextSize,
    maxInputSize: item.maxInputSize,
    maxOutputSize: item.maxOutputSize,
    reasoningKey: item.reasoningKey,
    capabilities: item.capabilities,
    supportEfforts: item.supportEfforts,
    defaultEffort: item.defaultEffort,
  };
}

function fromFormValues(
  values: KimiModelFormValues,
  providerKey: string,
  base?: KimiCatalogModel,
): KimiCatalogModel {
  const key = values.key.trim();
  const model = values.model.trim();
  const row: KimiCatalogModel = {
    key: key || model,
    model: model || key,
    // The dialog does not edit the provider reference, so an existing row keeps
    // whatever provider table it pointed at; new rows default to this
    // provider's key.
    provider: base?.provider?.trim() || providerKey,
  };
  const displayName = values.displayName?.trim();
  if (displayName) row.displayName = displayName;
  // The CLI validates these as int().min(1); a zero/negative input is dropped
  // by the normalizer rather than written back as an invalid value.
  if (typeof values.maxContextSize === 'number' && values.maxContextSize > 0) {
    row.maxContextSize = Math.floor(values.maxContextSize);
  }
  if (typeof values.maxInputSize === 'number' && values.maxInputSize > 0) {
    row.maxInputSize = Math.floor(values.maxInputSize);
  }
  if (typeof values.maxOutputSize === 'number' && values.maxOutputSize > 0) {
    row.maxOutputSize = Math.floor(values.maxOutputSize);
  }
  const reasoningKey = values.reasoningKey?.trim();
  if (reasoningKey) row.reasoningKey = reasoningKey;
  const capabilities = normalizeKimiCapabilities(values.capabilities);
  if (capabilities) row.capabilities = capabilities;
  const supportEfforts = normalizeKimiStringArray(values.supportEfforts);
  if (supportEfforts) row.supportEfforts = supportEfforts;
  const requestedDefaultEffort = values.defaultEffort?.trim();
  if (requestedDefaultEffort && supportEfforts?.includes(requestedDefaultEffort)) {
    row.defaultEffort = requestedDefaultEffort;
  }
  // The dialog does not edit the per-model keys carried in `extraConfig`
  // (protocol, adaptive_thinking, ...), so they must ride along or editing an
  // unrelated field would delete them from config.toml.
  if (base?.extraConfig && typeof base.extraConfig === 'object') {
    row.extraConfig = base.extraConfig;
  }
  return row;
}

/**
 * Kimi catalog model editor.
 *
 * The catalog maps 1:1 onto the `[models."<key>"]` tables projected into
 * `config.toml`, so every field here has a direct TOML counterpart.
 */
const KimiModelFormModal: React.FC<KimiModelFormModalProps> = ({
  open,
  isEdit,
  initialValues,
  providerKey,
  existingKeys = [],
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<KimiModelFormValues>();
  const [submitting, setSubmitting] = React.useState(false);
  const supportEfforts = Form.useWatch('supportEfforts', form) as string[] | undefined;

  // The alias key is the `[models."<key>"]` table name, so two rows sharing one
  // would collapse into a single config.toml table on projection and make the
  // card's delete action remove both. Reject the collision at the field.
  const takenKeys = React.useMemo(
    () => new Set(existingKeys.map((key) => key.trim()).filter(Boolean)),
    [existingKeys],
  );

  React.useEffect(() => {
    if (!open) {
      return;
    }
    form.setFieldsValue({
      key: '',
      model: '',
      displayName: undefined,
      maxContextSize: undefined,
      maxInputSize: undefined,
      maxOutputSize: undefined,
      reasoningKey: undefined,
      capabilities: undefined,
      supportEfforts: undefined,
      defaultEffort: undefined,
      ...(initialValues ? toFormValues(initialValues) : {}),
    });
  }, [form, initialValues, open]);

  /**
   * A default effort is only meaningful while it is one of the selected
   * efforts, but the cleanup must run on explicit user edits only — running it
   * from an effect would race the open-time backfill.
   */
  const handleSupportEffortsChange = (efforts: string[] | undefined) => {
    const current = form.getFieldValue('defaultEffort') as string | undefined;
    if (current && !(efforts ?? []).includes(current)) {
      form.setFieldValue('defaultEffort', undefined);
    }
  };

  const handleOk = async () => {
    try {
      const values = await form.validateFields();
      setSubmitting(true);
      await onSubmit(fromFormValues(values, providerKey, initialValues));
    } finally {
      setSubmitting(false);
    }
  };


  return (
    <Modal
      open={open}
      title={isEdit ? t('kimi.model.editTitle') : t('kimi.model.addTitle')}
      onCancel={onCancel}
      onOk={() => void handleOk()}
      confirmLoading={submitting}
      destroyOnHidden
      width={640}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form
        form={form}
        layout={MODEL_FORM_LAYOUT}
        classNames={{ label: modelFormStyles.label }}
        styles={{ extra: MODEL_FORM_EXTRA_STYLE }}
        style={{ marginTop: 24 }}
      >
        <Form.Item
          name="key"
          label={t('kimi.model.key')}
          rules={[
            { required: true, message: t('common.error') },
            {
              validator: (_rule, value: string) => {
                const key = (value ?? '').trim();
                if (key && takenKeys.has(key)) {
                  return Promise.reject(new Error(t('kimi.model.keyDuplicate')));
                }
                return Promise.resolve();
              },
            },
          ]}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('kimi.model.keyHint')}</Text>}
        >
          <Input placeholder="moonshotai/kimi-k3" />
        </Form.Item>

        <Form.Item
          name="model"
          label={t('kimi.model.upstreamId')}
          rules={[{ required: true, message: t('common.error') }]}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('kimi.model.upstreamIdHint')}</Text>}
        >
          <Input placeholder="kimi-k3" />
        </Form.Item>

        <Form.Item name="displayName" label={t('kimi.model.displayName')}>
          <Input placeholder={t('kimi.model.displayNamePlaceholder')} />
        </Form.Item>

        {/* Three sizes of the same payload — how much it holds, how much may go
            in, how much may come back — so they share one row, the same shape
            `ModelsJsonModelModal` uses for its limits (see
            `MODEL_FORM_TRIPLE_STYLE`).

            `ImeSafeAutoComplete` over the shared presets, not `InputNumber`:
            the common values are a short, known list (128K / 200K / 256K for a
            modern Kimi), and offering them beats asking a user to type six
            digits from memory — while the box still accepts any number the
            provider happens to use (user-directed, 2026-11-01: 「这3个也改成
            select 的 option」). `getValueFromEvent` keeps the form value a
            number, matching the stored shape. */}
        <div style={MODEL_FORM_TRIPLE_STYLE}>
          <Form.Item
            name="maxContextSize"
            getValueFromEvent={numberFromInput}
            label={
              <span>
                {t('kimi.model.contextSize')}:
                <Tooltip title={t('kimi.model.contextSizeTooltip')}>
                  <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
                </Tooltip>
              </span>
            }
          >
            <ImeSafeAutoComplete
              options={CONTEXT_LIMIT_PRESETS}
              placeholder="262144"
              style={{ width: '100%' }}
              filterOption={(inputValue, option) =>
                (option?.label.toLowerCase().includes(inputValue.toLowerCase())
                  || option?.value.includes(inputValue)) ?? false
              }
            />
          </Form.Item>

          <Form.Item
            name="maxInputSize"
            getValueFromEvent={numberFromInput}
            label={
              <span>
                {t('kimi.model.maxInputSize')}:
                <Tooltip title={t('kimi.model.maxInputSizeTooltip')}>
                  <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
                </Tooltip>
              </span>
            }
          >
            <ImeSafeAutoComplete
              options={CONTEXT_LIMIT_PRESETS}
              placeholder="262144"
              style={{ width: '100%' }}
              filterOption={(inputValue, option) =>
                (option?.label.toLowerCase().includes(inputValue.toLowerCase())
                  || option?.value.includes(inputValue)) ?? false
              }
            />
          </Form.Item>

          <Form.Item
            name="maxOutputSize"
            getValueFromEvent={numberFromInput}
            label={
              <span>
                {t('kimi.model.maxOutputSize')}:
                <Tooltip title={t('kimi.model.maxOutputSizeTooltip')}>
                  <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
                </Tooltip>
              </span>
            }
          >
            <ImeSafeAutoComplete
              options={OUTPUT_LIMIT_PRESETS}
              placeholder="262144"
              style={{ width: '100%' }}
              filterOption={(inputValue, option) =>
                (option?.label.toLowerCase().includes(inputValue.toLowerCase())
                  || option?.value.includes(inputValue)) ?? false
              }
            />
          </Form.Item>
        </div>

        <Form.Item
          name="reasoningKey"
          label={
            <span>
              {t('kimi.model.reasoningKey')}:
              <Tooltip title={t('kimi.model.reasoningKeyTooltip')}>
                <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
              </Tooltip>
            </span>
          }
        >
          <Input placeholder="reasoning_content" />
        </Form.Item>

        {/* Six checkboxes instead of a multi-select: the field is a set of
            independent flags, so a dropdown made the user open it just to see
            what could be declared, and each option was a bare identifier
            (`image_in`, `always_thinking`) whose meaning lived in the hint.

            `Checkbox.Group` rather than six `Form.Item name={[...]}`: the stored
            value is a flat `string[]`, and a nested field name would turn it
            into an object. */}
        <Form.Item
          name="capabilities"
          label={t('kimi.model.capabilities')}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('kimi.model.capabilitiesHint')}</Text>}
        >
          <Checkbox.Group
            className={modelFormStyles.capabilityCheckboxGroup}
            options={CAPABILITY_FIELDS.map((field) => ({ value: field.value, label: t(field.labelKey) }))}
          />
        </Form.Item>

        {/* The supported efforts and the default one are the same setting read
            two ways — the default is a member of the list — so they share a
            row (see `MODEL_FORM_PAIR_STYLE`). */}
        <div style={MODEL_FORM_PAIR_STYLE}>
          <Form.Item
            name="supportEfforts"
            label={t('kimi.model.supportEfforts')}
            extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('kimi.model.supportEffortsHint')}</Text>}
          >
            <Select
              mode="multiple"
              allowClear
              placeholder={t('kimi.model.supportEffortsPlaceholder')}
              options={EFFORT_OPTIONS}
              onChange={handleSupportEffortsChange}
            />
          </Form.Item>

          <Form.Item
            name="defaultEffort"
            label={t('kimi.model.defaultEffort')}
            extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('kimi.model.defaultEffortHint')}</Text>}
          >
            <Select
              allowClear
              disabled={!supportEfforts || supportEfforts.length === 0}
              placeholder={t('kimi.model.defaultEffort')}
              options={(supportEfforts ?? []).map((value) => ({ value, label: value }))}
            />
          </Form.Item>
        </div>

        <Form.Item style={{ marginBottom: 0 }}>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t('kimi.model.editModelHint')}
          </Text>
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default KimiModelFormModal;
