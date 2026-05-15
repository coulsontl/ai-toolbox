import React from 'react';
import { Form, Input, InputNumber, message, Modal, Button, Empty } from 'antd';
import { PlusOutlined, EditOutlined, DeleteOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import * as api from '../../services/skillsApi';
import type { SkillGroupRecord } from '../../types';
import styles from './SkillGroupsModal.module.less';

interface SkillGroupsModalProps {
  open: boolean;
  groups: SkillGroupRecord[];
  onClose: () => void;
  onSuccess: () => void;
}

interface GroupFormValues {
  name: string;
  note?: string;
  sortIndex?: number;
}

export const SkillGroupsModal: React.FC<SkillGroupsModalProps> = ({
  open,
  groups,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<GroupFormValues>();
  const [editingGroup, setEditingGroup] = React.useState<SkillGroupRecord | null>(null);
  const [saving, setSaving] = React.useState(false);

  const sortedGroups = React.useMemo(
    () =>
      [...groups].sort((a, b) => {
        const sortDiff = a.sort_index - b.sort_index;
        if (sortDiff !== 0) return sortDiff;
        return a.name.localeCompare(b.name);
      }),
    [groups],
  );

  const startEdit = (group?: SkillGroupRecord) => {
    setEditingGroup(group ?? null);
    form.setFieldsValue({
      name: group?.name ?? '',
      note: group?.note ?? '',
      sortIndex: group?.sort_index ?? groups.length,
    });
  };

  React.useEffect(() => {
    if (open) startEdit();
  }, [open]);

  const handleSubmit = async (values: GroupFormValues) => {
    setSaving(true);
    try {
      await api.saveSkillGroup(
        values.name,
        values.note?.trim() || null,
        values.sortIndex ?? editingGroup?.sort_index ?? groups.length,
        editingGroup?.id,
      );
      message.success(t('skills.groups.saveSuccess'));
      startEdit();
      onSuccess();
    } catch (error) {
      message.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = (group: SkillGroupRecord) => {
    Modal.confirm({
      title: t('skills.groups.deleteTitle'),
      content: t('skills.groups.deleteContent', { name: group.name }),
      okText: t('common.delete'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel'),
      onOk: async () => {
        await api.deleteSkillGroup(group.id);
        message.success(t('skills.groups.deleteSuccess'));
        onSuccess();
      },
    });
  };

  return (
    <Modal
      open={open}
      title={t('skills.groups.title')}
      onCancel={onClose}
      footer={null}
      width={980}
      destroyOnHidden
      className={styles.modal}
    >
      <div className={styles.content}>
        <section className={styles.sectionCard}>
          <div className={styles.panelHeader}>
            <div>
              <div className={styles.sectionEyebrow}>{t('skills.groups.listEyebrow')}</div>
              <div className={styles.sectionTitle}>{t('skills.groups.listTitle')}</div>
              <p className={styles.sectionDescription}>{t('skills.groups.listDescription')}</p>
            </div>
            <div className={styles.groupCount}>{t('skills.groups.count', { count: sortedGroups.length })}</div>
          </div>

          {sortedGroups.length === 0 ? (
            <div className={styles.emptyState}>
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={
                  <div className={styles.emptyCopy}>
                    <strong>{t('skills.groups.emptyTitle')}</strong>
                    <span>{t('skills.groups.empty')}</span>
                  </div>
                }
              />
              <Button type="primary" icon={<PlusOutlined />} onClick={() => startEdit()}>
                {t('skills.groups.createFirst')}
              </Button>
            </div>
          ) : (
            <div className={styles.groupList}>
              {sortedGroups.map((group, index) => {
                const isActive = editingGroup?.id === group.id;

                return (
                  <div
                    key={group.id}
                    role="button"
                    tabIndex={0}
                    className={isActive ? `${styles.groupRow} ${styles.groupRowActive}` : styles.groupRow}
                    onClick={() => startEdit(group)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault();
                        startEdit(group);
                      }
                    }}
                  >
                    <div className={styles.groupOrder}>{String(index + 1).padStart(2, '0')}</div>
                    <div className={styles.groupMain}>
                      <div className={styles.groupNameRow}>
                        <strong>{group.name}</strong>
                        <span className={styles.groupMeta}>{t('skills.groups.sortValue', { value: group.sort_index })}</span>
                      </div>
                      <span className={styles.groupNote}>
                        {group.note?.trim() || t('skills.groups.noteEmpty')}
                      </span>
                    </div>
                    <div className={styles.groupActions}>
                      <Button
                        size="small"
                        danger
                        icon={<DeleteOutlined />}
                        onClick={(event) => {
                          event.stopPropagation();
                          handleDelete(group);
                        }}
                      >
                        {t('common.delete')}
                      </Button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </section>

        <section className={styles.sectionCard}>
          <div className={styles.panelHeader}>
            <div>
              <div className={styles.sectionTitle}>
                {editingGroup ? t('skills.groups.editTitle', { name: editingGroup.name }) : t('skills.groups.createTitle')}
              </div>
              <p className={styles.sectionDescription}>
                {editingGroup ? t('skills.groups.editDescription') : t('skills.groups.createDescription')}
              </p>
            </div>
            {editingGroup ? (
              <Button onClick={() => startEdit()}>{t('skills.groups.newAction')}</Button>
            ) : null}
          </div>

          <Form
            form={form}
            layout="horizontal"
            labelCol={{ flex: '108px' }}
            wrapperCol={{ flex: 'auto' }}
            onFinish={handleSubmit}
            className={styles.form}
          >
            <Form.Item label={t('skills.groups.name')} name="name" rules={[{ required: true, message: t('skills.groups.nameRequired') }]}>
              <Input placeholder={t('skills.groups.namePlaceholder')} />
            </Form.Item>

            <Form.Item label={t('skills.groups.note')} name="note">
              <Input.TextArea rows={4} placeholder={t('skills.groups.notePlaceholder')} />
            </Form.Item>

            <Form.Item label={t('skills.groups.sortOrder')} name="sortIndex">
              <InputNumber min={0} precision={0} className={styles.sortInput} placeholder="0" />
            </Form.Item>

            <div className={styles.formActions}>
              <Button onClick={() => startEdit()}>{editingGroup ? t('common.cancel') : t('common.reset')}</Button>
              <Button type="primary" htmlType="submit" loading={saving} icon={editingGroup ? <EditOutlined /> : <PlusOutlined />}>
                {editingGroup ? t('common.save') : t('skills.groups.create')}
              </Button>
            </div>
          </Form>
        </section>
      </div>
    </Modal>
  );
};
