import React from 'react';
import { AutoComplete, Form, Input, message, Modal } from 'antd';
import { FileTextOutlined, TagsOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import * as api from '../../services/skillsApi';
import type { ManagedSkill } from '../../types';
import { findSkillGroupOptionByName, normalizeSkillMetadataText } from '../../utils/skillGrouping';
import styles from './SkillMetadataModal.module.less';

interface SkillMetadataModalProps {
  open: boolean;
  skill: ManagedSkill | null;
  groupOptions: Array<{ id: string; name: string }>;
  onClose: () => void;
  onSuccess: () => void;
}

interface SkillMetadataFormValues {
  userGroup?: string;
  userNote?: string;
}

export const SkillMetadataModal: React.FC<SkillMetadataModalProps> = ({
  open,
  skill,
  groupOptions,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<SkillMetadataFormValues>();
  const [saving, setSaving] = React.useState(false);
  const currentGroup = normalizeSkillMetadataText(skill?.user_group);

  React.useEffect(() => {
    if (!open || !skill) {
      return;
    }

    form.setFieldsValue({
      userGroup: skill.user_group ?? '',
      userNote: skill.user_note ?? '',
    });
  }, [form, open, skill]);

  const handleSubmit = async (values: SkillMetadataFormValues) => {
    if (!skill) {
      return;
    }

    const groupName = normalizeSkillMetadataText(values.userGroup);
    setSaving(true);
    try {
      const groupId = groupName
        ? findSkillGroupOptionByName(groupOptions, groupName)?.id
          ?? await api.saveSkillGroup(groupName, null, groupOptions.length)
        : null;
      await api.updateSkillMetadata(
        skill.id,
        groupId,
        normalizeSkillMetadataText(values.userNote),
      );
      message.success(t('skills.metadata.saveSuccess'));
      onSuccess();
    } catch (error) {
      message.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  if (!skill) {
    return null;
  }

  return (
    <Modal
      open={open}
      title={t('skills.metadata.title')}
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
            <div className={styles.summaryLabel}>{t('skills.metadata.skillLabel')}</div>
            <div className={styles.skillName}>{skill.name}</div>
          </div>
          <span className={`${styles.groupPreview}${currentGroup ? '' : ` ${styles.emptyGroup}`}`}>
            {currentGroup ?? t('skills.groupUngrouped')}
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
                  {t('skills.metadata.group')}
                </span>
              )}
              name="userGroup"
            >
              <AutoComplete
                allowClear
                autoFocus
                options={groupOptions.map((group) => ({ value: group.name }))}
                placeholder={t('skills.metadata.groupPlaceholder')}
                filterOption={(inputValue, option) =>
                  String(option?.value ?? '').toLowerCase().includes(inputValue.toLowerCase())}
              />
            </Form.Item>
            <Form.Item
              label={(
                <span className={styles.fieldLabel}>
                  <FileTextOutlined />
                  {t('skills.metadata.note')}
                </span>
              )}
              name="userNote"
            >
              <Input.TextArea
                rows={4}
                placeholder={t('skills.metadata.notePlaceholder')}
                autoSize={{ minRows: 4, maxRows: 8 }}
              />
            </Form.Item>
          </section>
        </Form>
      </div>
    </Modal>
  );
};
