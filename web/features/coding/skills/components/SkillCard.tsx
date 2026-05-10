import React from 'react';
import { Button, Tooltip, message, Dropdown, Checkbox } from 'antd';
import {
  GithubOutlined,
  FolderOutlined,
  AppstoreOutlined,
  SyncOutlined,
  DeleteOutlined,
  CopyOutlined,
  PlusOutlined,
  HolderOutlined,
  EyeOutlined,
  EllipsisOutlined,
  TagsOutlined,
} from '@ant-design/icons';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { ManagedSkill, ToolOption } from '../types';
import styles from './SkillCard.module.less';

interface SkillCardProps {
  skill: ManagedSkill;
  allTools: ToolOption[];
  loading: boolean;
  isUpdating?: boolean;
  dragDisabled?: boolean;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (skillId: string, checked: boolean) => void;
  getGithubInfo: (url: string | null | undefined) => { label: string; href: string } | null;
  getSkillSourceLabel: (skill: ManagedSkill) => string;
  formatRelative: (ms: number | null | undefined) => string;
  onUpdate: (skill: ManagedSkill) => void;
  onDelete: (skillId: string) => void;
  onToggleTool: (skill: ManagedSkill, toolId: string) => void;
  onEditMetadata: (skill: ManagedSkill) => void;
}

interface SkillCardContentProps extends Omit<SkillCardProps, 'dragDisabled'> {
  dragHandle?: React.ReactNode;
  containerRef?: (node: HTMLDivElement | null) => void;
  containerStyle?: React.CSSProperties;
}

const SkillCardContent: React.FC<SkillCardContentProps> = ({
  skill,
  allTools,
  loading,
  isUpdating = false,
  selectable,
  selected,
  onSelectChange,
  getGithubInfo,
  getSkillSourceLabel,
  formatRelative,
  onUpdate,
  onDelete,
  onToggleTool,
  onEditMetadata,
  dragHandle,
  containerRef,
  containerStyle,
}) => {
  const { t } = useTranslation();

  const typeKey = skill.source_type.toLowerCase();

  // These values are derived from stable inputs and are recalculated for every card.
  // Memoizing them keeps scroll and hover interactions cheaper when many cards are on screen.
  const github = React.useMemo(
    () => getGithubInfo(skill.source_ref),
    [getGithubInfo, skill.source_ref],
  );

  const copyValue = React.useMemo(
    () => (github?.href ?? skill.source_ref ?? '').trim(),
    [github, skill.source_ref],
  );

  const handleCopy = async () => {
    if (!copyValue) return;
    try {
      await navigator.clipboard.writeText(copyValue);
      message.success(t('skills.copied'));
    } catch {
      message.error(t('skills.copyFailed'));
    }
  };

  const handleIconClick = async () => {
    if (github) {
      await openUrl(github.href);
    } else if (skill.source_type === 'local' && skill.source_ref) {
      try {
        await revealItemInDir(skill.source_ref);
      } catch {
        message.error(t('skills.openFolderFailed'));
      }
    }
  };

  const handleOpenCentralPath = async () => {
    try {
      await revealItemInDir(`${skill.central_path}\\SKILL.md`);
    } catch {
      try {
        await revealItemInDir(skill.central_path);
      } catch {
        message.error(t('skills.openFolderFailed'));
      }
    }
  };

  const iconTooltip = React.useMemo(() => {
    if (github) {
      return t('skills.openRepo');
    }
    if (skill.source_type === 'local' && skill.source_ref) {
      return t('skills.openFolder');
    }
    return undefined;
  }, [github, skill.source_ref, skill.source_type, t]);

  const iconClickable = !!iconTooltip;

  const iconNode = typeKey.includes('git') ? (
    <GithubOutlined className={`${styles.icon}${iconClickable ? ` ${styles.clickableIcon}` : ''}`} />
  ) : typeKey.includes('local') ? (
    <FolderOutlined className={`${styles.icon}${iconClickable ? ` ${styles.clickableIcon}` : ''}`} />
  ) : (
    <AppstoreOutlined className={styles.icon} />
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
    return allTools.filter((tool) => tool.installed && !syncedToolIds.has(tool.id));
  }, [allTools, syncedToolIds]);

  // Dropdown items are also pure view data. Keep them memoized so large lists do not
  // recreate identical menu structures unless tools, translations, or handlers change.
  const dropdownItems = React.useMemo(
    () =>
      availableDropdownTools.map((tool) => ({
        key: tool.id,
        label: (
          <span>
            {tool.label}
          </span>
        ),
        onClick: () => onToggleTool(skill, tool.id),
      })),
    [availableDropdownTools, onToggleTool, skill],
  );

  const actionItems = React.useMemo(
    () => [
      {
        key: 'metadata',
        icon: <TagsOutlined />,
        label: t('skills.metadata.edit'),
        onClick: () => onEditMetadata(skill),
      },
      {
        key: 'delete',
        danger: true,
        icon: <DeleteOutlined />,
        label: t('skills.remove'),
        onClick: () => onDelete(skill.id),
      },
    ],
    [onDelete, onEditMetadata, skill, t],
  );

  return (
    <div ref={containerRef} style={containerStyle}>
      <div className={`${styles.card}${selectable && selected ? ` ${styles.selected}` : ''}`}>
        {selectable && (
          <div className={styles.checkboxArea}>
            <Checkbox
              checked={selected}
              onChange={(e) => onSelectChange?.(skill.id, e.target.checked)}
            />
          </div>
        )}
        {dragHandle}
        <Tooltip title={iconTooltip}>
          <div
            className={`${styles.iconArea}${iconClickable ? ` ${styles.clickableIconArea}` : ''}`}
            onClick={iconClickable ? handleIconClick : undefined}
          >
            {iconNode}
          </div>
        </Tooltip>
        <div className={styles.main}>
          <div className={styles.headerRow}>
            <div className={styles.name}>{skill.name}</div>
            <Tooltip title={t('skills.openDataDir')}>
              <EyeOutlined
                className={styles.detailIcon}
                onClick={handleOpenCentralPath}
              />
            </Tooltip>
            <Tooltip title={t('common.copy')}>
              <button
                className={styles.sourcePill}
                type="button"
                onClick={handleCopy}
                disabled={!copyValue}
              >
                <span className={styles.sourceText}>
                  {github ? github.label : getSkillSourceLabel(skill)}
                </span>
                <CopyOutlined className={styles.copyIcon} />
              </button>
            </Tooltip>
            <span className={styles.dot}>•</span>
            <span className={styles.time}>{formatRelative(skill.updated_at)}</span>
          </div>
          {(skill.user_group || skill.user_note) && (
            <div className={styles.metaRow}>
              {skill.user_group && (
                <Tooltip title={skill.user_group}>
                  <span className={styles.groupTag}>{skill.user_group}</span>
                </Tooltip>
              )}
              {skill.user_note && (
                <Tooltip title={skill.user_note}>
                  <span className={styles.note}>{skill.user_note}</span>
                </Tooltip>
              )}
            </div>
          )}
          <div className={styles.toolMatrix}>
            {syncedTools.map((tool) => {
              const target = skill.targets.find((t) => t.tool === tool.id);
              return (
                <Tooltip
                  key={`${skill.id}-${tool.id}`}
                  title={`${tool.label} (${target?.mode ?? t('skills.unknown')})`}
                >
                  <button
                    type="button"
                    className={`${styles.toolPill} ${styles.active}`}
                    onClick={() => onToggleTool(skill, tool.id)}
                    disabled={loading || isUpdating}
                  >
                    <span className={styles.statusBadge} />
                    {tool.label}
                  </button>
                </Tooltip>
              );
            })}
            {dropdownItems.length > 0 && (
              <Dropdown
                menu={{ items: dropdownItems }}
                trigger={['click']}
                disabled={loading || isUpdating}
              >
                <button
                  type="button"
                  className={styles.addToolBtn}
                  disabled={loading || isUpdating}
                >
                  <PlusOutlined />
                </button>
              </Dropdown>
            )}
          </div>
        </div>
        <div className={styles.actions}>
          <Dropdown
            menu={{ items: actionItems }}
            trigger={['click']}
            disabled={loading || isUpdating}
          >
            <Button
              type="text"
              icon={<EllipsisOutlined />}
              disabled={loading || isUpdating}
              title={t('skills.more')}
            />
          </Dropdown>
          <Button
            type="text"
            icon={<SyncOutlined />}
            onClick={() => onUpdate(skill)}
            loading={isUpdating}
            disabled={loading}
            title={t('skills.updateTooltip')}
          />
        </div>
      </div>
    </div>
  );
};

const SortableSkillCard: React.FC<Omit<SkillCardProps, 'dragDisabled'>> = (props) => {
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
        <div
          className={styles.dragHandle}
          {...attributes}
          {...listeners}
        >
          <HolderOutlined />
        </div>
      )}
    />
  );
};

export const SkillCard: React.FC<SkillCardProps> = ({
  dragDisabled,
  ...props
}) => {
  if (dragDisabled) {
    return <SkillCardContent {...props} />;
  }

  return <SortableSkillCard {...props} />;
};
