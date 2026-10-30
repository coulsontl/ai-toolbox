import React from 'react';
import { Form, Input, InputNumber, Modal, Space, Switch, Typography, type FormInstance } from 'antd';
import { useTranslation } from 'react-i18next';
import type {
  WorkbuddyFamilyModelInput,
  WorkbuddyFamilyProviderView,
} from '@/types/workbuddyFamily';

const { Text } = Typography;

const LABEL_COL = { span: 6 };
const WRAPPER_COL = { span: 18 };

interface Props {
  open: boolean;
  editing: boolean;
  form: FormInstance<WorkbuddyFamilyModelInput>;
  /** The channel the entry belongs to; `null` while adding a brand-new one. */
  channel: WorkbuddyFamilyProviderView | null;
  onCancel: () => void;
  onSubmit: (values: WorkbuddyFamilyModelInput) => Promise<void>;
}

/**
 * One model entry of a `models.json` channel.
 *
 * The endpoint and API key live on every entry (the file has no channel
 * object), so they are duplicated here rather than lifted into a shared
 * header — editing a model is also how a channel's endpoint is corrected.
 *
 * `useCustomProtocol` skips the product's URL normalization, so its hint sits
 * directly under the switch instead of in a page-level note.
 */
const ModelsJsonModelModal: React.FC<Props> = ({
  open,
  editing,
  form,
  channel,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [saving, setSaving] = React.useState(false);

  const handleOk = async () => {
    const values = await form.validateFields();
    setSaving(true);
    try {
      await onSubmit(values);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={editing ? t('workbuddy.editModel') : t('workbuddy.addModel')}
      onCancel={onCancel}
      onOk={() => {
        void handleOk();
      }}
      // The cancel button must stay clickable while saving: antd's `loading`
      // on okButtonProps also locks cancel, so the spinner lives in the body.
      okButtonProps={{ disabled: saving }}
      width={720}
      destroyOnHidden
    >
      <Form form={form} layout="horizontal" labelCol={LABEL_COL} wrapperCol={WRAPPER_COL}>
        {/*
          `previousId` must be a registered field, not just a value parked on the
          form: `validateFields()` collects only registered names and drops the
          rest, so the rename hint would silently never reach the backend and an
          ID edit became "add a second row".
        */}
        <Form.Item name="previousId" hidden noStyle>
          <Input />
        </Form.Item>
        <Form.Item
          name="id"
          label={t('workbuddy.model.id')}
          rules={[{ required: true, message: t('common.required') }]}
        >
          <Input placeholder="gpt-4.1" />
        </Form.Item>
        <Form.Item name="name" label={t('workbuddy.model.name')}>
          <Input />
        </Form.Item>
        <Form.Item name="vendor" label={t('workbuddy.model.vendor')}>
          <Input />
        </Form.Item>
        <Form.Item
          name="baseUrl"
          label={t('workbuddy.model.baseUrl')}
          rules={[{ required: true, message: t('common.required') }]}
          extra={channel
            ? t('workbuddy.model.baseUrlChannelHint')
            : undefined}
        >
          <Input placeholder="https://api.example.com/v1" />
        </Form.Item>
        <Form.Item name="apiKey" label={t('workbuddy.model.apiKey')}>
          {/* `Input.Password` ships its own show/hide toggle in antd 6. */}
          <Input.Password />
        </Form.Item>
        <Form.Item name="maxInputTokens" label={t('workbuddy.model.maxInputTokens')}>
          <InputNumber min={1} style={{ width: '100%' }} />
        </Form.Item>
        <Form.Item name="maxOutputTokens" label={t('workbuddy.model.maxOutputTokens')}>
          <InputNumber min={1} style={{ width: '100%' }} />
        </Form.Item>
        <Form.Item name="temperature" label={t('workbuddy.model.temperature')}>
          <InputNumber min={0} max={2} step={0.1} style={{ width: '100%' }} />
        </Form.Item>
        <Form.Item name="supportsToolCall" label={t('workbuddy.model.supportsToolCall')} valuePropName="checked">
          <Switch />
        </Form.Item>
        <Form.Item name="supportsImages" label={t('workbuddy.model.supportsImages')} valuePropName="checked">
          <Switch />
        </Form.Item>
        <Form.Item name="supportsReasoning" label={t('workbuddy.model.supportsReasoning')} valuePropName="checked">
          <Switch />
        </Form.Item>
        <Form.Item name="onlyReasoning" label={t('workbuddy.model.onlyReasoning')} valuePropName="checked">
          <Switch />
        </Form.Item>
        <Form.Item
          name="useCustomProtocol"
          label={t('workbuddy.model.useCustomProtocol')}
          valuePropName="checked"
        >
          <Switch />
        </Form.Item>
        <Form.Item wrapperCol={{ offset: LABEL_COL.span, span: WRAPPER_COL.span }}>
          <Space>
            <Text type="secondary">{t('workbuddy.model.useCustomProtocolHint')}</Text>
          </Space>
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default ModelsJsonModelModal;
