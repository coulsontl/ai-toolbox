import React from 'react';
import { Modal, Tabs, Form, Input, Select, Space, Button, Alert, message } from 'antd';
import { EyeInvisibleOutlined, EyeOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useAppStore } from '@/stores';
import type { ClaudeCodeProvider, ClaudeProviderFormValues, ClaudeSettingsConfig } from '@/types/claudecode';
import { listProviders, listModels } from '@/services/providerApi';
import type { Provider, Model } from '@/types/provider';

const { TextArea } = Input;

interface ClaudeProviderFormModalProps {
  open: boolean;
  provider?: ClaudeCodeProvider | null;
  defaultTab?: 'manual' | 'import';
  onCancel: () => void;
  onSubmit: (values: ClaudeProviderFormValues) => Promise<void>;
}

const ClaudeProviderFormModal: React.FC<ClaudeProviderFormModalProps> = ({
  open,
  provider,
  defaultTab = 'manual',
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const language = useAppStore((state) => state.language);
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);
  const [activeTab, setActiveTab] = React.useState<'manual' | 'import'>(defaultTab);

  const labelCol = { span: language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };

  // 从设置导入相关状态
  const [settingsProviders, setSettingsProviders] = React.useState<Provider[]>([]);
  const [selectedProvider, setSelectedProvider] = React.useState<Provider | null>(null);
  const [availableModels, setAvailableModels] = React.useState<Model[]>([]);
  const [loadingProviders, setLoadingProviders] = React.useState(false);
  const [processedBaseUrl, setProcessedBaseUrl] = React.useState<string>('');

  const isEdit = !!provider;

  // 当 Modal 打开时，根据 defaultTab 设置 activeTab
  React.useEffect(() => {
    if (open) {
      setActiveTab(defaultTab);
    }
  }, [open, defaultTab]);

  // 加载设置中的供应商列表
  React.useEffect(() => {
    if (open && activeTab === 'import') {
      loadSettingsProviders();
    }
  }, [open, activeTab]);

  // 初始化表单
  React.useEffect(() => {
    if (open && provider) {
      let settingsConfig: ClaudeSettingsConfig = {};
      try {
        settingsConfig = JSON.parse(provider.settingsConfig);
      } catch (error) {
        console.error('Failed to parse settingsConfig:', error);
      }

      form.setFieldsValue({
        name: provider.name,
        baseUrl: settingsConfig.env?.ANTHROPIC_BASE_URL,
        apiKey: settingsConfig.env?.ANTHROPIC_API_KEY,
        model: settingsConfig.model,
        haikuModel: settingsConfig.haikuModel,
        sonnetModel: settingsConfig.sonnetModel,
        opusModel: settingsConfig.opusModel,
        notes: provider.notes,
      });
    } else if (open && !provider) {
      form.resetFields();
    }
  }, [open, provider, form]);

  const loadSettingsProviders = async () => {
    setLoadingProviders(true);
    try {
      const providers = await listProviders();
      // 只显示 SDK 类型为 @ai-sdk/anthropic 的供应商（Claude）
      const claudeProviders = providers.filter((p) => p.provider_type === '@ai-sdk/anthropic');
      setSettingsProviders(claudeProviders);
    } catch (error) {
      console.error('Failed to load providers:', error);
      message.error(t('common.error'));
    } finally {
      setLoadingProviders(false);
    }
  };

  const handleProviderSelect = async (providerId: string) => {
    const provider = settingsProviders.find((p) => p.id === providerId);
    if (!provider) return;

    setSelectedProvider(provider);

    // 加载该供应商的模型
    try {
      const models = await listModels(providerId);
      setAvailableModels(models);

      // 处理 baseUrl：去掉末尾的 /v1 和末尾的 /
      let processedUrl = provider.base_url;
      // 去掉末尾的 /v1
      if (processedUrl.endsWith('/v1')) {
        processedUrl = processedUrl.slice(0, -3);
      }
      // 去掉末尾的 /
      if (processedUrl.endsWith('/')) {
        processedUrl = processedUrl.slice(0, -1);
      }
      setProcessedBaseUrl(processedUrl);

      // 自动填充表单
      form.setFieldsValue({
        name: provider.name,
        baseUrl: processedUrl,
        apiKey: provider.api_key,
      });
    } catch (error) {
      console.error('Failed to load models:', error);
      message.error(t('common.error'));
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      
      setLoading(true);
      
      const formValues: ClaudeProviderFormValues = {
        name: values.name,
        category: 'custom',
        baseUrl: values.baseUrl,
        apiKey: values.apiKey,
        model: values.model,
        haikuModel: values.haikuModel,
        sonnetModel: values.sonnetModel,
        opusModel: values.opusModel,
        notes: values.notes,
        sourceProviderId: activeTab === 'import' ? selectedProvider?.id : undefined,
      };

      await onSubmit(formValues);
      form.resetFields();
      setSelectedProvider(null);
      setAvailableModels([]);
      onCancel();
    } catch (error) {
      console.error('Form validation failed:', error);
    } finally {
      setLoading(false);
    }
  };

  const modelSelectOptions = availableModels.map((model) => ({
    label: `${model.name} (${model.id})`,
    value: model.id,
  }));

  const renderManualTab = () => (
    <Form
      form={form}
      layout="horizontal"
      labelCol={labelCol}
      wrapperCol={wrapperCol}
    >
      <Form.Item
        name="name"
        label={t('claudecode.provider.name')}
        rules={[{ required: true, message: t('common.error') }]}
      >
        <Input placeholder={t('claudecode.provider.namePlaceholder')} />
      </Form.Item>

      <Form.Item
        name="baseUrl"
        label={t('claudecode.provider.baseUrl')}
        rules={[{ required: true, message: t('common.error') }]}
      >
        <Input placeholder={t('claudecode.provider.baseUrlPlaceholder')} />
      </Form.Item>

      <Form.Item
        name="apiKey"
        label={t('claudecode.provider.apiKey')}
        rules={[{ required: true, message: t('common.error') }]}
      >
        <Input
          type={showApiKey ? 'text' : 'password'}
          placeholder={t('claudecode.provider.apiKeyPlaceholder')}
          addonAfter={
            <Button
              type="text"
              size="small"
              icon={showApiKey ? <EyeInvisibleOutlined /> : <EyeOutlined />}
              onClick={() => setShowApiKey(!showApiKey)}
            >
              {showApiKey ? t('claudecode.provider.hideApiKey') : t('claudecode.provider.showApiKey')}
            </Button>
          }
        />
      </Form.Item>

      <Form.Item name="model" label={t('claudecode.model.defaultModel')}>
        <Input placeholder={t('claudecode.model.defaultModelPlaceholder')} />
      </Form.Item>

      <Form.Item name="haikuModel" label={t('claudecode.model.haikuModel')}>
        <Input placeholder={t('claudecode.model.haikuModelPlaceholder')} />
      </Form.Item>

      <Form.Item name="sonnetModel" label={t('claudecode.model.sonnetModel')}>
        <Input placeholder={t('claudecode.model.sonnetModelPlaceholder')} />
      </Form.Item>

      <Form.Item name="opusModel" label={t('claudecode.model.opusModel')}>
        <Input placeholder={t('claudecode.model.opusModelPlaceholder')} />
      </Form.Item>

      <Form.Item name="notes" label={t('claudecode.provider.notes')}>
        <TextArea
          rows={3}
          placeholder={t('claudecode.provider.notesPlaceholder')}
        />
      </Form.Item>
    </Form>
  );

  const renderImportTab = () => (
    <div>
      <Alert
        message={t('claudecode.import.title')}
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
      />

      <Form
        form={form}
        layout="horizontal"
        labelCol={labelCol}
        wrapperCol={wrapperCol}
      >
        <Form.Item
          name="sourceProvider"
          label={t('claudecode.import.selectProvider')}
          rules={[{ required: true, message: t('common.error') }]}
        >
          <Select
            placeholder={t('claudecode.import.selectProviderPlaceholder')}
            loading={loadingProviders}
            onChange={handleProviderSelect}
            options={settingsProviders.map((p) => ({
              label: `${p.name} (${p.base_url})`,
              value: p.id,
            }))}
          />
        </Form.Item>

        {selectedProvider && (
          <Alert
            message={t('claudecode.import.importInfo')}
            description={
              <Space direction="vertical" size={4}>
                <div>• {t('claudecode.import.providerName')}: {selectedProvider.name}</div>
                <div>• {t('claudecode.import.baseUrl')}: {processedBaseUrl}</div>
                <div>• {t('claudecode.import.availableModels')}: {availableModels.length > 0 ? t('claudecode.import.modelsCount', { count: availableModels.length }) : '-'}</div>
              </Space>
            }
            type="success"
            showIcon
            style={{ marginBottom: 16 }}
          />
        )}

        <Form.Item name="name" label={t('claudecode.provider.name')}>
          <Input placeholder={t('claudecode.provider.namePlaceholder')} disabled />
        </Form.Item>

        <Form.Item name="baseUrl" label={t('claudecode.provider.baseUrl')}>
          <Input disabled />
        </Form.Item>

        <Form.Item name="apiKey" label={t('claudecode.provider.apiKey')}>
          <Input type="password" disabled />
        </Form.Item>

        {availableModels.length > 0 && (
          <>
            <Alert
              message={t('claudecode.model.selectFromProvider')}
              type="info"
              showIcon
              style={{ marginBottom: 16 }}
            />

            <Form.Item name="model" label={t('claudecode.import.selectDefaultModel')}>
              <Select
                placeholder={t('claudecode.model.defaultModelPlaceholder')}
                options={modelSelectOptions}
                allowClear
                showSearch
              />
            </Form.Item>

            <Form.Item name="haikuModel" label={t('claudecode.import.selectHaikuModel')}>
              <Select
                placeholder={t('claudecode.model.haikuModelPlaceholder')}
                options={modelSelectOptions}
                allowClear
                showSearch
              />
            </Form.Item>

            <Form.Item name="sonnetModel" label={t('claudecode.import.selectSonnetModel')}>
              <Select
                placeholder={t('claudecode.model.sonnetModelPlaceholder')}
                options={modelSelectOptions}
                allowClear
                showSearch
              />
            </Form.Item>

            <Form.Item name="opusModel" label={t('claudecode.import.selectOpusModel')}>
              <Select
                placeholder={t('claudecode.model.opusModelPlaceholder')}
                options={modelSelectOptions}
                allowClear
                showSearch
              />
            </Form.Item>
          </>
        )}

        <Form.Item name="notes" label={t('claudecode.provider.notes')}>
          <TextArea
            rows={3}
            placeholder={t('claudecode.provider.notesPlaceholder')}
          />
        </Form.Item>
      </Form>
    </div>
  );

  return (
    <Modal
      title={isEdit ? t('claudecode.provider.editProvider') : t('claudecode.provider.addProvider')}
      open={open}
      onCancel={onCancel}
      onOk={handleSubmit}
      confirmLoading={loading}
      width={600}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      {!isEdit && (
        <Tabs
          activeKey={activeTab}
          onChange={(key) => setActiveTab(key as 'manual' | 'import')}
          items={[
            {
              key: 'manual',
              label: t('claudecode.form.tabManual'),
              children: renderManualTab(),
            },
            {
              key: 'import',
              label: t('claudecode.form.tabImport'),
              children: renderImportTab(),
            },
          ]}
        />
      )}
      {isEdit && renderManualTab()}
    </Modal>
  );
};

export default ClaudeProviderFormModal;
