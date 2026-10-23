import React from 'react';
import {
  Button,
  Form,
  Input,
  Modal,
  Select,
  Typography,
} from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import type { DshRuntimeProviderView } from '@/types/dsh';
import { DSH_API_OPTIONS } from '../utils/apiOptions';
import { asRecord, getStringField } from '../utils/providerModels';
import {
  createDefaultProviderConfig,
  isCatalogOwnedRoute,
  isRecordEmpty,
} from '../utils/providerConfig';

const { Text } = Typography;

/** The form's own fields; the JSON blocks live outside `Form`. */
export interface DshProviderFormValues {
  providerKey: string;
  displayName?: string;
  api?: string;
  baseUrl?: string;
  providerApiKey?: string;
}

export interface DshProviderDraft {
  values: DshProviderFormValues;
  modelOverrides: Record<string, unknown>;
}

interface DshProviderFormModalProps {
  open: boolean;
  /** The provider being edited; absent while adding (or copying). */
  provider?: DshRuntimeProviderView;
  /** Pre-fills from the source provider, saves as a new key. */
  copy?: boolean;
  saving?: boolean;
  onCancel: () => void;
  onSubmit: (draft: DshProviderDraft) => void;
}

// Same label/wrapper split as every other provider form (kimi / openclaw /
// codex): labels sit in a left column rather than above the field.
const labelColFor = (language: string) => ({ span: language === 'zh-CN' ? 4 : 6 });
const wrapperCol = { span: 20 };

/**
 * The dsh provider editor, in the shared horizontal form shape.
 *
 * A built-in / catalog-backed route keeps its display name and wire protocol
 * from `llm-pi-ai`'s bundled catalog (official behavior), so editing one hides
 * those two fields instead of letting the page write values the engine ignores.
 *
 * The API key is not part of the provider entry: dsh stores it in
 * `.credentials.yaml` under a `REF` name, and the page derives that ref. The
 * input here is therefore "set the secret", not "edit a config field".
 */
const DshProviderFormModal: React.FC<DshProviderFormModalProps> = ({
  open,
  provider,
  copy = false,
  saving = false,
  onCancel,
  onSubmit,
}) => {
  const { t, i18n } = useTranslation();
  const [form] = Form.useForm<DshProviderFormValues>();
  const [modelOverridesJson, setModelOverridesJson] = React.useState<Record<string, unknown>>({});
  const [modelOverridesValid, setModelOverridesValid] = React.useState(true);
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);

  // Catalog-backed routes get their name/protocol from the adapter catalog, so
  // editing them must not offer fields that would be dropped on save.
  const isBuiltInChannel = isCatalogOwnedRoute(provider, copy);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    const isCopy = copy && Boolean(provider);
    const isExistingProviderEdit = Boolean(provider) && !isCopy;
    const providerConfig = provider?.provider
      ? asRecord(provider.provider)
      : isExistingProviderEdit
        ? {}
        : createDefaultProviderConfig();

    setModelOverridesJson(asRecord(providerConfig.modelOverrides));
    setModelOverridesValid(true);
    setAdvancedExpanded(false);
    form.setFieldsValue({
      providerKey: isCopy && provider ? `${provider.providerKey}_copy` : provider?.providerKey,
      displayName: getStringField(providerConfig, 'displayName'),
      api: getStringField(providerConfig, 'api') || undefined,
      baseUrl: getStringField(providerConfig, 'baseURL'),
      providerApiKey: provider?.apiKey || '',
    });
  }, [open, provider, copy, form]);

  const handleSubmit = async () => {
    if (!modelOverridesValid) {
      return;
    }
    let values: DshProviderFormValues;
    try {
      values = await form.validateFields();
    } catch {
      // antd renders the field errors itself.
      return;
    }
    onSubmit({ values, modelOverrides: { ...modelOverridesJson } });
  };

  return (
    <Modal
      // A copy saves as a new key, so it is an "add" form, not an edit of the
      // source (`isEdit` has the same shape everywhere, see OpenClaw).
      title={provider && !copy
        ? t('dsh.provider.editSupplierTitle', { defaultValue: '编辑供应商', name: provider.displayName })
        : t('dsh.provider.addSupplierTitle', { defaultValue: '新增供应商' })}
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
          label={t('dsh.provider.providerKey', { defaultValue: '供应商 Key' })}
          name="providerKey"
          rules={[{
            required: true,
            message: t('dsh.provider.providerKeyRequired', { defaultValue: '请输入供应商 Key' }),
          }]}
        >
          <Input
            disabled={Boolean(provider) && !copy}
            placeholder={t('dsh.provider.providerKeyPlaceholder', { defaultValue: '如 deepseek' })}
          />
        </Form.Item>
        {!isBuiltInChannel && (
          <Form.Item
            label={t('dsh.provider.displayName', { defaultValue: '显示名称' })}
            name="displayName"
          >
            <Input
              placeholder={t('dsh.provider.displayNamePlaceholder', { defaultValue: '供应商显示名称' })}
            />
          </Form.Item>
        )}
        {!isBuiltInChannel && (
          <Form.Item
            label={t('dsh.provider.apiType', { defaultValue: 'API 类型' })}
            name="api"
            extra={(
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t('dsh.provider.configSection', { defaultValue: '连接配置' })}
              </Text>
            )}
          >
            <Select
              allowClear
              showSearch
              options={DSH_API_OPTIONS}
              placeholder={t('dsh.provider.apiTypePlaceholder', { defaultValue: '选择 API 类型' })}
            />
          </Form.Item>
        )}
        <Form.Item label={t('dsh.provider.baseUrl', { defaultValue: 'Base URL' })} name="baseUrl">
          <Input placeholder="https://api.deepseek.com/v1" />
        </Form.Item>
        <Form.Item
          label={t('dsh.provider.providerApiKey', { defaultValue: 'API Key' })}
          name="providerApiKey"
        >
          <Input.Password
            autoComplete="off"
            placeholder={t('dsh.provider.providerApiKeyPlaceholder', {
              defaultValue: '输入 API Key 保存到凭证文件',
            })}
          />
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
          <div style={{ marginBottom: 16 }}>
            <Text type="secondary">{t('dsh.provider.modelOverridesJson', {
              defaultValue: '模型覆盖（modelOverrides）',
            })}</Text>
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
        )}
      </Form>
    </Modal>
  );
};

export default DshProviderFormModal;
