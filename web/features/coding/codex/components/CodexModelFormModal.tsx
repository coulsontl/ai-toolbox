import React from 'react';
import { Modal, Form, Input, InputNumber, Select, Tag, Divider, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import {
  PRESET_MODELS,
  getPresetModelsVersion,
  subscribePresetModels,
  type PresetModel,
} from '@/constants/presetModels';
import type { CodexCatalogModel } from '@/types/codex';
import {
  CODEX_REASONING_LEVELS,
  CODEX_SUPPORTED_MODALITIES,
  fillCodexCatalogModelFromPreset,
  fromCodexCatalogModelFormValues,
  toCodexCatalogModelFormValues,
  type CodexCatalogModelFormValues,
  type CodexCatalogPresetResolver,
} from '../utils/codexCatalogModels';
import { CODEX_SERVICE_TIER_OPTIONS } from './ServiceTiersEditor';
import { MODEL_FORM_LAYOUT } from '@/features/coding/shared/modelFormLayout';

const { Text } = Typography;

/** Modalities Codex understands; anything else is dropped at catalog generation. */
const CODEX_MODALITY_OPTIONS = CODEX_SUPPORTED_MODALITIES.map((value) => ({
  value,
  label: value,
}));

interface CodexModelFormModalProps {
  open: boolean;
  isEdit: boolean;
  /** Effective SDK (npm) type of the provider, used to order the preset picker. */
  presetNpmType?: string;
  /** Resolves a preset model for the provider's effective SDK, used to fill
   *  capability defaults when a known upstream model id is typed. */
  resolvePreset?: CodexCatalogPresetResolver;
  /** Row being edited; omitted when adding a new model. */
  initialValues?: CodexCatalogModel;
  onCancel: () => void;
  onSubmit: (model: CodexCatalogModel) => void | Promise<void>;
}

/**
 * Codex catalog model editor. The catalog only feeds model_catalog_json, so the
 * fields map 1:1 onto CodexCatalogModel; the config.toml model name is edited
 * separately on the provider card.
 */
const CodexModelFormModal: React.FC<CodexModelFormModalProps> = ({
  open,
  isEdit,
  presetNpmType,
  resolvePreset,
  initialValues,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<CodexCatalogModelFormValues>();
  const [submitting, setSubmitting] = React.useState(false);
  const [presetsExpanded, setPresetsExpanded] = React.useState(false);
  const reasoningLevels = Form.useWatch('reasoningLevels', form) as string[] | undefined;
  const presetModelsVersion = React.useSyncExternalStore(
    subscribePresetModels,
    getPresetModelsVersion,
    getPresetModelsVersion,
  );

  const presetModels = React.useMemo(
    () => (presetNpmType ? PRESET_MODELS[presetNpmType] ?? [] : []),
    [presetNpmType, presetModelsVersion],
  );
  const otherPresetModels = React.useMemo(() => {
    const seenIds = new Set<string>();
    return Object.entries(PRESET_MODELS)
      .filter(([npmType]) => npmType !== presetNpmType)
      .flatMap(([, models]) => models)
      .filter((preset) => {
        const id = preset.id?.trim();
        if (!id || seenIds.has(id)) {
          return false;
        }
        seenIds.add(id);
        return true;
      });
  }, [presetNpmType, presetModelsVersion]);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    form.setFieldsValue({
      model: '',
      displayName: undefined,
      contextWindow: undefined,
      reasoningLevels: undefined,
      defaultReasoningLevel: undefined,
      serviceTiers: undefined,
      inputModalities: undefined,
      outputModalities: undefined,
      imageSupport: 'auto',
      ...(initialValues ? toCodexCatalogModelFormValues(initialValues) : {}),
    });
    setPresetsExpanded(false);
  }, [form, initialValues, open]);

  /**
   * A default level is only meaningful while it is one of the selected levels,
   * but the cleanup must run on explicit user edits only. Doing it from an
   * effect races the open-time backfill: the first render still sees an empty
   * `reasoningLevels` watch value and wipes the restored default level.
   */
  const handleReasoningLevelsChange = (levels: string[] | undefined) => {
    const current = form.getFieldValue('defaultReasoningLevel') as string | undefined;
    if (current && !(levels ?? []).includes(current)) {
      form.setFieldValue('defaultReasoningLevel', undefined);
    }
  };

  const handleUpstreamModelBlur = (rawValue: string) => {
    const modelId = rawValue.trim();
    if (!modelId || !resolvePreset) {
      return;
    }
    const preset = resolvePreset(modelId);
    if (!preset) {
      return;
    }
    const current: CodexCatalogModelFormValues = form.getFieldsValue();
    const alreadyCustomized = Boolean(
      current.displayName?.trim()
      || current.contextWindow
      || (current.reasoningLevels && current.reasoningLevels.length > 0)
      || (current.inputModalities && current.inputModalities.length > 0)
      || (current.outputModalities && current.outputModalities.length > 0),
    );
    if (alreadyCustomized) {
      return;
    }
    const filled = fillCodexCatalogModelFromPreset({ model: modelId }, preset);
    form.setFieldsValue({
      displayName: filled.displayName,
      contextWindow: typeof filled.contextWindow === 'number'
        ? filled.contextWindow
        : undefined,
      reasoningLevels: filled.reasoningLevels,
      defaultReasoningLevel: filled.defaultReasoningLevel,
      inputModalities: filled.modalities?.input,
      outputModalities: filled.modalities?.output,
    });
  };

  const applyPreset = (preset: PresetModel) => {
    // Editing keeps the upstream model id; adding adopts the preset id.
    const currentModelId = (form.getFieldValue('model') as string | undefined)?.trim();
    const nextModelId = isEdit && currentModelId ? currentModelId : preset.id;
    const filled = fillCodexCatalogModelFromPreset({ model: nextModelId }, preset);
    form.setFieldsValue({
      model: nextModelId,
      displayName: filled.displayName,
      contextWindow: typeof filled.contextWindow === 'number' ? filled.contextWindow : undefined,
      reasoningLevels: filled.reasoningLevels,
      defaultReasoningLevel: filled.defaultReasoningLevel,
      inputModalities: filled.modalities?.input,
      outputModalities: filled.modalities?.output,
    });
    setPresetsExpanded(false);
  };

  const handleOk = async () => {
    try {
      await form.validateFields();
      // `getFieldsValue(true)` also returns values without a bound Form.Item —
      // the image-input choice is kept in the store but no longer edited here,
      // so an existing explicit supportsImage survives a save.
      const values = form.getFieldsValue(true) as CodexCatalogModelFormValues;
      setSubmitting(true);
      await onSubmit(fromCodexCatalogModelFormValues(values, initialValues));
    } finally {
      setSubmitting(false);
    }
  };


  return (
    <Modal
      open={open}
      title={isEdit ? t('codex.model.editTitle') : t('codex.model.addTitle')}
      onCancel={onCancel}
      onOk={() => void handleOk()}
      confirmLoading={submitting}
      destroyOnHidden
      width={640}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form
        form={form}
        layout={MODEL_FORM_LAYOUT}
        style={{ marginTop: 24 }}
      >
        <Form.Item
          label={t('codex.model.upstreamId')}
          required
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.model.upstreamIdHint')}</Text>}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Form.Item
              name="model"
              noStyle
              rules={[{ required: true, message: t('common.error') }]}
            >
              <Input
                placeholder="gpt-5.5"
                style={{ flex: 1 }}
                onBlur={(event) => handleUpstreamModelBlur(event.target.value)}
              />
            </Form.Item>
            {presetModels.length > 0 && (
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
                onClick={() => setPresetsExpanded((prev) => !prev)}
              >
                {t('codex.model.selectPreset')}
                {presetsExpanded ? ' ▴' : ' ▾'}
              </a>
            )}
          </div>
        </Form.Item>

        {presetsExpanded && presetModels.length > 0 && (
          <Form.Item style={{ marginTop: -8 }}>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, maxHeight: 220, overflowY: 'auto' }}>
              {presetModels.map((preset) => (
                <Tag
                  key={`primary-${preset.id}`}
                  style={{ cursor: 'pointer' }}
                  onClick={() => applyPreset(preset)}
                >
                  {preset.name}
                </Tag>
              ))}
            </div>
            {otherPresetModels.length > 0 && (
              <>
                <Divider style={{ margin: '12px 0', fontSize: 12 }}>
                  {t('codex.model.otherPresets')}
                </Divider>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, maxHeight: 220, overflowY: 'auto' }}>
                  {otherPresetModels.map((preset) => (
                    <Tag
                      key={`other-${preset.id}`}
                      style={{ cursor: 'pointer' }}
                      onClick={() => applyPreset(preset)}
                    >
                      {preset.name}
                    </Tag>
                  ))}
                </div>
              </>
            )}
          </Form.Item>
        )}

        <Form.Item name="displayName" label={t('codex.model.displayName')}>
          <Input placeholder={t('codex.model.displayNamePlaceholder')} />
        </Form.Item>

        <Form.Item name="contextWindow" label={t('codex.model.contextWindow')}>
          <InputNumber
            min={1}
            style={{ width: '100%' }}
            placeholder={t('codex.model.contextWindowPlaceholder')}
          />
        </Form.Item>

        <Form.Item
          name="reasoningLevels"
          label={t('codex.model.reasoningLevels')}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.model.reasoningLevelsHint')}</Text>}
        >
          <Select
            mode="multiple"
            allowClear
            placeholder={t('codex.model.reasoningLevelsPlaceholder')}
            options={CODEX_REASONING_LEVELS.map((value) => ({ value, label: value }))}
            onChange={handleReasoningLevelsChange}
          />
        </Form.Item>

        <Form.Item
          name="defaultReasoningLevel"
          label={t('codex.model.defaultReasoningLevel')}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.model.defaultReasoningLevelHint')}</Text>}
        >
          <Select
            allowClear
            disabled={!reasoningLevels || reasoningLevels.length === 0}
            placeholder={t('codex.model.defaultReasoningLevelAuto')}
            options={(reasoningLevels ?? []).map((value) => ({ value, label: value }))}
          />
        </Form.Item>

        <Form.Item
          name="serviceTiers"
          label={t('codex.model.serviceTiers')}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.model.serviceTiersHint')}</Text>}
        >
          <Select
            mode="multiple"
            allowClear
            placeholder={t('codex.model.serviceTiersPlaceholder')}
            options={CODEX_SERVICE_TIER_OPTIONS}
          />
        </Form.Item>

        <Form.Item
          name="inputModalities"
          label={t('codex.model.inputModalities')}
          extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('codex.model.capabilitiesHint')}</Text>}
        >
          <Select
            mode="multiple"
            allowClear
            placeholder={t('codex.model.inputModalitiesPlaceholder')}
            options={CODEX_MODALITY_OPTIONS}
          />
        </Form.Item>

        <Form.Item name="outputModalities" label={t('codex.model.outputModalities')}>
          <Select
            mode="multiple"
            allowClear
            placeholder={t('codex.model.outputModalitiesPlaceholder')}
            options={CODEX_MODALITY_OPTIONS}
          />
        </Form.Item>

        <Form.Item style={{ marginBottom: 0 }}>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t('codex.model.editModelHint')}
          </Text>
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default CodexModelFormModal;
