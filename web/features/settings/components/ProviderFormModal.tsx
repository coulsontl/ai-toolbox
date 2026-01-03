import React from 'react';
import { Modal, Form, Input, Select, Button, message, Typography } from 'antd';
import { EyeOutlined, EyeInvisibleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { PROVIDER_TYPES } from '@/constants/providerTypes';
import { createProvider, updateProvider, listProviders } from '@/services/providerApi';
import type { Provider } from '@/types/provider';

const { Text } = Typography;

interface ProviderFormModalProps {
  open: boolean;
  provider?: Provider | null;
  onCancel: () => void;
  onSuccess: () => void;
}

const ProviderFormModal: React.FC<ProviderFormModalProps> = ({
  open,
  provider,
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);

  const isEdit = !!provider;

  React.useEffect(() => {
    if (open) {
      if (provider) {
        form.setFieldsValue(provider);
      } else {
        form.resetFields();
      }
      setShowApiKey(false);
    }
  }, [open, provider, form]);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      setLoading(true);

      if (isEdit) {
        // Update existing provider
        await updateProvider({
          ...provider,
          ...values,
        });
        message.success(t('common.success'));
      } else {
        // Create new provider - check for duplicates
        const existingProviders = await listProviders();
        if (existingProviders.some(p => p.id === values.id)) {
          message.error(t('settings.provider.idExists'));
          setLoading(false);
          return;
        }

        await createProvider({
          ...values,
          sort_order: existingProviders.length,
        });
        message.success(t('common.success'));
      }

      onSuccess();
      form.resetFields();
    } catch (error: unknown) {
      console.error('Provider save error:', error);
      // Handle different error types
      if (error && typeof error === 'object' && 'errorFields' in error) {
        // Form validation error - already shown by Form
        return;
      }
      const errorMessage = error instanceof Error 
        ? error.message 
        : typeof error === 'string' 
          ? error 
          : t('common.error');
      message.error(errorMessage);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Modal
      title={isEdit ? t('settings.provider.editProvider') : t('settings.provider.addProvider')}
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
      width={600}
    >
      <Form form={form} layout="vertical" style={{ marginTop: 24 }}>
        <Form.Item
          label={t('settings.provider.id')}
          name="id"
          rules={[{ required: true, message: t('settings.provider.idPlaceholder') }]}
        >
          <Input
            placeholder={t('settings.provider.idPlaceholder')}
            disabled={isEdit}
          />
        </Form.Item>

        <Form.Item
          label={t('settings.provider.name')}
          name="name"
          rules={[{ required: true, message: t('settings.provider.namePlaceholder') }]}
        >
          <Input placeholder={t('settings.provider.namePlaceholder')} />
        </Form.Item>

        <Form.Item
          label={t('settings.provider.providerType')}
          name="provider_type"
          rules={[{ required: true }]}
        >
          <Select
            placeholder={t('settings.provider.providerType')}
            showSearch
            optionFilterProp="label"
            optionRender={(option) => (
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span>{option.label}</span>
                <Text type="secondary" style={{ fontSize: 12 }}>{option.value}</Text>
              </div>
            )}
            options={PROVIDER_TYPES}
          />
        </Form.Item>

        <Form.Item
          label={t('settings.provider.baseUrl')}
          name="base_url"
          rules={[{ required: true, message: t('settings.provider.baseUrlPlaceholder') }]}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('settings.provider.baseUrlHint')}</Text>}
        >
          <Input placeholder={t('settings.provider.baseUrlPlaceholder')} />
        </Form.Item>

        <Form.Item
          label={t('settings.provider.apiKey')}
          name="api_key"
          rules={[{ required: true, message: t('settings.provider.apiKeyPlaceholder') }]}
        >
          <Input
            type={showApiKey ? 'text' : 'password'}
            placeholder={t('settings.provider.apiKeyPlaceholder')}
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
      </Form>
    </Modal>
  );
};

export default ProviderFormModal;
