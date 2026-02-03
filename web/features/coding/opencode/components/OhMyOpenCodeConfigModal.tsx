import React from 'react';
import { Modal, Form, Input, Button, Typography, Select, Collapse, Space, message, Divider } from 'antd';
import { MoreOutlined, PlusOutlined, DeleteOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  OH_MY_OPENCODE_AGENTS,
  OH_MY_OPENCODE_CATEGORIES,
  type OhMyOpenCodeAgentConfig,
  type OhMyOpenCodeCategoryDefinition,
} from '@/types/ohMyOpenCode';
import { getAgentDisplayName, getAgentDescription, getAgentRecommendedModel, getCategoryDescription } from '@/services/ohMyOpenCodeApi';
import JsonEditor from '@/components/common/JsonEditor';

const { Text } = Typography;

const getCategoryDefinitions = (): OhMyOpenCodeCategoryDefinition[] => OH_MY_OPENCODE_CATEGORIES;

interface OhMyOpenCodeConfigModalProps {
  open: boolean;
  isEdit: boolean;
  initialValues?: {
    id?: string;
    name: string;
    agents?: Record<string, OhMyOpenCodeAgentConfig | undefined> | null;
    categories?: Record<string, OhMyOpenCodeAgentConfig | undefined> | null;
    otherFields?: Record<string, unknown>;
  };
  modelOptions: { label: string; value: string }[];
  onCancel: () => void;
  onSuccess: (values: OhMyOpenCodeConfigFormValues) => void;
}

export interface OhMyOpenCodeConfigFormValues {
  id?: string;
  name: string;
  agents: Record<string, OhMyOpenCodeAgentConfig | undefined>;
  categories: Record<string, OhMyOpenCodeAgentConfig | undefined>;
  otherFields?: Record<string, unknown>;
}

const OhMyOpenCodeConfigModal: React.FC<OhMyOpenCodeConfigModalProps> = ({
  open,
  isEdit,
  initialValues,
  modelOptions,
  onCancel,
  onSuccess,
}) => {
  const { t, i18n } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [categoriesCollapsed, setCategoriesCollapsed] = React.useState(false);

  // Track which agents/categories have advanced settings expanded
  const [expandedAgents, setExpandedAgents] = React.useState<Record<string, boolean>>({});
  const [expandedCategories, setExpandedCategories] = React.useState<Record<string, boolean>>({});

  // Custom agents and categories (user-defined)
  const [customAgents, setCustomAgents] = React.useState<string[]>([]);
  const [customCategories, setCustomCategories] = React.useState<string[]>([]);
  const [newAgentKey, setNewAgentKey] = React.useState('');
  const [newCategoryKey, setNewCategoryKey] = React.useState('');
  const [showAddAgent, setShowAddAgent] = React.useState(false);
  const [showAddCategory, setShowAddCategory] = React.useState(false);

  // Store advanced settings values in refs to avoid re-renders
  const advancedSettingsRef = React.useRef<Record<string, Record<string, unknown>>>({});
  const advancedSettingsRawRef = React.useRef<Record<string, string>>({});
  
  const categoryAdvancedSettingsRef = React.useRef<Record<string, Record<string, unknown>>>({});
  const categoryAdvancedSettingsRawRef = React.useRef<Record<string, string>>({});
  const unknownCategoriesRef = React.useRef<Record<string, OhMyOpenCodeAgentConfig>>({});

  const otherFieldsRef = React.useRef<Record<string, unknown>>({});
  const otherFieldsRawRef = React.useRef<string>('');

  // Track if modal has been initialized to avoid re-initialization on parent re-renders
  const initializedRef = React.useRef(false);
  const prevOpenRef = React.useRef(false);

  // Agent types from centralized constant
  const allAgentKeys = React.useMemo(() => OH_MY_OPENCODE_AGENTS.map((agent) => agent.key), []);
  const categoryDefinitions = React.useMemo(() => getCategoryDefinitions(), []);
  const categoryKeys = React.useMemo(
    () => categoryDefinitions.map((category) => category.key),
    [categoryDefinitions]
  );

  const labelCol = 6;
  const wrapperCol = 18;

  // Initialize form values - only when modal opens (not on every parent re-render)
  React.useEffect(() => {
    prevOpenRef.current = open;

    // Reset initialized flag when modal closes
    if (!open) {
      initializedRef.current = false;
      return;
    }

    // Skip if already initialized (prevents overwriting user input on parent re-renders)
    if (initializedRef.current) {
      return;
    }

    initializedRef.current = true;

    if (initialValues) {
      // Parse agent models and advanced settings from config
      const agentFields: Record<string, string | undefined> = {};
      const categoryFields: Record<string, string | undefined> = {};

      // Built-in agent key set for detecting custom agents
      const builtInAgentKeySet = new Set(allAgentKeys);
      const detectedCustomAgents: string[] = [];

      // Process all agents (built-in + custom)
      if (initialValues.agents) {
        Object.entries(initialValues.agents).forEach(([agentType, agent]) => {
          if (!agent) return;
          
          // Extract model
          if (typeof agent.model === 'string' && agent.model) {
            agentFields[`agent_${agentType}`] = agent.model;
          }

          // Extract advanced fields (everything except model) and store in ref
          const advancedConfig: Record<string, unknown> = {};
          Object.keys(agent).forEach((key) => {
            if (key !== 'model' && agent[key as keyof OhMyOpenCodeAgentConfig] !== undefined) {
              advancedConfig[key] = agent[key as keyof OhMyOpenCodeAgentConfig];
            }
          });

          advancedSettingsRef.current[agentType] = advancedConfig;
          
          // Track custom agents
          if (!builtInAgentKeySet.has(agentType)) {
            detectedCustomAgents.push(agentType);
          }
        });
      }

      setCustomAgents(detectedCustomAgents);

      const categoryKeySet = new Set(categoryKeys);
      const detectedCustomCategories: string[] = [];

      // Process all categories (built-in + custom)
      if (initialValues.categories) {
        Object.entries(initialValues.categories).forEach(([categoryKey, category]) => {
          if (!category) return;
          
          // Extract model
          if (typeof category.model === 'string' && category.model) {
            categoryFields[`category_${categoryKey}`] = category.model;
          }

          // Extract advanced fields (everything except model) and store in ref
          const advancedConfig: Record<string, unknown> = {};
          Object.keys(category).forEach((key) => {
            if (key !== 'model' && category[key as keyof OhMyOpenCodeAgentConfig] !== undefined) {
              advancedConfig[key] = category[key as keyof OhMyOpenCodeAgentConfig];
            }
          });

          categoryAdvancedSettingsRef.current[categoryKey] = advancedConfig;
          
          // Track custom categories
          if (!categoryKeySet.has(categoryKey)) {
            detectedCustomCategories.push(categoryKey);
          }
        });
      }

      setCustomCategories(detectedCustomCategories);
      unknownCategoriesRef.current = {}; // No longer needed, we handle custom categories explicitly

      form.setFieldsValue({
        id: initialValues.id,
        name: initialValues.name,
        ...agentFields,
        ...categoryFields,
        otherFields: initialValues.otherFields || {},
      });

      otherFieldsRef.current = initialValues.otherFields || {};
      otherFieldsRawRef.current = initialValues.otherFields && Object.keys(initialValues.otherFields).length > 0
        ? JSON.stringify(initialValues.otherFields, null, 2)
        : '';
    } else {
      form.resetFields();
      form.setFieldsValue({
        otherFields: {},
      });

      // Reset refs
      allAgentKeys.forEach((agentType) => {
        advancedSettingsRef.current[agentType] = {};
        advancedSettingsRawRef.current[agentType] = '';
      });
      
      categoryKeys.forEach((categoryKey) => {
        categoryAdvancedSettingsRef.current[categoryKey] = {};
        categoryAdvancedSettingsRawRef.current[categoryKey] = '';
      });
      unknownCategoriesRef.current = {};

      // Reset custom agents and categories
      setCustomAgents([]);
      setCustomCategories([]);

      otherFieldsRef.current = {};
      otherFieldsRawRef.current = '';
    }
    setExpandedAgents({}); // Collapse all on open
    setExpandedCategories({}); // Collapse all categories on open
    setCategoriesCollapsed(false); // Categories expanded by default
    setShowAddAgent(false);
    setShowAddCategory(false);
    setNewAgentKey('');
    setNewCategoryKey('');
  }, [open, initialValues, form, allAgentKeys, categoryKeys]);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      setLoading(true);

      // Validate otherFields JSON at submit time
      const otherFieldsRaw = otherFieldsRawRef.current.trim();
      let parsedOtherFields: Record<string, unknown> = {};
      if (otherFieldsRaw !== '') {
        try {
          parsedOtherFields = JSON.parse(otherFieldsRaw);
          if (typeof parsedOtherFields !== 'object' || parsedOtherFields === null || Array.isArray(parsedOtherFields)) {
            message.error(t('opencode.ohMyOpenCode.invalidJson'));
            setLoading(false);
            return;
          }
        } catch {
          message.error(t('opencode.ohMyOpenCode.invalidJson'));
          setLoading(false);
          return;
        }
      }

      // Validate and parse all agent advanced settings at submit time (built-in + custom)
      const allAgentKeysWithCustom = [...allAgentKeys, ...customAgents];
      const parsedAdvancedSettings: Record<string, Record<string, unknown>> = {};
      for (const agentType of allAgentKeysWithCustom) {
        const rawAdvanced = advancedSettingsRawRef.current[agentType]?.trim() || '';
        if (rawAdvanced !== '') {
          try {
            const parsed = JSON.parse(rawAdvanced);
            if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
              message.error(t('opencode.ohMyOpenCode.invalidJson'));
              setLoading(false);
              return;
            }
            parsedAdvancedSettings[agentType] = parsed;
          } catch {
            message.error(t('opencode.ohMyOpenCode.invalidJson'));
            setLoading(false);
            return;
          }
        }
      }

      // Validate and parse all category advanced settings at submit time (built-in + custom)
      const allCategoryKeysWithCustom = [...categoryKeys, ...customCategories];
      const parsedCategorySettings: Record<string, Record<string, unknown>> = {};
      for (const categoryKey of allCategoryKeysWithCustom) {
        const rawAdvanced = categoryAdvancedSettingsRawRef.current[categoryKey]?.trim() || '';
        if (rawAdvanced !== '') {
          try {
            const parsed = JSON.parse(rawAdvanced);
            if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
              message.error(t('opencode.ohMyOpenCode.invalidJson'));
              setLoading(false);
              return;
            }
            parsedCategorySettings[categoryKey] = parsed;
          } catch {
            message.error(t('opencode.ohMyOpenCode.invalidJson'));
            setLoading(false);
            return;
          }
        }
      }

      // Build agents object with merged advanced settings (built-in + custom)
      const agents: Record<string, OhMyOpenCodeAgentConfig | undefined> = {};
      allAgentKeysWithCustom.forEach((agentType) => {
        // Skip separator entries
        if (agentType.startsWith('__') && agentType.endsWith('__')) return;

        const modelFieldName = `agent_${agentType}` as keyof typeof values;

        const modelValue = values[modelFieldName];
        const advancedValue = parsedAdvancedSettings[agentType];

        // Only create agent config if model is set OR advanced settings exist
        if (modelValue || (advancedValue && Object.keys(advancedValue).length > 0)) {
          agents[agentType] = {
            ...(modelValue ? { model: modelValue } : {}),
            ...(advancedValue || {}),
          } as OhMyOpenCodeAgentConfig;
        } else {
          agents[agentType] = undefined;
        }
      });

      // Build categories object with merged advanced settings (built-in + custom)
      const categories: Record<string, OhMyOpenCodeAgentConfig | undefined> = {};
      allCategoryKeysWithCustom.forEach((categoryKey) => {
        const modelFieldName = `category_${categoryKey}` as keyof typeof values;

        const modelValue = values[modelFieldName];
        const advancedValue = parsedCategorySettings[categoryKey];

        // Only create category config if model is set OR advanced settings exist
        if (modelValue || (advancedValue && Object.keys(advancedValue).length > 0)) {
          categories[categoryKey] = {
            ...(modelValue ? { model: modelValue } : {}),
            ...(advancedValue || {}),
          } as OhMyOpenCodeAgentConfig;
        } else {
          categories[categoryKey] = undefined;
        }
      });

      const result: OhMyOpenCodeConfigFormValues = {
        name: values.name,
        agents,
        categories, // Now includes custom categories directly
        otherFields: Object.keys(parsedOtherFields).length > 0 ? parsedOtherFields : undefined,
      };

      // Include id when editing (read from form values which were set from initialValues)
      if (isEdit && values.id) {
        result.id = values.id;
      }

      onSuccess(result);
      form.resetFields();
    } catch (error) {
      console.error('Form validation error:', error);
    } finally {
      setLoading(false);
    }
  };

  const toggleAdvancedSettings = (agentType: string) => {
    setExpandedAgents(prev => ({
      ...prev,
      [agentType]: !prev[agentType],
    }));
  };

  const toggleCategorySettings = (categoryKey: string) => {
    setExpandedCategories(prev => ({
      ...prev,
      [categoryKey]: !prev[categoryKey],
    }));
  };

  // Handle adding custom agent
  const handleAddCustomAgent = () => {
    const key = newAgentKey.trim();
    if (!key) {
      message.warning(t('opencode.ohMyOpenCode.customAgentKeyRequired'));
      return;
    }
    // Check for duplicates
    const allKeys = [...allAgentKeys, ...customAgents];
    if (allKeys.includes(key)) {
      message.warning(t('opencode.ohMyOpenCode.customAgentKeyDuplicate'));
      return;
    }
    setCustomAgents(prev => [...prev, key]);
    setNewAgentKey('');
    setShowAddAgent(false);
    // Initialize refs for the new agent
    advancedSettingsRef.current[key] = {};
    advancedSettingsRawRef.current[key] = '';
  };

  // Handle removing custom agent
  const handleRemoveCustomAgent = (agentKey: string) => {
    setCustomAgents(prev => prev.filter(k => k !== agentKey));
    // Clear form field
    form.setFieldValue(`agent_${agentKey}`, undefined);
    // Clear refs
    delete advancedSettingsRef.current[agentKey];
    delete advancedSettingsRawRef.current[agentKey];
  };

  // Handle adding custom category
  const handleAddCustomCategory = () => {
    const key = newCategoryKey.trim();
    if (!key) {
      message.warning(t('opencode.ohMyOpenCode.customCategoryKeyRequired'));
      return;
    }
    // Check for duplicates
    const allKeys = [...categoryKeys, ...customCategories];
    if (allKeys.includes(key)) {
      message.warning(t('opencode.ohMyOpenCode.customCategoryKeyDuplicate'));
      return;
    }
    setCustomCategories(prev => [...prev, key]);
    setNewCategoryKey('');
    setShowAddCategory(false);
    // Initialize refs for the new category
    categoryAdvancedSettingsRef.current[key] = {};
    categoryAdvancedSettingsRawRef.current[key] = '';
  };

  // Handle removing custom category
  const handleRemoveCustomCategory = (categoryKey: string) => {
    setCustomCategories(prev => prev.filter(k => k !== categoryKey));
    // Clear form field
    form.setFieldValue(`category_${categoryKey}`, undefined);
    // Clear refs
    delete categoryAdvancedSettingsRef.current[categoryKey];
    delete categoryAdvancedSettingsRawRef.current[categoryKey];
  };

  // Render agent item (built-in agents)
  const renderAgentItem = (agentType: string) => {
    const recommendedModel = getAgentRecommendedModel(agentType);
    // Special handling for Sisyphus-Junior: show recommended text directly as placeholder
    let placeholder: string;
    if (agentType === 'Sisyphus-Junior' && recommendedModel) {
      placeholder = recommendedModel;
    } else if (recommendedModel) {
      placeholder = `${t('opencode.ohMyOpenCode.selectModel')} (${t('opencode.ohMyOpenCode.recommended')}${recommendedModel})`;
    } else {
      placeholder = t('opencode.ohMyOpenCode.selectModel');
    }

    return (
      <div key={agentType}>
        <Form.Item
          label={getAgentDisplayName(agentType).split(' ')[0]}
          tooltip={getAgentDescription(agentType, i18n.language)}
          style={{ marginBottom: expandedAgents[agentType] ? 8 : 12 }}
        >
          <Space.Compact style={{ width: '100%' }}>
            <Form.Item name={`agent_${agentType}`} noStyle>
              <Select
                placeholder={placeholder}
                options={modelOptions}
                allowClear
                showSearch
                optionFilterProp="label"
                style={{ width: 'calc(100% - 32px)' }}
              />
            </Form.Item>
            <Button
              icon={<MoreOutlined />}
              onClick={() => toggleAdvancedSettings(agentType)}
              type={expandedAgents[agentType] ? 'primary' : 'default'}
              title={t('opencode.ohMyOpenCode.advancedSettings')}
            />
          </Space.Compact>
        </Form.Item>

        {expandedAgents[agentType] && (
          <Form.Item
            help={t('opencode.ohMyOpenCode.advancedSettingsHint')}
            labelCol={{ span: 24 }}
            wrapperCol={{ span: 24 }}
            style={{ marginBottom: 16, marginLeft: labelCol * 4 + 8 }}
          >
            <JsonEditor
              value={advancedSettingsRef.current[agentType] && Object.keys(advancedSettingsRef.current[agentType]).length > 0 ? advancedSettingsRef.current[agentType] : undefined}
              onChange={(value) => {
                // Store raw string for submit-time validation
                if (value === null || value === undefined) {
                  advancedSettingsRawRef.current[agentType] = '';
                } else if (typeof value === 'string') {
                  advancedSettingsRawRef.current[agentType] = value;
                } else {
                  advancedSettingsRawRef.current[agentType] = JSON.stringify(value, null, 2);
                }
              }}
              height={150}
              minHeight={100}
              maxHeight={300}
              resizable
              mode="text"
              placeholder={`{
    "temperature": 0.5
}`}
            />
          </Form.Item>
      )}
    </div>
  );
  };

  // Render custom agent item (with delete button)
  const renderCustomAgentItem = (agentType: string) => (
    <div key={agentType}>
      <Form.Item
        label={<span style={{ color: '#1890ff' }}>{agentType}</span>}
        tooltip={t('opencode.ohMyOpenCode.customAgentTooltip')}
        style={{ marginBottom: expandedAgents[agentType] ? 8 : 12 }}
      >
        <Space.Compact style={{ width: '100%' }}>
          <Form.Item name={`agent_${agentType}`} noStyle>
            <Select
              placeholder={t('opencode.ohMyOpenCode.selectModel')}
              options={modelOptions}
              allowClear
              showSearch
              optionFilterProp="label"
              style={{ width: 'calc(100% - 64px)' }}
            />
          </Form.Item>
          <Button
            icon={<MoreOutlined />}
            onClick={() => toggleAdvancedSettings(agentType)}
            type={expandedAgents[agentType] ? 'primary' : 'default'}
            title={t('opencode.ohMyOpenCode.advancedSettings')}
          />
          <Button
            icon={<DeleteOutlined />}
            onClick={() => handleRemoveCustomAgent(agentType)}
            danger
            title={t('common.delete')}
          />
        </Space.Compact>
      </Form.Item>

      {expandedAgents[agentType] && (
        <Form.Item
          help={t('opencode.ohMyOpenCode.advancedSettingsHint')}
          labelCol={{ span: 24 }}
          wrapperCol={{ span: 24 }}
          style={{ marginBottom: 16, marginLeft: labelCol * 4 + 8 }}
        >
          <JsonEditor
            value={advancedSettingsRef.current[agentType] && Object.keys(advancedSettingsRef.current[agentType]).length > 0 ? advancedSettingsRef.current[agentType] : undefined}
            onChange={(value) => {
              if (value === null || value === undefined) {
                advancedSettingsRawRef.current[agentType] = '';
              } else if (typeof value === 'string') {
                advancedSettingsRawRef.current[agentType] = value;
              } else {
                advancedSettingsRawRef.current[agentType] = JSON.stringify(value, null, 2);
              }
            }}
            height={150}
            minHeight={100}
            maxHeight={300}
            resizable
            mode="text"
            placeholder={`{
    "temperature": 0.5
}`}
          />
        </Form.Item>
      )}
    </div>
  );

  // Render category item
  const renderCategoryItem = (category: OhMyOpenCodeCategoryDefinition) => {
    const placeholder = category.recommendedModel
      ? `${t('opencode.ohMyOpenCode.selectModel')} (${t('opencode.ohMyOpenCode.recommended')}${category.recommendedModel})`
      : t('opencode.ohMyOpenCode.selectModel');

    return (
      <div key={category.key}>
        <Form.Item
          label={category.display}
          tooltip={getCategoryDescription(category.key, i18n.language)}
          style={{ marginBottom: expandedCategories[category.key] ? 8 : 12 }}
        >
          <Space.Compact style={{ width: '100%' }}>
            <Form.Item name={`category_${category.key}`} noStyle>
              <Select
                placeholder={placeholder}
                options={modelOptions}
                allowClear
                showSearch
                optionFilterProp="label"
                style={{ width: 'calc(100% - 32px)' }}
              />
            </Form.Item>
            <Button
              icon={<MoreOutlined />}
              onClick={() => toggleCategorySettings(category.key)}
              type={expandedCategories[category.key] ? 'primary' : 'default'}
              title={t('opencode.ohMyOpenCode.advancedSettings')}
            />
          </Space.Compact>
        </Form.Item>

        {expandedCategories[category.key] && (
          <Form.Item
            help={t('opencode.ohMyOpenCode.advancedSettingsHint')}
            labelCol={{ span: 24 }}
            wrapperCol={{ span: 24 }}
            style={{ marginBottom: 16, marginLeft: labelCol * 4 + 8 }}
          >
            <JsonEditor
              value={categoryAdvancedSettingsRef.current[category.key] && Object.keys(categoryAdvancedSettingsRef.current[category.key]).length > 0 ? categoryAdvancedSettingsRef.current[category.key] : undefined}
              onChange={(value) => {
                // Store raw string for submit-time validation
                if (value === null || value === undefined) {
                  categoryAdvancedSettingsRawRef.current[category.key] = '';
                } else if (typeof value === 'string') {
                  categoryAdvancedSettingsRawRef.current[category.key] = value;
                } else {
                  categoryAdvancedSettingsRawRef.current[category.key] = JSON.stringify(value, null, 2);
                }
              }}
              height={150}
              minHeight={100}
              maxHeight={300}
              resizable
              mode="text"
              placeholder={`{
    "temperature": 0.5
}`}
            />
          </Form.Item>
        )}
      </div>
    );
  };

  // Render custom category item (with delete button)
  const renderCustomCategoryItem = (categoryKey: string) => (
    <div key={categoryKey}>
      <Form.Item
        label={<span style={{ color: '#1890ff' }}>{categoryKey}</span>}
        tooltip={t('opencode.ohMyOpenCode.customCategoryTooltip')}
        style={{ marginBottom: expandedCategories[categoryKey] ? 8 : 12 }}
      >
        <Space.Compact style={{ width: '100%' }}>
          <Form.Item name={`category_${categoryKey}`} noStyle>
            <Select
              placeholder={t('opencode.ohMyOpenCode.selectModel')}
              options={modelOptions}
              allowClear
              showSearch
              optionFilterProp="label"
              style={{ width: 'calc(100% - 64px)' }}
            />
          </Form.Item>
          <Button
            icon={<MoreOutlined />}
            onClick={() => toggleCategorySettings(categoryKey)}
            type={expandedCategories[categoryKey] ? 'primary' : 'default'}
            title={t('opencode.ohMyOpenCode.advancedSettings')}
          />
          <Button
            icon={<DeleteOutlined />}
            onClick={() => handleRemoveCustomCategory(categoryKey)}
            danger
            title={t('common.delete')}
          />
        </Space.Compact>
      </Form.Item>

      {expandedCategories[categoryKey] && (
        <Form.Item
          help={t('opencode.ohMyOpenCode.advancedSettingsHint')}
          labelCol={{ span: 24 }}
          wrapperCol={{ span: 24 }}
          style={{ marginBottom: 16, marginLeft: labelCol * 4 + 8 }}
        >
          <JsonEditor
            value={categoryAdvancedSettingsRef.current[categoryKey] && Object.keys(categoryAdvancedSettingsRef.current[categoryKey]).length > 0 ? categoryAdvancedSettingsRef.current[categoryKey] : undefined}
            onChange={(value) => {
              if (value === null || value === undefined) {
                categoryAdvancedSettingsRawRef.current[categoryKey] = '';
              } else if (typeof value === 'string') {
                categoryAdvancedSettingsRawRef.current[categoryKey] = value;
              } else {
                categoryAdvancedSettingsRawRef.current[categoryKey] = JSON.stringify(value, null, 2);
              }
            }}
            height={150}
            minHeight={100}
            maxHeight={300}
            resizable
            mode="text"
            placeholder={`{
    "temperature": 0.5
}`}
          />
        </Form.Item>
      )}
    </div>
  );

  return (
    <Modal
      title={isEdit
        ? t('opencode.ohMyOpenCode.editConfig')
        : t('opencode.ohMyOpenCode.addConfig')}
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
      width={800}
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={{ span: labelCol }}
        wrapperCol={{ span: wrapperCol }}
        style={{ marginTop: 24 }}
      >
        {/* Hidden ID field for editing */}
        <Form.Item name="id" hidden>
          <Input />
        </Form.Item>

        <Form.Item
          label={t('opencode.ohMyOpenCode.configName')}
          name="name"
          rules={[{ required: true, message: t('opencode.ohMyOpenCode.configNamePlaceholder') }]}
        >
          <Input
            placeholder={t('opencode.ohMyOpenCode.configNamePlaceholder')}
          />
        </Form.Item>

        <div style={{ maxHeight: 500, overflowY: 'auto', paddingRight: 8, marginTop: 16 }}>
          {/* Agent Models */}
          <Collapse
            defaultActiveKey={['agents']}
            ghost
            items={[
              {
                key: 'agents',
                label: <Text strong>{t('opencode.ohMyOpenCode.agentModels')}</Text>,
                children: (
                  <>
                    <Text type="secondary" style={{ display: 'block', fontSize: 12, marginBottom: 12 }}>
                      有固定专长的"专家角色"，每个Agent有特定能力边界和工具权限
                    </Text>
                    {allAgentKeys.map((agentType) => {
                      // Render separator as a Divider instead of a form item
                      if (agentType.startsWith('__') && agentType.endsWith('_separator__')) {
                        const desc = getAgentDescription(agentType, i18n.language);
                        return (
                          <Divider key={agentType} style={{ margin: '16px 0 12px 0', fontSize: 12, color: '#666' }}>
                            {desc}
                          </Divider>
                        );
                      }
                      return renderAgentItem(agentType);
                    })}
                    
                    {/* Custom Agents */}
                    {customAgents.length > 0 && (
                      <>
                        <Divider style={{ margin: '12px 0', fontSize: 12 }}>
                          {t('opencode.ohMyOpenCode.customAgents')}
                        </Divider>
                        {customAgents.map(renderCustomAgentItem)}
                      </>
                    )}
                    
                    {/* Add Custom Agent */}
                    {showAddAgent ? (
                      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
                        <Input
                          placeholder={t('opencode.ohMyOpenCode.customAgentKeyPlaceholder')}
                          value={newAgentKey}
                          onChange={(e) => setNewAgentKey(e.target.value)}
                          onPressEnter={handleAddCustomAgent}
                          style={{ flex: 1 }}
                        />
                        <Button type="primary" onClick={handleAddCustomAgent}>
                          {t('common.confirm')}
                        </Button>
                        <Button onClick={() => { setShowAddAgent(false); setNewAgentKey(''); }}>
                          {t('common.cancel')}
                        </Button>
                      </div>
                    ) : (
                      <Button
                        type="dashed"
                        icon={<PlusOutlined />}
                        onClick={() => setShowAddAgent(true)}
                        style={{ width: '100%', marginTop: 12 }}
                      >
                        {t('opencode.ohMyOpenCode.addCustomAgent')}
                      </Button>
                    )}
                  </>
                ),
              },
            ]}
          />

          {/* Categories */}
          <Collapse
            defaultActiveKey={categoriesCollapsed ? [] : ['categories']}
            onChange={(keys) => setCategoriesCollapsed(!keys.includes('categories'))}
            style={{ marginTop: 8 }}
            ghost
            items={[
              {
                key: 'categories',
                label: <Text strong>{t('opencode.ohMyOpenCode.categories') || 'Categories'}</Text>,
                children: (
                  <>
                    <Text type="secondary" style={{ display: 'block', fontSize: 12, marginBottom: 12 }}>
                      可配置的"任务模板"，定义执行任务时使用的模型、推理强度和工作风格，由Sisyphus-Junior继承后执行具体实现工作。
                    </Text>
                    {categoryDefinitions.map(renderCategoryItem)}
                    
                    {/* Custom Categories */}
                    {customCategories.length > 0 && (
                      <>
                        <Divider style={{ margin: '12px 0', fontSize: 12 }}>
                          {t('opencode.ohMyOpenCode.customCategories')}
                        </Divider>
                        {customCategories.map(renderCustomCategoryItem)}
                      </>
                    )}
                    
                    {/* Add Custom Category */}
                    {showAddCategory ? (
                      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
                        <Input
                          placeholder={t('opencode.ohMyOpenCode.customCategoryKeyPlaceholder')}
                          value={newCategoryKey}
                          onChange={(e) => setNewCategoryKey(e.target.value)}
                          onPressEnter={handleAddCustomCategory}
                          style={{ flex: 1 }}
                        />
                        <Button type="primary" onClick={handleAddCustomCategory}>
                          {t('common.confirm')}
                        </Button>
                        <Button onClick={() => { setShowAddCategory(false); setNewCategoryKey(''); }}>
                          {t('common.cancel')}
                        </Button>
                      </div>
                    ) : (
                      <Button
                        type="dashed"
                        icon={<PlusOutlined />}
                        onClick={() => setShowAddCategory(true)}
                        style={{ width: '100%', marginTop: 12 }}
                      >
                        {t('opencode.ohMyOpenCode.addCustomCategory')}
                      </Button>
                    )}
                  </>
                ),
              },
            ]}
          />

          {/* 其他配置 */}
          <Collapse
            defaultActiveKey={[]}
            style={{ marginTop: 8 }}
            ghost
            items={[
              {
                key: 'other',
                label: <Text strong>{t('opencode.ohMyOpenCode.otherFields')}</Text>,
                children: (
                  <Form.Item
                    help={t('opencode.ohMyOpenCode.otherFieldsHint')}
                    labelCol={{ span: 24 }}
                    wrapperCol={{ span: 24 }}
                  >
                    <JsonEditor
                      value={otherFieldsRef.current && Object.keys(otherFieldsRef.current).length > 0 ? otherFieldsRef.current : undefined}
                      onChange={(value) => {
                        // Store raw string for submit-time validation
                        if (value === null || value === undefined) {
                          otherFieldsRawRef.current = '';
                        } else if (typeof value === 'string') {
                          otherFieldsRawRef.current = value;
                        } else {
                          otherFieldsRawRef.current = JSON.stringify(value, null, 2);
                        }
                      }}
                      height={200}
                      minHeight={150}
                      maxHeight={400}
                      resizable
                      mode="text"
                      placeholder={`{
    "background_task": {
        "defaultConcurrency": 5
    }
}`}
                    />
                  </Form.Item>
                ),
              },
            ]}
          />
        </div>
      </Form>
    </Modal>
  );
};

export default OhMyOpenCodeConfigModal;
