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
import { DownOutlined, QuestionCircleOutlined, RightOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import type { OmpRuntimeProviderView } from '@/types/ohMyPi';
import {
  OMP_API_DESCRIPTION_I18N_KEYS,
  OMP_API_OPTIONS,
} from '../utils/ompApiOptions';
import { asRecord, isRecordEmpty } from '../utils/ompProviderConfig';
import {
  resolveOmpAutomaticBaseUrlChange,
  resolveOmpProviderFormInitialState,
  type OmpProviderFormValues,
} from '../utils/ompProviderForm';

const { Text } = Typography;

/** The form's own fields; the three JSON blocks live outside `Form`. */
export type { OmpProviderFormValues };

export interface OmpProviderDraft {
  values: OmpProviderFormValues;
  headers: Record<string, unknown>;
  compat: Record<string, unknown>;
  modelOverrides: Record<string, unknown>;
}

interface OmpProviderFormModalProps {
  open: boolean;
  /** The provider being edited; absent while adding (or copying). */
  provider?: OmpRuntimeProviderView;
  /** Pre-fills from the source provider, saves as a new key. */
  copy?: boolean;
  saving?: boolean;
  onCancel: () => void;
  onSubmit: (draft: OmpProviderDraft) => void;
}

// Same label/wrapper split as every other provider form (kimi / openclaw /
// codex): labels sit in a left column rather than above the field.
const labelColFor = (language: string) => ({ span: language === 'zh-CN' ? 4 : 6 });
const wrapperCol = { span: 20 };

/**
 * The Oh My Pi provider editor, in the shared horizontal form shape.
 *
 * OMP has no separate credential file: the API key is a key on the provider in
 * `models.yml`, so there is no credential JSON editor here (unlike Pi). The
 * three advanced JSON blocks are the ones the round-trip preserves verbatim.
 *
 * Changing `api` fills in that protocol's default endpoint **only while the
 * user has not typed one**: the base URL may be auto-filled once, but once it
 * differs from the last automatic value it is the user's, and switching the
 * protocol must not overwrite it.
 */
const OmpProviderFormModal: React.FC<OmpProviderFormModalProps> = ({
  open,
  provider,
  copy = false,
  saving = false,
  onCancel,
  onSubmit,
}) => {
  const { t, i18n } = useTranslation();
  const [form] = Form.useForm<OmpProviderFormValues>();
  const [headersJson, setHeadersJson] = React.useState<Record<string, unknown>>({});
  const [compatJson, setCompatJson] = React.useState<Record<string, unknown>>({});
  const [modelOverridesJson, setModelOverridesJson] = React.useState<Record<string, unknown>>({});
  const [headersValid, setHeadersValid] = React.useState(true);
  const [compatValid, setCompatValid] = React.useState(true);
  const [modelOverridesValid, setModelOverridesValid] = React.useState(true);
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);
  const automaticBaseUrlRef = React.useRef<string | undefined>(undefined);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    const initialState = resolveOmpProviderFormInitialState(provider, copy);
    automaticBaseUrlRef.current = initialState.automaticBaseUrl;

    setHeadersJson(initialState.jsonBlocks.headers);
    setCompatJson(initialState.jsonBlocks.compat);
    setModelOverridesJson(initialState.jsonBlocks.modelOverrides);
    setHeadersValid(true);
    setCompatValid(true);
    setModelOverridesValid(true);
    setAdvancedExpanded(false);
    form.setFieldsValue(initialState.values);
  }, [open, provider, copy, form]);

  const handleApiChange = (apiValue?: string) => {
    const change = resolveOmpAutomaticBaseUrlChange(
      apiValue,
      form.getFieldValue('baseUrl'),
      automaticBaseUrlRef.current,
    );
    automaticBaseUrlRef.current = change.automaticBaseUrl;
    if (change.baseUrl !== undefined) {
      form.setFieldValue('baseUrl', change.baseUrl);
    }
  };

  const handleSubmit = async () => {
    if (!headersValid || !compatValid || !modelOverridesValid) {
      return;
    }
    let values: OmpProviderFormValues;
    try {
      values = await form.validateFields();
    } catch {
      // antd renders the field errors itself.
      return;
    }
    onSubmit({
      values,
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
        ? t('ohMyPi.provider.editSupplierTitle', { name: provider.displayName })
        : t('ohMyPi.provider.addSupplierTitle')}
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
          label={t('ohMyPi.provider.providerKey')}
          name="providerKey"
          rules={[{ required: true, message: t('ohMyPi.provider.providerKeyRequired') }]}
        >
          <Input
            disabled={Boolean(provider) && !copy}
            placeholder={t('ohMyPi.provider.providerKeyPlaceholder')}
          />
        </Form.Item>
        <Form.Item label={t('ohMyPi.provider.displayName')} name="displayName">
          <Input placeholder={t('ohMyPi.provider.displayNamePlaceholder')} />
        </Form.Item>
        <Form.Item
          label={t('ohMyPi.provider.apiType')}
          name="api"
          extra={(
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t('ohMyPi.provider.apiTypePlaceholder')}
            </Text>
          )}
        >
          <Select
            allowClear
            showSearch
            options={OMP_API_OPTIONS.map((option) => ({
              ...option,
              title: t(OMP_API_DESCRIPTION_I18N_KEYS[option.value]),
            }))}
            placeholder={t('ohMyPi.provider.apiTypePlaceholder')}
            onChange={handleApiChange}
            optionRender={(option) => (
              <div style={{ display: 'flex', flexDirection: 'column' }}>
                <span>{option.label}</span>
                {/* `whiteSpace: 'normal'` is load-bearing, not decoration: antd
                    gives every option row `text-ellipsis`, i.e. `nowrap` +
                    overflow hidden, so a 40+-character protocol description
                    would render as one cut-off line. The old inline page had
                    this rule on `.apiOptionDescription`; moving the form into
                    this modal dropped it (found 2026-10-22). */}
                <span
                  style={{
                    fontSize: 10,
                    lineHeight: '14px',
                    color: 'var(--color-text-tertiary)',
                    whiteSpace: 'normal',
                  }}
                >
                  {t(OMP_API_DESCRIPTION_I18N_KEYS[option.value as keyof typeof OMP_API_DESCRIPTION_I18N_KEYS])}
                </span>
              </div>
            )}
          />
        </Form.Item>
        <Form.Item label={t('ohMyPi.provider.baseUrl')} name="baseUrl">
          <Input
            placeholder="https://api.example.com/v1"
            onChange={() => {
              // Touching the endpoint makes it the user's: the next protocol
              // switch must not replace it.
              automaticBaseUrlRef.current = undefined;
            }}
          />
        </Form.Item>
        <Form.Item label={t('ohMyPi.provider.providerApiKey')} name="providerApiKey">
          <Input.Password autoComplete="off" />
        </Form.Item>
        <Form.Item
          label={(
            <Space size={4}>
              <span>{t('ohMyPi.provider.authHeader')}</span>
              <Tooltip title={t('ohMyPi.provider.authHeaderHint')}>
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
              <Text type="secondary">{t('ohMyPi.provider.headersJson')}</Text>
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
              <Text type="secondary">{t('ohMyPi.provider.compatJson')}</Text>
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
              <Text type="secondary">{t('ohMyPi.provider.modelOverridesJson')}</Text>
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

export default OmpProviderFormModal;
