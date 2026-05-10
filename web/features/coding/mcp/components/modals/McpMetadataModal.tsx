import React from 'react';
import { AutoComplete, Form, Input, message, Modal } from 'antd';
import { FileTextOutlined, TagsOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import * as api from '../../services/mcpApi';
import type { McpServer } from '../../types';
import { normalizeMcpMetadataText } from '../../utils/mcpGrouping';
import styles from './McpMetadataModal.module.less';

interface McpMetadataModalProps {
  open: boolean;
  server: McpServer | null;
  groupOptions: string[];
  onClose: () => void;
  onSuccess: () => void;
}

interface McpMetadataFormValues {
  userGroup?: string;
  userNote?: string;
}

export const McpMetadataModal: React.FC<McpMetadataModalProps> = ({
  open,
  server,
  groupOptions,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<McpMetadataFormValues>();
  const [saving, setSaving] = React.useState(false);
  const currentGroup = normalizeMcpMetadataText(server?.user_group);

  React.useEffect(() => {
    if (!open || !server) {
      return;
    }

    form.setFieldsValue({
      userGroup: server.user_group ?? '',
      userNote: server.user_note ?? '',
    });
  }, [form, open, server]);

  const handleSubmit = async (values: McpMetadataFormValues) => {
    if (!server) {
      return;
    }

    setSaving(true);
    try {
      await api.updateMcpMetadata(
        server.id,
        normalizeMcpMetadataText(values.userGroup),
        normalizeMcpMetadataText(values.userNote),
      );
      message.success(t('mcp.metadata.saveSuccess'));
      onSuccess();
    } catch (error) {
      message.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  if (!server) {
    return null;
  }

  return (
    <Modal
      open={open}
      title={t('mcp.metadata.title')}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      confirmLoading={saving}
      onCancel={onClose}
      onOk={() => form.submit()}
      destroyOnHidden
      className={styles.modal}
    >
      <div className={styles.content}>
        <section className={styles.summaryBand}>
          <div className={styles.summaryIcon}>
            <TagsOutlined />
          </div>
          <div className={styles.summaryMain}>
            <div className={styles.summaryLabel}>{t('mcp.metadata.serverLabel')}</div>
            <div className={styles.serverName}>{server.name}</div>
          </div>
          <span className={`${styles.groupPreview}${currentGroup ? '' : ` ${styles.emptyGroup}`}`}>
            {currentGroup ?? t('mcp.groupUngrouped')}
          </span>
        </section>
        <Form
          form={form}
          layout="horizontal"
          labelCol={{ span: 5 }}
          wrapperCol={{ span: 19 }}
          onFinish={handleSubmit}
          className={styles.form}
        >
          <section className={styles.sectionCard}>
            <Form.Item
              label={(
                <span className={styles.fieldLabel}>
                  <TagsOutlined />
                  {t('mcp.metadata.group')}
                </span>
              )}
              name="userGroup"
            >
              <AutoComplete
                allowClear
                autoFocus
                options={groupOptions.map((group) => ({ value: group }))}
                placeholder={t('mcp.metadata.groupPlaceholder')}
                filterOption={(inputValue, option) =>
                  String(option?.value ?? '').toLowerCase().includes(inputValue.toLowerCase())}
              />
            </Form.Item>
            <Form.Item
              label={(
                <span className={styles.fieldLabel}>
                  <FileTextOutlined />
                  {t('mcp.metadata.note')}
                </span>
              )}
              name="userNote"
            >
              <Input.TextArea
                rows={4}
                placeholder={t('mcp.metadata.notePlaceholder')}
                autoSize={{ minRows: 4, maxRows: 8 }}
              />
            </Form.Item>
          </section>
        </Form>
      </div>
    </Modal>
  );
};
