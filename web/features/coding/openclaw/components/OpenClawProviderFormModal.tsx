import React from 'react';
import { Modal, Form, Input, Select, Button } from 'antd';
import { ImportOutlined, EyeOutlined, EyeInvisibleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useAppStore } from '@/stores';
import type { OpenClawProviderConfig } from '@/types/openclaw';

const API_PROTOCOLS = [
  { value: 'openai-completions', label: 'OpenAI Completions' },
  { value: 'openai-responses', label: 'OpenAI Responses' },
  { value: 'anthropic-messages', label: 'Anthropic Messages' },
  { value: 'google-generative-ai', label: 'Google Generative AI' },
  { value: 'bedrock-converse-stream', label: 'Bedrock Converse Stream' },
];

export interface ProviderFormValues {
  providerId: string;
  baseUrl?: string;
  apiKey?: string;
  api?: string;
}

interface Props {
  open: boolean;
  editingProvider?: { id: string; config: OpenClawProviderConfig } | null;
  existingIds: string[];
  onCancel: () => void;
  onSubmit: (values: ProviderFormValues) => void;
  onOpenImport?: () => void;
}

const OpenClawProviderFormModal: React.FC<Props> = ({
  open: modalOpen,
  editingProvider,
  existingIds,
  onCancel,
  onSubmit,
  onOpenImport,
}) => {
  const { t } = useTranslation();
  const language = useAppStore((state) => state.language);
  const [form] = Form.useForm();
  const isEdit = !!editingProvider;
  const [showApiKey, setShowApiKey] = React.useState(false);

  const labelCol = { span: language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };

  React.useEffect(() => {
    if (modalOpen) {
      if (editingProvider) {
        form.setFieldsValue({
          providerId: editingProvider.id,
          baseUrl: editingProvider.config.baseUrl || '',
          apiKey: editingProvider.config.apiKey || '',
          api: editingProvider.config.api || 'openai-completions',
        });
      } else {
        form.resetFields();
        form.setFieldsValue({ api: 'openai-completions' });
      }
      setShowApiKey(false);
    }
  }, [modalOpen, editingProvider, form]);

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
      title={isEdit ? t('openclaw.providers.editProvider') : t('openclaw.providers.addProvider')}
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
      width={800}
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
          name="providerId"
          label={t('openclaw.providers.providerId')}
          rules={[
            { required: true, message: t('common.required') },
            {
              validator: (_, value) => {
                if (!isEdit && value && existingIds.includes(value)) {
                  return Promise.reject(new Error('Provider ID already exists'));
                }
                return Promise.resolve();
              },
            },
          ]}
        >
          <Input
            placeholder={t('openclaw.providers.providerIdPlaceholder')}
            disabled={isEdit}
          />
        </Form.Item>

        <Form.Item name="api" label={t('openclaw.providers.apiProtocol')}>
          <Select
            options={API_PROTOCOLS}
            showSearch
            optionFilterProp="label"
          />
        </Form.Item>

        <Form.Item name="baseUrl" label={t('openclaw.providers.baseUrl')}>
          <Input placeholder={t('openclaw.providers.baseUrlPlaceholder')} />
        </Form.Item>

        <Form.Item name="apiKey" label={t('openclaw.providers.apiKey')}>
          <Input
            type={showApiKey ? 'text' : 'password'}
            placeholder={t('openclaw.providers.apiKeyPlaceholder')}
            suffix={
              <Button
                type="text"
                size="small"
                icon={showApiKey ? <EyeInvisibleOutlined /> : <EyeOutlined />}
                onClick={() => setShowApiKey(!showApiKey)}
                style={{ marginRight: -8 }}
              />
            }
          />
        </Form.Item>

        {/* Import from OpenCode button — only in add mode */}
        {!isEdit && onOpenImport && (
          <Form.Item wrapperCol={{ offset: language === 'zh-CN' ? 4 : 6, span: 20 }} style={{ marginBottom: 0 }}>
            <Button
              type="dashed"
              icon={<ImportOutlined />}
              onClick={onOpenImport}
            >
              {t('openclaw.providers.importFromOpenCode')}
            </Button>
          </Form.Item>
        )}
      </Form>
    </Modal>
  );
};

export default OpenClawProviderFormModal;
