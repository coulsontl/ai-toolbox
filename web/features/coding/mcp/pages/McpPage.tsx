import React, { useState, useCallback } from 'react';
import { Typography, Button, Space, Modal } from 'antd';
import { PlusOutlined, UserOutlined, ImportOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { arrayMove } from '@dnd-kit/sortable';
import type { DragEndEvent } from '@dnd-kit/core';
import { useMcp } from '../hooks/useMcp';
import { useMcpActions } from '../hooks/useMcpActions';
import { useMcpTools } from '../hooks/useMcpTools';
import { useMcpStore } from '../stores/mcpStore';
import { McpList } from '../components/McpList';
import { AddMcpModal } from '../components/modals/AddMcpModal';
import { McpSettingsModal } from '../components/modals/McpSettingsModal';
import { ImportMcpModal } from '../components/modals/ImportMcpModal';
import type { McpServer, CreateMcpServerInput, UpdateMcpServerInput } from '../types';
import styles from './McpPage.module.less';

const { Title } = Typography;

const McpPage: React.FC = () => {
  const { t } = useTranslation();
  const { servers, loading, scanResult } = useMcp();
  const { tools } = useMcpTools();
  const { setServers, isSettingsModalOpen, setSettingsModalOpen, isImportModalOpen, setImportModalOpen, loadScanResult } = useMcpStore();
  const {
    createServer,
    editServer,
    deleteServer,
    toggleTool,
    reorderServers,
  } = useMcpActions();

  const [isAddModalOpen, setAddModalOpen] = useState(false);
  const [editingServer, setEditingServer] = useState<McpServer | null>(null);
  const [actionLoading, setActionLoading] = useState(false);

  const handleAddServer = async (input: CreateMcpServerInput) => {
    setActionLoading(true);
    try {
      await createServer(input);
      setAddModalOpen(false);
    } finally {
      setActionLoading(false);
    }
  };

  const handleUpdateServer = async (serverId: string, input: UpdateMcpServerInput) => {
    setActionLoading(true);
    try {
      await editServer(serverId, input);
      setEditingServer(null);
      setAddModalOpen(false);
    } finally {
      setActionLoading(false);
    }
  };

  const handleEdit = (server: McpServer) => {
    setEditingServer(server);
    setAddModalOpen(true);
  };

  const handleCloseModal = () => {
    setAddModalOpen(false);
    setEditingServer(null);
  };

  const handleDelete = (serverId: string) => {
    const serverToDelete = servers.find((s) => s.id === serverId);
    Modal.confirm({
      title: t('mcp.deleteConfirm'),
      content: t('mcp.deleteConfirmContent', { name: serverToDelete?.name }),
      okText: t('common.delete'),
      okType: 'danger',
      cancelText: t('common.cancel'),
      onOk: async () => {
        setActionLoading(true);
        try {
          await deleteServer(serverId);
        } finally {
          setActionLoading(false);
        }
      },
    });
  };

  const handleToggleTool = async (serverId: string, toolKey: string) => {
    setActionLoading(true);
    try {
      await toggleTool(serverId, toolKey);
    } finally {
      setActionLoading(false);
    }
  };

  const handleDragEnd = useCallback(
    async (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;

      const oldIndex = servers.findIndex((s) => s.id === active.id);
      const newIndex = servers.findIndex((s) => s.id === over.id);

      if (oldIndex !== -1 && newIndex !== -1) {
        const newServers = arrayMove(servers, oldIndex, newIndex);
        setServers(newServers);
        const ids = newServers.map((s) => s.id);
        await reorderServers(ids);
      }
    },
    [servers, setServers, reorderServers]
  );

  const discoveredCount = scanResult?.total_servers_found || 0;

  return (
    <div className={styles.mcpPage}>
      <div className={styles.pageHeader}>
        <div>
          <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
            {t('mcp.title')}
          </Title>
        </div>
        <Button
          type="text"
          icon={<UserOutlined />}
          onClick={() => setSettingsModalOpen(true)}
        >
          {t('mcp.settings')}
        </Button>
      </div>

      <div className={styles.toolbar}>
        <Space size="small">
          {discoveredCount > 0 && (
            <Button
              type="text"
              icon={<ImportOutlined />}
              onClick={() => setImportModalOpen(true)}
              style={{ color: 'var(--color-text-tertiary)' }}
            >
              {t('mcp.importExisting')} ({discoveredCount})
            </Button>
          )}
          <Button
            type="link"
            icon={<PlusOutlined />}
            onClick={() => setAddModalOpen(true)}
          >
            {t('mcp.addServer')}
          </Button>
        </Space>
      </div>

      <div className={styles.content}>
        <McpList
          servers={servers}
          tools={tools}
          loading={loading || actionLoading}
          onEdit={handleEdit}
          onDelete={handleDelete}
          onToggleTool={handleToggleTool}
          onDragEnd={handleDragEnd}
        />
      </div>

      <AddMcpModal
        open={isAddModalOpen}
        tools={tools}
        editingServer={editingServer}
        onClose={handleCloseModal}
        onSubmit={handleAddServer}
        onUpdate={handleUpdateServer}
      />

      <McpSettingsModal
        open={isSettingsModalOpen}
        onClose={() => setSettingsModalOpen(false)}
      />

      <ImportMcpModal
        open={isImportModalOpen}
        onClose={() => setImportModalOpen(false)}
        onSuccess={() => {
          setImportModalOpen(false);
          loadScanResult();
        }}
      />
    </div>
  );
};

export default McpPage;
