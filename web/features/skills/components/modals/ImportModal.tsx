import React from 'react';
import { Modal, Checkbox, Button, Empty, message, Spin, Tooltip } from 'antd';
import { WarningOutlined, FolderOpenOutlined } from '@ant-design/icons';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useSkillsStore } from '../../stores/skillsStore';
import * as api from '../../services/skillsApi';
import styles from './ImportModal.module.less';

interface ImportModalProps {
  open: boolean;
  onClose: () => void;
  onSuccess: () => void;
}

export const ImportModal: React.FC<ImportModalProps> = ({
  open,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const { onboardingPlan, loadOnboardingPlan, toolStatus } = useSkillsStore();
  const [selected, setSelected] = React.useState<Set<string>>(new Set());
  const [loading, setLoading] = React.useState(false);
  const [preferredTools, setPreferredTools] = React.useState<string[] | null>(null);

  React.useEffect(() => {
    if (open) {
      loadOnboardingPlan();
      setSelected(new Set());
      // Load preferred tools
      api.getPreferredTools().then(setPreferredTools).catch(console.error);
    }
  }, [open, loadOnboardingPlan]);

  // Get target tools: preferred tools if set, otherwise all installed tools
  const targetTools = React.useMemo(() => {
    if (preferredTools && preferredTools.length > 0) {
      return preferredTools;
    }
    // Fall back to installed tools
    return toolStatus?.installed || [];
  }, [preferredTools, toolStatus]);

  const groups = onboardingPlan?.groups || [];
  const allPaths = React.useMemo(() => {
    const paths: string[] = [];
    groups.forEach((g) => {
      g.variants.forEach((v) => {
        paths.push(v.path);
      });
    });
    return paths;
  }, [groups]);

  const handleToggle = (path: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const handleSelectAll = () => {
    if (selected.size === allPaths.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(allPaths));
    }
  };

  const isSkillExistsError = (errMsg: string) => errMsg.includes('SKILL_EXISTS|');

  const extractSkillName = (errMsg: string) => {
    const match = errMsg.match(/SKILL_EXISTS\|(.+)/);
    return match ? match[1] : '';
  };

  const confirmOverwrite = (skillName: string, hasMore: boolean): Promise<'overwrite' | 'overwriteAll' | 'skip'> => {
    return new Promise((resolve) => {
      const modal = Modal.confirm({
        title: t('skills.overwrite.title'),
        content: t('skills.overwrite.messageWithName', { name: skillName }),
        okText: t('skills.overwrite.confirm'),
        okType: 'danger',
        cancelText: t('skills.overwrite.skip'),
        onOk: () => resolve('overwrite'),
        onCancel: () => resolve('skip'),
        footer: (_, { OkBtn, CancelBtn }) => (
          <>
            <CancelBtn />
            {hasMore && (
              <Button
                danger
                onClick={() => {
                  modal.destroy();
                  resolve('overwriteAll');
                }}
              >
                {t('skills.overwrite.overwriteAll')}
              </Button>
            )}
            <OkBtn />
          </>
        ),
      });
    });
  };

  const syncToTools = async (skillId: string, centralPath: string, skillName: string) => {
    for (const toolId of targetTools) {
      try {
        await api.syncSkillToTool(centralPath, skillId, toolId, skillName);
      } catch (error) {
        const errMsg = String(error);
        if (errMsg.includes('TARGET_EXISTS|')) {
          // Auto-skip for import - the skill is already in that tool
          console.log(`Skipping ${toolId}: target exists (already synced)`);
        } else {
          console.warn(`Failed to sync to ${toolId}:`, error);
        }
      }
    }
  };

  const handleImport = async () => {
    if (selected.size === 0) return;

    setLoading(true);
    const selectedPaths = Array.from(selected);
    const skippedNames: string[] = [];
    let overwriteAll = false;

    try {
      for (let i = 0; i < selectedPaths.length; i++) {
        const path = selectedPaths[i];
        let result;
        try {
          result = await api.importExistingSkill(path);
        } catch (error) {
          const errMsg = String(error);
          if (isSkillExistsError(errMsg)) {
            const skillName = extractSkillName(errMsg);
            if (overwriteAll) {
              result = await api.importExistingSkill(path, true);
            } else {
              const hasMore = i < selectedPaths.length - 1;
              const action = await confirmOverwrite(skillName, hasMore);
              if (action === 'overwrite') {
                result = await api.importExistingSkill(path, true);
              } else if (action === 'overwriteAll') {
                overwriteAll = true;
                result = await api.importExistingSkill(path, true);
              } else {
                skippedNames.push(skillName);
                continue;
              }
            }
          } else {
            throw error;
          }
        }

        // Sync to target tools after successful import
        if (result && targetTools.length > 0) {
          await syncToTools(result.skill_id, result.central_path, result.name);
        }
      }

      if (skippedNames.length > 0) {
        message.info(t('skills.status.installWithSkipped', { skipped: skippedNames.join(', ') }));
      } else {
        message.success(t('skills.status.importCompleted'));
      }
      onSuccess();
    } catch (error) {
      message.error(String(error));
    } finally {
      setLoading(false);
    }
  };

  const handleOpenFolder = async (path: string, e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await revealItemInDir(path);
    } catch (error) {
      message.error(String(error));
    }
  };

  return (
    <Modal
      title={t('skills.importTitle')}
      open={open}
      onCancel={onClose}
      footer={null}
      width={600}
      destroyOnClose
    >
      <Spin spinning={loading}>
        <p className={styles.hint}>{t('skills.importSummary')}</p>

        {onboardingPlan && (
          <div className={styles.stats}>
            <span>{t('skills.toolsScanned', { count: onboardingPlan.total_tools_scanned })}</span>
            <span className={styles.dot}>•</span>
            <span>{t('skills.skillsFound', { count: onboardingPlan.total_skills_found })}</span>
          </div>
        )}

        {groups.length === 0 ? (
          <Empty description={t('skills.discoveredEmpty')} />
        ) : (
          <>
            <div className={styles.selectAll}>
              <Checkbox
                checked={selected.size === allPaths.length}
                indeterminate={selected.size > 0 && selected.size < allPaths.length}
                onChange={handleSelectAll}
              >
                {t('skills.selectAll')}
              </Checkbox>
              <span className={styles.count}>
                {t('skills.selectedCount', {
                  selected: selected.size,
                  total: allPaths.length,
                })}
              </span>
            </div>

            <div className={styles.list}>
              {groups.map((group) => (
                <div key={group.name} className={styles.group}>
                  <div className={styles.groupHeader}>
                    <span className={styles.groupName}>{group.name}</span>
                  </div>
                  {group.variants.map((v) => (
                    <div
                      key={v.path}
                      className={`${styles.variant} ${selected.has(v.path) ? styles.selected : ''}`}
                      onClick={() => handleToggle(v.path)}
                    >
                      <Checkbox checked={selected.has(v.path)} />
                      <div className={styles.variantInfo}>
                        <div className={styles.variantTool}>
                          {v.tool}
                          {v.conflicting_tools && v.conflicting_tools.length > 0 && (
                            <Tooltip title={t('skills.conflictWith', { tools: v.conflicting_tools.join(', ') })}>
                              <span className={styles.conflictBadge}>
                                <WarningOutlined /> {v.conflicting_tools.join(', ')}
                              </span>
                            </Tooltip>
                          )}
                        </div>
                        <div className={styles.variantPath}>
                          <span>
                            {v.is_link
                              ? t('skills.linkLabel', { target: v.link_target || v.path })
                              : v.path}
                          </span>
                          <FolderOpenOutlined
                            className={styles.openFolder}
                            onClick={(e) => handleOpenFolder(v.path, e)}
                          />
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              ))}
            </div>
          </>
        )}

        <div className={styles.footer}>
          <Button onClick={onClose}>{t('common.close')}</Button>
          <Button
            type="primary"
            onClick={handleImport}
            disabled={selected.size === 0}
            loading={loading}
          >
            {t('skills.importAndSync')}
          </Button>
        </div>
      </Spin>
    </Modal>
  );
};
