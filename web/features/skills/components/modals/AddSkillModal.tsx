import React from 'react';
import { Modal, Tabs, Input, Button, Checkbox, Space, message, Spin, Dropdown, AutoComplete, Select } from 'antd';
import { FolderOutlined, GithubOutlined, PlusOutlined, DeleteOutlined } from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import * as api from '../../services/skillsApi';
import type { ToolOption, GitSkillCandidate, SkillRepo } from '../../types';
import { GitPickModal } from './GitPickModal';
import { formatGitError, isGitError } from '../../utils/gitErrorParser';
import styles from './AddSkillModal.module.less';

interface AddSkillModalProps {
  open: boolean;
  onClose: () => void;
  allTools: ToolOption[];
  onSuccess: () => void;
}

export const AddSkillModal: React.FC<AddSkillModalProps> = ({
  open: isOpen,
  onClose,
  allTools,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = React.useState<'local' | 'git'>('local');
  const [localPath, setLocalPath] = React.useState('');
  const [gitUrl, setGitUrl] = React.useState('');
  const [gitBranch, setGitBranch] = React.useState('');
  const [selectedTools, setSelectedTools] = React.useState<string[]>([]);
  const [loading, setLoading] = React.useState(false);

  // Repos state
  const [repos, setRepos] = React.useState<SkillRepo[]>([]);
  const [preferredTools, setPreferredTools] = React.useState<string[] | null>(null);
  const [selectedRepo, setSelectedRepo] = React.useState<string | undefined>(undefined);

  // Branch options for AutoComplete
  const branchOptions = [
    { value: 'main' },
    { value: 'master' },
  ];

  // Git pick modal state
  const [gitCandidates, setGitCandidates] = React.useState<GitSkillCandidate[]>([]);
  const [showGitPick, setShowGitPick] = React.useState(false);

  // Split tools based on preferred tools setting
  const visibleTools = React.useMemo(() => {
    if (preferredTools && preferredTools.length > 0) {
      // If preferred tools are set, only show those
      return allTools.filter((t) => preferredTools.includes(t.id));
    }
    // Otherwise show installed tools
    return allTools.filter((t) => t.installed);
  }, [allTools, preferredTools]);

  // Hidden tools: everything not in visible list
  const hiddenTools = React.useMemo(() => {
    if (preferredTools && preferredTools.length > 0) {
      // If preferred tools are set, hide everything else
      return allTools.filter((t) => !preferredTools.includes(t.id));
    }
    // Otherwise hide uninstalled tools
    return allTools.filter((t) => !t.installed);
  }, [allTools, preferredTools]);

  // Load repos and preferred tools on open
  React.useEffect(() => {
    if (isOpen) {
      loadRepos();
      loadPreferredTools();
    }
  }, [isOpen]);

  const loadRepos = async () => {
    try {
      await api.initDefaultRepos();
      const data = await api.getSkillRepos();
      setRepos(data);
    } catch (error) {
      console.error('Failed to load repos:', error);
    }
  };

  const loadPreferredTools = async () => {
    try {
      const tools = await api.getPreferredTools();
      setPreferredTools(tools);
    } catch (error) {
      console.error('Failed to load preferred tools:', error);
    }
  };

  // Initialize selected tools: use preferred tools if set, otherwise installed tools
  React.useEffect(() => {
    if (isOpen) {
      if (preferredTools && preferredTools.length > 0) {
        setSelectedTools(preferredTools);
      } else {
        const installed = allTools.filter((t) => t.installed).map((t) => t.id);
        setSelectedTools(installed);
      }
    }
  }, [isOpen, allTools, preferredTools]);

  const handleBrowse = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('skills.selectLocalFolder'),
    });
    if (selected && typeof selected === 'string') {
      setLocalPath(selected);
    }
  };

  const handleToolToggle = (toolId: string) => {
    setSelectedTools((prev) =>
      prev.includes(toolId)
        ? prev.filter((id) => id !== toolId)
        : [...prev, toolId]
    );
  };

  const handleRepoSelect = (value: string) => {
    const repo = repos.find((r) => `${r.owner}/${r.name}` === value);
    if (repo) {
      setGitUrl(`https://github.com/${repo.owner}/${repo.name}`);
      setGitBranch(repo.branch);
      setSelectedRepo(value);
    }
  };

  const handleRemoveRepo = async (owner: string, name: string) => {
    try {
      await api.removeSkillRepo(owner, name);
      await loadRepos();
      message.success(t('common.success'));
    } catch (error) {
      message.error(String(error));
    }
  };

  const parseGitUrl = (url: string): { owner: string; name: string } | null => {
    const match = url.match(/github\.com[/:]([^/]+)\/([^/.]+)/);
    if (match) {
      return { owner: match[1], name: match[2] };
    }
    return null;
  };

  const syncToTools = async (skillId: string, centralPath: string, skillName: string) => {
    for (const toolId of selectedTools) {
      try {
        await api.syncSkillToTool(centralPath, skillId, toolId, skillName);
      } catch (error) {
        const errMsg = String(error);
        if (errMsg.includes('TARGET_EXISTS|')) {
          const match = errMsg.match(/TARGET_EXISTS\|(.+)/);
          const targetPath = match ? match[1] : '';
          const toolLabel = allTools.find((t) => t.id === toolId)?.label || toolId;
          const shouldOverwrite = await confirmTargetOverwrite(skillName, toolLabel, targetPath);
          if (shouldOverwrite) {
            try {
              await api.syncSkillToTool(centralPath, skillId, toolId, skillName, true);
            } catch (retryError) {
              console.error(`Failed to overwrite sync to ${toolId}:`, retryError);
            }
          }
        } else {
          console.error(`Failed to sync to ${toolId}:`, error);
        }
      }
    }
  };

  const confirmTargetOverwrite = (skillName: string, toolLabel: string, targetPath: string): Promise<boolean> => {
    return new Promise((resolve) => {
      Modal.confirm({
        title: t('skills.targetExists.title'),
        content: t('skills.targetExists.message', { skill: skillName, tool: toolLabel, path: targetPath }),
        okText: t('skills.overwrite.confirm'),
        okType: 'danger',
        cancelText: t('skills.overwrite.skip'),
        onOk: () => resolve(true),
        onCancel: () => resolve(false),
      });
    });
  };

  const isSkillExistsError = (errMsg: string) => errMsg.includes('SKILL_EXISTS|');

  const extractSkillName = (errMsg: string) => {
    const match = errMsg.match(/SKILL_EXISTS\|(.+)/);
    return match ? match[1] : '';
  };

  // Helper function to show error messages
  const showError = (errMsg: string) => {
    if (isGitError(errMsg)) {
      // For git errors, show a modal with detailed info
      Modal.error({
        title: t('common.error'),
        content: (
          <div style={{ whiteSpace: 'pre-wrap', maxHeight: '400px', overflow: 'auto' }}>
            {formatGitError(errMsg, t)}
          </div>
        ),
        width: 600,
      });
    } else {
      message.error(errMsg);
    }
  };

  const doLocalInstall = async (overwrite: boolean) => {
    setLoading(true);
    try {
      const result = await api.installLocalSkill(localPath, overwrite);
      if (selectedTools.length > 0) {
        await syncToTools(result.skill_id, result.central_path, result.name);
      }
      message.success(t('skills.status.localSkillCreated'));
      onSuccess();
      resetForm();
    } catch (error) {
      const errMsg = String(error);
      if (!overwrite && isSkillExistsError(errMsg)) {
        const skillName = extractSkillName(errMsg);
        confirmOverwrite(skillName, () => doLocalInstall(true));
      } else {
        showError(errMsg);
      }
    } finally {
      setLoading(false);
    }
  };

  const doGitInstall = async (overwrite: boolean) => {
    setLoading(true);
    try {
      const candidates = await api.listGitSkills(gitUrl, gitBranch || undefined);
      if (candidates.length > 1) {
        setGitCandidates(candidates);
        setShowGitPick(true);
        setLoading(false);
        return;
      }

      const result = await api.installGitSkill(gitUrl, gitBranch || undefined, overwrite);
      if (selectedTools.length > 0) {
        await syncToTools(result.skill_id, result.central_path, result.name);
      }

      // Save repo on success
      const parsed = parseGitUrl(gitUrl);
      if (parsed) {
        await api.addSkillRepo(parsed.owner, parsed.name, gitBranch || 'main');
        await loadRepos();
      }

      message.success(t('skills.status.gitSkillCreated'));
      onSuccess();
      resetForm();
    } catch (error) {
      const errMsg = String(error);
      if (!overwrite && isSkillExistsError(errMsg)) {
        const skillName = extractSkillName(errMsg);
        confirmOverwrite(skillName, () => doGitInstall(true));
      } else if (errMsg.startsWith('MULTI_SKILLS|')) {
        try {
          const candidates = await api.listGitSkills(gitUrl, gitBranch || undefined);
          if (candidates.length > 0) {
            setGitCandidates(candidates);
            setShowGitPick(true);
          } else {
            message.error(t('skills.errors.noSkillsFoundInRepo'));
          }
        } catch (listError) {
          showError(String(listError));
        }
      } else {
        showError(errMsg);
      }
    } finally {
      setLoading(false);
    }
  };

  const confirmOverwrite = (skillName: string, onOk: () => void) => {
    Modal.confirm({
      title: t('skills.overwrite.title'),
      content: t('skills.overwrite.messageWithName', { name: skillName }),
      okText: t('skills.overwrite.confirm'),
      okType: 'danger',
      cancelText: t('common.cancel'),
      onOk,
    });
  };

  const handleLocalInstall = () => {
    if (!localPath.trim()) {
      message.error(t('skills.errors.requireLocalPath'));
      return;
    }
    doLocalInstall(false);
  };

  const handleGitInstall = () => {
    if (!gitUrl.trim()) {
      message.error(t('skills.errors.requireGitUrl'));
      return;
    }
    doGitInstall(false);
  };

  const handleGitPickConfirm = async (selections: { subpath: string }[]) => {
    setShowGitPick(false);
    setLoading(true);

    const skippedNames: string[] = [];
    let overwriteAll = false;

    try {
      for (const sel of selections) {
        try {
          const result = await api.installGitSelection(gitUrl, sel.subpath, gitBranch || undefined);
          if (selectedTools.length > 0) {
            await syncToTools(result.skill_id, result.central_path, result.name);
          }
        } catch (error) {
          const errMsg = String(error);
          if (isSkillExistsError(errMsg)) {
            const skillName = extractSkillName(errMsg);
            if (overwriteAll) {
              const result = await api.installGitSelection(gitUrl, sel.subpath, gitBranch || undefined, true);
              if (selectedTools.length > 0) {
                await syncToTools(result.skill_id, result.central_path, result.name);
              }
            } else {
              const action = await confirmBatchOverwrite(skillName, selections.length > 1);
              if (action === 'overwrite') {
                const result = await api.installGitSelection(gitUrl, sel.subpath, gitBranch || undefined, true);
                if (selectedTools.length > 0) {
                  await syncToTools(result.skill_id, result.central_path, result.name);
                }
              } else if (action === 'overwriteAll') {
                overwriteAll = true;
                const result = await api.installGitSelection(gitUrl, sel.subpath, gitBranch || undefined, true);
                if (selectedTools.length > 0) {
                  await syncToTools(result.skill_id, result.central_path, result.name);
                }
              } else {
                skippedNames.push(skillName);
              }
            }
          } else {
            throw error;
          }
        }
      }

      // Save repo on success
      const parsed = parseGitUrl(gitUrl);
      if (parsed) {
        await api.addSkillRepo(parsed.owner, parsed.name, gitBranch || 'main');
        await loadRepos();
      }

      if (skippedNames.length > 0) {
        message.info(t('skills.status.installWithSkipped', { skipped: skippedNames.join(', ') }));
      } else {
        message.success(t('skills.status.selectedSkillsInstalled'));
      }
      onSuccess();
      resetForm();
    } catch (error) {
      showError(String(error));
    } finally {
      setLoading(false);
    }
  };

  const confirmBatchOverwrite = (skillName: string, hasMore: boolean): Promise<'overwrite' | 'overwriteAll' | 'skip'> => {
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

  const resetForm = () => {
    setLocalPath('');
    setGitUrl('');
    setGitBranch('');
    setGitCandidates([]);
    setSelectedRepo(undefined);
  };

  const handleClose = () => {
    resetForm();
    onClose();
  };

  return (
    <>
      <Modal
        title={t('skills.addSkillTitle')}
        open={isOpen}
        onCancel={handleClose}
        footer={null}
        width={700}
        destroyOnClose
      >
        <Spin spinning={loading}>
          <Tabs
            activeKey={activeTab}
            onChange={(key) => setActiveTab(key as 'local' | 'git')}
            items={[
              {
                key: 'local',
                label: (
                  <span>
                    <FolderOutlined /> {t('skills.localTab')}
                  </span>
                ),
                children: (
                  <div className={styles.tabContent}>
                    <div className={styles.field}>
                      <label>{t('skills.addLocal.pathLabel')}</label>
                      <div className={styles.fieldInput}>
                        <Space.Compact style={{ width: '100%' }}>
                          <Input
                            value={localPath}
                            onChange={(e) => setLocalPath(e.target.value)}
                            placeholder={t('skills.addLocal.pathPlaceholder')}
                          />
                          <Button onClick={handleBrowse}>{t('common.browse')}</Button>
                        </Space.Compact>
                      </div>
                    </div>
                  </div>
                ),
              },
              {
                key: 'git',
                label: (
                  <span>
                    <GithubOutlined /> {t('skills.gitTab')}
                  </span>
                ),
                children: (
                  <div className={styles.tabContent}>
                    <div className={styles.field}>
                      <label>{t('skills.addGit.repoLabel')}</label>
                      <div className={styles.fieldInput}>
                        <div className={styles.repoSelector}>
                          <Select
                            value={selectedRepo}
                            placeholder={t('skills.addGit.selectRepo')}
                            onChange={handleRepoSelect}
                            style={{ flex: 1 }}
                            allowClear
                            onClear={() => setSelectedRepo(undefined)}
                            options={repos.map((repo) => ({
                              value: `${repo.owner}/${repo.name}`,
                              label: `${repo.owner}/${repo.name}`,
                            }))}
                            optionRender={(option) => (
                              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                                <span>{option.label}</span>
                                <Button
                                  type="text"
                                  size="small"
                                  icon={<DeleteOutlined />}
                                  danger
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    const [owner, name] = String(option.value).split('/');
                                    handleRemoveRepo(owner, name);
                                  }}
                                />
                              </div>
                            )}
                            dropdownRender={(menu) => (
                              <>
                                {menu}
                                {repos.length > 0 && (
                                  <div style={{ padding: '8px', borderTop: '1px solid var(--color-border)' }}>
                                    <span style={{ fontSize: '12px', color: 'var(--color-text-tertiary)' }}>
                                      {t('skills.addGit.manageReposHint')}
                                    </span>
                                  </div>
                                )}
                              </>
                            )}
                          />
                        </div>
                      </div>
                    </div>
                    <div className={styles.field}>
                      <label>{t('skills.addGit.urlLabel')}</label>
                      <div className={styles.fieldInput}>
                        <Input
                          value={gitUrl}
                          onChange={(e) => setGitUrl(e.target.value)}
                          placeholder={t('skills.addGit.urlPlaceholder')}
                        />
                      </div>
                    </div>
                    <div className={styles.field}>
                      <label>{t('skills.addGit.branchLabel')}</label>
                      <div className={styles.fieldInput}>
                        <AutoComplete
                          value={gitBranch}
                          onChange={setGitBranch}
                          options={branchOptions}
                          placeholder={t('skills.addGit.branchPlaceholder')}
                          style={{ width: '100%' }}
                        />
                      </div>
                    </div>
                    <div className={styles.gitHints}>
                      <ul>
                        <li>{t('skills.addGit.hintAutoSave')}</li>
                        <li>{t('skills.addGit.hintMultiSkill')}</li>
                        <li>{t('skills.addGit.hintBranch')}</li>
                      </ul>
                    </div>
                  </div>
                ),
              },
            ]}
          />

          <div className={styles.toolsSection}>
            <div className={styles.toolsLabel}>{t('skills.installToTools')}</div>
            <div className={styles.toolsHint}>{t('skills.syncAfterCreate')}</div>
            <div className={styles.toolsGrid}>
              {visibleTools.length > 0 ? (
                visibleTools.map((tool) => (
                  <Checkbox
                    key={tool.id}
                    checked={selectedTools.includes(tool.id)}
                    onChange={() => handleToolToggle(tool.id)}
                  >
                    {tool.label}
                  </Checkbox>
                ))
              ) : (
                <span className={styles.noTools}>{t('skills.noToolsInstalled')}</span>
              )}
              {hiddenTools.length > 0 && (
                <Dropdown
                  trigger={['click']}
                  menu={{
                    items: hiddenTools.map((tool) => ({
                      key: tool.id,
                      label: tool.label,
                      onClick: () => handleToolToggle(tool.id),
                    })),
                  }}
                >
                  <Button type="dashed" size="small" icon={<PlusOutlined />} />
                </Dropdown>
              )}
            </div>
          </div>

          <div className={styles.footer}>
            <Button onClick={handleClose}>{t('common.cancel')}</Button>
            <Button
              type="primary"
              onClick={activeTab === 'local' ? handleLocalInstall : handleGitInstall}
              loading={loading}
            >
              {t('skills.install')}
            </Button>
          </div>
        </Spin>
      </Modal>

      <GitPickModal
        open={showGitPick}
        candidates={gitCandidates}
        onClose={() => setShowGitPick(false)}
        onConfirm={handleGitPickConfirm}
      />
    </>
  );
};
