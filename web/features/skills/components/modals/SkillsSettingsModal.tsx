import React from 'react';
import { Modal, InputNumber, Button, Checkbox, Tag, message, Form, Input, Space, Tooltip } from 'antd';
import { FolderOpenOutlined, DeleteOutlined, PlusOutlined } from '@ant-design/icons';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import type { ToolInfo, CustomTool } from '../../types';
import * as api from '../../services/skillsApi';
import { useSkillsStore } from '../../stores/skillsStore';
import styles from './SkillsSettingsModal.module.less';

interface SkillsSettingsModalProps {
  open: boolean;
  onClose: () => void;
}

export const SkillsSettingsModal: React.FC<SkillsSettingsModalProps> = ({
  open: isOpen,
  onClose,
}) => {
  const { t } = useTranslation();
  const { loadToolStatus } = useSkillsStore();
  const [form] = Form.useForm();
  const [path, setPath] = React.useState('');
  const [cleanupDays, setCleanupDays] = React.useState(30);
  const [ttlSecs, setTtlSecs] = React.useState(60);
  const [loading, setLoading] = React.useState(false);
  const [clearingCache, setClearingCache] = React.useState(false);
  const [allTools, setAllTools] = React.useState<ToolInfo[]>([]);
  const [preferredTools, setPreferredTools] = React.useState<string[]>([]);
  const [customTools, setCustomTools] = React.useState<CustomTool[]>([]);
  const [addingTool, setAddingTool] = React.useState(false);
  const [showAddCustomModal, setShowAddCustomModal] = React.useState(false);

  // Load settings on open
  React.useEffect(() => {
    if (isOpen) {
      api.getCentralRepoPath().then(setPath).catch(console.error);
      api.getGitCacheCleanupDays().then(setCleanupDays).catch(console.error);
      api.getGitCacheTtlSecs().then(setTtlSecs).catch(console.error);
      loadCustomTools();

      // Load tools and preferred tools together
      Promise.all([api.getToolStatus(), api.getPreferredTools()])
        .then(([status, saved]) => {
          // Sort: installed tools first
          const sorted = [...status.tools].sort((a, b) => {
            if (a.installed === b.installed) return 0;
            return a.installed ? -1 : 1;
          });
          setAllTools(sorted);

          // null = never set before, default to all installed tools
          if (saved === null) {
            setPreferredTools(status.installed);
          } else {
            setPreferredTools(saved);
          }
        })
        .catch(console.error);
    }
  }, [isOpen]);

  const loadCustomTools = async () => {
    try {
      const tools = await api.getCustomTools();
      setCustomTools(tools);
    } catch (error) {
      console.error('Failed to load custom tools:', error);
    }
  };

  const handleOpenFolder = async () => {
    if (path) {
      try {
        await revealItemInDir(path);
      } catch (error) {
        message.error(String(error));
      }
    }
  };

  const handleToolToggle = (toolKey: string, checked: boolean) => {
    setPreferredTools((prev) =>
      checked ? [...prev, toolKey] : prev.filter((k) => k !== toolKey)
    );
  };

  // Sort tools: installed built-in > custom tools > not installed built-in
  const sortedTools = React.useMemo(() => {
    const customKeys = new Set(customTools.map(c => c.key));
    const installedBuiltin = allTools.filter(t => t.installed && !customKeys.has(t.key));
    const customToolItems = allTools.filter(t => customKeys.has(t.key));
    const notInstalledBuiltin = allTools.filter(t => !t.installed && !customKeys.has(t.key));
    return [...installedBuiltin, ...customToolItems, ...notInstalledBuiltin];
  }, [allTools, customTools]);

  const handleSave = async () => {
    setLoading(true);
    try {
      await api.setGitCacheCleanupDays(cleanupDays);
      await api.setPreferredTools(preferredTools);
      await loadToolStatus(); // Refresh global store
      message.success(t('common.success'));
      onClose();
    } catch (error) {
      message.error(String(error));
    } finally {
      setLoading(false);
    }
  };

  const handleClearCache = async () => {
    setClearingCache(true);
    try {
      const count = await api.clearGitCache();
      message.success(t('skills.status.gitCacheCleared', { count }));
    } catch (error) {
      message.error(String(error));
    } finally {
      setClearingCache(false);
    }
  };

  const handleOpenCacheFolder = async () => {
    try {
      const cachePath = await api.getGitCachePath();
      await revealItemInDir(cachePath);
    } catch (error) {
      message.error(String(error));
    }
  };

  const handleAddCustomTool = async (values: {
    key: string;
    displayName: string;
    relativeSkillsDir: string;
  }) => {
    setAddingTool(true);
    try {
      // Check if the skills path exists
      const pathExists = await api.checkCustomToolPath(values.relativeSkillsDir);

      if (!pathExists) {
        // Prompt user to create the directory
        Modal.confirm({
          title: t('skills.customToolSettings.pathNotExist'),
          content: t('skills.customToolSettings.pathNotExistMessage', { path: `~/${values.relativeSkillsDir.replace(/^~\//, '')}` }),
          okText: t('skills.customToolSettings.createPath'),
          cancelText: t('common.cancel'),
          onOk: async () => {
            try {
              await api.createCustomToolPath(values.relativeSkillsDir);
              await doAddCustomTool(values);
            } catch (error) {
              message.error(String(error));
            }
          },
        });
        setAddingTool(false);
        return;
      }

      await doAddCustomTool(values);
    } catch (error) {
      message.error(String(error));
      setAddingTool(false);
    }
  };

  const doAddCustomTool = async (values: {
    key: string;
    displayName: string;
    relativeSkillsDir: string;
  }) => {
    try {
      // Derive detectDir from skillsDir by taking the parent directory
      const parts = values.relativeSkillsDir.replace(/\/$/, '').split('/');
      const relativeDetectDir = parts.length > 1 ? parts.slice(0, -1).join('/') : values.relativeSkillsDir;

      await api.addCustomTool(
        values.key,
        values.displayName,
        values.relativeSkillsDir,
        relativeDetectDir
      );
      message.success(t('common.success'));
      form.resetFields();
      setShowAddCustomModal(false);
      await loadCustomTools();
      // Refresh tool status to include new custom tool
      await loadToolStatus(); // Update global store
      const status = await api.getToolStatus();
      const sorted = [...status.tools].sort((a, b) => {
        if (a.installed === b.installed) return 0;
        return a.installed ? -1 : 1;
      });
      setAllTools(sorted);
    } catch (error) {
      message.error(String(error));
    } finally {
      setAddingTool(false);
    }
  };

  const handleRemoveCustomTool = async (key: string) => {
    try {
      await api.removeCustomTool(key);
      message.success(t('common.success'));
      await loadCustomTools();
      // Refresh tool status
      await loadToolStatus(); // Update global store
      const status = await api.getToolStatus();
      const sorted = [...status.tools].sort((a, b) => {
        if (a.installed === b.installed) return 0;
        return a.installed ? -1 : 1;
      });
      setAllTools(sorted);
    } catch (error) {
      message.error(String(error));
    }
  };

  return (
    <Modal
      title={t('skills.settings')}
      open={isOpen}
      onCancel={onClose}
      footer={null}
      width={700}
      destroyOnClose
    >
      <div className={styles.section}>
        <div className={styles.labelArea}>
          <label className={styles.label}>{t('skills.skillsStoragePath')}</label>
        </div>
        <div className={styles.inputArea}>
          <div className={styles.pathRow}>
            <span className={styles.pathText}>{path}</span>
            <Button
              type="link"
              size="small"
              icon={<FolderOpenOutlined />}
              onClick={handleOpenFolder}
            />
          </div>
          <p className={styles.hint}>{t('skills.skillsStorageHint')}</p>
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.labelArea}>
          <label className={styles.label}>{t('skills.preferredTools')}</label>
        </div>
        <div className={styles.inputArea}>
          <div className={styles.toolList}>
            {sortedTools.map((tool) => {
              const isCustomTool = customTools.some(c => c.key === tool.key);
              const isDisabled = !tool.installed && !isCustomTool;
              return (
                <div key={tool.key} className={styles.toolItem}>
                  <Tooltip title={tool.skills_dir}>
                    <Checkbox
                      checked={preferredTools.includes(tool.key)}
                      onChange={(e) => handleToolToggle(tool.key, e.target.checked)}
                      disabled={isDisabled}
                    >
                      {tool.label}
                    </Checkbox>
                  </Tooltip>
                  {isDisabled && (
                    <Tag color="default">{t('skills.notInstalled')}</Tag>
                  )}
                  {isCustomTool && (
                    <Button
                      type="text"
                      size="small"
                      icon={<DeleteOutlined />}
                      danger
                      onClick={() => handleRemoveCustomTool(tool.key)}
                    />
                  )}
                </div>
              );
            })}
            <Button
              type="dashed"
              size="small"
              icon={<PlusOutlined />}
              onClick={() => setShowAddCustomModal(true)}
            >
              {t('skills.customToolSettings.add')}
            </Button>
          </div>
          <p className={styles.hint}>{t('skills.preferredToolsHint')}</p>
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.labelArea}>
          <label className={styles.label}>{t('skills.gitCacheCleanupDays')}</label>
        </div>
        <div className={styles.inputArea}>
          <Space size="small">
            <InputNumber
              min={0}
              max={365}
              value={cleanupDays}
              onChange={(v) => setCleanupDays(v || 0)}
              style={{ width: 120 }}
            />
            <Button
              type="link"
              size="small"
              icon={<DeleteOutlined />}
              onClick={handleClearCache}
              loading={clearingCache}
            >
              {t('skills.cleanNow')}
            </Button>
            <Button
              type="link"
              size="small"
              icon={<FolderOpenOutlined />}
              onClick={handleOpenCacheFolder}
            />
          </Space>
          <p className={styles.hint}>{t('skills.gitCacheCleanupHint')}</p>
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.labelArea}>
          <label className={styles.label}>{t('skills.gitCacheTtlSecs')}</label>
        </div>
        <div className={styles.inputArea}>
          <InputNumber
            min={0}
            max={3600}
            value={ttlSecs}
            onChange={(v) => setTtlSecs(v || 0)}
            style={{ width: 120 }}
          />
          <p className={styles.hint}>{t('skills.gitCacheTtlHint')}</p>
        </div>
      </div>

      <div className={styles.footer}>
        <Button onClick={onClose}>{t('common.cancel')}</Button>
        <Button type="primary" onClick={handleSave} loading={loading}>
          {t('common.save')}
        </Button>
      </div>

      <Modal
        title={t('skills.customToolSettings.addTitle')}
        open={showAddCustomModal}
        onCancel={() => setShowAddCustomModal(false)}
        footer={null}
        destroyOnClose
      >
        <Form form={form} layout="vertical" onFinish={handleAddCustomTool}>
          <Form.Item
            name="key"
            label={t('skills.customToolSettings.key')}
            rules={[
              { required: true, message: t('skills.customToolSettings.keyRequired') },
              { pattern: /^[a-z][a-z0-9_]*$/, message: t('skills.customToolSettings.keyHint') },
            ]}
          >
            <Input placeholder="my_tool" />
          </Form.Item>
          <Form.Item
            name="displayName"
            label={t('skills.customToolSettings.displayName')}
            rules={[{ required: true, message: t('skills.customToolSettings.displayNameRequired') }]}
          >
            <Input placeholder="My Tool" />
          </Form.Item>
          <Form.Item
            name="relativeSkillsDir"
            label={t('skills.customToolSettings.skillsDir')}
            rules={[{ required: true, message: t('skills.customToolSettings.skillsDirRequired') }]}
          >
            <Input placeholder="~/.mytool/skills" />
          </Form.Item>
          <div style={{ textAlign: 'right' }}>
            <Space>
              <Button onClick={() => setShowAddCustomModal(false)}>{t('common.cancel')}</Button>
              <Button type="primary" htmlType="submit" loading={addingTool}>{t('common.add')}</Button>
            </Space>
          </div>
        </Form>
      </Modal>
    </Modal>
  );
};
