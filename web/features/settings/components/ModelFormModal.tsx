import React from 'react';
import { Modal, Form, Input, AutoComplete, Button, message } from 'antd';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import { createModel, updateModel, listModels } from '@/services/providerApi';
import type { Model } from '@/types/provider';

// Context limit options with display labels
const CONTEXT_LIMIT_OPTIONS = [
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
  { value: '128000', label: '128K' },
  { value: '200000', label: '200K' },
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

interface ModelFormModalProps {
  open: boolean;
  providerId: string;
  model?: Model | null;
  onCancel: () => void;
  onSuccess: () => void;
}

const ModelFormModal: React.FC<ModelFormModalProps> = ({
  open,
  providerId,
  model,
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [jsonOptions, setJsonOptions] = React.useState<unknown>({});
  const [jsonValid, setJsonValid] = React.useState(true);

  const isEdit = !!model;

  React.useEffect(() => {
    if (open) {
      if (model) {
        form.setFieldsValue({
          id: model.id,
          name: model.name,
          context_limit: model.context_limit,
          output_limit: model.output_limit,
        });
        
        // Parse options JSON
        try {
          const parsed = model.options ? JSON.parse(model.options) : {};
          setJsonOptions(parsed);
          setJsonValid(true);
        } catch {
          setJsonOptions({});
          setJsonValid(false);
        }
      } else {
        form.resetFields();
        setJsonOptions({});
        setJsonValid(true);
      }
    }
  }, [open, model, form]);

  const handleJsonChange = (value: unknown, isValid: boolean) => {
    // Only update jsonOptions when JSON is valid
    // This prevents passing raw text strings back to the editor
    // which would cause a value type change and trigger set()
    if (isValid) {
      setJsonOptions(value);
    }
    setJsonValid(isValid);
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      
      // Validate JSON
      if (!jsonValid) {
        message.error(t('settings.model.invalidJson'));
        return;
      }
      
      const optionsString = JSON.stringify(jsonOptions);
      
      setLoading(true);

      if (isEdit) {
        // Update existing model
        await updateModel({
          ...model,
          ...values,
          options: optionsString,
        });
        message.success(t('common.success'));
      } else {
        // Create new model - check for duplicates under this provider
        const existingModels = await listModels(providerId);
        if (existingModels.some(m => m.id === values.id)) {
          message.error(t('settings.model.idExists'));
          setLoading(false);
          return;
        }

        await createModel({
          ...values,
          provider_id: providerId,
          options: optionsString,
          sort_order: existingModels.length,
        });
        message.success(t('common.success'));
      }

      onSuccess();
      form.resetFields();
    } catch (error: unknown) {
      console.error('Model save error:', error);
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
      title={isEdit ? t('settings.model.editModel') : t('settings.model.addModel')}
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
      width={700}
    >
      <Form form={form} layout="vertical" style={{ marginTop: 24 }}>
        <Form.Item
          label={t('settings.model.id')}
          name="id"
          rules={[{ required: true, message: t('settings.model.idPlaceholder') }]}
        >
          <Input
            placeholder={t('settings.model.idPlaceholder')}
            disabled={isEdit}
          />
        </Form.Item>

        <Form.Item
          label={t('settings.model.name')}
          name="name"
          rules={[{ required: true, message: t('settings.model.namePlaceholder') }]}
        >
          <Input placeholder={t('settings.model.namePlaceholder')} />
        </Form.Item>

        <Form.Item
          label={t('settings.model.contextLimit')}
          name="context_limit"
          rules={[
            { required: true, message: t('settings.model.contextLimitPlaceholder') },
            {
              validator: (_, value) => {
                if (value && !/^\d+$/.test(String(value))) {
                  return Promise.reject(t('settings.model.invalidNumber'));
                }
                return Promise.resolve();
              },
            },
          ]}
          getValueFromEvent={(val) => {
            const num = parseInt(val, 10);
            return isNaN(num) ? val : num;
          }}
          normalize={(val) => (typeof val === 'number' ? String(val) : val)}
        >
          <AutoComplete
            options={CONTEXT_LIMIT_OPTIONS}
            placeholder={t('settings.model.contextLimitPlaceholder')}
            style={{ width: '100%' }}
            filterOption={(inputValue, option) =>
              (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
              option?.value.includes(inputValue)) ?? false
            }
          />
        </Form.Item>

        <Form.Item
          label={t('settings.model.outputLimit')}
          name="output_limit"
          rules={[
            { required: true, message: t('settings.model.outputLimitPlaceholder') },
            {
              validator: (_, value) => {
                if (value && !/^\d+$/.test(String(value))) {
                  return Promise.reject(t('settings.model.invalidNumber'));
                }
                return Promise.resolve();
              },
            },
          ]}
          getValueFromEvent={(val) => {
            const num = parseInt(val, 10);
            return isNaN(num) ? val : num;
          }}
          normalize={(val) => (typeof val === 'number' ? String(val) : val)}
        >
          <AutoComplete
            options={OUTPUT_LIMIT_OPTIONS}
            placeholder={t('settings.model.outputLimitPlaceholder')}
            style={{ width: '100%' }}
            filterOption={(inputValue, option) =>
              (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
              option?.value.includes(inputValue)) ?? false
            }
          />
        </Form.Item>

        <Form.Item label={t('settings.model.options')}>
          <JsonEditor
            value={jsonOptions}
            onChange={handleJsonChange}
            mode="text"
            height={300}
            resizable
          />
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default ModelFormModal;
