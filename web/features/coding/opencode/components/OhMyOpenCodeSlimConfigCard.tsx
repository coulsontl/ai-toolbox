import React from 'react';
import { Card, Typography, Space, Button, Tag, Tooltip } from 'antd';
import { EditOutlined, CopyOutlined, DeleteOutlined, CheckCircleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { SLIM_AGENT_TYPES, SLIM_AGENT_DISPLAY_NAMES, type OhMyOpenCodeSlimConfig } from '@/types/ohMyOpenCodeSlim';

const { Text } = Typography;

// Standard agent types count
const STANDARD_AGENT_COUNT = SLIM_AGENT_TYPES.length;

interface OhMyOpenCodeSlimConfigCardProps {
  config: OhMyOpenCodeSlimConfig;
  isSelected?: boolean;
  disabled?: boolean;
  onEdit: (config: OhMyOpenCodeSlimConfig) => void;
  onCopy: (config: OhMyOpenCodeSlimConfig) => void;
  onDelete: (config: OhMyOpenCodeSlimConfig) => void;
  onApply: (config: OhMyOpenCodeSlimConfig) => void;
}

const OhMyOpenCodeSlimConfigCard: React.FC<OhMyOpenCodeSlimConfigCardProps> = ({
  config,
  isSelected = false,
  disabled = false,
  onEdit,
  onCopy,
  onDelete,
  onApply,
}) => {
  const { t } = useTranslation();

  // Get configured agents as structured data (sorted)
  const getAgentsData = (): { name: string; model: string }[] => {
    const result: { name: string; model: string }[] = [];

    // Handle null agents
    if (!config.agents) {
      return result;
    }

    // Iterate in the predefined order
    SLIM_AGENT_TYPES.forEach((agentType) => {
      const agent = config.agents?.[agentType];
      if (agent && typeof agent.model === 'string' && agent.model) {
        const displayName = SLIM_AGENT_DISPLAY_NAMES[agentType].split(' ')[0]; // Get short name
        result.push({ name: displayName, model: agent.model });
      }
    });

    return result;
  };

  const agentsData = getAgentsData();

  // Get configured count
  const configuredCount = config.agents
    ? Object.values(config.agents).filter((a) => !!a && typeof a.model === 'string' && !!a.model).length
    : 0;
  const totalAgents = STANDARD_AGENT_COUNT;

  return (
    <Card
      size="small"
      style={{
        marginBottom: 8,
        borderColor: isSelected ? '#1890ff' : undefined,
        backgroundColor: isSelected ? '#e6f7ff' : undefined,
      }}
      styles={{ body: { padding: '8px 12px' } }}
    >
      {/* 第一行：配置名称、标签和操作按钮 */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <Text strong style={{ fontSize: 14, whiteSpace: 'nowrap' }}>{config.name}</Text>

          <Tag color="blue" style={{ margin: 0 }}>
            {configuredCount}/{totalAgents} Agent
          </Tag>

          {isSelected && (
            <Tag color="green" icon={<CheckCircleOutlined />} style={{ margin: 0 }}>
              {t('opencode.ohMyOpenCode.applied')}
            </Tag>
          )}
        </div>

        {/* 右侧：操作按钮 */}
        <Space size={4}>
          {!isSelected && (
            <Button
              type="link"
              size="small"
              onClick={() => onApply(config)}
              style={{ padding: '0 8px' }}
              disabled={disabled}
            >
              {t('opencode.ohMyOpenCode.apply')}
            </Button>
          )}
          <Tooltip title={t('common.edit')}>
            <Button
              type="text"
              size="small"
              icon={<EditOutlined />}
              onClick={() => onEdit(config)}
              disabled={disabled}
            />
          </Tooltip>
          <Tooltip title={t('common.copy')}>
            <Button
              type="text"
              size="small"
              icon={<CopyOutlined />}
              onClick={() => onCopy(config)}
              disabled={disabled}
            />
          </Tooltip>
          <Tooltip title={t('common.delete')}>
            <Button
              type="text"
              size="small"
              danger
              icon={<DeleteOutlined />}
              onClick={() => onDelete(config)}
              disabled={disabled}
            />
          </Tooltip>
        </Space>
      </div>

      {/* 第二行：Agent 详情（结构化展示） */}
      {agentsData.length > 0 && (
        <div style={{ marginTop: 6 }}>
          <div style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: '4px 12px',
            lineHeight: '1.6'
          }}>
            {agentsData.map((item, index) => (
              <span key={index} style={{ fontSize: 12, whiteSpace: 'nowrap' }}>
                <Text strong style={{ color: '#1890ff', fontSize: 12 }}>{item.name}</Text>
                <Text type="secondary" style={{ fontSize: 12 }}>: </Text>
                <Text type="secondary" style={{ fontSize: 12 }}>{item.model}</Text>
              </span>
            ))}
          </div>
        </div>
      )}

      {agentsData.length === 0 && (
        <div style={{ marginTop: 4 }}>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t('opencode.ohMyOpenCode.noAgentsConfigured')}
          </Text>
        </div>
      )}
    </Card>
  );
};

export default OhMyOpenCodeSlimConfigCard;
