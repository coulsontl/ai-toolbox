import React from 'react';
import { Alert, Button, Checkbox, Modal, Switch, Tooltip, message } from 'antd';
import { DeleteOutlined, PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import { ChevronDown, ChevronUp } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { useSkillsStore } from '../../stores/skillsStore';
import { ToolIcon } from '@/features/coding/shared/toolIcon/ToolIcon';
import * as api from '../../services/skillsApi';
import type { AutoSyncRunResult, AutoSyncSource } from '../../types';
import styles from './SkillsAutoSyncModal.module.less';

interface SkillsAutoSyncModalProps {
  open: boolean;
  onClose: () => void;
  onConfigSaved?: () => void;
}

/** The cross-tool agentskills.io directory, selected by default for a fresh setup. */
const DEFAULT_TARGET_TOOL = 'shared_agents';

/** Sources shown before "show all": central, Universal, CC Switch, Claude Code, Codex. */
const SOURCE_PREVIEW_COUNT = 5;

/** Rows shown before "show all": Universal, Claude Code, Codex. */
const TOOL_PREVIEW_COUNT = 3;

/** Pinned tool order; the backend keeps the same order for sources. */
const PINNED_TOOLS = ['shared_agents', 'claude_code', 'codex'];

/**
 * Dedicated auto-discover dialog (issue #446): scan sources + distribution
 * targets live here, not in the general Skills settings modal, because the
 * source list is long and needs its own space to be scannable.
 */
export const SkillsAutoSyncModal: React.FC<SkillsAutoSyncModalProps> = ({
  open: isOpen,
  onClose,
  onConfigSaved,
}) => {
  const { t } = useTranslation();
  const { skills, toolStatus, loadSkills } = useSkillsStore();

  const [enabled, setEnabled] = React.useState(false);
  const [sourceIds, setSourceIds] = React.useState<string[]>(['central']);
  const [sources, setSources] = React.useState<AutoSyncSource[]>([]);
  // The saved source selection, used only for folding: see `visibleSources`.
  const [savedSourceIds, setSavedSourceIds] = React.useState<string[]>(['central']);
  // `null` means "not configured yet": the backend then falls back to the
  // Universal directory. Materializing it into an array on load would persist an
  // explicit list the user never chose (and an empty one if `toolStatus` is
  // still loading, which would distribute nowhere).
  const [targetTools, setTargetTools] = React.useState<string[] | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [running, setRunning] = React.useState(false);
  const [result, setResult] = React.useState<AutoSyncRunResult | null>(null);
  const [savedSnapshot, setSavedSnapshot] = React.useState('');
  const [sourcesExpanded, setSourcesExpanded] = React.useState(false);
  const [toolsExpanded, setToolsExpanded] = React.useState(false);

  const allTools = toolStatus?.tools ?? [];

  const snapshotOf = (value: {
    enabled: boolean;
    sources: string[];
    targets: string[] | null;
  }) => JSON.stringify(value);

  const loadConfig = React.useCallback(async () => {
    try {
      const [config, sourceList] = await Promise.all([
        api.getAutoSync(),
        api.getAutoSyncSources(),
      ]);
      setSources(sourceList.sources);
      setEnabled(config.enabled);
      setSourceIds(config.sources);
      setSavedSourceIds(config.sources);
      setTargetTools(config.target_tools);
      setSavedSnapshot(
        snapshotOf({
          enabled: config.enabled,
          sources: config.sources,
          targets: config.target_tools,
        }),
      );
    } catch (error) {
      message.error(String(error));
    }
  }, []);

  React.useEffect(() => {
    if (isOpen) {
      setResult(null);
      void loadConfig();
    }
  }, [isOpen, loadConfig]);

  const dirty =
    savedSnapshot !==
    snapshotOf({ enabled, sources: sourceIds, targets: targetTools });

  // The first manual click on a checkbox pins the list: from then on the user's
  // selection is explicit and replaces the implicit default.
  const checkedTargets = targetTools ?? [DEFAULT_TARGET_TOOL];

  // The dialog keeps the backend's display order (central, Universal, CC Switch,
  // Claude Code, Codex, then the rest). Custom directories are never folded, and
  // a source enabled in the saved config stays visible — same click-stability
  // rule as the tool grid.
  const visibleSources = React.useMemo(() => {
    if (sourcesExpanded) {
      return sources;
    }
    const shown = new Set(
      sources.slice(0, SOURCE_PREVIEW_COUNT).map((source) => source.id),
    );
    for (const source of sources) {
      if (source.kind === 'custom' || source.locked || savedSourceIds.includes(source.id)) {
        shown.add(source.id);
      }
    }
    return sources.filter((source) => shown.has(source.id));
  }, [sources, sourcesExpanded, savedSourceIds]);

  const hiddenSourceCount = sources.length - visibleSources.length;

  // Same rule for tools: the three pinned rows first, then installed tools.
  const orderedTools = React.useMemo(() => {
    const rank = (key: string) => {
      const index = PINNED_TOOLS.indexOf(key);
      return index === -1 ? PINNED_TOOLS.length : index;
    };
    return [...allTools].sort(
      (left, right) => rank(left.key) - rank(right.key) || Number(right.installed) - Number(left.installed),
    );
  }, [allTools]);

  // Collapsing keys off the persisted selection, not the live one: unchecking a
  // row (or checking one `loadConfig` re-added) must not add or drop rows mid
  // click. Anything saved as checked stays visible, so the list always shows
  // every tool a distribution will target.
  const visibleTools = React.useMemo(() => {
    if (toolsExpanded) {
      return orderedTools;
    }
    const shown = new Set(
      orderedTools.slice(0, TOOL_PREVIEW_COUNT).map((tool) => tool.key),
    );
    for (const tool of orderedTools) {
      if (targetTools?.includes(tool.key)) {
        shown.add(tool.key);
      }
    }
    return orderedTools.filter((tool) => shown.has(tool.key));
  }, [orderedTools, toolsExpanded, targetTools]);

  const hiddenToolCount = orderedTools.length - visibleTools.length;

  const handleToggleSource = (sourceId: string, checked: boolean) => {
    setSourceIds((previous) =>
      checked ? [...previous, sourceId] : previous.filter((id) => id !== sourceId),
    );
  };

  const handleToggleTarget = (toolKey: string, checked: boolean) => {
    setTargetTools((previous) => {
      const current = previous ?? checkedTargets;
      return checked
        ? [...current, toolKey]
        : current.filter((key) => key !== toolKey);
    });
  };

  const handleAddDir = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== 'string') {
        return;
      }
      await api.addAutoSyncDir(selected);
      await loadConfig();
    } catch (error) {
      message.error(String(error));
    }
  };

  const handleRemoveDir = async (path: string) => {
    try {
      await api.removeAutoSyncDir(path);
      await loadConfig();
    } catch (error) {
      message.error(String(error));
    }
  };

  const handleSave = async () => {
    setLoading(true);
    try {
      await api.setAutoSync({
        enabled,
        sources: sourceIds,
        target_tools: targetTools,
      });
      setSavedSnapshot(
        snapshotOf({ enabled, sources: sourceIds, targets: targetTools }),
      );
      setSavedSourceIds(sourceIds);
      onConfigSaved?.();
      message.success(t('common.success'));
      onClose();
    } catch (error) {
      message.error(String(error));
    } finally {
      setLoading(false);
    }
  };

  const handleScanNow = async () => {
    setRunning(true);
    setResult(null);
    try {
      // Persist first so the manual pass uses exactly what the user sees.
      await api.setAutoSync({
        enabled,
        sources: sourceIds,
        target_tools: targetTools,
      });
      setSavedSnapshot(
        snapshotOf({ enabled, sources: sourceIds, targets: targetTools }),
      );
      setSavedSourceIds(sourceIds);
      const outcome = await api.runAutoSync();
      setResult(outcome);
      await loadSkills();
      await loadConfig();
      onConfigSaved?.();
    } catch (error) {
      message.error(String(error));
    } finally {
      setRunning(false);
    }
  };

  const newSkillCount = sources.reduce((total, source) => total + source.new_count, 0);
  const managedCount = skills.length;

  return (
    <Modal
      title={t('skills.autoSync.title')}
      open={isOpen}
      onCancel={onClose}
      width={680}
      footer={
        <div className={styles.footer}>
          <span className={styles.footerHint}>{t('skills.autoSync.footerHint')}</span>
          <Button onClick={onClose}>{t('common.cancel')}</Button>
          <Button type="primary" loading={loading} disabled={!dirty} onClick={handleSave}>
            {t('common.save')}
          </Button>
        </div>
      }
    >
      <div className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <div className={styles.sectionTitle}>{t('skills.autoSync.enabled')}</div>
            <p className={styles.hint}>{t('skills.autoSync.enabledHint')}</p>
          </div>
          <Switch checked={enabled} onChange={setEnabled} />
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <div className={styles.sectionTitle}>{t('skills.autoSync.sources')}</div>
            <p className={styles.hint}>{t('skills.autoSync.sourcesHint')}</p>
          </div>
        </div>
        <div className={styles.list}>
          {visibleSources.map((source) => {
            const unavailable = !source.exists;
            return (
              <div
                key={source.id}
                className={`${styles.row}${unavailable ? ` ${styles.rowMuted}` : ''}`}
              >
                <Checkbox
                  checked={source.locked || sourceIds.includes(source.id)}
                  disabled={source.locked || unavailable}
                  onChange={(event) => handleToggleSource(source.id, event.target.checked)}
                >
                  <span className={styles.rowLabel}>
                    {source.kind === 'tool' ? (
                      <ToolIcon
                        toolKey={source.id.replace('tool:', '')}
                        label={source.label}
                        size={14}
                      />
                    ) : null}
                    <span className={styles.rowName}>{source.label}</span>
                    {source.new_count > 0 && (
                      <span className={styles.newBadge}>
                        {t('skills.autoSync.newBadge', { count: source.new_count })}
                      </span>
                    )}
                  </span>
                </Checkbox>
                <div className={styles.rowMeta}>
                  <Tooltip title={source.path}>
                    <span className={styles.path} title={source.path}>
                      {source.path}
                    </span>
                  </Tooltip>
                  <span className={styles.count}>
                    {unavailable
                      ? t('skills.autoSync.sourceMissing')
                      : t('skills.autoSync.sourceCount', {
                          total: source.detected_count,
                          fresh: source.new_count,
                        })}
                  </span>
                  {source.kind === 'custom' && (
                    <Button
                      type="text"
                      size="small"
                      icon={<DeleteOutlined />}
                      aria-label={t('skills.autoSync.removeDir')}
                      onClick={() => handleRemoveDir(source.path)}
                    />
                  )}
                </div>
              </div>
            );
          })}
        </div>
        <div className={styles.listActions}>
          <Button
            type="dashed"
            size="small"
            icon={<PlusOutlined />}
            onClick={handleAddDir}
          >
            {t('skills.autoSync.addDir')}
          </Button>
          {(hiddenSourceCount > 0 || sourcesExpanded) && (
            <Button
              type="text"
              size="small"
              icon={sourcesExpanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
              onClick={() => setSourcesExpanded((previous) => !previous)}
            >
              {sourcesExpanded
                ? t('skills.autoSync.showLess')
                : t('skills.autoSync.showMore')}
            </Button>
          )}
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <div className={styles.sectionTitle}>{t('skills.autoSync.targetTools')}</div>
            <p className={styles.hint}>{t('skills.autoSync.targetToolsHint')}</p>
          </div>
        </div>
        <div className={styles.toolGrid}>
          {visibleTools.map((tool) => (
            <Tooltip key={tool.key} title={tool.skills_dir}>
              <label
                className={`${styles.toolCell}${checkedTargets.includes(tool.key) ? ` ${styles.toolCellActive}` : ''}${tool.installed ? '' : ` ${styles.rowMuted}`}`}
              >
                <Checkbox
                  checked={checkedTargets.includes(tool.key)}
                  disabled={!tool.installed}
                  onChange={(event) => handleToggleTarget(tool.key, event.target.checked)}
                >
                  <span className={styles.rowLabel}>
                    <ToolIcon
                      toolKey={tool.key}
                      label={tool.label}
                      size={14}
                      iconUrl={tool.icon_url ?? undefined}
                    />
                    {tool.label}
                  </span>
                </Checkbox>
              </label>
            </Tooltip>
          ))}
        </div>
        {(hiddenToolCount > 0 || toolsExpanded) && (
          <Button
            type="text"
            size="small"
            className={styles.expandTools}
            icon={toolsExpanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
            onClick={() => setToolsExpanded((previous) => !previous)}
          >
            {toolsExpanded
              ? t('skills.autoSync.showLess')
              : t('skills.autoSync.showAllTools', { count: hiddenToolCount })}
          </Button>
        )}
      </div>

      <div className={styles.runSection}>
        <div className={styles.runSummary}>
          <span>{t('skills.autoSync.detectedTotal', { count: managedCount })}</span>
          <span className={styles.runDivider} aria-hidden="true" />
          <span>{t('skills.autoSync.pendingTotal', { count: newSkillCount })}</span>
        </div>
        <Button
          size="small"
          icon={<ReloadOutlined />}
          loading={running}
          onClick={handleScanNow}
        >
          {t('skills.autoSync.scanNow')}
        </Button>
      </div>

      {result && (
        <Alert
          className={styles.result}
          type={result.errors.length > 0 ? 'warning' : 'success'}
          showIcon
          message={t('skills.autoSync.runSummary', {
            adopted: result.adopted,
            refreshed: result.refreshed,
            synced: result.synced_targets,
            skipped: result.skipped_conflicts.length,
          })}
          description={result.errors.length > 0 ? result.errors.join('\n') : undefined}
        />
      )}
    </Modal>
  );
};
