import React from 'react';
import { Tabs, Input, Button, Checkbox, Space, message, Spin, Dropdown, AutoComplete, Tag, Modal } from 'antd';
import { FolderOutlined, GithubOutlined, PlusOutlined } from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import * as api from '../../services/skillsApi';
import type { ToolOption, GitSkillCandidate, SkillRepo } from '../../types';
import { GitPickModal } from './GitPickModal';
import {
  isSkillExistsError,
  extractSkillName,
  showGitError,
  confirmSkillOverwrite,
  confirmBatchOverwrite,
} from '../../utils/errorHandlers';
import { syncSkillToTools } from '../../utils/syncHelpers';
import { refreshTrayMenu } from '@/services/appApi';
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
  const [repoExpanded, setRepoExpanded] = React.useState(false);

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

  // Load repos and preferred tools on mount
  React.useEffect(() => {
    loadRepos();
    loadPreferredTools();
  }, []);

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

  // Initialize selected tools on mount: use preferred tools if set, otherwise installed tools
  React.useEffect(() => {
    if (preferredTools && preferredTools.length > 0) {
      setSelectedTools(preferredTools);
    } else {
      const installed = allTools.filter((t) => t.installed).map((t) => t.id);
      setSelectedTools(installed);
    }
  }, [allTools, preferredTools]);

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

  const doLocalInstall = async (overwrite: boolean) => {
    setLoading(true);
    try {
      const result = await api.installLocalSkill(localPath, overwrite);
      if (selectedTools.length > 0) {
        await syncSkillToTools({
          skillId: result.skill_id,
          centralPath: result.central_path,
          skillName: result.name,
          selectedTools,
          allTools,
          t,
          onTargetExists: 'confirm',
        });
      }
      message.success(t('skills.status.localSkillCreated'));
      onSuccess();
      resetForm();
      refreshTrayMenu();
    } catch (error) {
      const errMsg = String(error);
      if (!overwrite && isSkillExistsError(errMsg)) {
        const skillName = extractSkillName(errMsg);
        confirmSkillOverwrite(skillName, t, () => doLocalInstall(true));
      } else {
        showGitError(errMsg, t, allTools);
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
        await syncSkillToTools({
          skillId: result.skill_id,
          centralPath: result.central_path,
          skillName: result.name,
          selectedTools,
          allTools,
          t,
          onTargetExists: 'confirm',
        });
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
      refreshTrayMenu();
    } catch (error) {
      const errMsg = String(error);
      if (!overwrite && isSkillExistsError(errMsg)) {
        const skillName = extractSkillName(errMsg);
        confirmSkillOverwrite(skillName, t, () => doGitInstall(true));
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
          showGitError(String(listError), t, allTools);
        }
      } else {
        showGitError(errMsg, t, allTools);
      }
    } finally {
      setLoading(false);
    }
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
            await syncSkillToTools({
              skillId: result.skill_id,
              centralPath: result.central_path,
              skillName: result.name,
              selectedTools,
              allTools,
              t,
              onTargetExists: 'confirm',
            });
          }
        } catch (error) {
          const errMsg = String(error);
          if (isSkillExistsError(errMsg)) {
            const skillName = extractSkillName(errMsg);
            if (overwriteAll) {
              const result = await api.installGitSelection(gitUrl, sel.subpath, gitBranch || undefined, true);
              if (selectedTools.length > 0) {
                await syncSkillToTools({
                  skillId: result.skill_id,
                  centralPath: result.central_path,
                  skillName: result.name,
                  selectedTools,
                  allTools,
                  t,
                  onTargetExists: 'confirm',
                });
              }
            } else {
              const action = await confirmBatchOverwrite(skillName, selections.length > 1, t);
              if (action === 'overwrite') {
                const result = await api.installGitSelection(gitUrl, sel.subpath, gitBranch || undefined, true);
                if (selectedTools.length > 0) {
                  await syncSkillToTools({
                    skillId: result.skill_id,
                    centralPath: result.central_path,
                    skillName: result.name,
                    selectedTools,
                    allTools,
                    t,
                    onTargetExists: 'confirm',
                  });
                }
              } else if (action === 'overwriteAll') {
                overwriteAll = true;
                const result = await api.installGitSelection(gitUrl, sel.subpath, gitBranch || undefined, true);
                if (selectedTools.length > 0) {
                  await syncSkillToTools({
                    skillId: result.skill_id,
                    centralPath: result.central_path,
                    skillName: result.name,
                    selectedTools,
                    allTools,
                    t,
                    onTargetExists: 'confirm',
                  });
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
      refreshTrayMenu();
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setLoading(false);
    }
  };

  const resetForm = () => {
    setLocalPath('');
    setGitUrl('');
    setGitBranch('');
    setGitCandidates([]);
    setRepoExpanded(false);
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
                      <label>{t('skills.addGit.urlLabel')}</label>
                      <div className={styles.fieldInput}>
                        <div className={styles.urlRow}>
                          <Input
                            value={gitUrl}
                            onChange={(e) => setGitUrl(e.target.value)}
                            placeholder={t('skills.addGit.urlPlaceholder')}
                          />
                          {repos.length > 0 && (
                            <a
                              className={styles.repoToggle}
                              onClick={() => setRepoExpanded(!repoExpanded)}
                            >
                              {t('skills.addGit.repoLabel')}
                              {repoExpanded ? ' ▴' : ' ▾'}
                            </a>
                          )}
                        </div>
                        {repoExpanded && repos.length > 0 && (
                          <div className={styles.repoTagsList}>
                            {repos.map((repo) => {
                              const key = `${repo.owner}/${repo.name}`;
                              return (
                                <Tag
                                  key={key}
                                  closable
                                  className={styles.repoTag}
                                  onClick={() => {
                                    handleRepoSelect(key);
                                    setRepoExpanded(false);
                                  }}
                                  onClose={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    Modal.confirm({
                                      title: t('skills.addGit.removeRepoTitle'),
                                      content: t('skills.addGit.removeRepoConfirm', { repo: key }),
                                      okText: t('common.confirm'),
                                      cancelText: t('common.cancel'),
                                      onOk: () => handleRemoveRepo(repo.owner, repo.name),
                                    });
                                  }}
                                >
                                  {key}
                                </Tag>
                              );
                            })}
                          </div>
                        )}
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
                      disabled: !tool.installed,
                      label: (
                        <span>
                          {tool.label}
                          {!tool.installed && (
                            <span className={styles.notInstalledTag}>{t('skills.notInstalled')}</span>
                          )}
                        </span>
                      ),
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

      {showGitPick && (
        <GitPickModal
          open={showGitPick}
          candidates={gitCandidates}
          onClose={() => setShowGitPick(false)}
          onConfirm={handleGitPickConfirm}
        />
      )}
    </>
  );
};
