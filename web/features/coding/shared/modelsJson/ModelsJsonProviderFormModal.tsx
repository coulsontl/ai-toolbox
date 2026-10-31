import React from 'react';
import { Form, Input, Modal, Typography } from 'antd';
import { useTranslation } from 'react-i18next';

const { Text } = Typography;

const LABEL_COL = { span: 6 };
const WRAPPER_COL = { span: 18 };

export interface ModelsJsonProviderFormValues {
  /** Shown as the channel's title; written onto the first model entry. */
  vendor: string;
  /** Endpoint for every model of this channel. */
  baseUrl: string;
  apiKey?: string;
  /** The first model entry this channel starts with. */
  modelId: string;
  modelName?: string;
}

interface Props {
  open: boolean;
  saving?: boolean;
  onCancel: () => void;
  onSubmit: (values: ModelsJsonProviderFormValues) => Promise<void> | void;
}

/**
 * Creates one channel.
 *
 * `models.json` has no channel record: a channel is a *view* over model entries
 * grouped by `(endpoint, api key)` (`workbuddy_family/channel.rs`). So the only
 * way to create a channel is to write a first model entry that carries the
 * connection — which is why this form asks for one model id alongside the
 * endpoint. Everything else about that model (limits, capability switches) is
 * edited afterwards from the card, exactly like the second and third models.
 *
 * Deliberately no model capability switches here: this dialog is the SOP's
 * "添加供应商" step, and putting per-model options in it would recreate the
 * "add provider asks you to pick a model" problem this replaces.
 */
const ModelsJsonProviderFormModal: React.FC<Props> = ({
  open,
  saving = false,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<ModelsJsonProviderFormValues>();

  const handleOk = async () => {
    const values = await form.validateFields();
    await onSubmit(values);
  };

  return (
    <Modal
      open={open}
      title={t('common.provider.add')}
      onCancel={onCancel}
      onOk={() => {
        void handleOk();
      }}
      // `loading` on okButtonProps also locks cancel; the caller keeps its own
      // spinner while saving.
      okButtonProps={{ disabled: saving }}
      width={720}
      destroyOnHidden
    >
      <Text type="secondary" style={{ display: 'block', marginBottom: 16 }}>
        {t('workbuddy.providerModal.intro')}
      </Text>
      <Form form={form} layout="horizontal" labelCol={LABEL_COL} wrapperCol={WRAPPER_COL}>
        <Form.Item name="vendor" label={t('workbuddy.model.vendor')}>
          <Input placeholder="Acme" />
        </Form.Item>
        <Form.Item
          name="baseUrl"
          label={t('workbuddy.model.baseUrl')}
          rules={[{ required: true, message: t('common.required') }]}
        >
          <Input placeholder="https://api.example.com/v1" />
        </Form.Item>
        <Form.Item name="apiKey" label={t('workbuddy.model.apiKey')}>
          <Input.Password />
        </Form.Item>
        <Form.Item
          name="modelId"
          label={t('workbuddy.providerModal.firstModelId')}
          rules={[{ required: true, message: t('common.required') }]}
          extra={t('workbuddy.providerModal.firstModelIdHint')}
        >
          <Input placeholder="gpt-4.1" />
        </Form.Item>
        <Form.Item name="modelName" label={t('workbuddy.model.name')}>
          <Input />
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default ModelsJsonProviderFormModal;
