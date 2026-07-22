import React, { useState, useEffect, useMemo } from 'react';
import { Modal, Form, Input, Select, Button, Space, Checkbox, Dropdown, Tag, message, InputNumber } from 'antd';
import type { FormListFieldData } from 'antd';
import { PlusOutlined, MinusCircleOutlined, ExportOutlined, HolderOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from '@dnd-kit/core';
import type { DragEndEvent } from '@dnd-kit/core';
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis, restrictToParentElement } from '@dnd-kit/modifiers';
import { CSS } from '@dnd-kit/utilities';
import * as mcpApi from '../../services/mcpApi';
import type { CreateMcpServerInput, UpdateMcpServerInput, McpTool, McpServer, StdioConfig, HttpConfig } from '../../types';
import styles from './AddMcpModal.module.less';

function reorderFormListFields(
  event: DragEndEvent,
  fields: FormListFieldData[],
  move: (from: number, to: number) => void,
) {
  const { active, over } = event;
  if (!over || active.id === over.id) {
    return;
  }

  const oldIndex = fields.findIndex((field) => field.key === active.id);
  const newIndex = fields.findIndex((field) => field.key === over.id);
  if (oldIndex < 0 || newIndex < 0 || oldIndex === newIndex) {
    return;
  }

  move(oldIndex, newIndex);
}

interface SortableFormListProps {
  fields: FormListFieldData[];
  move: (from: number, to: number) => void;
  children: React.ReactNode;
}

const SortableFormList: React.FC<SortableFormListProps> = ({ fields, move, children }) => {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8,
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      modifiers={[restrictToVerticalAxis, restrictToParentElement]}
      onDragEnd={(event) => reorderFormListFields(event, fields, move)}
    >
      <SortableContext
        items={fields.map((field) => field.key)}
        strategy={verticalListSortingStrategy}
      >
        {children}
      </SortableContext>
    </DndContext>
  );
};

interface SortableFormRowProps {
  id: string | number;
  className?: string;
  children: React.ReactNode;
}

const SortableFormRow: React.FC<SortableFormRowProps> = ({ id, className, children }) => {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
    position: 'relative',
    zIndex: isDragging ? 1 : undefined,
  };

  return (
    <div ref={setNodeRef} style={style} className={className}>
      <span className={styles.dragHandle} {...attributes} {...listeners}>
        <HolderOutlined />
      </span>
      {children}
    </div>
  );
};

interface AddMcpModalProps {
  open: boolean;
  tools: McpTool[];
  servers: McpServer[];
  editingServer?: McpServer | null;
  onClose: () => void;
  onSubmit: (input: CreateMcpServerInput) => Promise<void>;
  onUpdate?: (serverId: string, input: UpdateMcpServerInput) => Promise<void>;
  onSyncAll?: () => Promise<unknown>;
}

export const AddMcpModal: React.FC<AddMcpModalProps> = ({
  open,
  tools,
  servers,
  editingServer,
  onClose,
  onSubmit,
  onUpdate,
  onSyncAll,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = useState(false);
  const [serverType, setServerType] = useState<'stdio' | 'http' | 'sse'>('stdio');
  const [selectedTools, setSelectedTools] = useState<string[]>([]);
  const [favorites, setFavorites] = React.useState<mcpApi.FavoriteMcp[]>([]);
  const [favoritesExpanded, setFavoritesExpanded] = useState(false);
  const [preferredTools, setPreferredTools] = useState<string[] | null>(null);

  const isEditMode = !!editingServer;

  // Split tools based on preferred tools setting + selected tools
  const visibleTools = useMemo(() => {
    if (preferredTools && preferredTools.length > 0) {
      // If preferred tools are set, show those + any selected tools
      return tools.filter((t) => preferredTools.includes(t.key) || selectedTools.includes(t.key));
    }
    // Otherwise show installed tools + any selected tools
    return tools.filter((t) => t.installed || selectedTools.includes(t.key));
  }, [tools, preferredTools, selectedTools]);

  // Hidden dropdown only offers installed tools that are outside the preferred row.
  const hiddenTools = useMemo(() => {
    if (preferredTools && preferredTools.length > 0) {
      return tools.filter((t) => (
        t.installed
        && !preferredTools.includes(t.key)
        && !selectedTools.includes(t.key)
      ));
    }
    return [];
  }, [tools, preferredTools, selectedTools]);

  // Load favorites and preferred tools on mount
  useEffect(() => {
    loadFavorites();
    loadPreferredTools();
  }, []);

  const loadFavorites = async () => {
    try {
      // Initialize default favorites if empty
      await mcpApi.initMcpDefaultFavorites();
      // Then load the list
      const list = await mcpApi.listMcpFavorites();
      setFavorites(list);
    } catch (error) {
      console.error('Failed to load favorites:', error);
    }
  };

  const loadPreferredTools = async () => {
    try {
      const preferred = await mcpApi.getMcpPreferredTools();
      setPreferredTools(preferred);
    } catch (error) {
      console.error('Failed to load preferred tools:', error);
    }
  };

  // Initialize form on mount
  useEffect(() => {
    if (editingServer) {
      const config = editingServer.server_config;
      setServerType(editingServer.server_type);
      setSelectedTools(editingServer.enabled_tools);

      if (editingServer.server_type === 'stdio') {
        const stdioConfig = config as StdioConfig;
        // Convert env object to key-value array
        const envList = stdioConfig.env
          ? Object.entries(stdioConfig.env).map(([key, value]) => ({ key, value }))
          : [];
        form.setFieldsValue({
          name: editingServer.name,
          server_type: editingServer.server_type,
          command: stdioConfig.command,
          args: stdioConfig.args || [],
          env: envList,
          description: editingServer.description,
          timeout: editingServer.timeout,
          startup_timeout_sec: stdioConfig.startup_timeout_sec,
          tool_timeout_sec: stdioConfig.tool_timeout_sec,
        });
      } else {
        const httpConfig = config as HttpConfig;
        // Extract Bearer Token from headers if present
        let bearerToken = '';
        const headersList: { key: string; value: string }[] = [];
        if (httpConfig.headers) {
          Object.entries(httpConfig.headers).forEach(([key, value]) => {
            if (key.toLowerCase() === 'authorization' && typeof value === 'string' && value.startsWith('Bearer ')) {
              bearerToken = value.substring(7); // Remove "Bearer " prefix
            } else {
              headersList.push({ key, value: String(value) });
            }
          });
        }
        form.setFieldsValue({
          name: editingServer.name,
          server_type: editingServer.server_type,
          url: httpConfig.url,
          bearerToken,
          headers: headersList,
          description: editingServer.description,
          timeout: editingServer.timeout,
          startup_timeout_sec: httpConfig.startup_timeout_sec,
          tool_timeout_sec: httpConfig.tool_timeout_sec,
        });
      }
    } else {
      // Reset for add mode
      form.resetFields();
      setServerType('stdio');
    }
  }, [editingServer, form]);

  // Initialize selected tools based on preferredTools (same logic as Skills)
  useEffect(() => {
    if (editingServer) return; // Don't override when editing
    if (preferredTools && preferredTools.length > 0) {
      setSelectedTools(preferredTools);
    } else if (preferredTools !== null) {
      // preferredTools loaded but empty, use installed tools
      const installed = tools.filter((t) => t.installed).map((t) => t.key);
      setSelectedTools(installed);
    }
  }, [editingServer, tools, preferredTools]);

  const handleToolToggle = (toolKey: string) => {
    setSelectedTools((prev) =>
      prev.includes(toolKey)
        ? prev.filter((k) => k !== toolKey)
        : [...prev, toolKey]
    );
  };

  // Handle selecting a favorite MCP
  const handleSelectFavorite = (fav: mcpApi.FavoriteMcp) => {
    setServerType(fav.server_type);
    if (fav.server_type === 'stdio') {
      const config = fav.server_config as unknown as StdioConfig;
      const envList = config.env
        ? Object.entries(config.env).map(([key, value]) => ({ key, value }))
        : [];
      form.setFieldsValue({
        name: fav.name,
        server_type: fav.server_type,
        command: config.command,
        args: config.args || [],
        env: envList,
        description: fav.description,
        startup_timeout_sec: config.startup_timeout_sec,
        tool_timeout_sec: config.tool_timeout_sec,
      });
    } else {
      const config = fav.server_config as unknown as HttpConfig;
      const headersList = config.headers
        ? Object.entries(config.headers).filter(([key]) => key.toLowerCase() !== 'authorization').map(([key, value]) => ({ key, value }))
        : [];
      const bearerToken = config.headers?.['Authorization']?.replace('Bearer ', '') || '';
      form.setFieldsValue({
        name: fav.name,
        server_type: fav.server_type,
        url: config.url,
        bearerToken,
        headers: headersList,
        description: fav.description,
        startup_timeout_sec: config.startup_timeout_sec,
        tool_timeout_sec: config.tool_timeout_sec,
      });
    }
    setFavoritesExpanded(false);
  };

  // Handle removing a favorite MCP
  const handleRemoveFavorite = (fav: mcpApi.FavoriteMcp) => {
    Modal.confirm({
      title: t('mcp.favorites.removeTitle'),
      content: t('mcp.favorites.removeConfirm', { name: fav.name }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        await mcpApi.deleteMcpFavorite(fav.id);
        setFavorites((prev) => prev.filter((f) => f.id !== fav.id));
      },
    });
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();

      setLoading(true);

      const optionalTimeoutSec = (value: unknown): number | undefined => {
        if (value === null || value === undefined || value === '') {
          return undefined;
        }
        const numeric = Number(value);
        return Number.isFinite(numeric) && numeric > 0 ? numeric : undefined;
      };
      const startupTimeoutSec = optionalTimeoutSec(values.startup_timeout_sec);
      const toolTimeoutSec = optionalTimeoutSec(values.tool_timeout_sec);

      let serverConfig: StdioConfig | HttpConfig;
      if (serverType === 'stdio') {
        const command = values.command?.trim() || '';
        const args = values.args?.filter((a: string) => a) || [];

        // Convert env key-value array to object
        const envObj: Record<string, string> = {};
        if (values.env && Array.isArray(values.env)) {
          values.env.forEach((item: { key?: string; value?: string }) => {
            if (item.key && item.key.trim()) {
              envObj[item.key.trim()] = item.value || '';
            }
          });
        }
        serverConfig = {
          command,
          args,
          env: Object.keys(envObj).length > 0 ? envObj : undefined,
          startup_timeout_sec: startupTimeoutSec,
          tool_timeout_sec: toolTimeoutSec,
        };
      } else {
        // Convert headers key-value array to object and merge Bearer Token
        const headersObj: Record<string, string> = {};
        if (values.headers && Array.isArray(values.headers)) {
          values.headers.forEach((item: { key?: string; value?: string }) => {
            if (item.key && item.key.trim()) {
              headersObj[item.key.trim()] = item.value || '';
            }
          });
        }
        // Add Bearer Token to headers if provided
        if (values.bearerToken && values.bearerToken.trim()) {
          headersObj['Authorization'] = `Bearer ${values.bearerToken.trim()}`;
        }
        serverConfig = {
          url: values.url,
          headers: Object.keys(headersObj).length > 0 ? headersObj : undefined,
          startup_timeout_sec: startupTimeoutSec,
          tool_timeout_sec: toolTimeoutSec,
        };
      }

      const doSubmit = async (overwrite: boolean, existingId?: string) => {
        if (overwrite && existingId && onUpdate) {
          await onUpdate(existingId, {
            name: values.name,
            server_type: serverType,
            server_config: serverConfig,
            enabled_tools: selectedTools,
            description: values.description,
            timeout: values.timeout ?? null,
          });
          // Sync all tools after overwrite
          if (onSyncAll) {
            await onSyncAll();
          }
        } else if (isEditMode && onUpdate && editingServer) {
          await onUpdate(editingServer.id, {
            name: values.name,
            server_type: serverType,
            server_config: serverConfig,
            enabled_tools: selectedTools,
            description: values.description,
            timeout: values.timeout ?? null,
          });
        } else {
          await onSubmit({
            name: values.name,
            server_type: serverType,
            server_config: serverConfig,
            enabled_tools: selectedTools,
            description: values.description,
            tags: values.tags?.filter((t: string) => t) || [],
            timeout: values.timeout ?? null,
          });
        }
        // Upsert favorite
        await mcpApi.upsertMcpFavorite({
          name: values.name,
          server_type: serverType,
          server_config: serverConfig as unknown as Record<string, unknown>,
          description: values.description,
          tags: values.tags?.filter((t: string) => t) || [],
        });
        form.resetFields();
        setSelectedTools([]);
        onClose();
      };

      // Check for duplicate name when adding (not editing)
      if (!isEditMode) {
        const existing = servers.find((s) => s.name === values.name);
        if (existing) {
          setLoading(false);
          Modal.confirm({
            title: t('mcp.duplicateName.title'),
            content: t('mcp.duplicateName.content', { name: values.name }),
            okText: t('mcp.duplicateName.overwrite'),
            cancelText: t('common.cancel'),
            onOk: async () => {
              setLoading(true);
              try {
                await doSubmit(true, existing.id);
              } finally {
                setLoading(false);
              }
            },
          });
          return;
        }
      }

      await doSubmit(false);
    } catch (error) {
      console.error('Form validation failed:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleCancel = () => {
    form.resetFields();
    setSelectedTools([]);
    onClose();
  };

  // Build server config JSON for export
  const buildExportJson = (): Record<string, unknown> | null => {
    const values = form.getFieldsValue();
    const name = values.name?.trim();
    if (!name) {
      message.warning(t('mcp.nameRequired'));
      return null;
    }

    const optionalTimeoutSec = (value: unknown): number | undefined => {
      if (value === null || value === undefined || value === '') {
        return undefined;
      }
      const numeric = Number(value);
      return Number.isFinite(numeric) && numeric > 0 ? numeric : undefined;
    };
    const startupTimeoutSec = optionalTimeoutSec(values.startup_timeout_sec);
    const toolTimeoutSec = optionalTimeoutSec(values.tool_timeout_sec);

    let serverConfig: Record<string, unknown>;
    if (serverType === 'stdio') {
      const envObj: Record<string, string> = {};
      if (values.env && Array.isArray(values.env)) {
        values.env.forEach((item: { key?: string; value?: string }) => {
          if (item.key && item.key.trim()) {
            envObj[item.key.trim()] = item.value || '';
          }
        });
      }
      serverConfig = {
        type: 'stdio',
        command: values.command || '',
        args: values.args?.filter((a: string) => a) || [],
      };
      if (Object.keys(envObj).length > 0) {
        serverConfig.env = envObj;
      }
    } else {
      const headersObj: Record<string, string> = {};
      if (values.headers && Array.isArray(values.headers)) {
        values.headers.forEach((item: { key?: string; value?: string }) => {
          if (item.key && item.key.trim()) {
            headersObj[item.key.trim()] = item.value || '';
          }
        });
      }
      if (values.bearerToken && values.bearerToken.trim()) {
        headersObj['Authorization'] = `Bearer ${values.bearerToken.trim()}`;
      }
      serverConfig = {
        type: serverType,
        url: values.url || '',
      };
      if (Object.keys(headersObj).length > 0) {
        serverConfig.headers = headersObj;
      }
    }

    if (startupTimeoutSec !== undefined) {
      serverConfig.startup_timeout_sec = startupTimeoutSec;
    }
    if (toolTimeoutSec !== undefined) {
      serverConfig.tool_timeout_sec = toolTimeoutSec;
    }

    return { [name]: serverConfig };
  };

  const handleExportJson = async () => {
    const json = buildExportJson();
    if (!json) return;

    const jsonStr = JSON.stringify(json, null, 2);
    try {
      await navigator.clipboard.writeText(jsonStr);
      message.success(t('mcp.exportCopied'));
    } catch {
      // Fallback: show in a modal or alert
      Modal.info({
        title: t('mcp.exportJson'),
        content: <pre style={{ maxHeight: 400, overflow: 'auto' }}>{jsonStr}</pre>,
        width: 600,
      });
    }
  };

  return (
    <Modal
      title={isEditMode ? t('mcp.editServer') : t('mcp.addServer')}
      open={open}
      onCancel={handleCancel}
      footer={[
        <Button key="export" icon={<ExportOutlined />} onClick={handleExportJson}>
          {t('mcp.exportJson')}
        </Button>,
        <Button key="cancel" onClick={handleCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="submit" type="primary" loading={loading} onClick={handleSubmit}>
          {t('common.save')}
        </Button>,
      ]}
      width={700}
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={{ span: 6 }}
        wrapperCol={{ span: 18 }}
        initialValues={{ server_type: 'stdio' }}
      >
        <Form.Item
          label={t('mcp.name')}
          required
        >
          <div className={styles.nameRow}>
            <Form.Item
              name="name"
              noStyle
              rules={[{ required: true, message: t('mcp.nameRequired') }]}
            >
              <Input placeholder={t('mcp.namePlaceholder')} disabled={isEditMode} />
            </Form.Item>
            {!isEditMode && favorites.length > 0 && (
              <a
                className={styles.favoritesToggle}
                onClick={() => setFavoritesExpanded(!favoritesExpanded)}
              >
                {t('mcp.favorites.label')}
                {favoritesExpanded ? ' ▴' : ' ▾'}
              </a>
            )}
          </div>
        </Form.Item>

        {!isEditMode && favoritesExpanded && (
          <Form.Item wrapperCol={{ offset: 6, span: 18 }} style={{ marginTop: -8 }}>
            <div className={styles.favoritesTagsList}>
              {favorites.map((fav) => (
                <Tag
                  key={fav.id}
                  closable
                  className={styles.favoriteTag}
                  onClick={() => handleSelectFavorite(fav)}
                  onClose={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    handleRemoveFavorite(fav);
                  }}
                >
                  {fav.name}
                </Tag>
              ))}
            </div>
          </Form.Item>
        )}

        <Form.Item label={t('mcp.type')} name="server_type">
          <Select
            value={serverType}
            onChange={(v) => setServerType(v)}
            options={[
              { label: 'Stdio', value: 'stdio' },
              { label: 'HTTP', value: 'http' },
              { label: 'SSE', value: 'sse' },
            ]}
          />
        </Form.Item>

        {serverType === 'stdio' ? (
          <>
            <Form.Item
              label={t('mcp.command')}
              name="command"
              rules={[{ required: true, message: t('mcp.commandRequired') }]}
            >
              <Input placeholder="npx" />
            </Form.Item>

            <Form.Item label={t('mcp.args')}>
              <Form.List name="args">
                {(fields, { add, remove, move }) => (
                  <>
                    <SortableFormList fields={fields} move={move}>
                      {fields.map((field, index) => (
                        <SortableFormRow key={field.key} id={field.key} className={styles.argRow}>
                          <Form.Item {...field} noStyle>
                            <Input placeholder={`${t('mcp.arg')} ${index + 1}`} />
                          </Form.Item>
                          <MinusCircleOutlined onClick={() => remove(field.name)} />
                        </SortableFormRow>
                      ))}
                    </SortableFormList>
                    <Button type="dashed" onClick={() => add()} block icon={<PlusOutlined />}>
                      {t('mcp.addArg')}
                    </Button>
                  </>
                )}
              </Form.List>
            </Form.Item>

            <Form.Item label={t('mcp.env')}>
              <Form.List name="env">
                {(fields, { add, remove, move }) => (
                  <>
                    <SortableFormList fields={fields} move={move}>
                      {fields.map((field) => (
                        <SortableFormRow key={field.key} id={field.key} className={styles.kvRow}>
                          <Form.Item
                            {...field}
                            name={[field.name, 'key']}
                            noStyle
                          >
                            <Input placeholder={t('mcp.envKey')} className={styles.kvKey} />
                          </Form.Item>
                          <Form.Item
                            {...field}
                            name={[field.name, 'value']}
                            noStyle
                          >
                            <Input placeholder={t('mcp.envValue')} className={styles.kvValue} />
                          </Form.Item>
                          <MinusCircleOutlined onClick={() => remove(field.name)} />
                        </SortableFormRow>
                      ))}
                    </SortableFormList>
                    <Button type="dashed" onClick={() => add()} block icon={<PlusOutlined />}>
                      {t('mcp.addEnv')}
                    </Button>
                  </>
                )}
              </Form.List>
            </Form.Item>
          </>
        ) : (
          <>
            <Form.Item
              label={t('mcp.url')}
              name="url"
              rules={[{ required: true, message: t('mcp.urlRequired') }]}
            >
              <Input placeholder="https://example.com/mcp" />
            </Form.Item>

            <Form.Item label={t('mcp.bearerToken')} name="bearerToken">
              <Input.Password placeholder={t('mcp.bearerTokenPlaceholder')} />
            </Form.Item>

            <Form.Item label={t('mcp.headers')}>
              <Form.List name="headers">
                {(fields, { add, remove, move }) => (
                  <>
                    <SortableFormList fields={fields} move={move}>
                      {fields.map((field) => (
                        <SortableFormRow key={field.key} id={field.key} className={styles.kvRow}>
                          <Form.Item
                            {...field}
                            name={[field.name, 'key']}
                            noStyle
                          >
                            <Input placeholder={t('mcp.headerKey')} className={styles.kvKey} />
                          </Form.Item>
                          <Form.Item
                            {...field}
                            name={[field.name, 'value']}
                            noStyle
                          >
                            <Input placeholder={t('mcp.headerValue')} className={styles.kvValue} />
                          </Form.Item>
                          <MinusCircleOutlined onClick={() => remove(field.name)} />
                        </SortableFormRow>
                      ))}
                    </SortableFormList>
                    <Button type="dashed" onClick={() => add()} block icon={<PlusOutlined />}>
                      {t('mcp.addHeader')}
                    </Button>
                  </>
                )}
              </Form.List>
            </Form.Item>
          </>
        )}

        <Form.Item label={t('mcp.description')} name="description">
          <Input.TextArea rows={2} placeholder={t('mcp.descriptionPlaceholder')} />
        </Form.Item>

        <Form.Item label={t('mcp.timeouts')} extra={t('mcp.timeoutsHint')}>
          <Space direction="vertical" size={8} style={{ width: '100%' }}>
            <Space align="center" wrap>
              <Form.Item name="timeout" noStyle>
                <InputNumber
                  min={1}
                  placeholder="5000"
                  style={{ width: 160 }}
                  addonAfter="ms"
                />
              </Form.Item>
              <span style={{ fontSize: 12, color: 'var(--ant-color-text-secondary)' }}>
                {t('mcp.timeoutOpenCodeLabel')}
              </span>
            </Space>
            <Space align="center" wrap>
              <Form.Item name="startup_timeout_sec" noStyle>
                <InputNumber
                  min={1}
                  placeholder="10"
                  style={{ width: 160 }}
                  addonAfter="s"
                />
              </Form.Item>
              <span style={{ fontSize: 12, color: 'var(--ant-color-text-secondary)' }}>
                {t('mcp.startupTimeoutLabel')}
              </span>
            </Space>
            <Space align="center" wrap>
              <Form.Item name="tool_timeout_sec" noStyle>
                <InputNumber
                  min={1}
                  placeholder="60"
                  style={{ width: 160 }}
                  addonAfter="s"
                />
              </Form.Item>
              <span style={{ fontSize: 12, color: 'var(--ant-color-text-secondary)' }}>
                {t('mcp.toolTimeoutLabel')}
              </span>
            </Space>
          </Space>
        </Form.Item>
      </Form>

      <div className={styles.toolsSection}>
        <div className={styles.toolsLabel}>{t('mcp.enabledTools')}</div>
        <div className={styles.toolsHint}>{t('mcp.enabledToolsHint')}</div>
        <div className={styles.toolsGrid}>
          {visibleTools.length > 0 ? (
            visibleTools.map((tool) => (
              <Checkbox
                key={tool.key}
                checked={selectedTools.includes(tool.key)}
                onChange={() => handleToolToggle(tool.key)}
              >
                {tool.display_name}
              </Checkbox>
            ))
          ) : (
            <span className={styles.noTools}>{t('mcp.noToolsInstalled')}</span>
          )}
          {hiddenTools.length > 0 && (
            <Dropdown
              trigger={['click']}
              menu={{
                items: hiddenTools.map((tool) => ({
                  key: tool.key,
                  label: (
                    <Checkbox
                      checked={selectedTools.includes(tool.key)}
                      onClick={(e) => e.stopPropagation()}
                    >
                      {tool.display_name}
                    </Checkbox>
                  ),
                  onClick: () => handleToolToggle(tool.key),
                })),
              }}
            >
              <Button type="dashed" size="small" icon={<PlusOutlined />} />
            </Dropdown>
          )}
        </div>
      </div>
    </Modal>
  );
};

export default AddMcpModal;
