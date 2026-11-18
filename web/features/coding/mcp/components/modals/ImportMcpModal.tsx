import React, { useMemo } from 'react';
import { Modal, Checkbox, Button, Empty, message, Spin, Tag, Dropdown } from 'antd';
import { PlusOutlined, WarningOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useMcpStore } from '../../stores/mcpStore';
import { useMcpTools } from '../../hooks/useMcpTools';
import type { McpServer, McpDiscoveredServer, StdioConfig, HttpConfig } from '../../types';
import * as mcpApi from '../../services/mcpApi';
import styles from './ImportMcpModal.module.less';
import addMcpStyles from './AddMcpModal.module.less';

interface ImportMcpModalProps {
  open: boolean;
  onClose: () => void;
  onSuccess: () => void;
}

/** Synthetic tool_key for the CC Switch scan group (mirrors backend CC_SWITCH_MCP_TOOL_KEY). */
const CC_SWITCH_TOOL_KEY = 'cc_switch';

export const ImportMcpModal: React.FC<ImportMcpModalProps> = ({
  open,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const { fetchServers, scanResult, loadScanResult, servers: existingServers } = useMcpStore();
  const { tools } = useMcpTools();
  const [selected, setSelected] = React.useState<Set<string>>(new Set());
  const [selectedTools, setSelectedTools] = React.useState<string[]>([]);
  const [followCcSwitchMarks, setFollowCcSwitchMarks] = React.useState(true);
  const [loading, setLoading] = React.useState(false);
  const [scanning, setScanning] = React.useState(false);
  const [preferredTools, setPreferredTools] = React.useState<string[] | null>(null);
  const [showDuplicateModal, setShowDuplicateModal] = React.useState(false);
  const [overlappingNames, setOverlappingNames] = React.useState<string[]>([]);

  // Group discovered servers by tool_key
  const serversByTool = React.useMemo(() => {
    const map = new Map<string, McpDiscoveredServer[]>();
    if (scanResult?.servers) {
      for (const server of scanResult.servers) {
        const list = map.get(server.tool_key) || [];
        list.push(server);
        map.set(server.tool_key, list);
      }
    }
    return map;
  }, [scanResult]);

  // Only show tools that have discovered servers
  const toolsWithServers = React.useMemo(() => {
    return tools.filter((tool) => {
      const servers = serversByTool.get(tool.key);
      return servers && servers.length > 0 && tool.installed;
    });
  }, [tools, serversByTool]);

  // Plugin groups: scan results whose tool_key is not in the standard tools list
  const pluginGroups = React.useMemo(() => {
    const toolKeys = new Set(tools.map((t) => t.key));
    const groups: { key: string; display_name: string; servers: McpDiscoveredServer[] }[] = [];
    for (const [toolKey, servers] of serversByTool) {
      if (!toolKeys.has(toolKey) && servers.length > 0) {
        // Use tool_name from the first server (set by backend)
        groups.push({ key: toolKey, display_name: servers[0].tool_name, servers });
      }
    }
    return groups;
  }, [tools, serversByTool]);

  // Combined selectable items (standard tools + plugin groups)
  const allSelectableKeys = React.useMemo(() => {
    return [
      ...toolsWithServers.map((t) => t.key),
      ...pluginGroups.map((g) => g.key),
    ];
  }, [toolsWithServers, pluginGroups]);

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

  // Track if we've initialized selection for this open session
  const initializedRef = React.useRef(false);
  const toolsInitializedRef = React.useRef(false);

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

  // Reset initialized state when modal closes
  React.useEffect(() => {
    if (!open) {
      initializedRef.current = false;
      toolsInitializedRef.current = false;
    }
  }, [open]);

  // Trigger scan when modal opens
  React.useEffect(() => {
    if (open) {
      setScanning(true);
      loadScanResult().finally(() => setScanning(false));
    }
  }, [open, loadScanResult]);

  React.useEffect(() => {
    if (!initializedRef.current && toolsWithServers.length > 0) {
      // Don't pre-select any tools by default
      setSelected(new Set());
      initializedRef.current = true;
    }
  }, [toolsWithServers]);

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

  const handleToolToggle = (toolKey: string) => {
    setSelectedTools((prev) =>
      prev.includes(toolKey)
        ? prev.filter((k) => k !== toolKey)
        : [...prev, toolKey]
    );
  };

  const handleSelectAll = () => {
    if (selected.size === allSelectableKeys.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(allSelectableKeys));
    }
  };

  /** Get a command-based fingerprint for a server (works for both McpServer and McpDiscoveredServer) */
  const getServerKey = (server: { server_type: string; server_config: StdioConfig | HttpConfig }): string => {
    if (server.server_type === 'stdio') {
      const config = server.server_config as StdioConfig;
      return `stdio:${config.command}:${JSON.stringify([...(config.args || [])].sort())}`;
    }
    const config = server.server_config as HttpConfig;
    return `${server.server_type}:${config.url}`;
  };

  const toolDisplayName = (toolKey: string): string =>
    tools.find((tool) => tool.key === toolKey)?.display_name ?? toolKey;

  /** Server name tag plus small agent-mark badges for CC Switch sources. */
  const renderServerTag = (server: McpDiscoveredServer) => (
    <span key={server.name} className={styles.serverTagGroup}>
      <Tag className={styles.serverTag}>{server.name}</Tag>
      {(server.source_enabled_tools ?? []).map((toolKey) => (
        <span key={toolKey} className={styles.serverMarkTag}>
          {toolDisplayName(toolKey)}
        </span>
      ))}
    </span>
  );

  // Detect potential duplicates using command fingerprints:
  // 1. Scan servers whose command fingerprint matches an existing server in the store
  // 2. Scan servers across multiple selected tools that share the same command fingerprint
  const getOverlappingNames = (): string[] => {
    const existingKeys = new Set(existingServers.map((s) => getServerKey(s)));
    const seen = new Map<string, string>(); // fingerprint -> first server name
    const duplicates = new Set<string>();

    for (const toolKey of selected) {
      const servers = serversByTool.get(toolKey) || [];
      for (const server of servers) {
        const key = getServerKey(server);
        if (existingKeys.has(key)) {
          // Command already exists in the store
          duplicates.add(server.name);
        } else if (seen.has(key)) {
          // Same command in another selected tool
          duplicates.add(server.name);
          duplicates.add(seen.get(key)!);
        } else {
          seen.set(key, server.name);
        }
      }
    }
    return Array.from(duplicates);
  };

  const handleImportClick = () => {
    if (selected.size === 0) return;

    // Check for potential duplicates before importing
    const overlap = getOverlappingNames();
    if (overlap.length > 0) {
      setOverlappingNames(overlap);
      setShowDuplicateModal(true);
    } else {
      doImport(false);
    }
  };

  const doImport = async (removeDuplicatesAfter: boolean) => {
    setLoading(true);
    let totalImported = 0;
    let totalSkipped = 0;
    const allDuplicated: string[] = [];
    const errors: string[] = [];

    // Snapshot existing server IDs before import, so we can scope dedup to new servers only
    const preImportIds = new Set(existingServers.map((s) => s.id));

    try {
      for (const toolKey of selected) {
        try {
          const result = await mcpApi.importMcpFromTool(toolKey, selectedTools, followCcSwitchMarks);
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

      if (errors.length > 0) {
        console.error('Import errors:', errors);
      }

      await fetchServers();

      // Remove duplicates if requested — only among newly imported servers
      if (removeDuplicatesAfter) {
        const { servers: currentServers } = useMcpStore.getState();
        const newServers = currentServers.filter((s) => !preImportIds.has(s.id));
        const toDelete = findImportDuplicates(currentServers, newServers);

        if (toDelete.length > 0) {
          for (const server of toDelete) {
            await mcpApi.deleteMcpServer(server.id);
          }
          await fetchServers();
          totalImported -= toDelete.length;
        }

        if (totalImported > 0) {
          message.success(t('mcp.importSuccess', { count: totalImported }));
        } else if (totalSkipped > 0) {
          message.info(t('mcp.importSkipped', { count: totalSkipped }));
        }
        if (toDelete.length > 0) {
          message.success(t('mcp.clearDuplicates.success', { count: toDelete.length }));
        }
      } else {
        if (totalImported > 0) {
          message.success(t('mcp.importSuccess', { count: totalImported }));
        } else if (totalSkipped > 0) {
          message.info(t('mcp.importSkipped', { count: totalSkipped }));
        } else {
          message.info(t('mcp.importNoServers'));
        }
      }

      onSuccess();
    } catch (error) {
      message.error(t('mcp.importFailed') + ': ' + String(error));
    } finally {
      setLoading(false);
    }
  };

  /**
   * Find newly imported servers that are command-duplicates of existing ones.
   *
   * For each new server, if an older server (pre-existing OR earlier in the new batch)
   * shares the same command fingerprint, the new server is marked for deletion.
   * This ensures only new duplicates are removed — user's pre-existing servers are never touched.
   */
  const findImportDuplicates = (allServers: McpServer[], newServers: McpServer[]): McpServer[] => {
    const newIds = new Set(newServers.map((s) => s.id));

    // Group ALL servers by command key
    const groups = new Map<string, McpServer[]>();
    for (const server of allServers) {
      const key = getServerKey(server);
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(server);
    }

    const toDelete: McpServer[] = [];
    for (const group of groups.values()) {
      if (group.length <= 1) continue;
      // Sort by created_at ascending — oldest first
      group.sort((a, b) => a.created_at - b.created_at);
      // Keep the oldest one, delete the rest — but ONLY if they are new servers
      for (let i = 1; i < group.length; i++) {
        if (newIds.has(group[i].id)) {
          toDelete.push(group[i]);
        }
      }
    }
    return toDelete;
  };

  const handleCancelDuplicate = () => {
    setShowDuplicateModal(false);
    setOverlappingNames([]);
  };

  const handleContinueImport = () => {
    setShowDuplicateModal(false);
    setOverlappingNames([]);
    doImport(false);
  };

  const handleRemoveDuplicates = () => {
    setShowDuplicateModal(false);
    setOverlappingNames([]);
    doImport(true);
  };

  const totalServersFound = scanResult?.total_servers_found || 0;

  return (
    <Modal
      title={t('mcp.importTitle')}
      open={open}
      onCancel={onClose}
      footer={null}
      width={600}
    >
      <Spin spinning={loading || scanning}>
        <p className={styles.hint}>{t('mcp.importSummary')}</p>

        {scanning ? (
          <div className={styles.scanningHint}>
            {t('mcp.scanning')}
          </div>
        ) : (
          <>
            <div className={styles.stats}>
              <span>{t('mcp.serversFound', { count: totalServersFound })}</span>
            </div>

            {allSelectableKeys.length === 0 ? (
              <Empty description={t('mcp.noToolsToImport')} />
            ) : (
              <>
                <div className={styles.selectAll}>
                  <Checkbox
                    checked={selected.size === allSelectableKeys.length}
                    indeterminate={selected.size > 0 && selected.size < allSelectableKeys.length}
                    onChange={handleSelectAll}
                  >
                    {t('mcp.selectAll')}
                  </Checkbox>
                  <span className={styles.count}>
                    {t('mcp.selectedCount', {
                      selected: selected.size,
                      total: allSelectableKeys.length,
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
                            {servers.map((s) => renderServerTag(s))}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                  {pluginGroups.map((group) => (
                    <div
                      key={group.key}
                      className={`${styles.toolItem} ${selected.has(group.key) ? styles.selected : ''}`}
                      onClick={() => handleToggle(group.key)}
                    >
                      <Checkbox checked={selected.has(group.key)} />
                      <div className={styles.toolInfo}>
                        <div className={styles.toolHeader}>
                          <span className={styles.toolName}>{group.display_name}</span>
                          <span className={styles.toolPath}>{t('mcp.pluginSource')}</span>
                        </div>
                        <div className={styles.serverList}>
                          {group.servers.map((s) => renderServerTag(s))}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>

                <div className={addMcpStyles.toolsSection}>
                  {selected.has(CC_SWITCH_TOOL_KEY) && (
                    <div className={styles.followMarksRow}>
                      <Checkbox
                        checked={followCcSwitchMarks}
                        onChange={(e) => setFollowCcSwitchMarks(e.target.checked)}
                      >
                        {t('mcp.followCcSwitchMarks')}
                      </Checkbox>
                      <div className={styles.followMarksHint}>
                        {t(
                          followCcSwitchMarks
                            ? 'mcp.followCcSwitchMarksHintOn'
                            : 'mcp.followCcSwitchMarksHintOff'
                        )}
                      </div>
                    </div>
                  )}
                  <div className={addMcpStyles.toolsLabel}>{t('mcp.enabledTools')}</div>
                  <div className={addMcpStyles.toolsHint}>{t('mcp.enabledToolsHint')}</div>
                  <div className={addMcpStyles.toolsGrid}>
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
                      <span className={addMcpStyles.noTools}>{t('mcp.noToolsInstalled')}</span>
                    )}
                    {hiddenTools.length > 0 && (
                      <Dropdown
                        trigger={['click']}
                        menu={{
                          items: hiddenTools.map((tool) => ({
                            key: tool.key,
                            label: (
                              <span onClick={(e) => e.stopPropagation()}>
                                <Checkbox
                                  checked={selectedTools.includes(tool.key)}
                                  onChange={() => handleToolToggle(tool.key)}
                                  onClick={(e) => e.stopPropagation()}
                                >
                                  {tool.display_name}
                                </Checkbox>
                              </span>
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
              </>
            )}
          </>
        )}

        <div className={styles.footer}>
          <Button onClick={onClose}>{t('common.close')}</Button>
          <Button
            type="primary"
            onClick={handleImportClick}
            disabled={selected.size === 0 || scanning}
            loading={loading}
          >
            {t('mcp.importAndSync')}
          </Button>
        </div>
      </Spin>

      {showDuplicateModal && (
        <Modal
          title={
            <span>
              <WarningOutlined style={{ color: '#faad14', marginRight: 8 }} />
              {t('mcp.importDuplicateModal.title')}
            </span>
          }
          open={showDuplicateModal}
          onCancel={handleCancelDuplicate}
          footer={null}
          width={500}
        >
          <div style={{ marginBottom: 16 }}>
            <p>{t('mcp.importDuplicateModal.message')}</p>
            <div style={{ margin: '12px 0', display: 'flex', flexWrap: 'wrap', gap: 4 }}>
              {overlappingNames.map((name) => (
                <Tag key={name}>{name}</Tag>
              ))}
            </div>
            <p style={{ color: 'var(--color-text-tertiary)', fontSize: 12 }}>
              {t('mcp.importDuplicateModal.hint')}
            </p>
          </div>
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
            <Button onClick={handleCancelDuplicate}>
              {t('common.cancel')}
            </Button>
            <Button onClick={handleContinueImport}>
              {t('mcp.importDuplicateModal.keepAll')}
            </Button>
            <Button
              type="primary"
              danger
              onClick={handleRemoveDuplicates}
            >
              {t('mcp.importDuplicateModal.removeDuplicates')}
            </Button>
          </div>
        </Modal>
      )}
    </Modal>
  );
};
