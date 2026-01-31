import React from 'react';
import { Modal, Checkbox, Button, Empty } from 'antd';
import { useTranslation } from 'react-i18next';
import type { GitSkillCandidate } from '../../types';
import styles from './GitPickModal.module.less';

interface GitPickModalProps {
  open: boolean;
  candidates: GitSkillCandidate[];
  onClose: () => void;
  onConfirm: (selections: { subpath: string }[]) => void;
}

export const GitPickModal: React.FC<GitPickModalProps> = ({
  open,
  candidates,
  onClose,
  onConfirm,
}) => {
  const { t } = useTranslation();
  const [selected, setSelected] = React.useState<Set<string>>(new Set());

  React.useEffect(() => {
    if (open) {
      setSelected(new Set());
    }
  }, [open]);

  const handleToggle = (subpath: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(subpath)) {
        next.delete(subpath);
      } else {
        next.add(subpath);
      }
      return next;
    });
  };

  const handleSelectAll = () => {
    if (selected.size === candidates.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(candidates.map((c) => c.subpath)));
    }
  };

  const handleConfirm = () => {
    const selections = candidates
      .filter((c) => selected.has(c.subpath))
      .map((c) => ({ subpath: c.subpath }));
    onConfirm(selections);
  };

  return (
    <Modal
      title={t('skills.gitPick.title')}
      open={open}
      onCancel={onClose}
      footer={null}
      width={700}
      destroyOnClose
    >
      <p className={styles.hint}>{t('skills.gitPick.description')}</p>

      {candidates.length === 0 ? (
        <Empty description={t('skills.errors.noSkillsFoundInRepo')} />
      ) : (
        <>
          <div className={styles.selectAll}>
            <Checkbox
              checked={selected.size === candidates.length}
              indeterminate={selected.size > 0 && selected.size < candidates.length}
              onChange={handleSelectAll}
            >
              {t('skills.selectAll')}
            </Checkbox>
            <span className={styles.count}>
              {t('skills.selectedCount', {
                selected: selected.size,
                total: candidates.length,
              })}
            </span>
          </div>

          <div className={styles.list}>
            {candidates.map((c) => (
              <div
                key={c.subpath}
                className={`${styles.item} ${selected.has(c.subpath) ? styles.selected : ''}`}
                onClick={() => handleToggle(c.subpath)}
              >
                <Checkbox checked={selected.has(c.subpath)} />
                <div className={styles.info}>
                  <div className={styles.name}>{c.name}</div>
                  {c.description && (
                    <div className={styles.description}>{c.description}</div>
                  )}
                  <div className={styles.path}>{c.subpath}</div>
                </div>
              </div>
            ))}
          </div>
        </>
      )}

      <div className={styles.footer}>
        <Button onClick={onClose}>{t('common.cancel')}</Button>
        <Button
          type="primary"
          onClick={handleConfirm}
          disabled={selected.size === 0}
        >
          {t('skills.installSelected')}
        </Button>
      </div>
    </Modal>
  );
};
