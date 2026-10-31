import React from 'react';
import { Modal, Form, Input, InputNumber, Select, Tooltip, Typography } from 'antd';
import { InfoCircleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { KimiCatalogModel } from '@/types/kimi';
import {
  KIMI_SUPPORTED_CAPABILITIES,
  KIMI_SUPPORTED_EFFORTS,
  normalizeKimiCapabilities,
  normalizeKimiStringArray,
} from '../utils/kimiCatalogModels';

import { MODEL_FORM_LAYOUT } from '@/features/coding/shared/modelFormLayout';

const { Text } = Typography;

const CAPABILITY_OPTIONS = KIMI_SUPPORTED_CAPABILITIES.map((value) => ({
  value,
  label: value,
}));

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

        <Form.Item
          name="maxContextSize"
          label={
            <span>
              {t('kimi.model.contextSize')}
              <Tooltip title={t('kimi.model.contextSizeTooltip')}>
                <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
              </Tooltip>
            </span>
          }
        >
          <InputNumber min={1} style={{ width: '100%' }} placeholder="262144" />
        </Form.Item>

        <Form.Item
          name="maxInputSize"
          label={
            <span>
              {t('kimi.model.maxInputSize')}
              <Tooltip title={t('kimi.model.maxInputSizeTooltip')}>
                <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
              </Tooltip>
            </span>
          }
        >
          <InputNumber min={1} style={{ width: '100%' }} placeholder="262144" />
        </Form.Item>

        <Form.Item
          name="maxOutputSize"
          label={
            <span>
              {t('kimi.model.maxOutputSize')}
              <Tooltip title={t('kimi.model.maxOutputSizeTooltip')}>
                <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
              </Tooltip>
            </span>
          }
        >
          <InputNumber min={1} style={{ width: '100%' }} placeholder="262144" />
        </Form.Item>

        <Form.Item
          name="reasoningKey"
          label={
            <span>
              {t('kimi.model.reasoningKey')}
              <Tooltip title={t('kimi.model.reasoningKeyTooltip')}>
                <InfoCircleOutlined style={{ marginLeft: 6, color: 'var(--color-text-tertiary)' }} />
              </Tooltip>
            </span>
          }
        >
          <Input placeholder="reasoning_content" />
        </Form.Item>

        <Form.Item
          name="capabilities"
          label={t('kimi.model.capabilities')}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('kimi.model.capabilitiesHint')}</Text>}
        >
          <Select
            mode="multiple"
            allowClear
            placeholder={t('kimi.model.capabilitiesPlaceholder')}
            options={CAPABILITY_OPTIONS}
          />
        </Form.Item>

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
