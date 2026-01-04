import React from 'react';
import { Modal, Form, Input, Select, Button, Typography, message } from 'antd';
import { EyeOutlined, EyeInvisibleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { PROVIDER_TYPES } from '@/constants/providerTypes';
import HeadersEditor from '@/components/common/HeadersEditor';
import type { I18nPrefix } from '@/components/common/ProviderCard/types';

const { Text } = Typography;

/**
 * Form values for provider form
 */
export interface ProviderFormValues {
  id: string;
  name: string;
  sdkType: string;
  baseUrl: string;
  apiKey?: string;
  headers?: string | Record<string, string>;
}

interface ProviderFormModalProps {
  open: boolean;
  
  /** Whether this is an edit operation */
  isEdit?: boolean;
  /** Initial form values */
  initialValues?: Partial<ProviderFormValues>;
  
  /** Existing provider IDs for duplicate check (only used when !isEdit) */
  existingIds?: string[];
  /** Whether API key is required (settings page: true, OpenCode: false) */
  apiKeyRequired?: boolean;
  
  /** Callbacks */
  onCancel: () => void;
  onSuccess: (values: ProviderFormValues) => void;
  /** Custom duplicate ID error handler */
  onDuplicateId?: (id: string) => void;
  
  /** i18n prefix for translations */
  i18nPrefix?: I18nPrefix;
  
  /** Headers output format */
  headersOutputFormat?: 'string' | 'object';
}

/**
 * A reusable provider form modal component
 */
const ProviderFormModal: React.FC<ProviderFormModalProps> = ({
  open,
  isEdit = false,
  initialValues,
  existingIds = [],
  apiKeyRequired = true,
  onCancel,
  onSuccess,
  onDuplicateId,
  i18nPrefix = 'settings',
  headersOutputFormat = 'string',
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);
  const [headersValid, setHeadersValid] = React.useState(true);

  React.useEffect(() => {
    if (open) {
      if (initialValues) {
        form.setFieldsValue(initialValues);
      } else {
        form.resetFields();
      }
      setShowApiKey(false);
      setHeadersValid(true);
    }
  }, [open, initialValues, form]);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      
      // Validate headers JSON
      if (!headersValid) {
        message.error(t('settings.provider.invalidHeaders'));
        return;
      }
      
      setLoading(true);

      // Check for duplicate ID when creating
      if (!isEdit && existingIds.includes(values.id)) {
        if (onDuplicateId) {
          onDuplicateId(values.id);
        }
        setLoading(false);
        return;
      }

      onSuccess(values as ProviderFormValues);
      form.resetFields();
    } catch (error: unknown) {
      console.error('Provider form validation error:', error);
      // Form validation errors are already shown by Form
    } finally {
      setLoading(false);
    }
  };

  // Build i18n keys based on prefix
  const getKey = (key: string) => `${i18nPrefix}.provider.${key}`;

  return (
    <Modal
      title={isEdit ? t(`${i18nPrefix}.editProvider`) : t(`${i18nPrefix}.addProvider`)}
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
      <Form
        form={form}
        layout="horizontal"
        labelCol={{ span: 4 }}
        wrapperCol={{ span: 20 }}
        style={{ marginTop: 24 }}
      >
        <Form.Item
          label={t(getKey('id'))}
          name="id"
          rules={[{ required: true, message: t(getKey('idPlaceholder')) }]}
        >
          <Input
            placeholder={t(getKey('idPlaceholder'))}
            disabled={isEdit}
          />
        </Form.Item>

        <Form.Item
          label={t(getKey('name'))}
          name="name"
          rules={[{ required: true, message: t(getKey('namePlaceholder')) }]}
        >
          <Input placeholder={t(getKey('namePlaceholder'))} />
        </Form.Item>

        <Form.Item
          label={i18nPrefix === 'settings' ? t('settings.provider.providerType') : t('opencode.provider.npm')}
          name="sdkType"
          rules={[{ required: true }]}
          initialValue="@ai-sdk/openai-compatible"
        >
          <Select
            placeholder={i18nPrefix === 'settings' ? t('settings.provider.providerType') : t('opencode.provider.npmPlaceholder')}
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
          label={i18nPrefix === 'settings' ? t('settings.provider.baseUrl') : t('opencode.provider.baseURL')}
          name="baseUrl"
          rules={[{ required: true, message: i18nPrefix === 'settings' ? t('settings.provider.baseUrlPlaceholder') : t('opencode.provider.baseURLPlaceholder') }]}
          extra={i18nPrefix === 'settings' ? <Text type="secondary" style={{ fontSize: 12 }}>{t('settings.provider.baseUrlHint')}</Text> : undefined}
        >
          <Input placeholder={i18nPrefix === 'settings' ? t('settings.provider.baseUrlPlaceholder') : t('opencode.provider.baseURLPlaceholder')} />
        </Form.Item>

        <Form.Item
          label={t(getKey('apiKey'))}
          name="apiKey"
          rules={apiKeyRequired ? [{ required: true, message: t(getKey('apiKeyPlaceholder')) }] : undefined}
        >
          <Input
            type={showApiKey ? 'text' : 'password'}
            placeholder={t(getKey('apiKeyPlaceholder'))}
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

        <Form.Item
          label={t(getKey('headers'))}
          name="headers"
          extra={i18nPrefix === 'settings' ? <Text type="secondary" style={{ fontSize: 12 }}>{t('settings.provider.headersHint')}</Text> : undefined}
        >
          <HeadersEditor 
            outputFormat={headersOutputFormat} 
            height={120}
            onValidationChange={setHeadersValid}
          />
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default ProviderFormModal;
