import React from 'react';
import {
  Checkbox,
  Form,
  Input,
  InputNumber,
  Modal,
  Tag,
  Typography,
  type FormInstance,
} from 'antd';
import { useTranslation } from 'react-i18next';
import {
  PRESET_MODELS,
  getPresetModelsVersion,
  subscribePresetModels,
  type PresetModel,
} from '@/constants/presetModels';
import type {
  WorkbuddyFamilyModelInput,
  WorkbuddyFamilyProviderView,
} from '@/types/workbuddyFamily';
import { MODELS_JSON_PRESET_NPM, presetToModelsJsonFields } from './modelsJsonPresetFields';

import {
  MODEL_FORM_CHIP_ROW_STYLE,
  MODEL_FORM_GROUP_LABEL_STYLE,
  MODEL_FORM_LAYOUT,
  MODEL_FORM_TRIPLE_STYLE,
} from '@/features/coding/shared/modelFormLayout';
const { Text } = Typography;

interface Props {
  open: boolean;
  editing: boolean;
  form: FormInstance<WorkbuddyFamilyModelInput>;
  /** The channel the entry belongs to; `null` while adding a brand-new one. */
  channel: WorkbuddyFamilyProviderView | null;
  onCancel: () => void;
  onSubmit: (values: WorkbuddyFamilyModelInput) => Promise<void>;
}

/**
 * One model entry of a `models.json` channel.
 *
 * The endpoint and API key live on every entry (the file has no channel
 * object), so they are duplicated here rather than lifted into a shared
 * header — editing a model is also how a channel's endpoint is corrected.
 *
 * `useCustomProtocol` skips the product's URL normalization, so its hint sits
 * directly under the switch instead of in a page-level note.
 *
 * The 「选择预设模型」 picker beside the id field is SOP §4.2.4's
 * "most important capability, do not omit": a models.dev preset fills the name,
 * both limits and the capability switches in one click instead of leaving the
 * user to look up a dozen values. Two of its rules apply here — the id stays
 * editable only when adding (an edit that changed the id would orphan the row),
 * and selecting a preset while editing updates everything *except* the id.
 */
const ModelsJsonModelModal: React.FC<Props> = ({
  open,
  editing,
  form,
  channel,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [saving, setSaving] = React.useState(false);
  const [presetsExpanded, setPresetsExpanded] = React.useState(false);
  const presetModelsVersion = React.useSyncExternalStore(
    subscribePresetModels,
    getPresetModelsVersion,
    getPresetModelsVersion,
  );

  /**
   * Every preset the catalogue knows, deduped by id.
   *
   * All `models.json` entries are OpenAI-compatible (the file stores a `url`,
   * not a protocol), so that group is listed first — but entries from the other
   * groups stay reachable rather than being hidden, because a `useCustomProtocol`
   * entry may legitimately point at a differently shaped endpoint.
   */
  const presetModels = React.useMemo(() => {
    const seen = new Set<string>();
    const result: PresetModel[] = [];
    const pushUnique = (preset: PresetModel) => {
      const id = preset.id?.trim();
      if (!id || seen.has(id)) {
        return;
      }
      seen.add(id);
      result.push(preset);
    };
    (PRESET_MODELS[MODELS_JSON_PRESET_NPM] || []).forEach(pushUnique);
    Object.entries(PRESET_MODELS).forEach(([npmKey, models]) => {
      if (npmKey === MODELS_JSON_PRESET_NPM) {
        return;
      }
      models.forEach(pushUnique);
    });
    return result;
    // The version bump is the only signal that the catalogue was (re)loaded.
  }, [presetModelsVersion]);

  /** Fills the form from a preset; the id is left alone while editing. */
  const handlePresetSelect = (preset: PresetModel) => {
    form.setFieldsValue({
      ...(editing ? {} : { id: preset.id }),
      ...presetToModelsJsonFields(preset),
    });
  };

  const handleOk = async () => {
    const values = await form.validateFields();
    setSaving(true);
    try {
      await onSubmit(values);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={editing ? t('workbuddy.editModel') : t('workbuddy.addModel')}
      onCancel={onCancel}
      onOk={() => {
        void handleOk();
      }}
      // The cancel button must stay clickable while saving: antd's `loading`
      // on okButtonProps also locks cancel, so the spinner lives in the body.
      okButtonProps={{ disabled: saving }}
      width={720}
      destroyOnHidden
    >
      <Form form={form} layout={MODEL_FORM_LAYOUT}>
        {/*
          `previousId` must be a registered field, not just a value parked on the
          form: `validateFields()` collects only registered names and drops the
          rest, so the rename hint would silently never reach the backend and an
          ID edit became "add a second row".
        */}
        <Form.Item name="previousId" hidden noStyle>
          <Input />
        </Form.Item>
        <Form.Item label={t('workbuddy.model.id')} required>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Form.Item
              name="id"
              noStyle
              rules={[{ required: true, message: t('common.required') }]}
            >
              <Input placeholder="gpt-4.1" disabled={editing} style={{ flex: 1 }} />
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
                onClick={() => setPresetsExpanded(!presetsExpanded)}
              >
                {t('common.model.selectPreset')}
                {presetsExpanded ? ' ▴' : ' ▾'}
              </a>
            )}
          </div>
        </Form.Item>

        {presetsExpanded && presetModels.length > 0 && (
          <Form.Item style={{ marginTop: -8 }}>
            {/* Capped and scrollable: the catalogue is over 100 entries, and
                letting it run the full height pushed every other field below
                the fold — the dialog looked like a preset browser with a form
                attached. */}
            <div
              style={{
                display: 'flex',
                flexWrap: 'wrap',
                gap: 8,
                maxHeight: 168,
                overflowY: 'auto',
                paddingRight: 4,
              }}
            >
              {presetModels.map((preset) => (
                <Tag
                  key={preset.id}
                  style={{ cursor: 'pointer', transition: 'all 0.2s', marginInlineEnd: 0 }}
                  onClick={() => handlePresetSelect(preset)}
                >
                  {preset.name}
                </Tag>
              ))}
            </div>
          </Form.Item>
        )}
        <Form.Item name="name" label={t('workbuddy.model.name')}>
          <Input />
        </Form.Item>
        <Form.Item name="vendor" label={t('workbuddy.model.vendor')}>
          <Input />
        </Form.Item>
        <Form.Item
          name="baseUrl"
          label={t('workbuddy.model.baseUrl')}
          rules={[{ required: true, message: t('common.required') }]}
          extra={channel
            ? t('workbuddy.model.baseUrlChannelHint')
            : undefined}
        >
          <Input placeholder="https://api.example.com/v1" />
        </Form.Item>
        <Form.Item name="apiKey" label={t('workbuddy.model.apiKey')}>
          {/* `Input.Password` ships its own show/hide toggle in antd 6. */}
          <Input.Password />
        </Form.Item>
        {/* One row: three numbers that describe the same thing (how big the
            model is, how hot it runs). Stacked they were three full-width rows
            of six digits each. */}
        <div style={MODEL_FORM_TRIPLE_STYLE}>
          <Form.Item name="maxInputTokens" label={t('workbuddy.model.maxInputTokens')}>
            <InputNumber min={1} style={{ width: '100%' }} />
          </Form.Item>
          <Form.Item name="maxOutputTokens" label={t('workbuddy.model.maxOutputTokens')}>
            <InputNumber min={1} style={{ width: '100%' }} />
          </Form.Item>
          <Form.Item name="temperature" label={t('workbuddy.model.temperature')}>
            <InputNumber min={0} max={2} step={0.1} style={{ width: '100%' }} />
          </Form.Item>
        </div>

        {/* Five booleans, one caption, one wrapping row. Each on its own
            Form.Item spent a label line plus a control line per bit — ten rows
            for five switches. */}
        <Form.Item label={<span style={MODEL_FORM_GROUP_LABEL_STYLE}>{t('workbuddy.model.capabilities')}</span>}>
          <div style={MODEL_FORM_CHIP_ROW_STYLE}>
            <Form.Item name="supportsToolCall" valuePropName="checked" noStyle>
              <Checkbox>{t('workbuddy.model.supportsToolCall')}</Checkbox>
            </Form.Item>
            <Form.Item name="supportsImages" valuePropName="checked" noStyle>
              <Checkbox>{t('workbuddy.model.supportsImages')}</Checkbox>
            </Form.Item>
            <Form.Item name="supportsReasoning" valuePropName="checked" noStyle>
              <Checkbox>{t('workbuddy.model.supportsReasoning')}</Checkbox>
            </Form.Item>
            <Form.Item name="onlyReasoning" valuePropName="checked" noStyle>
              <Checkbox>{t('workbuddy.model.onlyReasoning')}</Checkbox>
            </Form.Item>
            <Form.Item name="useCustomProtocol" valuePropName="checked" noStyle>
              <Checkbox>{t('workbuddy.model.useCustomProtocol')}</Checkbox>
            </Form.Item>
          </div>
        </Form.Item>
        <Form.Item style={{ marginTop: -12 }}>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t('workbuddy.model.useCustomProtocolHint')}
          </Text>
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default ModelsJsonModelModal;
