import React from 'react';
import {
  Modal,
  Form,
  Input,
  Radio,
  Space,
  Button,
  InputNumber,
  Switch,
  Divider,
  message,
  List,
  Tag,
  Tooltip,
  Select,
  Popconfirm,
  Typography,
  type RadioChangeEvent,
} from 'antd';
import {
  DeleteOutlined,
  EditOutlined,
  FileOutlined,
  FolderOutlined,
  FolderOpenOutlined,
  PlusOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettingsStore, type WebDAVConfigFE } from '@/stores';
import {
  listBackupFileFilterPathOptions,
  normalizeBackupCustomEntryPath,
  testWebDAVConnection,
  type BackupCustomEntry,
  type BackupCustomEntryType,
  type BackupFileFilterPathOption,
  type BackupFileFilterRule,
} from '@/services';

interface BackupSettingsModalProps {
  open: boolean;
  onClose: () => void;
}

interface BackupCustomEntryFormValues {
  name: string;
  entryType: BackupCustomEntryType;
  sourcePath: string;
  restorePath?: string;
}

interface FileFilterRuleFormValues {
  tool: string;
  filePath: string;
}

const TOOL_ORDER = ['opencode', 'claude', 'codex', 'grok', 'openclaw', 'geminicli', 'pi'];

const BackupSettingsModal: React.FC<BackupSettingsModalProps> = ({
  open: isOpen,
  onClose,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [customEntryForm] = Form.useForm<BackupCustomEntryFormValues>();
  const {
    backupType,
    localBackupPath,
    webdav,
    backupImageAssetsEnabled,
    backupCustomEntries,
    backupFileFilterRules,
    setBackupSettings,
    autoBackupEnabled,
    autoBackupIntervalDays,
    autoBackupMaxKeep,
    setAutoBackupSettings,
  } = useSettingsStore();

  const [currentBackupType, setCurrentBackupType] = React.useState<'local' | 'webdav'>(backupType);
  const [currentLocalPath, setCurrentLocalPath] = React.useState(localBackupPath);
  const [testingConnection, setTestingConnection] = React.useState(false);
  const [currentBackupImageAssetsEnabled, setCurrentBackupImageAssetsEnabled] =
    React.useState(backupImageAssetsEnabled);
  const [currentAutoBackupEnabled, setCurrentAutoBackupEnabled] = React.useState(autoBackupEnabled);
  const [currentIntervalDays, setCurrentIntervalDays] = React.useState(autoBackupIntervalDays);
  const [currentMaxKeep, setCurrentMaxKeep] = React.useState(autoBackupMaxKeep);
  const [currentBackupCustomEntries, setCurrentBackupCustomEntries] =
    React.useState<BackupCustomEntry[]>(backupCustomEntries);
  const [customEntryModalOpen, setCustomEntryModalOpen] = React.useState(false);
  const [editingCustomEntry, setEditingCustomEntry] = React.useState<BackupCustomEntry | null>(null);
  const [customEntryType, setCustomEntryType] = React.useState<BackupCustomEntryType>('file');

  // File filter rules state
  const [filterRuleForm] = Form.useForm<FileFilterRuleFormValues>();
  const [currentFileFilterRules, setCurrentFileFilterRules] =
    React.useState<BackupFileFilterRule[]>(backupFileFilterRules);
  const [filterRuleModalOpen, setFilterRuleModalOpen] = React.useState(false);
  const [filterPathOptions, setFilterPathOptions] = React.useState<BackupFileFilterPathOption[]>(
    []
  );
  const [filterPathOptionsLoading, setFilterPathOptionsLoading] = React.useState(false);
  const selectedFilterRuleTool = Form.useWatch('tool', filterRuleForm) as string | undefined;

  const loadFilterPathOptions = React.useCallback(async () => {
    setFilterPathOptionsLoading(true);
    try {
      setFilterPathOptions(await listBackupFileFilterPathOptions());
    } catch (error) {
      console.error('Failed to load backup file filter path options:', error);
      setFilterPathOptions([]);
    } finally {
      setFilterPathOptionsLoading(false);
    }
  }, []);

  const filterToolOptions = React.useMemo(() => {
    const tools = Array.from(new Set(filterPathOptions.map((option) => option.tool)));
    tools.sort((a, b) => {
      const indexA = TOOL_ORDER.indexOf(a);
      const indexB = TOOL_ORDER.indexOf(b);
      if (indexA === -1 && indexB === -1) return a.localeCompare(b);
      if (indexA === -1) return 1;
      if (indexB === -1) return -1;
      return indexA - indexB;
    });

    return tools.map((tool) => ({
      value: tool,
      label: tool,
    }));
  }, [filterPathOptions]);

  const selectedFilterPathOptions = React.useMemo(
    () => filterPathOptions.filter((option) => option.tool === selectedFilterRuleTool),
    [filterPathOptions, selectedFilterRuleTool]
  );

  const selectedToolExistingFilterPaths = React.useMemo(() => {
    if (!selectedFilterRuleTool) return new Set<string>();

    return new Set(
      currentFileFilterRules
        .filter((rule) => rule.tool === selectedFilterRuleTool)
        .map((rule) => rule.file_path)
    );
  }, [currentFileFilterRules, selectedFilterRuleTool]);

  React.useEffect(() => {
    if (isOpen) {
      setCurrentBackupType(backupType);
      setCurrentLocalPath(localBackupPath);
      setCurrentBackupImageAssetsEnabled(backupImageAssetsEnabled);
      setCurrentAutoBackupEnabled(autoBackupEnabled);
      setCurrentIntervalDays(autoBackupIntervalDays);
      setCurrentMaxKeep(autoBackupMaxKeep);
      setCurrentBackupCustomEntries(backupCustomEntries);
      setCurrentFileFilterRules(backupFileFilterRules);
      form.setFieldsValue({
        backupType,
        webdav,
      });
      void loadFilterPathOptions();
    }
  }, [
    isOpen,
    backupType,
    localBackupPath,
    webdav,
    backupImageAssetsEnabled,
    backupCustomEntries,
    backupFileFilterRules,
    autoBackupEnabled,
    autoBackupIntervalDays,
    autoBackupMaxKeep,
    form,
    loadFilterPathOptions,
  ]);

  const handleSelectFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t('settings.backupSettings.selectFolder'),
      });
      if (selected) {
        setCurrentLocalPath(selected as string);
      }
    } catch {
      // User cancelled
    }
  };

  const handleSave = async () => {
    try {
      const values = await form.validateFields();
      await setBackupSettings({
        backupType: currentBackupType,
        localBackupPath: currentLocalPath,
        webdav: values.webdav as Partial<WebDAVConfigFE>,
        backupImageAssetsEnabled: currentBackupImageAssetsEnabled,
        backupCustomEntries: currentBackupCustomEntries,
        backupFileFilterRules: currentFileFilterRules,
      });
      await setAutoBackupSettings({
        enabled: currentAutoBackupEnabled,
        intervalDays: currentIntervalDays,
        maxKeep: currentMaxKeep,
      });
      onClose();
    } catch {
      // Validation failed
    }
  };

  const handleBackupTypeChange = (e: RadioChangeEvent) => {
    setCurrentBackupType(e.target.value as 'local' | 'webdav');
  };

  const normalizeCustomEntryPath = async (path: string): Promise<string> => {
    const trimmedPath = path.trim();
    if (!trimmedPath) {
      return '';
    }

    try {
      return await normalizeBackupCustomEntryPath(trimmedPath);
    } catch (error) {
      console.error('Failed to normalize custom backup path:', error);
      return trimmedPath;
    }
  };

  const handleOpenCustomEntryModal = (entry?: BackupCustomEntry) => {
    const nextEntryType = entry?.entry_type ?? 'file';
    setEditingCustomEntry(entry ?? null);
    setCustomEntryType(nextEntryType);
    customEntryForm.setFieldsValue({
      name: entry?.name ?? '',
      entryType: nextEntryType,
      sourcePath: entry?.source_path ?? '',
      restorePath: entry?.restore_path ?? '',
    });
    setCustomEntryModalOpen(true);
  };

  const handleSelectCustomEntrySource = async () => {
    try {
      const selected = await open({
        directory: customEntryType === 'directory',
        multiple: false,
        title: customEntryType === 'directory'
          ? t('settings.backupSettings.customEntries.selectDirectory')
          : t('settings.backupSettings.customEntries.selectFile'),
      });
      if (selected && typeof selected === 'string') {
        customEntryForm.setFieldsValue({
          sourcePath: await normalizeCustomEntryPath(selected),
        });
      }
    } catch {
      // User cancelled
    }
  };

  const handleSaveCustomEntry = async () => {
    try {
      const values = await customEntryForm.validateFields();
      const sourcePath = await normalizeCustomEntryPath(values.sourcePath);
      const restorePath = values.restorePath?.trim()
        ? await normalizeCustomEntryPath(values.restorePath)
        : null;
      const nextEntry: BackupCustomEntry = {
        id: editingCustomEntry?.id ?? `custom-backup-${Date.now()}`,
        name: values.name.trim(),
        source_path: sourcePath,
        restore_path: restorePath,
        entry_type: values.entryType,
        enabled: editingCustomEntry?.enabled ?? true,
      };

      setCurrentBackupCustomEntries((entries) => {
        if (!editingCustomEntry) {
          return [...entries, nextEntry];
        }
        return entries.map((entry) => entry.id === editingCustomEntry.id ? nextEntry : entry);
      });
      setCustomEntryModalOpen(false);
    } catch {
      // Validation failed
    }
  };

  const handleToggleCustomEntry = (entryId: string, enabled: boolean) => {
    setCurrentBackupCustomEntries((entries) => entries.map((entry) => (
      entry.id === entryId ? { ...entry, enabled } : entry
    )));
  };

  const handleDeleteCustomEntry = (entryId: string) => {
    setCurrentBackupCustomEntries((entries) => entries.filter((entry) => entry.id !== entryId));
  };

  // File filter rule handlers
  const handleDeleteFilterRule = (index: number) => {
    setCurrentFileFilterRules((rules) => rules.filter((_, i) => i !== index));
  };

  const handleOpenFilterRuleModal = () => {
    filterRuleForm.resetFields();
    void loadFilterPathOptions();
    setFilterRuleModalOpen(true);
  };

  const handleSaveFilterRule = async () => {
    try {
      const values = await filterRuleForm.validateFields();
      const newRule: BackupFileFilterRule = {
        tool: values.tool,
        file_path: values.filePath.trim(),
      };

      setCurrentFileFilterRules((rules) => (
        rules.some((rule) => rule.tool === newRule.tool && rule.file_path === newRule.file_path)
          ? rules
          : [...rules, newRule]
      ));
      setFilterRuleModalOpen(false);
    } catch {
      // Validation failed
    }
  };

  const handleTestConnection = async () => {
    try {
      const values = await form.validateFields(['webdav']);
      const webdavConfig = values.webdav as Partial<WebDAVConfigFE>;

      if (!webdavConfig.url) {
        message.warning(t('settings.webdav.errors.checkUrl'));
        return;
      }

      setTestingConnection(true);
      await testWebDAVConnection(
        webdavConfig.url,
        webdavConfig.username || '',
        webdavConfig.password || '',
        webdavConfig.remotePath || ''
      );
      message.success(t('settings.webdav.testSuccess'));
    } catch (error) {
      console.error('WebDAV connection test failed:', error);

      // Parse error if it's JSON
      let errorMessage = String(error);
      try {
        const errorObj = JSON.parse(String(error));
        if (errorObj.suggestion) {
          errorMessage = t(errorObj.suggestion);
        }
      } catch {
        // Not JSON, use as is
      }

      message.error(`${t('settings.webdav.testFailed')}: ${errorMessage}`);
    } finally {
      setTestingConnection(false);
    }
  };

  return (
    <>
    <Modal
      title={t('settings.backupSettings.title')}
      open={isOpen}
      onOk={handleSave}
      onCancel={onClose}
      width={640}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form form={form} layout="horizontal" size="small" labelCol={{ span: 6 }} wrapperCol={{ span: 18 }}>
        <Form.Item label={t('settings.backupSettings.storageType')}>
          <Radio.Group value={currentBackupType} onChange={handleBackupTypeChange}>
            <Radio value="local">{t('settings.backupSettings.local')}</Radio>
            <Radio value="webdav">{t('settings.backupSettings.webdav')}</Radio>
          </Radio.Group>
        </Form.Item>

        {currentBackupType === 'local' && (
          <Form.Item label={t('settings.backupSettings.localPath')}>
            <Space.Compact style={{ width: '100%' }}>
              <Input
                value={currentLocalPath}
                readOnly
                placeholder={t('settings.backupSettings.selectFolder')}
                style={{ flex: 1 }}
              />
              <Button icon={<FolderOpenOutlined />} onClick={handleSelectFolder} style={{ fontSize: 14 }}>
                {t('common.browse')}
              </Button>
            </Space.Compact>
          </Form.Item>
        )}

        {currentBackupType === 'webdav' && (
          <>
            <Form.Item label={t('settings.webdav.url')} name={['webdav', 'url']}>
              <Input placeholder="https://dav.example.com" />
            </Form.Item>
            <Form.Item label={t('settings.webdav.username')} name={['webdav', 'username']}>
              <Input />
            </Form.Item>
            <Form.Item label={t('settings.webdav.password')} name={['webdav', 'password']}>
              <Input.Password visibilityToggle />
            </Form.Item>
            <Form.Item label={t('settings.webdav.remotePath')} name={['webdav', 'remotePath']}>
              <Input placeholder="/backup" />
            </Form.Item>
            <Form.Item label={t('settings.webdav.hostLabel')} name={['webdav', 'hostLabel']}>
              <Input placeholder={t('settings.webdav.hostLabelPlaceholder')} />
            </Form.Item>
            <Form.Item wrapperCol={{ offset: 6, span: 18 }}>
              <Button
                onClick={handleTestConnection}
                loading={testingConnection}
              >
                {testingConnection ? t('settings.webdav.testing') : t('settings.webdav.testConnection')}
              </Button>
            </Form.Item>
          </>
        )}

        <Form.Item label={t('settings.backupSettings.imageAssets')}>
          <Switch
            checked={currentBackupImageAssetsEnabled}
            onChange={setCurrentBackupImageAssetsEnabled}
          />
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.customEntries.title')}
          colon={false}
        >
          <Space direction="vertical" style={{ width: '100%' }} size="small">
            <div style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              gap: 12,
            }}>
              <Space direction="vertical" size={0} style={{ minWidth: 0 }}>
                <Typography.Text type="secondary">
                  {t('settings.backupSettings.customEntries.description')}
                </Typography.Text>
                {currentBackupCustomEntries.length === 0 && (
                  <Typography.Text type="secondary">
                    {t('settings.backupSettings.customEntries.empty')}
                  </Typography.Text>
                )}
              </Space>
              <Button
                size="small"
                icon={<PlusOutlined />}
                onClick={() => handleOpenCustomEntryModal()}
              >
                {t('settings.backupSettings.customEntries.add')}
              </Button>
            </div>
            {currentBackupCustomEntries.length > 0 && (
              <List
                size="small"
                bordered
                dataSource={currentBackupCustomEntries}
                renderItem={(entry) => (
                  <List.Item
                    actions={[
                      <Switch
                        key="enabled"
                        size="small"
                        checked={entry.enabled}
                        onChange={(checked) => handleToggleCustomEntry(entry.id, checked)}
                      />,
                      <Tooltip
                        key="edit"
                        title={t('settings.backupSettings.customEntries.edit')}
                      >
                        <Button
                          type="text"
                          size="small"
                          icon={<EditOutlined />}
                          aria-label={t('settings.backupSettings.customEntries.edit')}
                          onClick={() => handleOpenCustomEntryModal(entry)}
                        />
                      </Tooltip>,
                      <Popconfirm
                        key="delete"
                        title={t('settings.backupSettings.customEntries.deleteConfirm')}
                        onConfirm={() => handleDeleteCustomEntry(entry.id)}
                        okText={t('common.delete')}
                        cancelText={t('common.cancel')}
                      >
                        <Tooltip title={t('settings.backupSettings.customEntries.delete')}>
                          <Button
                            danger
                            type="text"
                            size="small"
                            icon={<DeleteOutlined />}
                            aria-label={t('settings.backupSettings.customEntries.delete')}
                          />
                        </Tooltip>
                      </Popconfirm>,
                    ]}
                  >
                    <List.Item.Meta
                      avatar={entry.entry_type === 'directory' ? <FolderOutlined /> : <FileOutlined />}
                      title={(
                        <Space size="small" wrap>
                          <span>{entry.name}</span>
                          <Tag>
                            {entry.entry_type === 'directory'
                              ? t('settings.backupSettings.customEntries.directory')
                              : t('settings.backupSettings.customEntries.file')}
                          </Tag>
                        </Space>
                      )}
                      description={(
                        <Space direction="vertical" size={0} style={{ width: '100%' }}>
                          <Typography.Text type="secondary" ellipsis={{ tooltip: entry.source_path }}>
                            {t('settings.backupSettings.customEntries.sourcePathShort')}: {entry.source_path}
                          </Typography.Text>
                          {entry.restore_path && (
                            <Typography.Text type="secondary" ellipsis={{ tooltip: entry.restore_path }}>
                              {t('settings.backupSettings.customEntries.restorePathShort')}: {entry.restore_path}
                            </Typography.Text>
                          )}
                        </Space>
                      )}
                    />
                  </List.Item>
                )}
              />
            )}
          </Space>
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.fileFilterRules.title')}
          colon={false}
        >
          <Space direction="vertical" style={{ width: '100%' }} size="small">
            <div style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              gap: 12,
            }}>
              <Space direction="vertical" size={0} style={{ minWidth: 0 }}>
                <Typography.Text type="secondary">
                  {t('settings.backupSettings.fileFilterRules.description')}
                </Typography.Text>
                {currentFileFilterRules.length === 0 && (
                  <Typography.Text type="secondary">
                    {t('settings.backupSettings.fileFilterRules.empty')}
                  </Typography.Text>
                )}
              </Space>
              <Button
                size="small"
                icon={<PlusOutlined />}
                onClick={() => handleOpenFilterRuleModal()}
              >
                {t('settings.backupSettings.fileFilterRules.add')}
              </Button>
            </div>
            {currentFileFilterRules.length > 0 && (
              <List
                size="small"
                bordered
                dataSource={currentFileFilterRules}
                renderItem={(rule, index) => (
                  <List.Item
                    actions={[
                      <Popconfirm
                        key="delete"
                        title={t('settings.backupSettings.fileFilterRules.deleteConfirm')}
                        onConfirm={() => handleDeleteFilterRule(index)}
                        okText={t('common.delete')}
                        cancelText={t('common.cancel')}
                      >
                        <Tooltip title={t('settings.backupSettings.fileFilterRules.delete')}>
                          <Button
                            danger
                            type="text"
                            size="small"
                            icon={<DeleteOutlined />}
                            aria-label={t('settings.backupSettings.fileFilterRules.delete')}
                          />
                        </Tooltip>
                      </Popconfirm>,
                    ]}
                  >
                    <List.Item.Meta
                      avatar={<FileOutlined />}
                      title={
                        <Space size="small" wrap>
                          <span>{rule.tool}</span>
                          <Tag>{rule.file_path}</Tag>
                        </Space>
                      }
                    />
                  </List.Item>
                )}
              />
            )}
          </Space>
        </Form.Item>

        <Divider />

        <Form.Item label={t('settings.autoBackup.title')}>
          <Switch
            checked={currentAutoBackupEnabled}
            onChange={setCurrentAutoBackupEnabled}
          />
        </Form.Item>
        {currentAutoBackupEnabled && (
          <>
            <Form.Item label={t('settings.autoBackup.interval')}>
              <InputNumber
                value={currentIntervalDays}
                onChange={(v) => setCurrentIntervalDays(v && v >= 1 ? Math.floor(v) : 1)}
                min={1}
                precision={0}
                style={{ width: 60 }}
                addonAfter={t('settings.autoBackup.days')}
              />
            </Form.Item>
            <Form.Item label={t('settings.autoBackup.maxKeep')}>
              <InputNumber
                value={currentMaxKeep}
                onChange={(v) => setCurrentMaxKeep(v != null && v >= 0 ? Math.floor(v) : 0)}
                min={0}
                precision={0}
                style={{ width: 60 }}
                addonAfter={t('settings.autoBackup.count')}
              />
              {currentMaxKeep === 0 && (
                <div style={{ marginTop: 4 }}>
                  <Typography.Text type="warning">
                    {t('settings.autoBackup.unlimitedHint')}
                  </Typography.Text>
                </div>
              )}
            </Form.Item>
          </>
        )}
      </Form>
    </Modal>
    <Modal
      title={editingCustomEntry
        ? t('settings.backupSettings.customEntries.edit')
        : t('settings.backupSettings.customEntries.add')}
      open={customEntryModalOpen}
      onOk={handleSaveCustomEntry}
      onCancel={() => setCustomEntryModalOpen(false)}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      width={640}
    >
      <Form
        form={customEntryForm}
        layout="horizontal"
        size="small"
        labelCol={{ span: 6 }}
        wrapperCol={{ span: 18 }}
      >
        <Form.Item
          label={t('settings.backupSettings.customEntries.name')}
          name="name"
          rules={[{ required: true, message: t('settings.backupSettings.customEntries.nameRequired') }]}
        >
          <Input placeholder={t('settings.backupSettings.customEntries.namePlaceholder')} />
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.customEntries.type')}
          name="entryType"
          rules={[{ required: true }]}
        >
          <Select
            onChange={(value: BackupCustomEntryType) => setCustomEntryType(value)}
            options={[
              {
                value: 'file',
                label: t('settings.backupSettings.customEntries.file'),
              },
              {
                value: 'directory',
                label: t('settings.backupSettings.customEntries.directory'),
              },
            ]}
          />
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.customEntries.sourcePath')}
          name="sourcePath"
          rules={[{ required: true, message: t('settings.backupSettings.customEntries.sourcePathRequired') }]}
          extra={t('settings.backupSettings.customEntries.pathHint')}
        >
          <Input
            placeholder={t('settings.backupSettings.customEntries.sourcePathPlaceholder')}
            addonAfter={
              <Tooltip title={t('common.browse')}>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleSelectCustomEntrySource}
                  style={{ margin: -7 }}
                />
              </Tooltip>
            }
          />
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.customEntries.restorePath')}
          name="restorePath"
          extra={t('settings.backupSettings.customEntries.restorePathHint')}
        >
          <Input placeholder={t('settings.backupSettings.customEntries.restorePathPlaceholder')} />
        </Form.Item>
      </Form>
    </Modal>
    <Modal
      title={t('settings.backupSettings.fileFilterRules.addTitle')}
      open={filterRuleModalOpen}
      onOk={handleSaveFilterRule}
      onCancel={() => setFilterRuleModalOpen(false)}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      width={480}
    >
      <Form
        form={filterRuleForm}
        layout="horizontal"
        size="small"
        labelCol={{ span: 6 }}
        wrapperCol={{ span: 18 }}
      >
        <Form.Item
          label={t('settings.backupSettings.fileFilterRules.tool')}
          name="tool"
          rules={[
            { required: true, message: t('settings.backupSettings.fileFilterRules.toolRequired') },
          ]}
        >
          <Select
            loading={filterPathOptionsLoading}
            onChange={() => filterRuleForm.setFieldValue('filePath', undefined)}
            options={filterToolOptions}
          />
        </Form.Item>
        <Form.Item
          label={t('settings.backupSettings.fileFilterRules.filePath')}
          name="filePath"
          rules={[
            {
              required: true,
              message: t('settings.backupSettings.fileFilterRules.filePathRequired'),
            },
          ]}
        >
          <Select
            disabled={!selectedFilterRuleTool || filterPathOptionsLoading}
            loading={filterPathOptionsLoading}
            placeholder={
              selectedFilterRuleTool
                ? t('settings.backupSettings.fileFilterRules.filePathPlaceholder')
                : t('settings.backupSettings.fileFilterRules.selectToolFirst')
            }
            options={selectedFilterPathOptions.map((option) => ({
              value: option.file_path,
              label: option.file_path,
              disabled: selectedToolExistingFilterPaths.has(option.file_path),
            }))}
          />
        </Form.Item>
      </Form>
    </Modal>
    </>
  );
};

export default BackupSettingsModal;
