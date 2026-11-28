import React from 'react';
import { Tabs, Input, Button, Checkbox, Space, message, Spin, Dropdown, Tag, Modal } from 'antd';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';
import { FolderOutlined, GithubOutlined, PlusOutlined } from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import * as api from '../../services/skillsApi';
import type { ToolOption, GitSkillCandidate, SkillRepo, InstallResult } from '../../types';
import { useSkillsStore } from '../../stores/skillsStore';
import { GitPickModal } from './GitPickModal';
import { useSkillConflictPrompt } from './SkillConflictModal';
import { extractRepoNameFromGitUrl } from '../../utils/skillAlias';
import {
  isSkillExistsError,
  extractSkillName,
  showGitError,
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
  const skills = useSkillsStore((state) => state.skills);
  const existingNames = React.useMemo(
    () => new Set(skills.map((s) => s.name.trim().toLowerCase())),
    [skills],
  );
  const { conflictModal, promptSkillConflict } = useSkillConflictPrompt();
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

  // Local pick modal state
  const [localCandidates, setLocalCandidates] = React.useState<GitSkillCandidate[]>([]);
  const [showLocalPick, setShowLocalPick] = React.useState(false);

  // Split tools based on preferred tools setting + selected tools
  const visibleTools = React.useMemo(() => {
    if (preferredTools && preferredTools.length > 0) {
      // If preferred tools are set, show those + any selected tools
      return allTools.filter((t) => preferredTools.includes(t.id) || selectedTools.includes(t.id));
    }
    // Otherwise show installed tools + any selected tools
    return allTools.filter((t) => t.installed || selectedTools.includes(t.id));
  }, [allTools, preferredTools, selectedTools]);

  // Hidden dropdown only offers installed tools that are outside the preferred row.
  const hiddenTools = React.useMemo(() => {
    if (preferredTools && preferredTools.length > 0) {
      return allTools.filter((t) => (
        t.installed
        && !preferredTools.includes(t.id)
        && !selectedTools.includes(t.id)
      ));
    }
    return [];
  }, [allTools, preferredTools, selectedTools]);

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

  const installSingleWithConflictResolution = async (
    runInstall: (overwrite: boolean, customName?: string) => Promise<InstallResult>,
    repoHint?: string | null,
  ): Promise<InstallResult | null> => {
    const knownNames = new Set(existingNames);
    let overwrite = false;
    let customName: string | undefined;

    while (true) {
      setLoading(true);
      try {
        return await runInstall(overwrite, customName);
      } catch (error) {
        const errMsg = String(error);
        if (!overwrite && isSkillExistsError(errMsg)) {
          const conflictName = extractSkillName(errMsg);
          if (conflictName.trim()) {
            knownNames.add(conflictName.trim().toLowerCase());
          }
          setLoading(false);
          const resolution = await promptSkillConflict({
            skillName: conflictName,
            existingNames: new Set(knownNames),
            repoHint,
          });
          if (resolution.action === 'skip') {
            return null;
          }
          if (resolution.action === 'overwrite') {
            overwrite = true;
          } else if (resolution.action === 'alias') {
            customName = resolution.alias;
            overwrite = false;
          }
        } else {
          throw error;
        }
      }
    }
  };

  const syncAndFinishSingleInstall = async (
    result: InstallResult,
    options: { saveGitRepo: boolean; successMessageKey: string },
  ) => {
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

    if (options.saveGitRepo) {
      const parsed = parseGitUrl(gitUrl);
      if (parsed) {
        await api.addSkillRepo(parsed.owner, parsed.name, gitBranch || 'main');
        await loadRepos();
      }
    }

    message.success(t(options.successMessageKey));
    onSuccess();
    resetForm();
    refreshTrayMenu();
  };

  const doLocalInstall = async () => {
    setLoading(true);
    try {
      // Scan local folder for skills
      const candidates = await api.listLocalSkills(localPath);

      if (candidates.length > 1) {
        setLocalCandidates(candidates);
        setShowLocalPick(true);
        setLoading(false);
        return;
      }

      if (candidates.length === 0) {
        // No SKILL.md found - ask user to confirm importing whole folder
        setLoading(false);
        Modal.confirm({
          title: t('skills.errors.noSkillsFoundInFolder'),
          content: t('skills.errors.confirmImportWholeFolder'),
          okText: t('common.confirm'),
          cancelText: t('common.cancel'),
          onOk: () => doLocalInstallWhole(),
        });
        return;
      }

      // Single skill found - install via selection API
      const subpath = candidates[0].subpath;
      const result = await installSingleWithConflictResolution(
        (overwrite, customName) => api.installLocalSelection(localPath, subpath, overwrite, customName),
      );
      if (!result) {
        return;
      }
      await syncAndFinishSingleInstall(result, {
        saveGitRepo: false,
        successMessageKey: 'skills.status.localSkillCreated',
      });
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setLoading(false);
    }
  };

  // Install whole folder as single skill (when no SKILL.md found, user confirmed)
  const doLocalInstallWhole = async () => {
    setLoading(true);
    try {
      const result = await installSingleWithConflictResolution(
        (overwrite, customName) => api.installLocalSkill(localPath, overwrite, customName),
      );
      if (!result) {
        return;
      }
      await syncAndFinishSingleInstall(result, {
        saveGitRepo: false,
        successMessageKey: 'skills.status.localSkillCreated',
      });
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setLoading(false);
    }
  };

  const doGitInstall = async () => {
    const repoHint = extractRepoNameFromGitUrl(gitUrl);
    setLoading(true);
    try {
      const candidates = await api.listGitSkills(gitUrl, gitBranch || undefined);
      if (candidates.length > 1) {
        setGitCandidates(candidates);
        setShowGitPick(true);
        setLoading(false);
        return;
      }

      if (candidates.length === 0) {
        // No SKILL.md found - ask user to confirm importing whole repo
        setLoading(false);
        Modal.confirm({
          title: t('skills.errors.noSkillsFoundInRepo'),
          content: t('skills.errors.confirmImportWholeRepo'),
          okText: t('common.confirm'),
          cancelText: t('common.cancel'),
          onOk: () => doGitInstallDirect(),
        });
        return;
      }

      // Single skill - install directly
      const result = await installSingleWithConflictResolution(
        (overwrite, customName) =>
          api.installGitSkill(gitUrl, gitBranch || undefined, overwrite, customName),
        repoHint,
      );
      if (!result) {
        return;
      }
      await syncAndFinishSingleInstall(result, {
        saveGitRepo: true,
        successMessageKey: 'skills.status.gitSkillCreated',
      });
    } catch (error) {
      const errMsg = String(error);
      if (errMsg.startsWith('MULTI_SKILLS|')) {
        try {
          const fallbackCandidates = await api.listGitSkills(gitUrl, gitBranch || undefined);
          if (fallbackCandidates.length > 0) {
            setGitCandidates(fallbackCandidates);
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

  // Install whole repo as single skill (when no SKILL.md found, user confirmed)
  const doGitInstallDirect = async () => {
    const repoHint = extractRepoNameFromGitUrl(gitUrl);
    setLoading(true);
    try {
      const result = await installSingleWithConflictResolution(
        (overwrite, customName) =>
          api.installGitSkill(gitUrl, gitBranch || undefined, overwrite, customName),
        repoHint,
      );
      if (!result) {
        return;
      }
      await syncAndFinishSingleInstall(result, {
        saveGitRepo: true,
        successMessageKey: 'skills.status.gitSkillCreated',
      });
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setLoading(false);
    }
  };

  const handleLocalInstall = () => {
    if (!localPath.trim()) {
      message.error(t('skills.errors.requireLocalPath'));
      return;
    }
    doLocalInstall();
  };

  const handleGitInstall = () => {
    if (!gitUrl.trim()) {
      message.error(t('skills.errors.requireGitUrl'));
      return;
    }
    doGitInstall();
  };

  const runBatchInstall = async (
    selections: { subpath: string }[],
    installSelection: (
      subpath: string,
      overwrite: boolean,
      customName?: string,
    ) => Promise<InstallResult>,
    options: { repoHint?: string | null; saveGitRepo?: boolean },
  ) => {
    setLoading(true);

    const knownNames = new Set(existingNames);
    const skippedNames: string[] = [];
    let overwriteAll = false;
    let skipAll = false;

    try {
      for (let i = 0; i < selections.length; i += 1) {
        const sel = selections[i];
        let overwrite = overwriteAll;
        let customName: string | undefined;
        let result: InstallResult | null = null;

        while (true) {
          setLoading(true);
          try {
            result = await installSelection(sel.subpath, overwrite, customName);
            break;
          } catch (error) {
            const errMsg = String(error);
            if (!overwrite && isSkillExistsError(errMsg)) {
              const conflictName = extractSkillName(errMsg);
              if (conflictName.trim()) {
                knownNames.add(conflictName.trim().toLowerCase());
              }
              if (skipAll) {
                skippedNames.push(conflictName);
                break;
              }
              if (overwriteAll) {
                overwrite = true;
                continue;
              }
              setLoading(false);
              const resolution = await promptSkillConflict({
                skillName: conflictName,
                existingNames: new Set(knownNames),
                repoHint: options.repoHint,
                isBatch: true,
                hasMore: i < selections.length - 1,
              });
              if (resolution.action === 'overwrite') {
                overwrite = true;
              } else if (resolution.action === 'overwriteAll') {
                overwriteAll = true;
                overwrite = true;
              } else if (resolution.action === 'alias') {
                customName = resolution.alias;
                overwrite = false;
              } else if (resolution.action === 'skipAll') {
                skipAll = true;
                skippedNames.push(conflictName);
                break;
              } else {
                skippedNames.push(conflictName);
                break;
              }
            } else {
              throw error;
            }
          }
        }

        if (result) {
          if (result.name.trim()) {
            knownNames.add(result.name.trim().toLowerCase());
          }
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
        }
      }

      if (options.saveGitRepo) {
        const parsed = parseGitUrl(gitUrl);
        if (parsed) {
          await api.addSkillRepo(parsed.owner, parsed.name, gitBranch || 'main');
          await loadRepos();
        }
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

  const handleGitPickConfirm = async (selections: { subpath: string }[]) => {
    setShowGitPick(false);
    await runBatchInstall(
      selections,
      (subpath, overwrite, customName) =>
        api.installGitSelection(gitUrl, subpath, gitBranch || undefined, overwrite, customName),
      { repoHint: extractRepoNameFromGitUrl(gitUrl), saveGitRepo: true },
    );
  };

  const handleLocalPickConfirm = async (selections: { subpath: string }[]) => {
    setShowLocalPick(false);
    await runBatchInstall(
      selections,
      (subpath, overwrite, customName) =>
        api.installLocalSelection(localPath, subpath, overwrite, customName),
      { saveGitRepo: false },
    );
  };

  const resetForm = () => {
    setLocalPath('');
    setGitUrl('');
    setGitBranch('');
    setGitCandidates([]);
    setLocalCandidates([]);
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
                        <div className={styles.fieldHint}>{t('skills.addLocal.pathHint')}</div>
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
                        <ImeSafeAutoComplete
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
                      label: (
                        <span onClick={(e) => e.stopPropagation()}>
                          <Checkbox
                            checked={selectedTools.includes(tool.id)}
                            onChange={() => handleToolToggle(tool.id)}
                            onClick={(e) => e.stopPropagation()}
                          >
                            {tool.label}
                          </Checkbox>
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
          existingNames={existingNames}
          onClose={() => setShowGitPick(false)}
          onConfirm={handleGitPickConfirm}
        />
      )}

      {showLocalPick && (
        <GitPickModal
          open={showLocalPick}
          candidates={localCandidates}
          existingNames={existingNames}
          onClose={() => setShowLocalPick(false)}
          onConfirm={handleLocalPickConfirm}
        />
      )}

      {conflictModal}
    </>
  );
};
