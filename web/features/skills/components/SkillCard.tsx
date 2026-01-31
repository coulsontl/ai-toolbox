import React from 'react';
import { Button, Tooltip, message, Dropdown } from 'antd';
import {
  GithubOutlined,
  FolderOutlined,
  AppstoreOutlined,
  SyncOutlined,
  DeleteOutlined,
  CopyOutlined,
  PlusOutlined,
  HolderOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { ManagedSkill, ToolOption } from '../types';
import styles from './SkillCard.module.less';

interface SkillCardProps {
  skill: ManagedSkill;
  allTools: ToolOption[];
  loading: boolean;
  getGithubInfo: (url: string | null | undefined) => { label: string; href: string } | null;
  getSkillSourceLabel: (skill: ManagedSkill) => string;
  formatRelative: (ms: number | null | undefined) => string;
  onUpdate: (skill: ManagedSkill) => void;
  onDelete: (skillId: string) => void;
  onToggleTool: (skill: ManagedSkill, toolId: string) => void;
}

export const SkillCard: React.FC<SkillCardProps> = ({
  skill,
  allTools,
  loading,
  getGithubInfo,
  getSkillSourceLabel,
  formatRelative,
  onUpdate,
  onDelete,
  onToggleTool,
}) => {
  const { t } = useTranslation();

  // Drag-and-drop sortable
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: skill.id });

  const sortableStyle = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  const typeKey = skill.source_type.toLowerCase();
  const github = getGithubInfo(skill.source_ref);
  const copyValue = (github?.href ?? skill.source_ref ?? '').trim();

  const handleCopy = async () => {
    if (!copyValue) return;
    try {
      await navigator.clipboard.writeText(copyValue);
      message.success(t('skills.copied'));
    } catch {
      message.error(t('skills.copyFailed'));
    }
  };

  const iconNode = typeKey.includes('git') ? (
    <GithubOutlined className={styles.icon} />
  ) : typeKey.includes('local') ? (
    <FolderOutlined className={styles.icon} />
  ) : (
    <AppstoreOutlined className={styles.icon} />
  );

  // Split tools: synced tools (in skill.targets) vs unsynced tools
  const syncedToolIds = new Set(skill.targets.map((t) => t.tool));
  const syncedTools = allTools.filter((tool) => syncedToolIds.has(tool.id));
  const unsyncedTools = allTools.filter((tool) => !syncedToolIds.has(tool.id));

  // Sort unsynced tools: installed first, then uninstalled
  const sortedUnsyncedTools = [...unsyncedTools].sort((a, b) => {
    if (a.installed === b.installed) return 0;
    return a.installed ? -1 : 1;
  });

  const dropdownItems = sortedUnsyncedTools.map((tool) => ({
    key: tool.id,
    label: (
      <span>
        {tool.label}
        {!tool.installed && (
          <span className={styles.notInstalledTag}>{t('skills.notInstalled')}</span>
        )}
      </span>
    ),
    onClick: () => onToggleTool(skill, tool.id),
  }));

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <div className={styles.card}>
        <div
          className={styles.dragHandle}
          {...attributes}
          {...listeners}
        >
          <HolderOutlined />
        </div>
        <div className={styles.iconArea}>{iconNode}</div>
        <div className={styles.main}>
          <div className={styles.headerRow}>
            <div className={styles.name}>{skill.name}</div>
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
                disabled={loading}
              >
                <button type="button" className={styles.addToolBtn}>
                  <PlusOutlined />
                </button>
              </Dropdown>
            )}
          </div>
        </div>
        <div className={styles.actions}>
          <Button
            type="text"
            icon={<SyncOutlined />}
            onClick={() => onUpdate(skill)}
            disabled={loading}
            title={t('skills.updateTooltip')}
          />
          <Button
            type="text"
            danger
            icon={<DeleteOutlined />}
            onClick={() => onDelete(skill.id)}
            disabled={loading}
            title={t('skills.remove')}
          />
        </div>
      </div>
    </div>
  );
};
