import React from 'react';
import { Modal, Form, Input, AutoComplete, Button, Select, message, Typography, Tag } from 'antd';
import { RightOutlined, DownOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useAppStore } from '@/stores';
import JsonEditor from '@/components/common/JsonEditor';
import type { I18nPrefix } from '@/components/common/ProviderCard/types';
import { PRESET_MODELS, type PresetModel } from '@/constants/presetModels';

const { Text } = Typography;

// Context limit options with display labels
const CONTEXT_LIMIT_OPTIONS = [
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
  { value: '128000', label: '128K' },
  { value: '200000', label: '200K' },
  { value: '256000', label: '256K' },
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

// Modality options for input/output
const MODALITY_OPTIONS = [
  { value: 'text', label: 'Text' },
  { value: 'image', label: 'Image' },
  { value: 'pdf', label: 'PDF' },
  { value: 'video', label: 'Video' },
  { value: 'audio', label: 'Audio' },
];

/**
 * Form values for model form
 */
export interface ModelFormValues {
  id: string;
  name: string;
  contextLimit?: number;
  outputLimit?: number;
  options?: string;
  variants?: string;
  modalities?: string;
}

interface ModelFormModalProps {
  open: boolean;

  /** Whether this is an edit operation */
  isEdit?: boolean;
  /** Initial form values */
  initialValues?: Partial<ModelFormValues>;

  /** Existing model IDs for duplicate check (only used when !isEdit) */
  existingIds?: string[];

  /** Whether to show options field (settings page: true, OpenCode: false) */
  showOptions?: boolean;
  /** Whether to show variants field (OpenCode only) */
  showVariants?: boolean;
  /** Whether to show modalities field (OpenCode only) */
  showModalities?: boolean;
  /** Whether limit fields are required (settings page: true, OpenCode: false) */
  limitRequired?: boolean;
  /** Whether name field is required (settings page: true, OpenCode: false) */
  nameRequired?: boolean;

  /** NPM SDK type for preset models dropdown */
  npmType?: string;

  /** Callbacks */
  onCancel: () => void;
  onSuccess: (values: ModelFormValues) => void;
  /** Custom duplicate ID error handler */
  onDuplicateId?: (id: string) => void;

  /** i18n prefix for translations */
  i18nPrefix?: I18nPrefix;
}

/**
 * A reusable model form modal component
 */
const ModelFormModal: React.FC<ModelFormModalProps> = ({
  open,
  isEdit = false,
  initialValues,
  existingIds = [],
  showOptions = true,
  showVariants = false,
  showModalities = false,
  limitRequired = true,
  nameRequired = true,
  npmType,
  onCancel,
  onSuccess,
  onDuplicateId,
  i18nPrefix = 'settings',
}) => {
  const { t } = useTranslation();
  const language = useAppStore((state) => state.language);
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [jsonOptions, setJsonOptions] = React.useState<unknown>({});
  const [jsonValid, setJsonValid] = React.useState(true);
  const [jsonVariants, setJsonVariants] = React.useState<unknown>({});
  const [variantsValid, setVariantsValid] = React.useState(true);
  const [inputModalities, setInputModalities] = React.useState<string[]>([]);
  const [outputModalities, setOutputModalities] = React.useState<string[]>([]);
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);
  const [presetsExpanded, setPresetsExpanded] = React.useState(false);

  // Get preset models for current npm type
  const presetModels = React.useMemo(() => {
    if (!npmType) return [];
    return PRESET_MODELS[npmType] || [];
  }, [npmType]);

  // Handle preset model selection
  const handlePresetSelect = (preset: PresetModel) => {
    form.setFieldsValue({
      id: preset.id,
      name: preset.name,
      contextLimit: preset.contextLimit,
      outputLimit: preset.outputLimit,
    });

    // Set options if present
    if (preset.options && Object.keys(preset.options).length > 0) {
      setJsonOptions(preset.options);
      setJsonValid(true);
    }

    // Set variants if present
    if (preset.variants && Object.keys(preset.variants).length > 0) {
      setJsonVariants(preset.variants);
      setVariantsValid(true);
      // Auto expand advanced settings if variants has content
      setAdvancedExpanded(true);
    }

    // Set modalities if present
    if (preset.modalities) {
      if (preset.modalities.input) {
        setInputModalities(preset.modalities.input);
      }
      if (preset.modalities.output) {
        setOutputModalities(preset.modalities.output);
      }
      // Auto expand advanced settings if modalities has content
      if ((preset.modalities.input && preset.modalities.input.length > 0) ||
          (preset.modalities.output && preset.modalities.output.length > 0)) {
        setAdvancedExpanded(true);
      }
    }

    setPresetsExpanded(false);
  };

  const labelCol = { span: language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };

  // Check if options or variants or modalities has content
  const hasAdvancedContent = React.useMemo(() => {
    const hasOptions = typeof jsonOptions === 'object' && jsonOptions !== null &&
      Object.keys(jsonOptions).length > 0;
    const hasVariants = showVariants &&
      typeof jsonVariants === 'object' && jsonVariants !== null &&
      Object.keys(jsonVariants as object).length > 0;
    const hasModalities = showModalities &&
      (inputModalities.length > 0 || outputModalities.length > 0);
    return hasOptions || hasVariants || hasModalities;
  }, [jsonOptions, jsonVariants, inputModalities, outputModalities, showVariants, showModalities]);

  React.useEffect(() => {
    if (open) {
      if (initialValues) {
        form.setFieldsValue({
          id: initialValues.id,
          name: initialValues.name,
          contextLimit: initialValues.contextLimit,
          outputLimit: initialValues.outputLimit,
        });
        
        let shouldExpand = false;
        
        // Parse options JSON
        if (initialValues.options) {
          try {
            const parsed = JSON.parse(initialValues.options);
            setJsonOptions(parsed);
            setJsonValid(true);
            // Auto expand if options has content
            if (typeof parsed === 'object' && parsed !== null && Object.keys(parsed).length > 0) {
              shouldExpand = true;
            }
          } catch {
            setJsonOptions({});
            setJsonValid(false);
          }
        } else {
          setJsonOptions({});
          setJsonValid(true);
        }
        
        // Parse variants JSON
        if (initialValues.variants) {
          try {
            const parsed = JSON.parse(initialValues.variants);
            setJsonVariants(parsed);
            setVariantsValid(true);
            // Auto expand if variants has content
            if (typeof parsed === 'object' && parsed !== null && Object.keys(parsed).length > 0) {
              shouldExpand = true;
            }
          } catch {
            setJsonVariants({});
            setVariantsValid(false);
          }
        } else {
          setJsonVariants({});
          setVariantsValid(true);
        }
        
        // Parse modalities JSON
        if (initialValues.modalities) {
          try {
            const parsed = JSON.parse(initialValues.modalities);
            if (parsed && typeof parsed === 'object') {
              if (Array.isArray(parsed.input)) {
                setInputModalities(parsed.input);
              }
              if (Array.isArray(parsed.output)) {
                setOutputModalities(parsed.output);
              }
              // Auto expand if modalities has content
              if ((parsed.input && parsed.input.length > 0) || (parsed.output && parsed.output.length > 0)) {
                shouldExpand = true;
              }
            }
          } catch {
            setInputModalities([]);
            setOutputModalities([]);
          }
        } else {
          setInputModalities([]);
          setOutputModalities([]);
        }
        
        setAdvancedExpanded(shouldExpand);
      } else {
        form.resetFields();
        setJsonOptions({});
        setJsonValid(true);
        setJsonVariants({});
        setVariantsValid(true);
        setInputModalities([]);
        setOutputModalities([]);
        setAdvancedExpanded(false);
      }
    }
  }, [open, initialValues, form]);

  const handleJsonChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setJsonOptions(value);
    }
    setJsonValid(isValid);
  };

  const handleVariantsChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setJsonVariants(value);
    }
    setVariantsValid(isValid);
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      
      // Validate JSON if showing options
      if (showOptions && !jsonValid) {
        message.error(t('settings.model.invalidJson'));
        return;
      }
      
      // Validate variants JSON if showing variants
      if (showVariants && !variantsValid) {
        message.error(t('opencode.model.invalidVariants'));
        return;
      }
      
      // Validate modalities: either both selected or both empty
      if (showModalities) {
        const hasInput = inputModalities.length > 0;
        const hasOutput = outputModalities.length > 0;
        if (hasInput !== hasOutput) {
          message.error(t('opencode.model.modalitiesBothRequired'));
          return;
        }
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

      const result: ModelFormValues = {
        id: values.id,
        name: values.name,
        contextLimit: values.contextLimit,
        outputLimit: values.outputLimit,
      };

      if (showOptions) {
        result.options = JSON.stringify(jsonOptions);
      }
      
      if (showVariants) {
        result.variants = JSON.stringify(jsonVariants);
      }

      if (showModalities && inputModalities.length > 0 && outputModalities.length > 0) {
        result.modalities = JSON.stringify({
          input: inputModalities,
          output: outputModalities,
        });
      }

      onSuccess(result);
      form.resetFields();
    } catch (error: unknown) {
      console.error('Model form validation error:', error);
      // Form validation errors are already shown by Form
    } finally {
      setLoading(false);
    }
  };

  // Build i18n keys based on prefix
  const getKey = (key: string) => `${i18nPrefix}.model.${key}`;

  const limitRules = limitRequired
    ? [
        { required: true, message: t(getKey('contextLimitPlaceholder')) },
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ]
    : [
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ];

  const outputLimitRules = limitRequired
    ? [
        { required: true, message: t(getKey('outputLimitPlaceholder')) },
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ]
    : [
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ];

  return (
    <Modal
      title={isEdit ? t(getKey('editModel')) : t(getKey('addModel'))}
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
      width={showOptions ? 700 : 500}
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={labelCol}
        wrapperCol={wrapperCol}
        style={{ marginTop: 24 }}
      >
        <Form.Item
          label={t(getKey('id'))}
          required
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Form.Item
              name="id"
              noStyle
              rules={[{ required: true, message: t(getKey('idPlaceholder')) }]}
            >
              <Input
                placeholder={t(getKey('idPlaceholder'))}
                disabled={isEdit}
                style={{ flex: 1 }}
              />
            </Form.Item>
            {!isEdit && npmType && presetModels.length > 0 && (
              <a
                style={{
                  flexShrink: 0,
                  fontSize: 12,
                  fontWeight: 500,
                  color: 'var(--ant-color-text-secondary)',
                  cursor: 'pointer',
                  userSelect: 'none',
                  whiteSpace: 'nowrap',
                }}
                onClick={() => setPresetsExpanded(!presetsExpanded)}
              >
                {t('opencode.model.selectPreset')}
                {presetsExpanded ? ' ▴' : ' ▾'}
              </a>
            )}
          </div>
        </Form.Item>

        {!isEdit && presetsExpanded && presetModels.length > 0 && (
          <Form.Item wrapperCol={{ offset: language === 'zh-CN' ? 4 : 6, span: 20 }} style={{ marginTop: -8 }}>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
              {presetModels.map((preset) => (
                <Tag
                  key={preset.id}
                  style={{
                    cursor: 'pointer',
                    transition: 'all 0.2s',
                  }}
                  onClick={() => handlePresetSelect(preset)}
                >
                  {preset.name}
                </Tag>
              ))}
            </div>
          </Form.Item>
        )}

        <Form.Item
          label={t(getKey('name'))}
          name="name"
          rules={nameRequired ? [{ required: true, message: t(getKey('namePlaceholder')) }] : []}
        >
          <Input placeholder={nameRequired ? t(getKey('namePlaceholder')) : t(getKey('nameOptionalPlaceholder'))} />
        </Form.Item>

        <Form.Item
          label={t(getKey('contextLimit'))}
          name="contextLimit"
          rules={limitRules}
          getValueFromEvent={(val) => {
            const num = parseInt(val, 10);
            return isNaN(num) ? undefined : num;
          }}
        >
          <AutoComplete
            options={CONTEXT_LIMIT_OPTIONS}
            placeholder={t(getKey('contextLimitPlaceholder'))}
            style={{ width: '100%' }}
            filterOption={(inputValue, option) =>
              (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
              option?.value.includes(inputValue)) ?? false
            }
          />
        </Form.Item>

        <Form.Item
          label={t(getKey('outputLimit'))}
          name="outputLimit"
          rules={outputLimitRules}
          getValueFromEvent={(val) => {
            const num = parseInt(val, 10);
            return isNaN(num) ? undefined : num;
          }}
        >
          <AutoComplete
            options={OUTPUT_LIMIT_OPTIONS}
            placeholder={t(getKey('outputLimitPlaceholder'))}
            style={{ width: '100%' }}
            filterOption={(inputValue, option) =>
              (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
              option?.value.includes(inputValue)) ?? false
            }
          />
        </Form.Item>

        {(showOptions || showVariants || showModalities) && (
          <>
            <div style={{ marginBottom: advancedExpanded ? 16 : 0 }}>
              <Button
                type="link"
                onClick={() => setAdvancedExpanded(!advancedExpanded)}
                style={{ padding: 0, height: 'auto' }}
              >
                {advancedExpanded ? <DownOutlined /> : <RightOutlined />}
                <span style={{ marginLeft: 4 }}>
                  {t('common.advancedSettings')}
                  {hasAdvancedContent && !advancedExpanded && (
                    <span style={{ marginLeft: 4, color: '#1890ff' }}>*</span>
                  )}
                </span>
              </Button>
            </div>
            {advancedExpanded && (
              <>
                {showModalities && (
                  <>
                    <Form.Item
                      label={t('opencode.model.inputModalities')}
                    >
                      <Select
                        mode="multiple"
                        allowClear
                        placeholder={t('opencode.model.inputModalitiesPlaceholder')}
                        options={MODALITY_OPTIONS}
                        value={inputModalities}
                        onChange={setInputModalities}
                      />
                    </Form.Item>
                    <Form.Item
                      label={t('opencode.model.outputModalities')}
                      extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('opencode.model.modalitiesHint')}</Text>}
                    >
                      <Select
                        mode="multiple"
                        allowClear
                        placeholder={t('opencode.model.outputModalitiesPlaceholder')}
                        options={MODALITY_OPTIONS}
                        value={outputModalities}
                        onChange={setOutputModalities}
                      />
                    </Form.Item>
                  </>
                )}

                {showVariants && (
                  <Form.Item
                    label={t('opencode.model.variants')}
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('opencode.model.variantsHint')}</Text>}
                  >
                    <JsonEditor
                      value={typeof jsonVariants === 'object' && jsonVariants !== null && Object.keys(jsonVariants as object).length === 0 ? undefined : jsonVariants}
                      onChange={handleVariantsChange}
                      mode="text"
                      height={200}
                      resizable
                      placeholder={`{
    "minimal": { "thinkingLevel": "minimal" },
    "low": { "thinkingLevel": "low" },
    "medium": { "thinkingLevel": "medium" },
    "high": { "thinkingLevel": "high" }
}`}
                    />
                  </Form.Item>
                )}

                {showOptions && (
                  <Form.Item label={t('settings.model.options')}>
                    <JsonEditor
                      value={typeof jsonOptions === 'object' && jsonOptions !== null && Object.keys(jsonOptions).length === 0 ? undefined : jsonOptions}
                      onChange={handleJsonChange}
                      mode="text"
                      height={200}
                      resizable
                      placeholder={`{
    "store": false
}`}
                    />
                  </Form.Item>
                )}
              </>
            )}
          </>
        )}
      </Form>
    </Modal>
  );
};

export default ModelFormModal;
