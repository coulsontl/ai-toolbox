import React from 'react';
import { Form, Modal, Select, Typography, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { setOpenClawAgentsDefaults } from '@/services/openclawApi';
import JsonEditor from '@/components/common/JsonEditor';
import type { OpenClawAgentsDefaults, OpenClawConfig } from '@/types/openclaw';

const { Text } = Typography;

interface Props {
  defaults: OpenClawAgentsDefaults | null;
  config: OpenClawConfig | null;
  onSaved: () => void;
}

export interface AgentsDefaultsCardRef {
  openMoreParams: () => void;
}

const formItemLayout = {
  labelCol: { span: 2 },
  wrapperCol: { span: 22 },
};

/** Keys managed by dedicated form fields — excluded from "more params" editor */
const MANAGED_KEYS = new Set(['model', 'models']);

const AgentsDefaultsCard = React.forwardRef<AgentsDefaultsCardRef, Props>(({ defaults, config, onSaved }, ref) => {
  const { t } = useTranslation();

  // Local editable state
  const [primaryModel, setPrimaryModel] = React.useState<string | undefined>(undefined);
  const [fallbacks, setFallbacks] = React.useState<string[]>([]);

  // More params modal
  const [moreParamsOpen, setMoreParamsOpen] = React.useState(false);
  const [extraParams, setExtraParams] = React.useState<Record<string, unknown>>({});
  const [extraParamsValid, setExtraParamsValid] = React.useState(true);

  React.useEffect(() => {
    if (defaults) {
      setPrimaryModel(defaults.model?.primary || undefined);
      setFallbacks(defaults.model?.fallbacks || []);
    }
  }, [defaults]);

  // Build model options from all providers
  const modelOptions = React.useMemo(() => {
    if (!config?.models?.providers) return [];
    const options: { label: string; value: string }[] = [];
    for (const [providerId, provider] of Object.entries(config.models.providers)) {
      for (const model of provider.models || []) {
        const fullId = `${providerId}/${model.id}`;
        const modelName = model.name || model.id;
        options.push({
          label: `${providerId} / ${modelName}`,
          value: fullId,
        });
      }
    }
    return options;
  }, [config]);

  // Build the full defaults object from current state + extra params
  const buildDefaults = React.useCallback((overrides?: {
    primaryModel?: string | undefined;
    fallbacks?: string[];
    extra?: Record<string, unknown>;
  }): OpenClawAgentsDefaults => {
    const pm = overrides?.primaryModel !== undefined ? overrides.primaryModel : primaryModel;
    const fb = overrides?.fallbacks !== undefined ? overrides.fallbacks : fallbacks;

    // Start from extra/unknown fields in defaults (excluding managed keys)
    const extraFields: Record<string, unknown> = {};
    if (defaults) {
      for (const [k, v] of Object.entries(defaults)) {
        if (!MANAGED_KEYS.has(k)) {
          extraFields[k] = v;
        }
      }
    }

    // Merge explicit extra overrides if provided
    const extra = overrides?.extra;
    const merged = extra !== undefined ? extra : extraFields;

    const result: OpenClawAgentsDefaults = {
      ...merged,
      model: { primary: pm || '', fallbacks: fb },
      models: defaults?.models,
    };

    return result;
  }, [defaults, primaryModel, fallbacks]);

  const doSave = React.useCallback(async (overrides?: {
    primaryModel?: string | undefined;
    fallbacks?: string[];
    extra?: Record<string, unknown>;
  }) => {
    try {
      const newDefaults = buildDefaults(overrides);
      await setOpenClawAgentsDefaults(newDefaults);
      onSaved();
    } catch (error) {
      console.error('Failed to save agents defaults:', error);
      message.error(t('common.error'));
    }
  }, [buildDefaults, onSaved, t]);

  // Select changes save immediately
  const handlePrimaryModelChange = (value: string | undefined) => {
    setPrimaryModel(value);
    doSave({ primaryModel: value });
  };

  const handleFallbacksChange = (value: string[]) => {
    setFallbacks(value);
    doSave({ fallbacks: value });
  };

  // More params modal
  const handleOpenMoreParams = () => {
    // Extract non-managed fields
    const extra: Record<string, unknown> = {};
    if (defaults) {
      for (const [k, v] of Object.entries(defaults)) {
        if (!MANAGED_KEYS.has(k)) {
          extra[k] = v;
        }
      }
    }
    setExtraParams(extra);
    setExtraParamsValid(true);
    setMoreParamsOpen(true);
  };

  const handleSaveMoreParams = async () => {
    if (!extraParamsValid) {
      message.error(t('common.error'));
      return;
    }
    await doSave({ extra: extraParams });
    setMoreParamsOpen(false);
  };

  // Expose openMoreParams to parent via ref
  React.useImperativeHandle(ref, () => ({
    openMoreParams: handleOpenMoreParams,
  }));

  return (
    <>
      <Form layout="horizontal" {...formItemLayout}>
        {/* Primary Model */}
        <Form.Item label={<Text strong>{t('openclaw.agents.primaryModel')}</Text>}>
          <Select
            value={primaryModel}
            onChange={handlePrimaryModelChange}
            placeholder={t('openclaw.agents.primaryModelPlaceholder')}
            allowClear
            showSearch
            options={modelOptions}
            optionLabelProp="label"
            style={{ width: '100%' }}
            notFoundContent={t('openclaw.agents.noModels')}
          />
        </Form.Item>

        {/* Fallbacks */}
        <Form.Item label={<Text strong>{t('openclaw.agents.fallbacks')}</Text>}>
          <Select
            mode="multiple"
            value={fallbacks}
            onChange={handleFallbacksChange}
            placeholder={t('openclaw.agents.fallbacksPlaceholder')}
            allowClear
            showSearch
            options={modelOptions}
            optionLabelProp="label"
            style={{ width: '100%' }}
            notFoundContent={t('openclaw.agents.noModels')}
          />
        </Form.Item>
      </Form>

      {/* More Parameters Modal */}
      <Modal
        title={t('openclaw.agents.moreParamsTitle')}
        open={moreParamsOpen}
        onCancel={() => setMoreParamsOpen(false)}
        onOk={handleSaveMoreParams}
        okText={t('common.save')}
        cancelText={t('common.cancel')}
        width={600}
        destroyOnClose
      >
        <JsonEditor
          value={extraParams}
          onChange={(val, valid) => {
            if (typeof val === 'object' && val !== null && !Array.isArray(val)) {
              setExtraParams(val as Record<string, unknown>);
            }
            setExtraParamsValid(valid);
          }}
          height={300}
        />
      </Modal>
    </>
  );
});

AgentsDefaultsCard.displayName = 'AgentsDefaultsCard';

export default AgentsDefaultsCard;
