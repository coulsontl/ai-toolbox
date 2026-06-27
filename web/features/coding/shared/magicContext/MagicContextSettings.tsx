import React from 'react';
import {
  Alert,
  App,
  Button,
  Collapse,
  Empty,
  Form,
  Input,
  InputNumber,
  Modal,
  Space,
  Switch,
  Tabs,
  Tag,
  Typography,
} from 'antd';
import {
  BugOutlined,
  EditOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  PlusOutlined,
  ReloadOutlined,
} from '@ant-design/icons';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import JSON5 from 'json5';
import { Brain } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import JsoncEditor from '@/components/common/JsoncEditor';
import {
  createMagicContextConfig,
  readMagicContextConfig,
  runMagicContextDoctor,
  saveMagicContextConfig,
} from '@/services/magicContextApi';
import type {
  MagicContextCommandResult,
  MagicContextConfigFile,
  MagicContextHarness,
} from '@/types/magicContext';

import styles from './MagicContextSettings.module.less';

const { Link, Text } = Typography;

const MAGIC_CONTEXT_DOCS_URL = 'https://docs.cortexkit.io/magic-context';

const DEFAULT_CONFIG_TEXT = `{
  "$schema": "https://raw.githubusercontent.com/cortexkit/magic-context/master/assets/magic-context.schema.json",
  "enabled": true,
  "ctx_reduce_enabled": true,
  "temporal_awareness": true,
  "smart_drops": false,
  "memory": {
    "enabled": true
  }
}
`;

interface MagicContextSettingsProps {
  harness: MagicContextHarness;
}

interface CommonConfigFormValues {
  enabled?: boolean;
  ctxReduceEnabled?: boolean;
  temporalAwareness?: boolean;
  smartDrops?: boolean;
  autoUpdate?: boolean;
  language?: string;
  memoryEnabled?: boolean;
  memoryBudget?: number | null;
  historianModel?: string;
  dreamerModel?: string;
  sidekickModel?: string;
}

interface EditingState {
  config: MagicContextConfigFile;
  content: string;
  isValid: boolean;
}

type EditorTabKey = 'common' | 'jsonc';

const parseJsoncObject = (content: string): Record<string, unknown> => {
  const trimmedContent = content.trim();
  if (!trimmedContent) return {};
  const parsed = JSON5.parse(trimmedContent);
  return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
    ? parsed as Record<string, unknown>
    : {};
};

const getObjectField = (value: unknown): Record<string, unknown> => (
  value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
);

const getBoolean = (value: unknown): boolean | undefined => (
  typeof value === 'boolean' ? value : undefined
);

const getString = (value: unknown): string | undefined => (
  typeof value === 'string' ? value : undefined
);

const getNumber = (value: unknown): number | undefined => (
  typeof value === 'number' && Number.isFinite(value) ? value : undefined
);

const setNestedValue = (
  root: Record<string, unknown>,
  path: string[],
  value: unknown,
) => {
  let current = root;
  path.slice(0, -1).forEach((part) => {
    const next = getObjectField(current[part]);
    current[part] = next;
    current = next;
  });
  current[path[path.length - 1]] = value;
};

const deleteNestedValue = (
  root: Record<string, unknown>,
  path: string[],
) => {
  let current = root;
  path.slice(0, -1).forEach((part) => {
    current = getObjectField(current[part]);
  });
  delete current[path[path.length - 1]];
};

const buildFormValues = (config: MagicContextConfigFile): CommonConfigFormValues => {
  const root = getObjectField(config.parsed);
  const memory = getObjectField(root.memory);
  const historian = getObjectField(root.historian);
  const dreamer = getObjectField(root.dreamer);
  const sidekick = getObjectField(root.sidekick);

  return {
    enabled: getBoolean(root.enabled),
    ctxReduceEnabled: getBoolean(root.ctx_reduce_enabled),
    temporalAwareness: getBoolean(root.temporal_awareness),
    smartDrops: getBoolean(root.smart_drops),
    autoUpdate: getBoolean(root.auto_update),
    language: getString(root.language),
    memoryEnabled: getBoolean(memory.enabled),
    memoryBudget: getNumber(memory.injection_budget_tokens),
    historianModel: getString(historian.model),
    dreamerModel: getString(dreamer.model),
    sidekickModel: getString(sidekick.model),
  };
};

const applyFormValuesToContent = (
  content: string,
  values: CommonConfigFormValues,
): string => {
  const root = parseJsoncObject(content || DEFAULT_CONFIG_TEXT);

  if (values.enabled !== undefined) setNestedValue(root, ['enabled'], values.enabled);
  if (values.ctxReduceEnabled !== undefined) {
    setNestedValue(root, ['ctx_reduce_enabled'], values.ctxReduceEnabled);
  }
  if (values.temporalAwareness !== undefined) {
    setNestedValue(root, ['temporal_awareness'], values.temporalAwareness);
  }
  if (values.smartDrops !== undefined) setNestedValue(root, ['smart_drops'], values.smartDrops);
  if (values.autoUpdate !== undefined) setNestedValue(root, ['auto_update'], values.autoUpdate);
  if (values.language !== undefined) {
    const trimmedLanguage = values.language.trim();
    if (trimmedLanguage) {
      setNestedValue(root, ['language'], trimmedLanguage);
    } else {
      deleteNestedValue(root, ['language']);
    }
  }
  if (values.memoryEnabled !== undefined) {
    setNestedValue(root, ['memory', 'enabled'], values.memoryEnabled);
  }
  if (typeof values.memoryBudget === 'number') {
    setNestedValue(root, ['memory', 'injection_budget_tokens'], values.memoryBudget);
  } else if (values.memoryBudget === null) {
    deleteNestedValue(root, ['memory', 'injection_budget_tokens']);
  }
  if (values.historianModel !== undefined) {
    const trimmedHistorianModel = values.historianModel.trim();
    if (trimmedHistorianModel) {
      setNestedValue(root, ['historian', 'model'], trimmedHistorianModel);
    } else {
      deleteNestedValue(root, ['historian', 'model']);
    }
  }
  if (values.dreamerModel !== undefined) {
    const trimmedDreamerModel = values.dreamerModel.trim();
    if (trimmedDreamerModel) {
      setNestedValue(root, ['dreamer', 'model'], trimmedDreamerModel);
    } else {
      deleteNestedValue(root, ['dreamer', 'model']);
    }
  }
  if (values.sidekickModel !== undefined) {
    const trimmedSidekickModel = values.sidekickModel.trim();
    if (trimmedSidekickModel) {
      setNestedValue(root, ['sidekick', 'model'], trimmedSidekickModel);
    } else {
      deleteNestedValue(root, ['sidekick', 'model']);
    }
  }

  return `${JSON.stringify(root, null, 2)}\n`;
};

const summarizeConfig = (config: MagicContextConfigFile, t: (key: string, options?: Record<string, unknown>) => string) => {
  const root = getObjectField(config.parsed);
  const memory = getObjectField(root.memory);
  const historian = getObjectField(root.historian);
  const dreamer = getObjectField(root.dreamer);
  const sidekick = getObjectField(root.sidekick);

  return [
    {
      label: t('magicContext.summary.enabled'),
      value: getBoolean(root.enabled) === false
        ? t('common.disabled')
        : t('common.enabled'),
    },
    {
      label: t('magicContext.summary.memory'),
      value: getBoolean(memory.enabled) === false
        ? t('common.disabled')
        : t('common.enabled'),
    },
    {
      label: t('magicContext.summary.historian'),
      value: getString(historian.model) || '-',
    },
    {
      label: t('magicContext.summary.dreamer'),
      value: getString(dreamer.model) || '-',
    },
    {
      label: t('magicContext.summary.sidekick'),
      value: getString(sidekick.model) || '-',
    },
    {
      label: t('magicContext.summary.ctxReduce'),
      value: getBoolean(root.ctx_reduce_enabled) === false
        ? t('common.disabled')
        : t('common.enabled'),
    },
  ];
};

const MagicContextSettings: React.FC<MagicContextSettingsProps> = ({ harness }) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [form] = Form.useForm<CommonConfigFormValues>();
  const [userConfig, setUserConfig] = React.useState<MagicContextConfigFile | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [doctorLoading, setDoctorLoading] = React.useState(false);
  const [editing, setEditing] = React.useState<EditingState | null>(null);
  const [activeEditorTab, setActiveEditorTab] = React.useState<EditorTabKey>('common');
  const [saving, setSaving] = React.useState(false);
  const [commandResult, setCommandResult] = React.useState<MagicContextCommandResult | null>(null);

  const loadUserConfig = React.useCallback(async () => {
    setLoading(true);
    try {
      setUserConfig(await readMagicContextConfig({ harness }));
    } catch (error) {
      void message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  }, [harness, message]);

  React.useEffect(() => {
    void loadUserConfig();
  }, [loadUserConfig]);

  const openDirectory = async (directory: string) => {
    try {
      await invoke('open_folder', { path: directory });
    } catch (error) {
      void message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const createConfig = async () => {
    try {
      const createdConfig = await createMagicContextConfig({ harness });
      setUserConfig(createdConfig);
      void message.success(t('common.success'));
    } catch (error) {
      void message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const openEditor = (config: MagicContextConfigFile) => {
    const nextContent = config.exists ? config.content : DEFAULT_CONFIG_TEXT;
    form.resetFields();
    let parsedConfig = config.parsed ?? null;
    if (!parsedConfig) {
      try {
        parsedConfig = parseJsoncObject(nextContent);
      } catch {
        setEditing({ config, content: nextContent, isValid: false });
        setActiveEditorTab('jsonc');
        return;
      }
    }

    setEditing({ config, content: nextContent, isValid: true });
    setActiveEditorTab('common');
    form.setFieldsValue(buildFormValues({
      ...config,
      content: nextContent,
      parsed: parsedConfig,
    }));
  };

  const buildContentFromCommonForm = async (baseContent: string): Promise<string> => {
    const values = await form.validateFields();
    return applyFormValuesToContent(baseContent || DEFAULT_CONFIG_TEXT, values);
  };

  const saveEditing = async () => {
    if (!editing) return;
    let contentToSave = editing.content;
    try {
      if (activeEditorTab === 'common') {
        contentToSave = await buildContentFromCommonForm(contentToSave);
      }
    } catch (error) {
      void message.error(error instanceof Error ? error.message : String(error));
      return;
    }

    let parsed: Record<string, unknown>;
    try {
      parsed = parseJsoncObject(contentToSave);
    } catch (error) {
      void message.error(error instanceof Error ? error.message : String(error));
      return;
    }
    if (!parsed || Array.isArray(parsed)) {
      void message.error(t('magicContext.invalidObject'));
      return;
    }

    setSaving(true);
    try {
      const savedConfig = await saveMagicContextConfig({
        harness,
        content: contentToSave,
      });
      setUserConfig(savedConfig);
      setEditing(null);
      void message.success(t('common.success'));
    } catch (error) {
      void message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  };

  const handleEditorTabChange = async (nextKey: string) => {
    const nextTab = nextKey as EditorTabKey;
    if (!editing) {
      setActiveEditorTab(nextTab);
      return;
    }

    if (activeEditorTab === 'common' && nextTab === 'jsonc') {
      try {
        const nextContent = await buildContentFromCommonForm(editing.content);
        setEditing((current) => current ? { ...current, content: nextContent, isValid: true } : current);
      } catch (error) {
        void message.error(error instanceof Error ? error.message : String(error));
        return;
      }
    }

    if (nextTab === 'common') {
      try {
        const parsed = parseJsoncObject(editing.content || DEFAULT_CONFIG_TEXT);
        form.resetFields();
        form.setFieldsValue(buildFormValues({
          ...editing.config,
          content: editing.content,
          parsed,
        }));
      } catch (error) {
        void message.error(error instanceof Error ? error.message : String(error));
        return;
      }
    }

    setActiveEditorTab(nextTab);
  };

  const handleDoctor = async () => {
    setDoctorLoading(true);
    try {
      const result = await runMagicContextDoctor({ harness });
      setCommandResult(result);
    } catch (error) {
      void message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setDoctorLoading(false);
    }
  };

  const renderSettingRow = (
    label: string,
    control: React.ReactNode,
  ) => (
    <div className={styles.settingRow}>
      <Text className={styles.settingLabel}>{label}</Text>
      <div className={styles.settingControl}>{control}</div>
    </div>
  );

  const renderSettingSection = (
    title: string,
    children: React.ReactNode,
    options?: { agents?: boolean },
  ) => (
    <section className={styles.settingSection}>
      <div className={styles.sectionHeader}>
        <Text strong>{title}</Text>
      </div>
      <div className={`${styles.settingsGrid} ${options?.agents ? styles.agentGrid : ''}`}>
        {children}
      </div>
    </section>
  );

  const renderSummary = (config: MagicContextConfigFile) => {
    if (!config.exists) {
      return (
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description={t('magicContext.configNotFound')}
        />
      );
    }

    if (config.parseError) {
      return (
        <Alert
          type="error"
          showIcon
          message={t('magicContext.parseError')}
          description={config.parseError}
        />
      );
    }

    return (
      <div className={styles.summaryGrid}>
        {summarizeConfig(config, t).map((item) => (
          <div key={item.label} className={styles.summaryItem}>
            <Text type="secondary" className={styles.summaryLabel}>{item.label}</Text>
            <Text strong className={styles.codeText}>{item.value}</Text>
          </div>
        ))}
      </div>
    );
  };

  const renderConfigCard = (
    config: MagicContextConfigFile | null,
    isLoading: boolean,
  ) => {
    return (
      <div className={styles.configCard}>
        <div className={styles.cardHeader}>
          <div className={styles.cardTitle}>
            <Space size={8} wrap className={styles.cardTitleLine}>
              <Text strong>{t('magicContext.scope.user')}</Text>
              {config?.exists ? (
                <Tag color="success">{t('magicContext.exists')}</Tag>
              ) : (
                <Tag>{t('magicContext.missing')}</Tag>
              )}
              <Text code className={styles.cardPathText}>
                {config?.path || '-'}
              </Text>
            </Space>
          </div>
          <Space size={4}>
            {config?.directory && (
              <Button
                type="text"
                size="small"
                icon={<FolderOpenOutlined />}
                onClick={() => void openDirectory(config.directory)}
              />
            )}
            <Button
              size="small"
              icon={config?.exists ? <EditOutlined /> : <PlusOutlined />}
              loading={isLoading}
              onClick={() => {
                if (config?.exists) {
                  openEditor(config);
                } else {
                  void createConfig();
                }
              }}
            >
              {config?.exists ? t('common.edit') : t('magicContext.createConfig')}
            </Button>
          </Space>
        </div>

        {config?.warnings.map((warning) => (
          <Alert key={warning} type="warning" showIcon message={warning} />
        ))}

        {config ? renderSummary(config) : (
          <div className={styles.summaryItem}>
            <Text type="secondary">{t('common.loading')}</Text>
          </div>
        )}
      </div>
    );
  };

  return (
    <>
      <Collapse
        className={`${styles.collapseCard} ${harness === 'pi' ? styles.piCollapseCard : ''}`}
        size={harness === 'pi' ? 'small' : undefined}
        bordered={harness !== 'pi'}
        defaultActiveKey={['magic-context']}
        items={[
          {
            key: 'magic-context',
            label: (
              <span className={styles.collapseTitle}>
                {harness !== 'pi' && (
                  <span className={styles.titleIcon} aria-hidden="true">
                    <Brain size={16} />
                  </span>
                )}
                <Text strong>{t('magicContext.title')}</Text>
                <Link
                  type="secondary"
                  className={styles.docsLink}
                  onClick={(event) => {
                    event.stopPropagation();
                    void openUrl(MAGIC_CONTEXT_DOCS_URL);
                  }}
                >
                  <LinkOutlined /> {t('magicContext.docs')}
                </Link>
              </span>
            ),
            extra: (
              <Space onClick={(event) => event.stopPropagation()}>
                <Button
                  type="link"
                  size="small"
                  icon={<BugOutlined />}
                  loading={doctorLoading}
                  onClick={handleDoctor}
                >
                  {t('magicContext.runDoctor')}
                </Button>
                <Button
                  type="link"
                  size="small"
                  icon={<ReloadOutlined />}
                  loading={loading}
                  onClick={() => void loadUserConfig()}
                >
                  {t('common.refresh')}
                </Button>
              </Space>
            ),
            children: (
              <div className={styles.content}>
                <div className={styles.toolbar}>
                  <Text type="secondary">{t('magicContext.description')}</Text>
                </div>

                {renderConfigCard(userConfig, loading)}
              </div>
            ),
          },
        ]}
      />

      <Modal
        title={editing ? t('magicContext.editTitle') : undefined}
        open={!!editing}
        width={920}
        okText={t('common.save')}
        okButtonProps={{ loading: saving, disabled: editing ? !editing.isValid : true }}
        cancelText={t('common.cancel')}
        onOk={saveEditing}
        onCancel={() => {
          setEditing(null);
          setActiveEditorTab('common');
        }}
        destroyOnHidden
      >
        {editing && (
          <div className={styles.modalBody}>
            <div className={styles.pathBar}>
              <Text type="secondary" className={styles.pathLabel}>
                {t('magicContext.scope.user')}
              </Text>
              <Text code className={styles.pathText}>{editing.config.path}</Text>
            </div>
            <Tabs
              activeKey={activeEditorTab}
              onChange={(key) => void handleEditorTabChange(key)}
              items={[
                {
                  key: 'common',
                  label: t('magicContext.commonConfig'),
                  children: (
                    <Form form={form} component={false}>
                      <div className={styles.settingsForm}>
                        {renderSettingSection(t('magicContext.sections.basic'), (
                          <>
                            {renderSettingRow(t('magicContext.fields.enabled'), (
                              <Form.Item name="enabled" valuePropName="checked" noStyle>
                                <Switch size="small" aria-label={t('magicContext.fields.enabled')} />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.ctxReduceEnabled'), (
                              <Form.Item name="ctxReduceEnabled" valuePropName="checked" noStyle>
                                <Switch size="small" aria-label={t('magicContext.fields.ctxReduceEnabled')} />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.temporalAwareness'), (
                              <Form.Item name="temporalAwareness" valuePropName="checked" noStyle>
                                <Switch size="small" aria-label={t('magicContext.fields.temporalAwareness')} />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.smartDrops'), (
                              <Form.Item name="smartDrops" valuePropName="checked" noStyle>
                                <Switch size="small" aria-label={t('magicContext.fields.smartDrops')} />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.autoUpdate'), (
                              <Form.Item name="autoUpdate" valuePropName="checked" noStyle>
                                <Switch size="small" aria-label={t('magicContext.fields.autoUpdate')} />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.language'), (
                              <Form.Item name="language" noStyle>
                                <Input
                                  size="small"
                                  className={styles.fullWidthControl}
                                  placeholder={t('magicContext.fields.languagePlaceholder')}
                                  aria-label={t('magicContext.fields.language')}
                                />
                              </Form.Item>
                            ))}
                          </>
                        ))}

                        {renderSettingSection(t('magicContext.sections.memory'), (
                          <>
                            {renderSettingRow(t('magicContext.fields.memoryEnabled'), (
                              <Form.Item name="memoryEnabled" valuePropName="checked" noStyle>
                                <Switch size="small" aria-label={t('magicContext.fields.memoryEnabled')} />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.memoryBudget'), (
                              <Form.Item name="memoryBudget" noStyle>
                                <InputNumber
                                  min={0}
                                  step={500}
                                  size="small"
                                  className={styles.fullWidthControl}
                                  aria-label={t('magicContext.fields.memoryBudget')}
                                />
                              </Form.Item>
                            ))}
                          </>
                        ))}

                        {renderSettingSection(t('magicContext.sections.agents'), (
                          <>
                            {renderSettingRow(t('magicContext.fields.historianModel'), (
                              <Form.Item name="historianModel" noStyle>
                                <Input
                                  size="small"
                                  className={styles.fullWidthControl}
                                  placeholder="github-copilot/gpt-5.4"
                                  aria-label={t('magicContext.fields.historianModel')}
                                />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.dreamerModel'), (
                              <Form.Item name="dreamerModel" noStyle>
                                <Input
                                  size="small"
                                  className={styles.fullWidthControl}
                                  placeholder="github-copilot/gpt-5.4"
                                  aria-label={t('magicContext.fields.dreamerModel')}
                                />
                              </Form.Item>
                            ))}
                            {renderSettingRow(t('magicContext.fields.sidekickModel'), (
                              <Form.Item name="sidekickModel" noStyle>
                                <Input
                                  size="small"
                                  className={styles.fullWidthControl}
                                  placeholder="github-copilot/gpt-5.4"
                                  aria-label={t('magicContext.fields.sidekickModel')}
                                />
                              </Form.Item>
                            ))}
                          </>
                        ), { agents: true })}
                      </div>
                    </Form>
                  ),
                },
                {
                  key: 'jsonc',
                  label: t('magicContext.fullJsonc'),
                  children: (
                    <JsoncEditor
                      value={editing.content}
                      height={420}
                      minHeight={260}
                      placeholder={DEFAULT_CONFIG_TEXT}
                      onChange={(content, isValid) => {
                        setEditing((current) => current ? { ...current, content, isValid } : current);
                      }}
                    />
                  ),
                },
              ]}
            />
          </div>
        )}
      </Modal>

      <Modal
        title={t('magicContext.doctorResultTitle')}
        open={!!commandResult}
        footer={[
          <Button key="close" type="primary" onClick={() => setCommandResult(null)}>
            {t('common.close')}
          </Button>,
        ]}
        onCancel={() => setCommandResult(null)}
        destroyOnHidden
      >
        {commandResult && (
          <pre className={styles.commandOutput}>
            {`${commandResult.command}\n${commandResult.output || t('magicContext.emptyCommandOutput')}`}
          </pre>
        )}
      </Modal>
    </>
  );
};

export default MagicContextSettings;
