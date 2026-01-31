import React from 'react';
import { Modal, Checkbox, Button, Empty, message, Spin, Tag } from 'antd';
import { useTranslation } from 'react-i18next';
import { useMcpStore } from '../../stores/mcpStore';
import { useMcpTools } from '../../hooks/useMcpTools';
import * as mcpApi from '../../services/mcpApi';
import styles from './ImportMcpModal.module.less';

interface ImportMcpModalProps {
  open: boolean;
  onClose: () => void;
  onSuccess: () => void;
}

export const ImportMcpModal: React.FC<ImportMcpModalProps> = ({
  open,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const { fetchServers, scanResult } = useMcpStore();
  const { installedMcpTools } = useMcpTools();
  const [selected, setSelected] = React.useState<Set<string>>(new Set());
  const [loading, setLoading] = React.useState(false);

  // Group discovered servers by tool_key
  const serversByTool = React.useMemo(() => {
    const map = new Map<string, string[]>();
    if (scanResult?.servers) {
      for (const server of scanResult.servers) {
        const list = map.get(server.tool_key) || [];
        list.push(server.name);
        map.set(server.tool_key, list);
      }
    }
    return map;
  }, [scanResult]);

  // Only show tools that have discovered servers
  const toolsWithServers = React.useMemo(() => {
    return installedMcpTools.filter((tool) => {
      const servers = serversByTool.get(tool.key);
      return servers && servers.length > 0;
    });
  }, [installedMcpTools, serversByTool]);

  // Track if we've initialized selection for this open session
  const initializedRef = React.useRef(false);

  React.useEffect(() => {
    if (open && !initializedRef.current) {
      // Pre-select all tools that have servers (only on first open)
      const allToolKeys = toolsWithServers.map((t) => t.key);
      setSelected(new Set(allToolKeys));
      initializedRef.current = true;
    } else if (!open) {
      // Reset when modal closes
      initializedRef.current = false;
    }
  }, [open, toolsWithServers]);

  const handleToggle = (toolKey: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(toolKey)) {
        next.delete(toolKey);
      } else {
        next.add(toolKey);
      }
      return next;
    });
  };

  const handleSelectAll = () => {
    const allKeys = toolsWithServers.map((t) => t.key);
    if (selected.size === allKeys.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(allKeys));
    }
  };

  const handleImport = async () => {
    if (selected.size === 0) return;

    setLoading(true);
    let totalImported = 0;
    let totalSkipped = 0;
    const allDuplicated: string[] = [];
    const errors: string[] = [];

    try {
      for (const toolKey of selected) {
        try {
          const result = await mcpApi.importMcpFromTool(toolKey);
          totalImported += result.servers_imported;
          totalSkipped += result.servers_skipped;
          if (result.servers_duplicated?.length > 0) {
            allDuplicated.push(...result.servers_duplicated);
          }
          if (result.errors.length > 0) {
            errors.push(...result.errors);
          }
        } catch (error) {
          errors.push(`${toolKey}: ${String(error)}`);
        }
      }

      if (totalImported > 0) {
        message.success(t('mcp.importSuccess', { count: totalImported }));
      } else if (totalSkipped > 0) {
        message.info(t('mcp.importSkipped', { count: totalSkipped }));
      } else {
        message.info(t('mcp.importNoServers'));
      }

      // Show warning for duplicated servers with different configs
      if (allDuplicated.length > 0) {
        message.warning(
          t('mcp.importDuplicated', { names: allDuplicated.join(', ') }),
          5
        );
      }

      if (errors.length > 0) {
        console.error('Import errors:', errors);
      }

      await fetchServers();
      onSuccess();
    } catch (error) {
      message.error(t('mcp.importFailed') + ': ' + String(error));
    } finally {
      setLoading(false);
    }
  };

  const totalServersFound = scanResult?.total_servers_found || 0;

  return (
    <Modal
      title={t('mcp.importTitle')}
      open={open}
      onCancel={onClose}
      footer={null}
      width={600}
      destroyOnClose
    >
      <Spin spinning={loading}>
        <p className={styles.hint}>{t('mcp.importSummary')}</p>

        <div className={styles.stats}>
          <span>{t('mcp.serversFound', { count: totalServersFound })}</span>
        </div>

        {toolsWithServers.length === 0 ? (
          <Empty description={t('mcp.noToolsToImport')} />
        ) : (
          <>
            <div className={styles.selectAll}>
              <Checkbox
                checked={selected.size === toolsWithServers.length}
                indeterminate={selected.size > 0 && selected.size < toolsWithServers.length}
                onChange={handleSelectAll}
              >
                {t('mcp.selectAll')}
              </Checkbox>
              <span className={styles.count}>
                {t('mcp.selectedCount', {
                  selected: selected.size,
                  total: toolsWithServers.length,
                })}
              </span>
            </div>

            <div className={styles.list}>
              {toolsWithServers.map((tool) => {
                const servers = serversByTool.get(tool.key) || [];
                return (
                  <div
                    key={tool.key}
                    className={`${styles.toolItem} ${selected.has(tool.key) ? styles.selected : ''}`}
                    onClick={() => handleToggle(tool.key)}
                  >
                    <Checkbox checked={selected.has(tool.key)} />
                    <div className={styles.toolInfo}>
                      <div className={styles.toolHeader}>
                        <span className={styles.toolName}>{tool.display_name}</span>
                        <span className={styles.toolPath}>{tool.mcp_config_path}</span>
                      </div>
                      <div className={styles.serverList}>
                        {servers.map((name) => (
                          <Tag key={name} className={styles.serverTag}>{name}</Tag>
                        ))}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        )}

        <div className={styles.footer}>
          <Button onClick={onClose}>{t('common.close')}</Button>
          <Button
            type="primary"
            onClick={handleImport}
            disabled={selected.size === 0}
            loading={loading}
          >
            {t('mcp.importAndSync')}
          </Button>
        </div>
      </Spin>
    </Modal>
  );
};
