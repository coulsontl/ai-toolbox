import React from 'react';
import {
  Button,
  Form,
  Input,
  Modal,
  Select,
  Space,
  Switch,
  Tooltip,
  Typography,
} from 'antd';
import {
  DownOutlined,
  QuestionCircleOutlined,
  RightOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import type { PiRuntimeProviderView } from '@/types/pi';
import { PI_API_OPTIONS } from '../utils/piApiOptions';
import { asRecord, getStringField } from '../utils/providerModels';
import { createDefaultProviderConfig, isRecordEmpty } from '../utils/providerConfig';

const { Text } = Typography;

/** The form's own fields; the four JSON blocks live outside `Form`. */
export interface PiProviderFormValues {
  providerKey: string;
  displayName?: string;
  api?: string;
  baseUrl?: string;
  providerApiKey?: string;
  authHeader?: boolean;
}

export interface PiProviderDraft {
  values: PiProviderFormValues;
  credential: Record<string, unknown>;
  headers: Record<string, unknown>;
  compat: Record<string, unknown>;
  modelOverrides: Record<string, unknown>;
}

interface PiProviderFormModalProps {
  open: boolean;
  /** The provider being edited; absent while adding (or copying). */
  provider?: PiRuntimeProviderView;
  /** Pre-fills from the source provider, saves as a new key. */
  copy?: boolean;
  saving?: boolean;
  onCancel: () => void;
  onSubmit: (draft: PiProviderDraft) => void;
}

// Same label/wrapper split as every other provider form (kimi / openclaw /
// codex): labels sit in a left column rather than above the field.
const labelColFor = (language: string) => ({ span: language === 'zh-CN' ? 4 : 6 });
const wrapperCol = { span: 20 };

/**
 * The Pi provider editor, in the shared horizontal form shape (SOP §4.0.2-A).
 *
 * Pi splits one provider across two files: the channel (`models.json`, edited
 * through the form fields) and the credential (`auth.json`, edited as free-form
 * JSON). The credential block therefore stays a JSON editor instead of becoming
 * an API-key field — it also carries OAuth objects and `$ENV` templates that a
 * single input cannot express.
 *
 * The four advanced JSON blocks overwrite wholesale when non-empty and delete
 * their key when emptied. That is how `models.json` keeps its unknown fields:
 * this modal never rewrites what it does not own.
 */
const PiProviderFormModal: React.FC<PiProviderFormModalProps> = ({
  open,
  provider,
  copy = false,
  saving = false,
  onCancel,
  onSubmit,
}) => {
  const { t, i18n } = useTranslation();
  const [form] = Form.useForm<PiProviderFormValues>();
  const [credentialJson, setCredentialJson] = React.useState<Record<string, unknown>>({});
  const [headersJson, setHeadersJson] = React.useState<Record<string, unknown>>({});
  const [compatJson, setCompatJson] = React.useState<Record<string, unknown>>({});
  const [modelOverridesJson, setModelOverridesJson] = React.useState<Record<string, unknown>>({});
  const [credentialValid, setCredentialValid] = React.useState(true);
  const [headersValid, setHeadersValid] = React.useState(true);
  const [compatValid, setCompatValid] = React.useState(true);
  const [modelOverridesValid, setModelOverridesValid] = React.useState(true);
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    const isCopy = copy && Boolean(provider);
    const isExistingProviderEdit = Boolean(provider) && !isCopy;
    const providerConfig = provider?.modelsProvider
      ? asRecord(provider.modelsProvider)
      : isExistingProviderEdit
        ? {}
        : createDefaultProviderConfig();

    setCredentialJson(provider?.credential ? asRecord(provider.credential) : {});
    setHeadersJson(asRecord(providerConfig.headers));
    setCompatJson(asRecord(providerConfig.compat));
    setModelOverridesJson(asRecord(providerConfig.modelOverrides));
    setCredentialValid(true);
    setHeadersValid(true);
    setCompatValid(true);
    setModelOverridesValid(true);
    setAdvancedExpanded(false);
    form.setFieldsValue({
      providerKey: isCopy && provider ? `${provider.providerKey}_copy` : provider?.providerKey,
      displayName: getStringField(providerConfig, 'name'),
      api: getStringField(providerConfig, 'api') || undefined,
      baseUrl: getStringField(providerConfig, 'baseUrl'),
      providerApiKey: getStringField(providerConfig, 'apiKey'),
      authHeader: typeof providerConfig.authHeader === 'boolean'
        ? providerConfig.authHeader
        : undefined,
    });
  }, [open, provider, copy, form]);

  const handleSubmit = async () => {
    if (!credentialValid || !headersValid || !compatValid || !modelOverridesValid) {
      return;
    }
    let values: PiProviderFormValues;
    try {
      values = await form.validateFields();
    } catch {
      // antd renders the field errors itself.
      return;
    }
    onSubmit({
      values,
      credential: { ...credentialJson },
      headers: { ...headersJson },
      compat: { ...compatJson },
      modelOverrides: { ...modelOverridesJson },
    });
  };

  return (
    <Modal
      // A copy saves as a new key, so it is an "add" form, not an edit of the
      // source (`isEdit` has the same shape everywhere, see OpenClaw).
      title={provider && !copy
        ? t('pi.provider.editSupplierTitle', { name: provider.displayName })
        : t('pi.provider.addSupplierTitle')}
      open={open}
      width={860}
      confirmLoading={saving}
      onCancel={onCancel}
      onOk={() => void handleSubmit()}
      destroyOnHidden
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={labelColFor(i18n.language)}
        wrapperCol={wrapperCol}
        style={{ marginTop: 24 }}
      >
        <Form.Item
          label={t('pi.provider.providerKey')}
          name="providerKey"
          rules={[{ required: true, message: t('pi.provider.providerKeyRequired') }]}
        >
          <Input
            disabled={Boolean(provider) && !copy}
            placeholder={t('pi.provider.providerKeyPlaceholder')}
          />
        </Form.Item>
        <Form.Item label={t('pi.provider.displayName')} name="displayName">
          <Input placeholder={t('pi.provider.displayNamePlaceholder')} />
        </Form.Item>
        <Form.Item label={t('pi.provider.apiType')} name="api">
          <Select
            allowClear
            showSearch
            options={PI_API_OPTIONS}
            placeholder={t('pi.provider.apiTypePlaceholder')}
          />
        </Form.Item>
        <Form.Item label={t('pi.provider.baseUrl')} name="baseUrl">
          <Input placeholder="https://api.example.com/v1" />
        </Form.Item>
        <Form.Item label={t('pi.provider.providerApiKey')} name="providerApiKey">
          <Input.Password autoComplete="off" />
        </Form.Item>
        <Form.Item
          label={(
            <Space size={4}>
              <span>{t('pi.provider.authHeader')}</span>
              <Tooltip title={t('pi.provider.authHeaderHint')}>
                <QuestionCircleOutlined style={{ color: 'var(--color-text-tertiary)' }} />
              </Tooltip>
            </Space>
          )}
          name="authHeader"
          valuePropName="checked"
        >
          <Switch />
        </Form.Item>

        <div style={{ marginBottom: advancedExpanded ? 16 : 0 }}>
          <Button
            type="link"
            onClick={() => setAdvancedExpanded((value) => !value)}
            style={{ padding: 0, height: 'auto' }}
          >
            {advancedExpanded ? <DownOutlined /> : <RightOutlined />}
            <span style={{ marginLeft: 4 }}>{t('common.advancedSettings')}</span>
          </Button>
        </div>

        {advancedExpanded && (
          <>
            <div style={{ marginBottom: 16 }}>
              <Text type="secondary">{t('pi.provider.credentialAdvancedJson')}</Text>
              <JsonEditor
                value={isRecordEmpty(credentialJson) ? undefined : credentialJson}
                height={180}
                onChange={(value, isValid) => {
                  if (isValid) {
                    setCredentialJson(asRecord(value));
                  }
                  setCredentialValid(isValid);
                }}
              />
            </div>
            <div style={{ marginBottom: 16 }}>
              <Text type="secondary">{t('pi.provider.headersJson')}</Text>
              <JsonEditor
                value={isRecordEmpty(headersJson) ? undefined : headersJson}
                height={160}
                onChange={(value, isValid) => {
                  if (isValid) {
                    setHeadersJson(asRecord(value));
                  }
                  setHeadersValid(isValid);
                }}
              />
            </div>
            <div style={{ marginBottom: 16 }}>
              <Text type="secondary">{t('pi.provider.compatJson')}</Text>
              <JsonEditor
                value={isRecordEmpty(compatJson) ? undefined : compatJson}
                height={180}
                onChange={(value, isValid) => {
                  if (isValid) {
                    setCompatJson(asRecord(value));
                  }
                  setCompatValid(isValid);
                }}
              />
            </div>
            <div style={{ marginBottom: 16 }}>
              <Text type="secondary">{t('pi.provider.modelOverridesJson')}</Text>
              <JsonEditor
                value={isRecordEmpty(modelOverridesJson) ? undefined : modelOverridesJson}
                height={200}
                onChange={(value, isValid) => {
                  if (isValid) {
                    setModelOverridesJson(asRecord(value));
                  }
                  setModelOverridesValid(isValid);
                }}
              />
            </div>
          </>
        )}
      </Form>
    </Modal>
  );
};

export default PiProviderFormModal;
