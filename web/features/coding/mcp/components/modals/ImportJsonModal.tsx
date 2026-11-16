import React, { useState, useMemo } from 'react';
import { Modal, Button, Tag, Checkbox, message, Dropdown } from 'antd';
import { PlusOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useMcpTools } from '../../hooks/useMcpTools';
import { useMcpStore } from '../../stores/mcpStore';
import * as mcpApi from '../../services/mcpApi';
import type { CreateMcpServerInput, McpServer, StdioConfig, HttpConfig } from '../../types';
import { parseMcpServersFromJsonValue } from '../../utils/mcpJsonImport';
import JsonEditor from '@/components/common/JsonEditor';
import styles from './ImportMcpModal.module.less';
import addMcpStyles from './AddMcpModal.module.less';

interface ParsedServer {
  name: string;
  server_type: 'stdio' | 'http' | 'sse';
  server_config: StdioConfig | HttpConfig;
  isDuplicate: boolean;
  existingId?: string;
}

interface ImportJsonModalProps {
  open: boolean;
  servers: McpServer[];
  onClose: () => void;
  onSuccess: () => void;
  onSyncAll?: () => Promise<unknown>;
}

export const ImportJsonModal: React.FC<ImportJsonModalProps> = ({
  open,
  servers,
  onClose,
  onSuccess,
  onSyncAll,
}) => {
  const { t } = useTranslation();
  const { tools } = useMcpTools();
  const { fetchServers } = useMcpStore();
  const [jsonValue, setJsonValue] = useState<unknown>(null);
  const [jsonValid, setJsonValid] = useState(false);
  const [parsedServers, setParsedServers] = useState<ParsedServer[]>([]);
  const [selectedServers, setSelectedServers] = useState<Set<string>>(new Set());
  const [selectedTools, setSelectedTools] = useState<string[]>([]);
  const [step, setStep] = useState<'input' | 'confirm'>('input');
  const [loading, setLoading] = useState(false);
  const [parseError, setParseError] = useState<string | null>(null);
  const [preferredTools, setPreferredTools] = useState<string[] | null>(null);

  // Track if we've initialized tools for this open session
  const toolsInitializedRef = React.useRef(false);

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

  // Load preferred tools on mount
  React.useEffect(() => {
    const loadPreferredTools = async () => {
      try {
        const preferred = await mcpApi.getMcpPreferredTools();
        setPreferredTools(preferred);
      } catch (error) {
        console.error('Failed to load preferred tools:', error);
      }
    };
    loadPreferredTools();
  }, []);

  // Reset state when modal closes
  React.useEffect(() => {
    if (!open) {
      toolsInitializedRef.current = false;
    }
  }, [open]);

  // Initialize selected tools based on preferredTools (same logic as AddMcpModal)
  React.useEffect(() => {
    if (open && !toolsInitializedRef.current && preferredTools !== null) {
      if (preferredTools.length > 0) {
        setSelectedTools(preferredTools);
      } else {
        // preferredTools loaded but empty, use installed tools
        const installed = tools.filter((t) => t.installed).map((t) => t.key);
        setSelectedTools(installed);
      }
      toolsInitializedRef.current = true;
    }
  }, [open, tools, preferredTools]);

  const resetState = () => {
    setJsonValue(null);
    setJsonValid(false);
    setParsedServers([]);
    setSelectedServers(new Set());
    setStep('input');
    setParseError(null);
  };

  const handleClose = () => {
    resetState();
    onClose();
  };

  const handleParse = () => {
    if (!jsonValid || !jsonValue) {
      setParseError(t('mcp.importJson.invalidJson'));
      return;
    }

    try {
      const result: ParsedServer[] = parseMcpServersFromJsonValue(jsonValue).map((server) => {
        const existing = servers.find((s) => s.name === server.name);

        return {
          name: server.name,
          server_type: server.server_type,
          server_config: server.server_config,
          isDuplicate: !!existing,
          existingId: existing?.id,
        };
      });

      if (result.length === 0) {
        throw new Error(t('mcp.importJson.noServersFound'));
      }

      setParsedServers(result);
      setSelectedServers(new Set(result.map((s) => s.name)));
      setParseError(null);
      setStep('confirm');
    } catch (error) {
      setParseError(String(error));
    }
  };

  const handleToggleServer = (name: string) => {
    setSelectedServers((prev) => {
      const next = new Set(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  };

  const handleSelectAllServers = () => {
    if (selectedServers.size === parsedServers.length) {
      setSelectedServers(new Set());
    } else {
      setSelectedServers(new Set(parsedServers.map((s) => s.name)));
    }
  };

  const handleToggleTool = (toolKey: string) => {
    setSelectedTools((prev) =>
      prev.includes(toolKey)
        ? prev.filter((k) => k !== toolKey)
        : [...prev, toolKey]
    );
  };

  const handleImport = async () => {
    if (selectedServers.size === 0) return;

    // Check if any selected servers are duplicates
    const duplicates = parsedServers.filter(
      (s) => selectedServers.has(s.name) && s.isDuplicate
    );

    const doImport = async () => {
      setLoading(true);
      let successCount = 0;
      let overwriteCount = 0;
      const errors: string[] = [];

      try {
        for (const server of parsedServers) {
          if (!selectedServers.has(server.name)) continue;

          try {
            if (server.isDuplicate && server.existingId) {
              // Overwrite existing server
              await mcpApi.updateMcpServer(server.existingId, {
                server_type: server.server_type,
                server_config: server.server_config,
                enabled_tools: selectedTools,
              });
              overwriteCount++;
            } else {
              // Create new server
              const input: CreateMcpServerInput = {
                name: server.name,
                server_type: server.server_type,
                server_config: server.server_config,
                enabled_tools: selectedTools,
              };
              await mcpApi.createMcpServer(input);
            }
            successCount++;
          } catch (error) {
            errors.push(`${server.name}: ${String(error)}`);
          }
        }

        if (successCount > 0) {
          message.success(t('mcp.importSuccess', { count: successCount }));
          await fetchServers();
          // Auto sync all tools if any servers were overwritten
          if (overwriteCount > 0 && onSyncAll) {
            await onSyncAll();
          }
          onSuccess();
          handleClose();
        } else if (errors.length > 0) {
          message.error(errors.join('; '));
        }
      } catch (error) {
        message.error(t('mcp.importFailed') + ': ' + String(error));
      } finally {
        setLoading(false);
      }
    };

    if (duplicates.length > 0) {
      Modal.confirm({
        title: t('mcp.duplicateName.title'),
        content: t('mcp.duplicateName.batchContent', {
          names: duplicates.map((s) => s.name).join(', '),
        }),
        okText: t('mcp.duplicateName.overwrite'),
        cancelText: t('common.cancel'),
        onOk: doImport,
      });
    } else {
      await doImport();
    }
  };

  const renderInputStep = () => (
    <>
      <p className={styles.hint}>{t('mcp.importJson.hint')}</p>
      <JsonEditor
        value={jsonValue}
        onChange={(val, isValid) => {
          setJsonValue(val);
          setJsonValid(isValid);
          if (parseError) setParseError(null);
        }}
        height={350}
        placeholder={t('mcp.importJson.placeholder')}
      />
      {parseError && (
        <div style={{ color: 'var(--color-error)', marginTop: 8, fontSize: 13 }}>
          {parseError}
        </div>
      )}
      <div className={styles.footer}>
        <Button onClick={handleClose}>{t('common.cancel')}</Button>
        <Button
          type="primary"
          onClick={handleParse}
          disabled={!jsonValid}
        >
          {t('mcp.importJson.parse')}
        </Button>
      </div>
    </>
  );

  const renderConfirmStep = () => (
    <>
      <p className={styles.hint}>{t('mcp.importJson.confirmHint')}</p>

      <div className={styles.selectAll}>
        <Checkbox
          checked={selectedServers.size === parsedServers.length}
          indeterminate={selectedServers.size > 0 && selectedServers.size < parsedServers.length}
          onChange={handleSelectAllServers}
        >
          {t('mcp.selectAll')}
        </Checkbox>
        <span className={styles.count}>
          {t('mcp.selectedCount', {
            selected: selectedServers.size,
            total: parsedServers.length,
          })}
        </span>
      </div>

      <div className={styles.list} style={{ maxHeight: 250 }}>
        {parsedServers.map((server) => (
          <div
            key={server.name}
            className={`${styles.toolItem} ${selectedServers.has(server.name) ? styles.selected : ''}`}
            onClick={() => handleToggleServer(server.name)}
          >
            <Checkbox checked={selectedServers.has(server.name)} />
            <div className={styles.toolInfo}>
              <div className={styles.toolHeader}>
                <span className={styles.toolName}>{server.name}</span>
                <Tag style={{ margin: 0 }}>{server.server_type}</Tag>
                {server.isDuplicate && (
                  <Tag color="orange" style={{ margin: 0 }}>{t('mcp.importJson.duplicate')}</Tag>
                )}
              </div>
              <div className={styles.toolPath}>
                {server.server_type === 'stdio'
                  ? (server.server_config as StdioConfig).command
                  : (server.server_config as HttpConfig).url}
              </div>
            </div>
          </div>
        ))}
      </div>

      <div className={addMcpStyles.toolsSection}>
        <div className={addMcpStyles.toolsLabel}>{t('mcp.enabledTools')}</div>
        <div className={addMcpStyles.toolsHint}>{t('mcp.enabledToolsHint')}</div>
        <div className={addMcpStyles.toolsGrid}>
          {visibleTools.length > 0 ? (
            visibleTools.map((tool) => (
              <Checkbox
                key={tool.key}
                checked={selectedTools.includes(tool.key)}
                onChange={() => handleToggleTool(tool.key)}
              >
                {tool.display_name}
              </Checkbox>
            ))
          ) : (
            <span className={addMcpStyles.noTools}>{t('mcp.noToolsInstalled')}</span>
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
                      onChange={() => handleToggleTool(tool.key)}
                      onClick={(e) => e.stopPropagation()}
                    >
                      {tool.display_name}
                    </Checkbox>
                  ),
                  onClick: () => handleToggleTool(tool.key),
                })),
              }}
            >
              <Button type="dashed" size="small" icon={<PlusOutlined />} />
            </Dropdown>
          )}
        </div>
      </div>

      <div className={styles.footer}>
        <Button onClick={() => setStep('input')}>{t('common.back')}</Button>
        <Button
          type="primary"
          onClick={handleImport}
          disabled={selectedServers.size === 0}
          loading={loading}
        >
          {t('mcp.importJson.import')} ({selectedServers.size})
        </Button>
      </div>
    </>
  );

  return (
    <Modal
      title={t('mcp.importJson.title')}
      open={open}
      onCancel={handleClose}
      footer={null}
      width={700}
    >
      {step === 'input' ? renderInputStep() : renderConfirmStep()}
    </Modal>
  );
};
