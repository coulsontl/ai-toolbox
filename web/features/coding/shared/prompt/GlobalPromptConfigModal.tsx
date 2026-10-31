import React from 'react';
import { Alert, Button, Form, Input, Modal } from 'antd';
import { useTranslation } from 'react-i18next';
import { LOCAL_CONFIG_ID } from '../localConfig';
import MarkdownEditor from '@/components/common/MarkdownEditor';
import type { GlobalPromptConfig } from '@/types/globalPrompt';
import styles from './GlobalPromptSettings.module.less';

export interface GlobalPromptConfigFormValues {
  name: string;
  content: string;
}

interface GlobalPromptConfigModalProps {
  open: boolean;
  /** Tool name interpolated into the name placeholder ("Default <tool> assistant"). */
  toolName: string;
  /** Prompt file the content is applied to (e.g. `AGENTS.md`), used in the
   *  content placeholder and the local-file hint. */
  promptFileName: string;
  initialValues?: Partial<GlobalPromptConfig>;
  /**
   * Shows the name but refuses to change it.
   *
   * The identity files are a fixed set whose name *is* the file on disk
   * (`IDENTITY.md`), so renaming one is not an operation that exists. The field
   * stays a real `Form.Item` — `validateFields` only collects registered fields,
   * and the submit path needs the name — it is simply disabled.
   */
  nameReadOnly?: boolean;
  onCancel: () => void;
  onSuccess: (values: GlobalPromptConfigFormValues) => Promise<void> | void;
}

const GlobalPromptConfigModal: React.FC<GlobalPromptConfigModalProps> = ({
  open,
  toolName,
  promptFileName,
  initialValues,
  nameReadOnly = false,
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<GlobalPromptConfigFormValues>();
  const [saving, setSaving] = React.useState(false);

  React.useEffect(() => {
    if (!open) {
      return;
    }

    form.setFieldsValue({
      name: initialValues?.name || '',
      content: initialValues?.content || '',
    });
  }, [form, initialValues, open]);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      setSaving(true);
      await onSuccess(values);
      form.resetFields();
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      title={initialValues?.id ? t('common.prompt.editConfig') : t('common.prompt.addConfig')}
      open={open}
      onCancel={onCancel}
      width={920}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="submit" type="primary" loading={saving} onClick={handleSubmit}>
          {t('common.save')}
        </Button>,
      ]}
    >
      <div className={styles.modalBody}>
        {initialValues?.id === LOCAL_CONFIG_ID && (
          <Alert
            message={t('common.prompt.localConfigHint', { file: promptFileName })}
            type="warning"
            showIcon
            style={{ marginBottom: 16 }}
          />
        )}
        <Form
          form={form}
          layout="horizontal"
          labelCol={{ span: 2 }}
          wrapperCol={{ span: 22 }}
        >
          <Form.Item
            label={t('common.prompt.name')}
            name="name"
            rules={[
              { required: true, message: t('common.prompt.nameRequired') },
              { max: 100, message: t('common.prompt.nameTooLong') },
            ]}
          >
            <Input
              disabled={nameReadOnly}
              placeholder={t('common.prompt.namePlaceholder', { tool: toolName })}
            />
          </Form.Item>
          <Form.Item
            label={t('common.prompt.content')}
            name="content"
            rules={[{ required: true, message: t('common.prompt.contentRequired') }]}
          >
            <MarkdownEditor
              height={320}
              minHeight={220}
              maxHeight={520}
              resizable
              placeholder={t('common.prompt.contentPlaceholder', { file: promptFileName })}
            />
          </Form.Item>
        </Form>
      </div>
    </Modal>
  );
};

export default GlobalPromptConfigModal;
