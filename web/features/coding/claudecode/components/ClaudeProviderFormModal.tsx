import React from 'react';
import { Modal, Form, Input, Select, Space, Button, Alert, message, AutoComplete, Radio } from 'antd';
import { EyeInvisibleOutlined, EyeOutlined, CloudDownloadOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '@/stores';
import type { ClaudeCodeProvider, ClaudeProviderFormValues, ClaudeSettingsConfig } from '@/types/claudecode';
import { listFavoriteProviders } from '@/services/opencodeApi';

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
  mode?: 'manual' | 'import';
  onCancel: () => void;
  onSubmit: (values: ClaudeProviderFormValues) => Promise<void>;
}

// 获取模型 API 响应类型
interface FetchedModel {
  id: string;
  name?: string;
  ownedBy?: string;
}

interface FetchModelsResponse {
  models: FetchedModel[];
  total: number;
}

const ClaudeProviderFormModal: React.FC<ClaudeProviderFormModalProps> = ({
  open,
  provider,
  isCopy = false,
  mode = 'manual',
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const language = useAppStore((state) => state.language);
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);

  const labelCol = { span: language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };

  // 从 OpenCode 导入相关状态
  const [openCodeProviders, setOpenCodeProviders] = React.useState<OpenCodeProviderDisplay[]>([]);
  const [selectedProvider, setSelectedProvider] = React.useState<OpenCodeProviderDisplay | null>(null);
  const [availableModels, setAvailableModels] = React.useState<{ id: string; name: string }[]>([]);
  const [loadingProviders, setLoadingProviders] = React.useState(false);
  const [processedBaseUrl, setProcessedBaseUrl] = React.useState<string>('');
  // 动态获取的模型列表
  const [fetchedModels, setFetchedModels] = React.useState<FetchedModel[]>([]);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [fetchApiType, setFetchApiType] = React.useState<'openai_compat' | 'native'>('native');
  // 当前表单的 baseUrl（用于匹配供应商）
  const [currentBaseUrl, setCurrentBaseUrl] = React.useState<string>('');

  const isEdit = !!provider && !isCopy;

  // 加载 OpenCode 中的供应商列表
  React.useEffect(() => {
    if (mode === 'import' || isEdit) {
      loadOpenCodeProviders();
    }
  }, [mode, isEdit]);

  // 初始化表单（组件挂载时执行一次）
  React.useEffect(() => {
    if (provider) {
      let settingsConfig: ClaudeSettingsConfig = {};
      try {
        settingsConfig = JSON.parse(provider.settingsConfig);
      } catch (error) {
        console.error('Failed to parse settingsConfig:', error);
      }

      const baseUrl = settingsConfig.env?.ANTHROPIC_BASE_URL || '';
      setCurrentBaseUrl(baseUrl);

      form.setFieldsValue({
        name: provider.name,
        baseUrl,
        apiKey: settingsConfig.env?.ANTHROPIC_AUTH_TOKEN || settingsConfig.env?.ANTHROPIC_API_KEY,
        model: settingsConfig.model,
        haikuModel: settingsConfig.haikuModel,
        sonnetModel: settingsConfig.sonnetModel,
        opusModel: settingsConfig.opusModel,
        reasoningModel: settingsConfig.reasoningModel || settingsConfig.env?.ANTHROPIC_REASONING_MODEL,
        notes: provider.notes,
      });
    }
  }, [provider, form]);

  const loadOpenCodeProviders = async () => {
    setLoadingProviders(true);
    try {
      const favoriteProviders = await listFavoriteProviders();

      // 筛选 npm === '@ai-sdk/anthropic' 的供应商
      const anthropicProviders: OpenCodeProviderDisplay[] = favoriteProviders
        .filter((fp) => fp.providerConfig.npm === '@ai-sdk/anthropic')
        .map((fp) => {
          const models = Object.entries(fp.providerConfig.models || {}).map(([modelId, model]) => ({
            id: modelId,
            name: model.name || modelId,
          }));

          return {
            id: fp.providerId,
            name: fp.providerConfig.name || fp.providerId,
            baseUrl: fp.providerConfig.options?.baseURL,
            apiKey: fp.providerConfig.options?.apiKey,
            models,
          };
        });

      setOpenCodeProviders(anthropicProviders);
    } catch (error) {
      console.error('Failed to load favorite providers:', error);
      message.error(t('common.error'));
    } finally {
      setLoadingProviders(false);
    }
  };

  // 获取模型列表（调用 fetch_provider_models API）
  const handleFetchModels = async () => {
    const baseUrl = form.getFieldValue('baseUrl');
    const apiKey = form.getFieldValue('apiKey');

    if (!baseUrl) {
      message.warning(t('claudecode.fetchModels.baseUrlRequired'));
      return;
    }

    // 构建 customUrl：在 baseUrl 后追加 /v1/models
    const base = baseUrl.replace(/\/$/, '');
    const customUrl = `${base}/v1/models`;

    setLoadingModels(true);
    try {
      const response = await invoke<FetchModelsResponse>('fetch_provider_models', {
        request: {
          baseUrl: `${base}/v1`,
          apiKey,
          apiType: fetchApiType,
          sdkType: '@ai-sdk/anthropic',
          customUrl,
        },
      });

      setFetchedModels(response.models);
      if (response.models.length > 0) {
        message.success(t('claudecode.fetchModels.success', { count: response.models.length }));
      } else {
        message.info(t('claudecode.fetchModels.noModels'));
      }
    } catch (error) {
      console.error('Failed to fetch models:', error);
      message.error(t('claudecode.fetchModels.failed'));
    } finally {
      setLoadingModels(false);
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
      const fieldsToValidate = mode === 'import'
        ? ['sourceProvider', 'name', 'baseUrl', 'apiKey', 'model', 'haikuModel', 'sonnetModel', 'opusModel', 'reasoningModel', 'notes']
        : ['name', 'baseUrl', 'apiKey', 'model', 'haikuModel', 'sonnetModel', 'opusModel', 'reasoningModel', 'notes'];
      
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
        reasoningModel: values.reasoningModel,
        notes: values.notes,
        sourceProviderId: mode === 'import' ? selectedProvider?.id : undefined,
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
    label: model.name && model.name !== model.id ? `${model.name} (${model.id})` : model.id,
    value: model.id,
  }));

  // 根据 baseUrl 匹配供应商的模型列表
  // OpenCode 的 URL 可能包含 /v1，所以用包含匹配
  const matchedProviderModels = React.useMemo(() => {
    if (!currentBaseUrl || openCodeProviders.length === 0) {
      return [];
    }

    // 标准化 URL：去掉末尾的 / 和 /v1
    const normalizeUrl = (url: string) => {
      let normalized = url.replace(/\/$/, '');
      if (normalized.endsWith('/v1')) {
        normalized = normalized.slice(0, -3);
      }
      return normalized.toLowerCase();
    };

    const normalizedCurrentUrl = normalizeUrl(currentBaseUrl);

    // 查找匹配的供应商
    const matchedProvider = openCodeProviders.find((p) => {
      if (!p.baseUrl) return false;
      const normalizedProviderUrl = normalizeUrl(p.baseUrl);
      // OpenCode 的 URL 包含 ClaudeCode 的 URL，或者反过来
      return normalizedProviderUrl.includes(normalizedCurrentUrl) ||
             normalizedCurrentUrl.includes(normalizedProviderUrl);
    });

    return matchedProvider?.models || [];
  }, [currentBaseUrl, openCodeProviders]);

  // 计算 AutoComplete 选项（使用动态获取的模型列表）
  const modelOptions = React.useMemo(() => {
    const options: { label: string; value: string }[] = [];
    const seenIds = new Set<string>();

    // 1. 添加动态获取的模型
    fetchedModels.forEach((model) => {
      if (!seenIds.has(model.id)) {
        seenIds.add(model.id);
        const name = model.name || model.id;
        options.push({
          label: name && name !== model.id ? `${name} (${model.id})` : model.id,
          value: model.id,
        });
      }
    });

    // 2. 添加根据 URL 匹配的供应商模型
    matchedProviderModels.forEach((model) => {
      if (!seenIds.has(model.id)) {
        seenIds.add(model.id);
        options.push({
          label: model.name && model.name !== model.id ? `${model.name} (${model.id})` : model.id,
          value: model.id,
        });
      }
    });

    return options;
  }, [fetchedModels, matchedProviderModels]);

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
        <Input
          placeholder={t('claudecode.provider.baseUrlPlaceholder')}
          onChange={(e) => setCurrentBaseUrl(e.target.value)}
        />
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

      {/* 获取模型列表 */}
      <Form.Item wrapperCol={{ offset: labelCol.span, span: wrapperCol.span }}>
        <Space size="middle" style={{ width: '100%' }}>
          <Radio.Group
            value={fetchApiType}
            onChange={(e) => setFetchApiType(e.target.value)}
            size="small"
          >
            <Radio value="openai_compat">{t('claudecode.fetchModels.openaiCompat')}</Radio>
            <Radio value="native">{t('claudecode.fetchModels.native')}</Radio>
          </Radio.Group>
          <Button
            type="default"
            icon={<CloudDownloadOutlined />}
            loading={loadingModels}
            onClick={handleFetchModels}
          >
            {t('claudecode.fetchModels.button')}
          </Button>
          {fetchedModels.length > 0 && (
            <span style={{ color: '#52c41a' }}>
              {t('claudecode.fetchModels.loaded', { count: fetchedModels.length })}
            </span>
          )}
        </Space>
      </Form.Item>

      <Form.Item name="model" label={t('claudecode.model.defaultModel')}>
        <AutoComplete
          options={modelOptions}
          placeholder={t('claudecode.model.defaultModelPlaceholder')}
          style={{ width: '100%' }}
          filterOption={(inputValue, option) =>
            (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
            option?.value.toLowerCase().includes(inputValue.toLowerCase())) ?? false
          }
        />
      </Form.Item>

      <Form.Item name="haikuModel" label={t('claudecode.model.haikuModel')}>
        <AutoComplete
          options={modelOptions}
          placeholder={t('claudecode.model.haikuModelPlaceholder')}
          style={{ width: '100%' }}
          filterOption={(inputValue, option) =>
            (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
            option?.value.toLowerCase().includes(inputValue.toLowerCase())) ?? false
          }
        />
      </Form.Item>

      <Form.Item name="sonnetModel" label={t('claudecode.model.sonnetModel')}>
        <AutoComplete
          options={modelOptions}
          placeholder={t('claudecode.model.sonnetModelPlaceholder')}
          style={{ width: '100%' }}
          filterOption={(inputValue, option) =>
            (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
            option?.value.toLowerCase().includes(inputValue.toLowerCase())) ?? false
          }
        />
      </Form.Item>

      <Form.Item name="opusModel" label={t('claudecode.model.opusModel')}>
        <AutoComplete
          options={modelOptions}
          placeholder={t('claudecode.model.opusModelPlaceholder')}
          style={{ width: '100%' }}
          filterOption={(inputValue, option) =>
            (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
            option?.value.toLowerCase().includes(inputValue.toLowerCase())) ?? false
          }
        />
      </Form.Item>

      <Form.Item name="reasoningModel" label={t('claudecode.model.reasoningModel')}>
        <AutoComplete
          options={modelOptions}
          placeholder={t('claudecode.model.reasoningModelPlaceholder')}
          style={{ width: '100%' }}
          filterOption={(inputValue, option) =>
            (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
            option?.value.toLowerCase().includes(inputValue.toLowerCase())) ?? false
          }
        />
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

            <Form.Item name="reasoningModel" label={t('claudecode.import.selectReasoningModel')}>
              <Select
                placeholder={t('claudecode.model.reasoningModelPlaceholder')}
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
      title={
        isEdit
          ? t('claudecode.provider.editProvider')
          : mode === 'import'
            ? t('claudecode.import.title')
            : t('claudecode.provider.addProvider')
      }
      open={open}
      onCancel={onCancel}
      onOk={handleSubmit}
      confirmLoading={loading}
      width={800}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      {isEdit || mode === 'manual' ? renderManualTab() : renderImportTab()}
    </Modal>
  );
};

export default ClaudeProviderFormModal;
