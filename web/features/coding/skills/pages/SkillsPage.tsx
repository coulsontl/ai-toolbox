import React from 'react';
import {
  AutoComplete,
  Typography,
  Button,
  Space,
  Input,
  Segmented,
  Modal,
  Dropdown,
  Tooltip,
} from 'antd';
import {
  PlusOutlined,
  EllipsisOutlined,
  ImportOutlined,
  LinkOutlined,
  AppstoreOutlined,
  BarsOutlined,
  SyncOutlined,
  DeleteOutlined,
  PlusCircleOutlined,
  MinusCircleOutlined,
  DownOutlined,
  UpOutlined,
  DragOutlined,
  TagsOutlined,
} from '@ant-design/icons';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useSkillsStore } from '../stores/skillsStore';
import { useSkills } from '../hooks/useSkills';
import { useSkillActions } from '../hooks/useSkillActions';
import { SkillsList } from '../components/SkillsList';
import { SkillsGroupedList } from '../components/SkillsGroupedList';
import { AddSkillModal } from '../components/modals/AddSkillModal';
import { ImportModal } from '../components/modals/ImportModal';
import { SkillsSettingsModal } from '../components/modals/SkillsSettingsModal';
import { DeleteConfirmModal } from '../components/modals/DeleteConfirmModal';
import { NewToolsModal } from '../components/modals/NewToolsModal';
import { SkillMetadataModal } from '../components/modals/SkillMetadataModal';
import {
  buildSkillGroups,
  filterSkillsBySearch,
  getSkillGroupOptions,
  normalizeSkillMetadataText,
  type SkillGroupingMode,
} from '../utils/skillGrouping';
import type { ManagedSkill, SkillGroup } from '../types';
import styles from './SkillsPage.module.less';

const { Title, Text, Link } = Typography;
const AUTO_EXPAND_SKILL_THRESHOLD = 20;

const SkillsPage: React.FC = () => {
  const { t } = useTranslation();
  const {
    isAddModalOpen,
    setAddModalOpen,
    isImportModalOpen,
    setImportModalOpen,
    isSettingsModalOpen,
    setSettingsModalOpen,
    isNewToolsModalOpen,
    loading,
  } = useSkillsStore();

  const {
    skills,
    getAllTools,
    formatRelative,
    getGithubInfo,
    getSkillSourceLabel,
    refresh,
  } = useSkills();

  const [searchText, setSearchText] = React.useState('');
  const [viewMode, setViewMode] = React.useState<'flat' | 'grouped'>('flat');
  const [groupMode, setGroupMode] = React.useState<SkillGroupingMode>('custom');
  const [groupActiveKeys, setGroupActiveKeys] = React.useState<string[]>([]);
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(new Set());
  const [reorderMode, setReorderMode] = React.useState(false);
  const [metadataSkill, setMetadataSkill] = React.useState<ManagedSkill | null>(null);
  const [batchGroupModalOpen, setBatchGroupModalOpen] = React.useState(false);
  const [batchGroupValue, setBatchGroupValue] = React.useState('');
  const previousViewModeRef = React.useRef<'flat' | 'grouped'>('flat');
  const previousAutoExpandRef = React.useRef(false);

  // Initialize data on mount
  React.useEffect(() => {
    refresh();
  }, []);

  const allTools = getAllTools();

  const {
    actionLoading,
    updatingSkillIds,
    deleteSkillId,
    setDeleteSkillId,
    skillToDelete,
    batchDeleteIds,
    setBatchDeleteIds,
    handleToggleTool,
    handleUpdate,
    handleDelete,
    confirmDelete,
    handleDragEnd,
    handleBatchRefresh,
    handleBatchDelete,
    confirmBatchDelete,
    handleBatchAddTool,
    handleBatchRemoveTool,
    handleBatchSetGroup,
  } = useSkillActions({ allTools });

  // Filter skills by search text
  const filteredSkills = React.useMemo(() => {
    return filterSkillsBySearch(skills, searchText);
  }, [skills, searchText]);

  const isSearchActive = !!searchText.trim();
  const isFlatReorderEnabled = viewMode === 'flat' && reorderMode && !isSearchActive;
  const groupOptions = React.useMemo(() => getSkillGroupOptions(skills), [skills]);
  const groupOptionItems = React.useMemo(
    () => groupOptions.map((group) => ({ value: group })),
    [groupOptions],
  );

  React.useEffect(() => {
    if (viewMode !== 'flat' || isSearchActive) {
      setReorderMode(false);
    }
  }, [viewMode, isSearchActive]);

  // Clear selection when switching view mode or when skills change
  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      setSelectedIds(new Set());
    } else {
      setSelectedIds((prev) => {
        const allSkillIds = new Set(filteredSkills.map((s) => s.id));
        const next = new Set([...prev].filter((id) => allSkillIds.has(id)));
        return next.size === prev.size ? prev : next;
      });
    }
  }, [viewMode, filteredSkills]);

  const handleSelectChange = React.useCallback((skillId: string, checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (checked) {
        next.add(skillId);
      } else {
        next.delete(skillId);
      }
      return next;
    });
  }, []);

  const handleSelectAllGroup = React.useCallback((group: SkillGroup, checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const skill of group.skills) {
        if (checked) {
          next.add(skill.id);
        } else {
          next.delete(skill.id);
        }
      }
      return next;
    });
  }, []);

  const selectedArray = React.useMemo(() => [...selectedIds], [selectedIds]);
  const hasSelection = selectedArray.length > 0;
  const installedTools = React.useMemo(() => allTools.filter((tool) => tool.installed), [allTools]);

  const handleConfirmBatchGroup = React.useCallback(async () => {
    const saved = await handleBatchSetGroup(
      selectedArray,
      normalizeSkillMetadataText(batchGroupValue),
    );
    if (!saved) {
      return;
    }
    setBatchGroupModalOpen(false);
    setBatchGroupValue('');
    setSelectedIds(new Set());
  }, [batchGroupValue, handleBatchSetGroup, selectedArray]);

  const groupedSkills = React.useMemo<SkillGroup[]>(() => {
    if (viewMode !== 'grouped') return [];

    return buildSkillGroups(
      filteredSkills,
      groupMode,
      {
        groupLocal: t('skills.groupLocal'),
        groupImport: t('skills.groupImport'),
        groupUngrouped: t('skills.groupUngrouped'),
      },
      getGithubInfo,
    );
  }, [filteredSkills, viewMode, groupMode, getGithubInfo, t]);

  const shouldAutoExpandGroups =
    filteredSkills.length > 0 && filteredSkills.length < AUTO_EXPAND_SKILL_THRESHOLD;

  // Entering grouped view or crossing the auto-expand threshold applies the default strategy once.
  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      previousViewModeRef.current = viewMode;
      previousAutoExpandRef.current = false;
      return;
    }

    const enteredGroupedView = previousViewModeRef.current !== 'grouped';
    const autoExpandChanged = previousAutoExpandRef.current !== shouldAutoExpandGroups;
    previousViewModeRef.current = viewMode;
    previousAutoExpandRef.current = shouldAutoExpandGroups;
    if (!enteredGroupedView && !autoExpandChanged) {
      return;
    }

    if (shouldAutoExpandGroups) {
      setGroupActiveKeys(groupedSkills.map((group) => group.key));
      return;
    }

    setGroupActiveKeys([]);
  }, [groupedSkills, shouldAutoExpandGroups, viewMode]);

  // Refreshes should only prune removed groups, not overwrite user-expanded state.
  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      return;
    }

    const validGroupKeys = new Set(groupedSkills.map((group) => group.key));
    setGroupActiveKeys((previousKeys) => {
      const nextKeys = previousKeys.filter((key) => validGroupKeys.has(key));
      return nextKeys.length === previousKeys.length ? previousKeys : nextKeys;
    });
  }, [groupedSkills, viewMode]);

  return (
    <div className={styles.skillsPage}>
      <div className={styles.pageHeader}>
        <div>
          <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
            {t('skills.title')}
          </Title>
          <Link
            type="secondary"
            style={{ fontSize: 12 }}
            onClick={(e) => {
              e.stopPropagation();
              openUrl('https://code.claude.com/docs/en/skills');
            }}
          >
            <LinkOutlined /> {t('skills.viewDocs')}
          </Link>
        </div>
        <Button
          type="text"
          icon={<EllipsisOutlined />}
          onClick={() => setSettingsModalOpen(true)}
        >
          {t('skills.settings')}
        </Button>
      </div>

      <Text type="secondary" style={{ fontSize: 12, marginBottom: 16, marginTop: -16 }}>
        {t('skills.pageHint')}
      </Text>

      <div className={styles.toolbar}>
        <Space size={8}>
          <Input.Search
            placeholder={t('skills.searchPlaceholder')}
            allowClear
            style={{ width: 200 }}
            value={searchText}
            onChange={(e) => setSearchText(e.target.value)}
          />
          <Button
            type="text"
            icon={<ImportOutlined />}
            onClick={() => setImportModalOpen(true)}
            style={{ color: 'var(--color-text-tertiary)' }}
          >
            {t('skills.importExisting')}
          </Button>
          <Button
            type="link"
            icon={<PlusOutlined />}
            onClick={() => setAddModalOpen(true)}
          >
            {t('skills.addSkill')}
          </Button>
        </Space>
        <Space size={4}>
          {viewMode === 'flat' && (
            <Tooltip
              title={
                isSearchActive
                  ? t('skills.reorderDisabledWhileSearching')
                  : t('skills.reorderHint')
              }
            >
              <Button
                type={reorderMode ? 'primary' : 'text'}
                size="small"
                icon={<DragOutlined />}
                className={styles.reorderButton}
                onClick={() => setReorderMode((prev) => !prev)}
                disabled={isSearchActive}
              >
                {t('skills.reorder')}
              </Button>
            </Tooltip>
          )}
          {viewMode === 'grouped' && (
            <>
              <Tooltip title={hasSelection ? t('skills.batch.refresh') : t('skills.batch.noneSelected')}>
                <Button
                  type="text"
                  size="small"
                  icon={<SyncOutlined />}
                  disabled={!hasSelection || loading || actionLoading}
                  onClick={() => handleBatchRefresh(selectedArray)}
                />
              </Tooltip>
              <Dropdown
                menu={{
                  items: installedTools.map((tool) => ({
                    key: `add-${tool.id}`,
                    label: tool.label,
                    onClick: () => handleBatchAddTool(selectedArray, tool.id),
                  })),
                }}
                trigger={['click']}
                disabled={!hasSelection || loading || actionLoading}
              >
                <Tooltip title={hasSelection ? t('skills.batch.addTool') : t('skills.batch.noneSelected')}>
                  <Button
                    type="text"
                    size="small"
                    icon={<PlusCircleOutlined />}
                    disabled={!hasSelection || loading || actionLoading}
                  />
                </Tooltip>
              </Dropdown>
              <Dropdown
                menu={{
                  items: installedTools.map((tool) => ({
                    key: `remove-${tool.id}`,
                    label: tool.label,
                    onClick: () => handleBatchRemoveTool(selectedArray, tool.id),
                  })),
                }}
                trigger={['click']}
                disabled={!hasSelection || loading || actionLoading}
              >
                <Tooltip title={hasSelection ? t('skills.batch.removeTool') : t('skills.batch.noneSelected')}>
                  <Button
                    type="text"
                    size="small"
                    icon={<MinusCircleOutlined />}
                    disabled={!hasSelection || loading || actionLoading}
                  />
                </Tooltip>
              </Dropdown>
              <Tooltip title={hasSelection ? t('skills.batch.setGroup') : t('skills.batch.noneSelected')}>
                <Button
                  type="text"
                  size="small"
                  icon={<TagsOutlined />}
                  disabled={!hasSelection || loading || actionLoading}
                  onClick={() => {
                    setBatchGroupValue('');
                    setBatchGroupModalOpen(true);
                  }}
                />
              </Tooltip>
              <Tooltip title={hasSelection ? t('skills.batch.delete') : t('skills.batch.noneSelected')}>
                <Button
                  type="text"
                  size="small"
                  danger
                  icon={<DeleteOutlined />}
                  disabled={!hasSelection || loading || actionLoading}
                  onClick={() => handleBatchDelete(selectedArray)}
                />
              </Tooltip>
              <span className={styles.batchDivider} />
            </>
          )}
          {viewMode === 'grouped' && (
            <Segmented
              size="small"
              value={groupMode}
              onChange={(v) => setGroupMode(v as SkillGroupingMode)}
              options={[
                { value: 'custom', label: t('skills.groupByCustom') },
                { value: 'source', label: t('skills.groupBySource') },
              ]}
            />
          )}
          {viewMode === 'grouped' && (
            <>
              <Tooltip title={t('skills.expandAll')}>
                <Button
                  type="text"
                  size="small"
                  icon={<DownOutlined />}
                  onClick={() => setGroupActiveKeys(groupedSkills.map((g) => g.key))}
                />
              </Tooltip>
              <Tooltip title={t('skills.collapseAll')}>
                <Button
                  type="text"
                  size="small"
                  icon={<UpOutlined />}
                  onClick={() => setGroupActiveKeys([])}
                />
              </Tooltip>
            </>
          )}
          <Tooltip title={t('skills.groupedViewTip')}>
            <Segmented
              size="small"
              value={viewMode}
              onChange={(v) => setViewMode(v as 'flat' | 'grouped')}
              options={[
                { value: 'flat', icon: <AppstoreOutlined />, label: t('skills.viewFlat') },
                { value: 'grouped', icon: <BarsOutlined />, label: t('skills.viewGrouped') },
              ]}
            />
          </Tooltip>
        </Space>
      </div>

      <div className={styles.content}>
        {viewMode === 'flat' ? (
          <SkillsList
            skills={filteredSkills}
            allTools={allTools}
            loading={loading || actionLoading}
            updatingSkillIds={updatingSkillIds}
            dragDisabled={!isFlatReorderEnabled}
            getGithubInfo={getGithubInfo}
            getSkillSourceLabel={getSkillSourceLabel}
            formatRelative={formatRelative}
            onUpdate={handleUpdate}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onEditMetadata={setMetadataSkill}
            onDragEnd={handleDragEnd}
          />
        ) : (
          <SkillsGroupedList
            groups={groupedSkills}
            allTools={allTools}
            loading={loading || actionLoading}
            updatingSkillIds={updatingSkillIds}
            activeKeys={groupActiveKeys}
            onActiveKeysChange={setGroupActiveKeys}
            selectedIds={selectedIds}
            onSelectChange={handleSelectChange}
            onSelectAllGroup={handleSelectAllGroup}
            getGithubInfo={getGithubInfo}
            getSkillSourceLabel={getSkillSourceLabel}
            formatRelative={formatRelative}
            onUpdate={handleUpdate}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onEditMetadata={setMetadataSkill}
          />
        )}
      </div>

      {isAddModalOpen && (
        <AddSkillModal
          open={isAddModalOpen}
          onClose={() => setAddModalOpen(false)}
          allTools={allTools}
          onSuccess={() => {
            setAddModalOpen(false);
            refresh();
          }}
        />
      )}

      {isImportModalOpen && (
        <ImportModal
          open={isImportModalOpen}
          onClose={() => setImportModalOpen(false)}
          onSuccess={() => {
            setImportModalOpen(false);
            refresh();
          }}
        />
      )}

      {isSettingsModalOpen && (
        <SkillsSettingsModal
          open={isSettingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
        />
      )}

      <DeleteConfirmModal
        open={!!deleteSkillId}
        skillName={skillToDelete?.name || ''}
        onClose={() => setDeleteSkillId(null)}
        onConfirm={confirmDelete}
        loading={actionLoading}
      />

      <Modal
        open={batchDeleteIds.length > 0}
        title={t('skills.batch.deleteConfirmTitle')}
        onCancel={() => setBatchDeleteIds([])}
        onOk={confirmBatchDelete}
        okButtonProps={{ danger: true, loading: actionLoading }}
        okText={t('skills.batch.delete')}
      >
        {t('skills.batch.deleteConfirmMessage', { count: batchDeleteIds.length })}
      </Modal>

      <Modal
        open={batchGroupModalOpen}
        title={t('skills.batch.setGroupTitle')}
        onCancel={() => setBatchGroupModalOpen(false)}
        onOk={handleConfirmBatchGroup}
        okButtonProps={{ loading: actionLoading }}
        okText={t('common.save')}
        cancelText={t('common.cancel')}
      >
        <div className={styles.batchGroupEditor}>
          <AutoComplete
            allowClear
            value={batchGroupValue}
            options={groupOptionItems}
            placeholder={t('skills.metadata.groupPlaceholder')}
            onChange={setBatchGroupValue}
            filterOption={(inputValue, option) =>
              String(option?.value ?? '').toLowerCase().includes(inputValue.toLowerCase())}
          />
          <Text type="secondary" className={styles.batchGroupHint}>
            {t('skills.batch.setGroupHint')}
          </Text>
        </div>
      </Modal>

      <SkillMetadataModal
        open={!!metadataSkill}
        skill={metadataSkill}
        groupOptions={groupOptions}
        onClose={() => setMetadataSkill(null)}
        onSuccess={() => {
          setMetadataSkill(null);
          refresh();
        }}
      />

      <NewToolsModal
        open={isNewToolsModalOpen}
      />
    </div>
  );
};

export default SkillsPage;
