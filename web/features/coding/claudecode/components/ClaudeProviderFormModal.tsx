import React from 'react';
import { Modal, Tabs, Form, Input, Select, Space, Button, Alert, message } from 'antd';
import { EyeInvisibleOutlined, EyeOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useAppStore } from '@/stores';
import type { ClaudeCodeProvider, ClaudeProviderFormValues, ClaudeSettingsConfig } from '@/types/claudecode';
import { readOpenCodeConfig } from '@/services/opencodeApi';
import type { OpenCodeModel } from '@/types/opencode';

const { TextArea } = Input;

// OpenCode 供应商展示类型
interface OpenCodeProviderDisplay {
  id: string;
  name: string;
  baseUrl: string | undefined;
  apiKey?: string;
  models: { id: string; name: string }[];
}

interface ClaudeProviderFormModalProps {
  open: boolean;
  provider?: ClaudeCodeProvider | null;
  isCopy?: boolean;
  defaultTab?: 'manual' | 'import';
  onCancel: () => void;
  onSubmit: (values: ClaudeProviderFormValues) => Promise<void>;
}

const ClaudeProviderFormModal: React.FC<ClaudeProviderFormModalProps> = ({
  open,
  provider,
  isCopy = false,
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

  // 从 OpenCode 导入相关状态
  const [openCodeProviders, setOpenCodeProviders] = React.useState<OpenCodeProviderDisplay[]>([]);
  const [selectedProvider, setSelectedProvider] = React.useState<OpenCodeProviderDisplay | null>(null);
  const [availableModels, setAvailableModels] = React.useState<{ id: string; name: string }[]>([]);
  const [loadingProviders, setLoadingProviders] = React.useState(false);
  const [processedBaseUrl, setProcessedBaseUrl] = React.useState<string>('');

  const isEdit = !!provider && !isCopy;

  // 当 Modal 打开时，根据 defaultTab 设置 activeTab
  React.useEffect(() => {
    if (open) {
      setActiveTab(defaultTab);
    }
  }, [open, defaultTab]);

  // 加载 OpenCode 中的供应商列表
  React.useEffect(() => {
    if (open && activeTab === 'import') {
      loadOpenCodeProviders();
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

  const loadOpenCodeProviders = async () => {
    setLoadingProviders(true);
    try {
      const config = await readOpenCodeConfig();
      if (!config) {
        setOpenCodeProviders([]);
        return;
      }

      // 筛选 npm === '@ai-sdk/anthropic' 的供应商
      const anthropicProviders: OpenCodeProviderDisplay[] = [];
      for (const [id, providerData] of Object.entries(config.provider)) {
        if (providerData.npm === '@ai-sdk/anthropic') {
          const models = Object.entries(providerData.models || {}).map(([modelId, model]) => ({
            id: modelId,
            name: (model as OpenCodeModel).name || modelId,
          }));

          anthropicProviders.push({
            id,
            name: providerData.name || id,
            baseUrl: providerData.options?.baseURL,
            apiKey: providerData.options?.apiKey,
            models,
          });
        }
      }

      setOpenCodeProviders(anthropicProviders);
    } catch (error) {
      console.error('Failed to load OpenCode providers:', error);
      message.error(t('common.error'));
    } finally {
      setLoadingProviders(false);
    }
  };

  const handleProviderSelect = (providerId: string) => {
    const providerData = openCodeProviders.find((p) => p.id === providerId);
    if (!providerData) return;

    setSelectedProvider(providerData);
    setAvailableModels(providerData.models);

    // 处理 baseUrl：去掉末尾的 /v1 和末尾的 /
    let processedUrl = providerData.baseUrl || '';
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
      name: providerData.name,
      baseUrl: processedUrl,
      apiKey: providerData.apiKey || '',
    });
  };

  const handleSubmit = async () => {
    try {
      // 只验证当前模式需要的字段
      const fieldsToValidate = activeTab === 'import'
        ? ['sourceProvider', 'name', 'baseUrl', 'apiKey', 'model', 'haikuModel', 'sonnetModel', 'opusModel', 'notes']
        : ['name', 'baseUrl', 'apiKey', 'model', 'haikuModel', 'sonnetModel', 'opusModel', 'notes'];
      
      const values = await form.validateFields(fieldsToValidate);
      
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
            options={openCodeProviders.map((p) => ({
              label: `${p.name} (${p.baseUrl || ''})`,
              value: p.id,
            }))}
          />
        </Form.Item>

        {selectedProvider && (
          <Alert
            message={t('claudecode.import.importInfo')}
            description={
              <Space orientation="vertical" size={4}>
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
