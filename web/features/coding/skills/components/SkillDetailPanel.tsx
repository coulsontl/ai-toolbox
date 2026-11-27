import React from 'react';
import { message } from 'antd';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  ExternalLink,
  FileText,
  Folder,
  FolderOpen,
  Package,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import MarkdownPreview from '@/components/common/MarkdownPreview';
import * as api from '../services/skillsApi';
import type { ManagedSkill, SkillDocument, ToolOption } from '../types';
import { isBuiltinSkill } from '../utils/builtinSkill';
import { hashTagColorIndex, normalizeTagList } from '../utils/skillTags';
import { toGitWebUrl } from '../utils/gitUrl';
import { getSkillManifestPath } from '../utils/skillPath';
import { GitHubSourceIcon, ToolIcon } from '@/features/coding/shared/toolIcon/ToolIcon';
import cardStyles from './SkillCard.module.less';
import styles from './SkillDetailPanel.module.less';

const isMissingPathError = (error: unknown): boolean => String(error ?? '').includes('Path does not exist');

// Tag pill color classes come from the SkillCard stylesheet so the palette has
// a single definition (kept in sync with TAG_COLOR_COUNT in utils/skillTags).
const TAG_COLOR_CLASS_NAMES: readonly string[] = [
  cardStyles.tagColor0,
  cardStyles.tagColor1,
  cardStyles.tagColor2,
  cardStyles.tagColor3,
  cardStyles.tagColor4,
  cardStyles.tagColor5,
  cardStyles.tagColor6,
  cardStyles.tagColor7,
];
const tagPillColorClass = (tag: string): string =>
  TAG_COLOR_CLASS_NAMES[hashTagColorIndex(tag)] ?? cardStyles.tagColor0;

interface SkillDetailPanelProps {
  skill: ManagedSkill;
  allTools: ToolOption[];
  loading: boolean;
  updatingSkillIds: string[];
  getRepoInfo: (url: string | null | undefined) => { label: string; href: string } | null;
  formatRelative: (ms: number | null | undefined) => string;
  onClose: () => void;
  onToggleTool: (skill: ManagedSkill, toolId: string) => Promise<void>;
  onUpdate: (skill: ManagedSkill) => Promise<void>;
  onDelete: (skillId: string) => void;
  onSetManagementEnabled: (skill: ManagedSkill, enabled: boolean) => void;
  /** Distinct tag names across all skills, for inline autocomplete. */
  allTags?: string[];
  /** Persist an updated tag list; omit to render tags read-only. */
  onUpdateTags?: (skillId: string, nextTags: string[]) => void;
  /** Open the shared skill metadata modal (group + note editing). */
  onEditMetadata?: (skill: ManagedSkill) => void;
}

export const SkillDetailPanel: React.FC<SkillDetailPanelProps> = ({
  skill,
  allTools,
  loading,
  updatingSkillIds,
  getRepoInfo,
  formatRelative,
  onClose,
  onToggleTool,
  onUpdate,
  onDelete,
  onSetManagementEnabled,
  allTags = [],
  onUpdateTags,
  onEditMetadata,
}) => {
  const { t } = useTranslation();
  const [documents, setDocuments] = React.useState<SkillDocument[] | null>(null);
  const [docLoading, setDocLoading] = React.useState(false);
  const [activeDoc, setActiveDoc] = React.useState<string | null>(null);

  const typeKey = skill.source_type.toLowerCase();
  const builtin = isBuiltinSkill(skill);
  const canEditTags = !!onUpdateTags && !builtin;
  const repoInfo = React.useMemo(() => getRepoInfo(skill.source_ref), [getRepoInfo, skill.source_ref]);
  const syncedToolIds = React.useMemo(
    () => new Set(skill.targets.map((target) => target.tool)),
    [skill.targets],
  );
  const installedTools = React.useMemo(
    () => allTools.filter((tool) => tool.installed),
    [allTools],
  );
  const syncedToolCount = React.useMemo(
    () => installedTools.filter((tool) => syncedToolIds.has(tool.id)).length,
    [installedTools, syncedToolIds],
  );
  // Synced tools first, unsynced after — one flat list of per-tool rows.
  const orderedTools = React.useMemo(() => [
    ...installedTools.filter((tool) => syncedToolIds.has(tool.id)),
    ...installedTools.filter((tool) => !syncedToolIds.has(tool.id)),
  ], [installedTools, syncedToolIds]);

  const skillTagList = React.useMemo(() => normalizeTagList(skill.tags ?? []), [skill.tags]);
  const [tagEditorOpen, setTagEditorOpen] = React.useState(false);
  const [tagDraft, setTagDraft] = React.useState('');

  // The drawer stays mounted while the user switches skills; reset per-skill
  // view state so a previous skill's editor does not leak into the next.
  React.useEffect(() => {
    setTagEditorOpen(false);
    setTagDraft('');
  }, [skill.id]);

  const isUpdating = updatingSkillIds.includes(skill.id);
  // Shared disable state for the inline tag and group editors.
  const metaEditDisabled = loading || isUpdating || !skill.management_enabled || builtin;

  // Load documents whenever the selected skill changes.
  React.useEffect(() => {
    let cancelled = false;
    setDocuments(null);
    setActiveDoc(null);
    setDocLoading(true);
    api
      .getSkillDocuments(skill.id)
      .then((docs) => {
        if (cancelled) return;
        setDocuments(docs);
        setActiveDoc(docs[0]?.filename ?? null);
      })
      .catch(() => {
        if (!cancelled) setDocuments([]);
      })
      .finally(() => {
        if (!cancelled) setDocLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [skill.id]);

  const openExistingFolder = React.useCallback(async (path: string) => {
    try {
      await invoke('open_existing_folder', { path });
    } catch (error) {
      message.error(isMissingPathError(error) ? t('skills.sourceFolderMissing') : t('skills.openFolderFailed'));
    }
  }, [t]);

  const handleOpenCentral = React.useCallback(async () => {
    try {
      await invoke('open_existing_folder', { path: skill.central_path });
    } catch (error) {
      message.error(isMissingPathError(error) ? t('skills.sourceFolderMissing') : t('skills.openFolderFailed'));
    }
  }, [skill.central_path, t]);

  const handleRevealManifest = React.useCallback(async () => {
    const manifestPath = getSkillManifestPath(skill.central_path);
    if (manifestPath) {
      try {
        const { revealItemInDir } = await import('@tauri-apps/plugin-opener');
        await revealItemInDir(manifestPath);
        return;
      } catch {
        // fall through to opening the folder itself
      }
    }
    await handleOpenCentral();
  }, [handleOpenCentral, skill.central_path]);

  const handleCopyCentralPath = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(skill.central_path);
      message.success(t('skills.copied'));
    } catch {
      message.error(t('skills.copyFailed'));
    }
  }, [skill.central_path, t]);

  const handleOpenSource = React.useCallback(async () => {
    if (builtin) {
      await handleOpenCentral();
      return;
    }
    if (typeKey.includes('git')) {
      // Prefer the original ref verbatim for http(s) URLs so subfolder /tree/
      // links open the exact skill page; SSH/SCP refs fall back to the
      // normalized repo web URL, and finally to the managed central folder.
      const webUrl = toGitWebUrl(skill.source_ref) ?? repoInfo?.href;
      if (webUrl) {
        try {
          await openUrl(webUrl);
          return;
        } catch {
          // fall through to reveal the managed folder
        }
      }
      await handleOpenCentral();
      return;
    }
    const sourcePath = skill.source_ref;
    if (skill.source_type === 'local' && sourcePath) {
      void openExistingFolder(sourcePath);
    }
  }, [builtin, handleOpenCentral, openExistingFolder, repoInfo, skill.source_ref, skill.source_type, typeKey]);

  // Tag editing mirrors the card's inline editor: Enter or blur commits,
  // Escape cancels. Mutations go through the page-level onUpdateTags callback.
  const closeTagEditor = () => {
    setTagEditorOpen(false);
    setTagDraft('');
  };

  const commitTagDraft = () => {
    const trimmedDraft = tagDraft.trim();
    closeTagEditor();
    if (!trimmedDraft || !onUpdateTags) return;
    const nextTags = normalizeTagList([...(skill.tags ?? []), trimmedDraft]);
    if (nextTags.length !== skillTagList.length) onUpdateTags(skill.id, nextTags);
  };

  const removeSkillTag = (removedTag: string) => {
    if (!onUpdateTags) return;
    onUpdateTags(skill.id, skillTagList.filter((item) => item !== removedTag));
  };

  const activeDocument = documents?.find((doc) => doc.filename === activeDoc) ?? null;
  const sourceLabel = React.useMemo(() => {
    if (builtin) {
      return t('skills.builtin.versionLabel', { version: skill.source_revision || '' });
    }
    if (typeKey.includes('git')) return repoInfo?.label ?? skill.source_ref ?? 'Git';
    if (skill.source_type === 'local') {
      const path = skill.source_ref || '';
      const parts = path.split(/[\/\\]/);
      return parts[parts.length - 1] || 'Local';
    }
    if (skill.source_type === 'central') return t('skills.sourceCentral');
    return skill.source_type;
  }, [builtin, repoInfo, skill.source_ref, skill.source_revision, skill.source_type, t, typeKey]);

  const sourceIconNode = builtin ? (
    <Package size={13} aria-hidden="true" />
  ) : typeKey.includes('git') ? (
    <GitHubSourceIcon size={13} />
  ) : typeKey.includes('local') ? (
    <FolderOpen size={13} aria-hidden="true" />
  ) : (
    <FileText size={13} aria-hidden="true" />
  );

  return (
    <aside className={styles.panel} aria-label={`${skill.name} details`}>
      <div className={styles.header}>
        <div className={styles.titleBlock}>
          <div className={styles.titleRow}>
            <span
              className={`${styles.manageDot}${skill.management_enabled ? ` ${styles.manageDotEnabled}` : ` ${styles.manageDotDisabled}`}`}
              title={skill.management_enabled ? t('skills.enableSkill') : t('skills.disableSkill')}
            />
            <h3 className={styles.name} title={skill.name}>{skill.name}</h3>
          </div>
          <div className={styles.sourceLine}>
            <button
              type="button"
              className={styles.sourceBtn}
              title={builtin ? t('skills.builtin.sourceLabel') : sourceLabel}
              onClick={handleOpenSource}
            >
              {sourceIconNode}
              <span className={styles.sourceText}>{sourceLabel}</span>
            </button>
            <span className={styles.sourceSep} aria-hidden="true">·</span>
            <span>{formatRelative(skill.updated_at)}</span>
          </div>
        </div>
        <button type="button" className={styles.closeBtn} onClick={onClose} aria-label={t('common.close')}>
          <X size={16} aria-hidden="true" />
        </button>
      </div>

      <div className={styles.scroll}>
        {skill.description ? (
          <p className={styles.description}>{skill.description}</p>
        ) : null}

        {builtin && (
          <p className={styles.builtinHint} title={t('skills.builtin.cannotEdit')}>
            {t('skills.builtin.readOnlyHint')}
          </p>
        )}

        <button
          type="button"
          className={styles.pathRow}
          title={skill.central_path}
          onClick={handleCopyCentralPath}
          onDoubleClick={handleRevealManifest}
        >
          <Folder size={13} aria-hidden="true" />
          <span className={styles.pathText}>{skill.central_path}</span>
        </button>

        {(skill.user_group || skill.user_note || (onEditMetadata && !builtin)) && (
          <div className={styles.metaCard}>
            <div className={styles.metaRows}>
              <div className={styles.metaRow}>
                <span className={styles.metaLabel}>{t('skills.metadata.group')}</span>
                <span className={styles.metaValueGroup}>
                  {skill.user_group?.trim() || t('skills.groupUngrouped')}
                </span>
              </div>
              {skill.user_note?.trim() && (
                <div className={styles.metaRow}>
                  <span className={styles.metaLabel}>{t('skills.metadata.note')}</span>
                  <span className={styles.metaValueText}>{skill.user_note.trim()}</span>
                </div>
              )}
            </div>
            {onEditMetadata && !builtin && (
              <button
                type="button"
                className={styles.metaEditBtn}
                disabled={metaEditDisabled}
                onClick={() => onEditMetadata(skill)}
              >
                <Pencil size={11} aria-hidden="true" />
                {t('common.edit')}
              </button>
            )}
          </div>
        )}

        <div className={styles.tagRow}>
          {skillTagList.map((tagItem) => (
            <span key={tagItem} className={`${styles.tagPill} ${tagPillColorClass(tagItem)}`}>
              <span className={styles.tagPillText} title={tagItem}>{tagItem}</span>
              {canEditTags && (
                <button
                  type="button"
                  className={styles.tagRemoveBtn}
                  title={t('skills.tags.remove')}
                  aria-label={`${t('skills.tags.remove')}: ${tagItem}`}
                  disabled={metaEditDisabled}
                  onClick={() => removeSkillTag(tagItem)}
                >
                  <X size={10} aria-hidden="true" />
                </button>
              )}
            </span>
          ))}
          {tagEditorOpen && canEditTags ? (
            <>
              <input
                className={styles.tagAddInput}
                list={`skill-detail-tag-options-${skill.id}`}
                autoFocus
                value={tagDraft}
                placeholder={t('skills.tags.addPlaceholder')}
                aria-label={t('skills.tags.addPlaceholder')}
                onChange={(event) => setTagDraft(event.target.value)}
                onBlur={commitTagDraft}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') commitTagDraft();
                  if (event.key === 'Escape') closeTagEditor();
                }}
              />
              <datalist id={`skill-detail-tag-options-${skill.id}`}>
                {allTags.map((option) => (<option key={option} value={option} />))}
              </datalist>
            </>
          ) : canEditTags && skillTagList.length === 0 ? (
            <button
              type="button"
              className={styles.tagAddEmptyPill}
              title={t('skills.tags.add')}
              aria-label={t('skills.tags.add')}
              disabled={metaEditDisabled}
              onClick={() => setTagEditorOpen(true)}
            >
              <Plus size={11} aria-hidden="true" />
              {t('skills.tags.add')}
            </button>
          ) : canEditTags && (
            <button
              type="button"
              className={styles.tagAddBtn}
              title={t('skills.tags.add')}
              aria-label={t('skills.tags.add')}
              disabled={metaEditDisabled}
              onClick={() => setTagEditorOpen(true)}
            >
              <Plus size={11} aria-hidden="true" />
            </button>
          )}
        </div>

        <div className={styles.section}>
          <p className={styles.sectionTitle}>
            {t('skills.section.sync')}
            <span className={styles.sectionCount}>
              {t('skills.detail.syncSummary', { synced: syncedToolCount, total: installedTools.length })}
            </span>
          </p>
          {installedTools.length === 0 ? (
            <p className={styles.emptyTools}>{t('skills.detail.noTools')}</p>
          ) : (
            <div className={styles.toolGrid}>
              {orderedTools.map((tool) => {
                const synced = syncedToolIds.has(tool.id);
                // Hover text combines the tool's resolved skills directory and
                // its sync state; the state is otherwise conveyed by border and
                // dot styling only.
                const pathSuffix = tool.skillDir ? ` (${tool.skillDir})` : '';
                const syncText = synced ? t('skills.detail.toolSynced') : t('skills.detail.toolNotSynced');
                const cellLabelText = `${tool.label}${pathSuffix} — ${syncText}`;
                return (
                  <button
                    key={tool.id}
                    type="button"
                    className={`${styles.toolCell}${synced ? ` ${styles.toolCellSynced}` : ''}`}
                    disabled={loading || isUpdating || !skill.management_enabled}
                    onClick={() => onToggleTool(skill, tool.id)}
                    title={cellLabelText}
                    aria-label={cellLabelText}
                  >
                    <span
                      className={`${styles.toolCellDot}${synced ? '' : ` ${styles.toolCellDotOff}`}`}
                      aria-hidden="true"
                    />
                    <ToolIcon toolKey={tool.id} label={tool.label} size={16} iconUrl={tool.iconUrl ?? undefined} />
                    <span className={styles.toolCellLabel}>{tool.label}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className={styles.section}>
          <p className={styles.sectionTitle}>{t('skills.section.document')}</p>
          {(docLoading && !documents) ? (
            <p className={styles.docLoading}>{t('skills.detail.documentLoading')}</p>
          ) : (documents && documents.length > 0) ? (
            <>
              <div className={styles.docTabs}>
                {documents.map((doc) => (
                  <button
                    key={doc.filename}
                    type="button"
                    title={doc.filename}
                    className={`${styles.docTab}${activeDoc === doc.filename ? ` ${styles.docTabActive}` : ''}`}
                    onClick={() => setActiveDoc(doc.filename)}
                  >
                    {doc.filename.split('/').pop() || doc.filename}
                  </button>
                ))}
              </div>
              <div className={styles.docBody}>
                {activeDocument ? (
                  <>
                    <MarkdownPreview content={activeDocument.content || '—'} />
                    {activeDocument.truncated && (
                      <p className={styles.truncatedHint}>{t('skills.detail.truncated')}</p>
                    )}
                  </>
                ) : (
                  <p className={styles.docEmpty}>{t('skills.detail.documentEmpty')}</p>
                )}
              </div>
            </>
          ) : (
            <p className={styles.docEmpty}>{t('skills.detail.noDocument')}</p>
          )}
        </div>
      </div>

      <div className={styles.footer}>
        {typeKey.includes('git') && repoInfo?.href ? (
          <button
            type="button"
            className={styles.footerBtn}
            title={t('skills.openRepo')}
            onClick={handleOpenSource}
          >
            <ExternalLink size={13} aria-hidden="true" />
            <span>{t('skills.openRepo')}</span>
          </button>
        ) : (skill.source_type === 'local' && skill.source_ref) ? (
          <button
            type="button"
            className={styles.footerBtn}
            title={t('skills.openFolder')}
            onClick={handleOpenSource}
          >
            <Folder size={13} aria-hidden="true" />
            <span>{t('skills.openFolder')}</span>
          </button>
        ) : null}
        <button
          type="button"
          className={styles.footerBtn}
          title={t('skills.openDataDir')}
          onClick={handleOpenCentral}
        >
          <FolderOpen size={13} aria-hidden="true" />
          <span>{t('skills.openDataDir')}</span>
        </button>
        {!builtin && (
          <>
            <button
              type="button"
              className={styles.footerBtn}
              title={t('skills.updateTooltip')}
              disabled={loading || isUpdating || !skill.management_enabled}
              onClick={() => onUpdate(skill)}
            >
              <RefreshCw size={13} aria-hidden="true" />
              <span>{t('skills.update')}</span>
            </button>
            <button
              type="button"
              className={styles.footerBtn}
              title={skill.management_enabled ? t('skills.disableSkill') : t('skills.enableSkill')}
              disabled={loading || isUpdating}
              onClick={() => onSetManagementEnabled(skill, !skill.management_enabled)}
            >
              {skill.management_enabled ? <PowerOff size={13} aria-hidden="true" /> : <Power size={13} aria-hidden="true" />}
              <span>{skill.management_enabled ? t('skills.disableSkill') : t('skills.enableSkill')}</span>
            </button>
            <button
              type="button"
              className={`${styles.footerBtn} ${styles.footerBtnDanger}`}
              title={t('skills.remove')}
              disabled={loading || isUpdating}
              onClick={() => onDelete(skill.id)}
            >
              <Trash2 size={13} aria-hidden="true" />
              <span>{t('skills.remove')}</span>
            </button>
          </>
        )}
      </div>
    </aside>
  );
};
