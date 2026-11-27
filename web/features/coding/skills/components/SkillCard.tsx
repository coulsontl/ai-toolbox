import React from 'react';
import { message } from 'antd';
import { invoke } from '@tauri-apps/api/core';
import {
  Copy,
  FolderOpen,
  Globe,
  GripVertical,
  MoreHorizontal,
  Package,
  Plus,
  Power,
  RefreshCw,
  Tags,
  Trash2,
  TriangleAlert,
} from 'lucide-react';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import {
  ManagementCheckbox,
  ManagementMenu,
  type ManagementMenuItem,
} from '@/features/coding/shared/management';
import type { ManagedSkill, ToolOption } from '../types';
import { toGitWebUrl } from '../utils/gitUrl';
import { getSkillFolderOpenCandidates, getSkillManifestPath } from '../utils/skillPath';
import { flattenDescription } from '../utils/skillDescription';
import { isBuiltinSkill } from '../utils/builtinSkill';
import { hashTagColorIndex, normalizeTagList } from '../utils/skillTags';
import { GitHubSourceIcon, ToolIcon } from '@/features/coding/shared/toolIcon/ToolIcon';
import styles from './SkillCard.module.less';

// Tag pill color classes, kept in sync with .tagColor0..7 below and with
// TAG_COLOR_COUNT in utils/skillTags.ts (locked by that module's unit tests).
const TAG_COLOR_CLASS_NAMES: readonly string[] = [
  styles.tagColor0,
  styles.tagColor1,
  styles.tagColor2,
  styles.tagColor3,
  styles.tagColor4,
  styles.tagColor5,
  styles.tagColor6,
  styles.tagColor7,
];
const tagPillColorClass = (tag: string): string =>
  TAG_COLOR_CLASS_NAMES[hashTagColorIndex(tag)] ?? styles.tagColor0;

const isMissingPathError = (error: unknown): boolean => String(error ?? '').includes('Path does not exist');

interface SkillCardProps {
  skill: ManagedSkill;
  allTools: ToolOption[];
  loading: boolean;
  isUpdating?: boolean;
  dragDisabled?: boolean;
  showGroupTag?: boolean;
  selectable?: boolean;
  selected?: boolean;
  toolsReadOnly?: boolean;
  preferredToolKeysForAddMore?: string[];
  limitAddMoreToPreferredTools?: boolean;
  onSelectChange?: (skillId: string, checked: boolean) => void;
  onOpenDetail?: (skill: ManagedSkill) => void;
  getRepoInfo: (url: string | null | undefined) => { label: string; href: string } | null;
  formatRelative: (ms: number | null | undefined) => string;
  onUpdate: (skill: ManagedSkill) => void;
  onDelete: (skillId: string) => void;
  onToggleTool: (skill: ManagedSkill, toolId: string) => void;
  onEditMetadata: (skill: ManagedSkill) => void;
  onSetManagementEnabled: (skill: ManagedSkill, enabled: boolean) => void;
}

interface SkillCardContentProps extends Omit<SkillCardProps, 'dragDisabled'> {
  dragHandle?: React.ReactNode;
  containerRef?: (node: HTMLDivElement | null) => void;
  containerStyle?: React.CSSProperties;
}

const SkillCardContent = React.memo(function SkillCardContent({
  skill,
  allTools,
  loading,
  isUpdating = false,
  showGroupTag = true,
  selectable,
  selected,
  toolsReadOnly,
  preferredToolKeysForAddMore,
  limitAddMoreToPreferredTools,
  onSelectChange,
  onOpenDetail,
  getRepoInfo,
  formatRelative,
  onUpdate,
  onDelete,
  onToggleTool,
  onEditMetadata,
  onSetManagementEnabled,
  dragHandle,
  containerRef,
  containerStyle,
}: SkillCardContentProps) {
  const { t } = useTranslation();

  const typeKey = skill.source_type.toLowerCase();
  const builtin = isBuiltinSkill(skill);
  const builtinVersionText = t('skills.builtin.versionLabel', {
    version: skill.source_revision || '',
  });
  const sourceWarningMessage = skill.source_health === 'warning'
    ? (skill.source_error || t('skills.sourceWarningFallback'))
    : undefined;
  const cardClassName = [
    styles.card,
    selectable && selected && !builtin ? styles.cardSelected : undefined,
    !skill.management_enabled ? styles.disabledCard : undefined,
    sourceWarningMessage ? styles.sourceWarningCard : undefined,
  ].filter(Boolean).join(' ');
  const groupLabel = skill.user_group?.trim() ?? '';
  const userNoteText = skill.user_note?.trim() ?? '';
  const shouldShowGroupTag = showGroupTag && groupLabel.length > 0;
  const hasUserNote = userNoteText.length > 0;
  // Description comes from the backend SKILL.md frontmatter cache. Block
  // scalars (`|`/`>`) keep newlines; flatten to one line so the card shows the
  // full text instead of just its first line.
  const skillDescriptionText = flattenDescription(skill.description);
  const hasSkillDescription = skillDescriptionText.length > 0;
  const skillTagList = React.useMemo(
    () => normalizeTagList(skill.tags ?? []),
    [skill.tags],
  );
  const managementToggleLabel = skill.management_enabled ? t('skills.disableSkill') : t('skills.enableSkill');

  // These values are derived from stable inputs and are recalculated for every card.
  // Memoizing them keeps scroll and hover interactions cheaper when many cards are on screen.
  const repoInfo = React.useMemo(
    () => getRepoInfo(skill.source_ref),
    [getRepoInfo, skill.source_ref],
  );

  // Web URL for opening the Git source in a browser. Prefer the original ref
  // verbatim when it is already an http(s) URL so subfolder /tree/ links open
  // the exact skill page; fall back to the normalized repo URL for SSH/SCP refs.
  const repoUrl = React.useMemo(
    () => toGitWebUrl(skill.source_ref) ?? repoInfo?.href ?? '',
    [repoInfo, skill.source_ref],
  );

  const copyValue = React.useMemo(
    () => (
      builtin
        ? skill.central_path
        : toGitWebUrl(skill.source_ref) ?? repoInfo?.href ?? skill.source_ref ?? ''
    ),
    [builtin, repoInfo, skill.central_path, skill.source_ref],
  );

  const sourceLabel = React.useMemo(() => {
    if (builtin) return t('skills.builtin.sourceLabel');
    if (typeKey.includes('git')) return repoInfo?.label ?? 'Git';
    if (skill.source_type === 'local') {
      const parts = (skill.source_ref || '').split(/[\/\\]/);
      return parts[parts.length - 1] || 'Local';
    }
    if (skill.source_type === 'central') return t('skills.sourceCentral');
    return skill.source_type;
  }, [builtin, repoInfo, skill.source_ref, skill.source_type, t, typeKey]);

  const handleCopy = async () => {
    if (!copyValue) return;
    try {
      await navigator.clipboard.writeText(copyValue);
      message.success(t('skills.copied'));
    } catch {
      message.error(t('skills.copyFailed'));
    }
  };

  const handleReadOnlyToolClick = React.useCallback(() => {
    message.info(t('skills.groupTools.cardToolReadOnly'));
  }, [t]);

  const openExistingFolder = React.useCallback(async (path: string) => {
    await invoke('open_existing_folder', { path });
  }, []);

  const openFirstPath = React.useCallback(async (paths: string[]) => {
    for (const path of paths) {
      try {
        await openExistingFolder(path);
        return true;
      } catch {
        // Try the next candidate for central-path reveal fallback.
      }
    }

    return false;
  }, [openExistingFolder]);

  const handleIconClick = async () => {
    if (typeKey.includes('git')) {
      if (!repoUrl) {
        // No usable web URL (e.g. malformed ref); fall back to the managed folder.
        await handleOpenCentralPath();
        return;
      }

      try {
        await openUrl(repoUrl);
      } catch {
        // Opening the remote URL failed (the host may be unreachable or the
        // opener rejected it). Fall back to revealing the local managed copy.
        await handleOpenCentralPath();
      }
      return;
    }

    if (skill.source_type === 'local') {
      const sourcePath = getSkillFolderOpenCandidates(skill)[0];
      if (!sourcePath) {
        message.error(t('skills.sourceFolderMissing'));
        return;
      }

      try {
        await openExistingFolder(sourcePath);
      } catch (error) {
        message.error(
          isMissingPathError(error) ? t('skills.sourceFolderMissing') : t('skills.openFolderFailed'),
        );
      }
    }
  };

  const handleOpenCentralPath = async () => {
    const manifestPath = getSkillManifestPath(skill.central_path);

    if (manifestPath) {
      try {
        await revealItemInDir(manifestPath);
        return;
      } catch {
        // If SKILL.md cannot be revealed, fall back to opening the managed folder.
      }
    }

    const opened = await openFirstPath(getSkillFolderOpenCandidates({
      source_type: 'central',
      central_path: skill.central_path,
    }));
    if (!opened) {
      message.error(t('skills.openFolderFailed'));
    }
  };

  const handleToggleManagement = React.useCallback(() => {
    if (loading || isUpdating) return;
    onSetManagementEnabled(skill, !skill.management_enabled);
  }, [isUpdating, loading, onSetManagementEnabled, skill]);

  const iconTooltip = React.useMemo(() => {
    if (builtin) {
      return t('skills.builtin.readOnlyHint');
    }
    if (typeKey.includes('git') && (repoUrl || skill.source_ref?.trim())) {
      return t('skills.openRepo');
    }
    if (skill.source_type === 'local' && skill.source_ref?.trim()) {
      return t('skills.openFolder');
    }
    return undefined;
  }, [builtin, repoUrl, skill.source_ref, skill.source_type, t, typeKey]);

  const iconClickable = !!iconTooltip && !builtin;

  const iconNode = builtin ? (
    <Package size={13} aria-hidden="true" />
  ) : typeKey.includes('git') ? (
    <GitHubSourceIcon size={13} />
  ) : typeKey.includes('local') ? (
    <FolderOpen size={13} aria-hidden="true" />
  ) : (
    <Globe size={13} aria-hidden="true" />
  );

  // Tool grouping is pure derived data based on the skill targets and tool list.
  // Memoizing avoids rebuilding the same sets and filtered arrays on every parent render.
  const syncedToolIds = React.useMemo(
    () => new Set(skill.targets.map((target) => target.tool)),
    [skill.targets],
  );

  const syncedTools = React.useMemo(
    () => allTools.filter((tool) => syncedToolIds.has(tool.id)),
    [allTools, syncedToolIds],
  );

  const availableDropdownTools = React.useMemo(() => {
    const candidates = allTools.filter((tool) => tool.installed && !syncedToolIds.has(tool.id));
    if (limitAddMoreToPreferredTools && preferredToolKeysForAddMore) {
      const preferredKeys = new Set(preferredToolKeysForAddMore);
      return candidates.filter((tool) => preferredKeys.has(tool.id));
    }
    return candidates;
  }, [allTools, syncedToolIds, limitAddMoreToPreferredTools, preferredToolKeysForAddMore]);

  // Dropdown items are also pure view data. Keep them memoized so large lists do not
  // recreate identical menu structures unless tools, translations, or handlers change.
  const dropdownItems = React.useMemo<ManagementMenuItem[]>(
    () =>
      availableDropdownTools.map((tool) => ({
        key: tool.id,
        label: tool.label,
        icon: <ToolIcon toolKey={tool.id} label={tool.label} size={14} iconUrl={tool.iconUrl ?? undefined} />,
        onSelect: () => onToggleTool(skill, tool.id),
      })),
    [availableDropdownTools, onToggleTool, skill],
  );

  const actionItems = React.useMemo<ManagementMenuItem[]>(
    () => (
      builtin
        ? []
        : [
          {
            key: 'metadata',
            icon: <Tags size={14} />,
            label: t('skills.metadata.edit'),
            onSelect: () => onEditMetadata(skill),
            disabled: loading || isUpdating,
          },
          {
            key: 'management-enabled',
            icon: <Power size={14} />,
            label: managementToggleLabel,
            onSelect: handleToggleManagement,
            disabled: loading || isUpdating,
          },
          {
            key: 'delete',
            danger: true,
            icon: <Trash2 size={14} />,
            label: t('skills.remove'),
            onSelect: () => onDelete(skill.id),
            disabled: loading || isUpdating,
          },
        ]
    ),
    [builtin, handleToggleManagement, isUpdating, loading, managementToggleLabel, onDelete, onEditMetadata, skill, t],
  );

  // Tag editing lives in the detail panel only; the card tag row is pure
  // display (skills-manager convention) and hides entirely when empty.
  const handleCardClick: React.MouseEventHandler<HTMLDivElement> = React.useCallback((event) => {
    // Selection mode uses the checkbox (and card body click is reserved for
    // selecting); interactive elements handle their own click. Only open the
    // detail panel from the card body in browse mode.
    if (selectable) {
      return;
    }
    const target = event.target as HTMLElement;
    if (target.closest('button, a, input, [role="menuitem"], [role="checkbox"], [data-skill-card-no-detail]')) {
      return;
    }
    onOpenDetail?.(skill);
  }, [onOpenDetail, selectable, skill]);

  return (
    <div ref={containerRef} style={containerStyle} className={styles.cardOuter}>
      <div className={cardClassName} onClick={onOpenDetail ? handleCardClick : undefined}>
        <div className={styles.headerRow}>
          {selectable ? (
            builtin ? (
              <span title={t('skills.builtin.cannotDelete')}>
                <ManagementCheckbox
                  ariaLabel={`${t('skills.builtin.badge')} ${skill.name}`}
                  checked={false}
                  disabled
                  onChange={() => undefined}
                />
              </span>
            ) : (
              <ManagementCheckbox
                ariaLabel={`${t('common.select')} ${skill.name}`}
                checked={!!selected}
                onChange={(checked) => onSelectChange?.(skill.id, checked)}
              />
            )
          ) : dragHandle}
          {!selectable && !dragHandle && (
            <span
              className={`${styles.statusDot}${skill.management_enabled ? ` ${styles.statusDotEnabled}` : ''}`}
              title={skill.management_enabled ? t('skills.enableSkill') : t('skills.disableSkill')}
              aria-hidden="true"
            />
          )}
          <span className={styles.name} title={skill.name}>{skill.name}</span>
          {sourceWarningMessage && (
            <span
              className={styles.sourceWarningMeta}
              title={sourceWarningMessage}
              aria-label={`${t('skills.sourceWarning')}: ${sourceWarningMessage}`}
            >
              <TriangleAlert size={11} aria-hidden="true" />
            </span>
          )}
          <span className={styles.hoverActions}>
            <button
              type="button"
              className={styles.miniBtn}
              title={t('skills.openDataDir')}
              aria-label={t('skills.openDataDir')}
              onClick={handleOpenCentralPath}
            >
              <FolderOpen size={13} aria-hidden="true" />
            </button>
            <button
              type="button"
              className={styles.miniBtn}
              title={copyValue ? `${t('common.copy')}: ${copyValue}` : t('common.copy')}
              aria-label={copyValue ? `${t('common.copy')}: ${copyValue}` : t('common.copy')}
              onClick={handleCopy}
              disabled={!copyValue}
            >
              <Copy size={12} aria-hidden="true" />
            </button>
            {actionItems.length > 0 && (
              <ManagementMenu
                items={actionItems}
                title={t('skills.more')}
                triggerClassName={styles.miniBtn}
              >
                <MoreHorizontal size={14} aria-hidden="true" />
              </ManagementMenu>
            )}
            {!builtin && (
              <button
                type="button"
                className={styles.miniBtn}
                title={t('skills.updateTooltip')}
                aria-label={t('skills.updateTooltip')}
                onClick={() => onUpdate(skill)}
                disabled={loading || isUpdating || !skill.management_enabled}
              >
                <RefreshCw size={13} aria-hidden="true" />
              </button>
            )}
          </span>
        </div>

        <div className={styles.body}>
          {hasSkillDescription && (
            <p className={styles.skillDescription} title={skillDescriptionText}>{skillDescriptionText}</p>
          )}
          {(skillTagList.length > 0 || shouldShowGroupTag || hasUserNote) && (
            <div className={styles.tagRow}>
              {skillTagList.map((tagItem) => (
                <span key={tagItem} className={`${styles.tagPill} ${tagPillColorClass(tagItem)}`}>
                  <span className={styles.tagPillText} title={tagItem}>{tagItem}</span>
                </span>
              ))}
              {shouldShowGroupTag && (
                <span className={styles.groupTag} title={groupLabel}>{groupLabel}</span>
              )}
              {hasUserNote && (
                <span className={styles.note} title={userNoteText}>{userNoteText}</span>
              )}
            </div>
          )}
        </div>

        <div className={styles.footerRow}>
          {builtin ? (
            <span
              className={styles.builtinSource}
              title={`${t('skills.builtin.badge')} · ${t('skills.builtin.readOnlyHint')}`}
            >
              <span className={styles.builtinSourceLabel} aria-label={t('skills.builtin.sourceLabel')}>
                {iconNode}
                <span className={styles.sourceText}>{builtinVersionText}</span>
              </span>
            </span>
          ) : (
            <button
              type="button"
              className={`${styles.sourceBtn}${iconClickable ? '' : ` ${styles.sourceBtnStatic}`}`}
              title={iconTooltip ?? sourceLabel}
              onClick={iconClickable ? handleIconClick : undefined}
              disabled={!iconClickable}
            >
              {iconNode}
              <span className={styles.sourceText}>{sourceLabel}</span>
            </button>
          )}
          <span className={styles.footerTime}>{formatRelative(skill.updated_at)}</span>
          <span className={styles.footerTools}>
            {syncedTools.map((tool) => {
              const target = skill.targets.find((t) => t.tool === tool.id);
              return (
                <button
                  key={`${skill.id}-${tool.id}`}
                  title={`${tool.label} (${target?.mode ?? t('skills.unknown')})`}
                  type="button"
                  className={`${styles.toolPill} ${styles.active}${toolsReadOnly ? ` ${styles.readOnlyTool}` : ''}`}
                  onClick={toolsReadOnly ? handleReadOnlyToolClick : () => onToggleTool(skill, tool.id)}
                  disabled={loading || isUpdating || !skill.management_enabled}
                  aria-disabled={toolsReadOnly || loading || isUpdating || !skill.management_enabled}
                >
                  <ToolIcon toolKey={tool.id} label={tool.label} size={14} iconUrl={tool.iconUrl ?? undefined} />
                </button>
              );
            })}
            {!toolsReadOnly && dropdownItems.length > 0 && (
              <ManagementMenu
                items={dropdownItems}
                disabled={loading || isUpdating || !skill.management_enabled}
                title={t('skills.batch.addTool')}
                triggerClassName={styles.addToolBtn}
              >
                <Plus size={12} aria-hidden="true" />
              </ManagementMenu>
            )}
          </span>
        </div>
      </div>
    </div>
  );
});

const SortableSkillCard: React.FC<Omit<SkillCardProps, 'dragDisabled'>> = (props) => {
  const { t } = useTranslation();
  const {
    skill,
  } = props;

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: skill.id });

  const sortableStyle: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  return (
    <SkillCardContent
      {...props}
      containerRef={setNodeRef}
      containerStyle={sortableStyle}
      dragHandle={(
        <span
          {...attributes}
          {...listeners}
          className={styles.dragHandle}
          data-skill-card-no-detail
          title={t('skills.reorderHint')}
          aria-label={t('skills.reorderHint')}
        >
          <GripVertical size={14} aria-hidden="true" />
        </span>
      )}
    />
  );
};

export const SkillCard = React.memo(function SkillCard({
  dragDisabled,
  ...props
}: SkillCardProps) {
  if (dragDisabled) {
    return <SkillCardContent {...props} />;
  }

  // In sortable/reorder mode, keep the card click focused on drag/sort and do
  // not open the detail panel accidentally.
  const { onOpenDetail: _ignored, ...sortableProps } = props;
  return <SortableSkillCard {...sortableProps} />;
});
