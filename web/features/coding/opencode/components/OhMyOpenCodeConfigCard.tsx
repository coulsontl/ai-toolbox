import React from 'react';
import { Card, Typography, Space, Button, Tag, Switch, Dropdown, message } from 'antd';
import { EditOutlined, CopyOutlined, DeleteOutlined, CheckCircleOutlined, MoreOutlined, HolderOutlined } from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { OH_MY_OPENCODE_AGENTS, type OhMyOpenCodeConfig, type OhMyOpenCodeAgentType } from '@/types/ohMyOpenCode';
import { getAgentDisplayName } from '@/services/ohMyOpenCodeApi';

const { Text } = Typography;

// Standard agent types count - auto-calculated from centralized constant
const STANDARD_AGENT_COUNT = OH_MY_OPENCODE_AGENTS.length;

interface OhMyOpenCodeConfigCardProps {
  config: OhMyOpenCodeConfig;
  isSelected?: boolean;
  disabled?: boolean;
  onEdit: (config: OhMyOpenCodeConfig) => void;
  onCopy: (config: OhMyOpenCodeConfig) => void;
  onDelete: (config: OhMyOpenCodeConfig) => void;
  onApply: (config: OhMyOpenCodeConfig) => void;
  onToggleDisabled: (config: OhMyOpenCodeConfig, isDisabled: boolean) => void;
}

const OhMyOpenCodeConfigCard: React.FC<OhMyOpenCodeConfigCardProps> = ({
  config,
  isSelected = false,
  disabled = false,
  onEdit,
  onCopy,
  onDelete,
  onApply,
  onToggleDisabled,
}) => {
  const { t } = useTranslation();

  // 拖拽排序
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: config.id });

  const sortableStyle = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : (config.isDisabled ? 0.6 : 1),
  };

  const handleToggleDisabled = (checked: boolean) => {
    if (isSelected && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(config, !checked);  // Switch 的 checked 表示"启用"，所以取反
  };

  // Agent display order - from centralized constant
  const AGENT_ORDER: OhMyOpenCodeAgentType[] = OH_MY_OPENCODE_AGENTS.map((a) => a.key);
  const BUILT_IN_AGENT_KEYS = new Set(AGENT_ORDER);

  // Get configured agents as structured data (sorted)
  const getAgentsData = (): { name: string; model: string; isCustom?: boolean }[] => {
    const result: { name: string; model: string; isCustom?: boolean }[] = [];

    // Handle null agents
    if (!config.agents) {
      return result;
    }

    // Iterate in the predefined order for built-in agents
    AGENT_ORDER.forEach((agentType) => {
      const agent = config.agents?.[agentType];
      if (agent && typeof agent.model === 'string' && agent.model) {
        const displayName = getAgentDisplayName(agentType).split(' ')[0]; // Get short name
        result.push({ name: displayName, model: agent.model });
      }
    });

    // Add custom agents (keys not in built-in list)
    Object.keys(config.agents).forEach((key) => {
      if (!BUILT_IN_AGENT_KEYS.has(key as OhMyOpenCodeAgentType)) {
        const agent = config.agents?.[key as OhMyOpenCodeAgentType];
        if (agent && typeof agent.model === 'string' && agent.model) {
          result.push({ name: key, model: agent.model, isCustom: true });
        }
      }
    });

    return result;
  };

  // Get custom agents count
  const getCustomAgentsCount = (): number => {
    if (!config.agents) return 0;
    return Object.keys(config.agents).filter(key => !BUILT_IN_AGENT_KEYS.has(key as OhMyOpenCodeAgentType)).length;
  };

  // Get custom categories count
  const getCustomCategoriesCount = (): number => {
    if (!config.categories) return 0;
    // All categories are considered custom since there's no built-in list
    return Object.keys(config.categories).length;
  };

  const agentsData = getAgentsData();
  const customAgentsCount = getCustomAgentsCount();
  const customCategoriesCount = getCustomCategoriesCount();

  // Get configured count (only built-in agents for the X/Y display)
  const configuredCount = config.agents 
    ? Object.keys(config.agents).filter((key) => {
        const agent = config.agents?.[key as OhMyOpenCodeAgentType];
        return BUILT_IN_AGENT_KEYS.has(key as OhMyOpenCodeAgentType) && agent && typeof agent.model === 'string' && !!agent.model;
      }).length
    : 0;
  const totalAgents = STANDARD_AGENT_COUNT; // Use standard agent count instead of actual keys

  const menuItems: MenuProps['items'] = [
    {
      key: 'toggle',
      label: (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span>{t('common.enable')}</span>
            <Text type="secondary" style={{ fontSize: 11 }}>
              {config.isDisabled
                ? t('opencode.ohMyOpenCode.configDisabled')
                : t('opencode.ohMyOpenCode.configEnabled')}
            </Text>
          </div>
          <Switch
            checked={!config.isDisabled}
            onChange={handleToggleDisabled}
            size="small"
            disabled={disabled}
          />
        </div>
      ),
    },
    {
      key: 'edit',
      label: t('common.edit'),
      icon: <EditOutlined />,
      onClick: () => onEdit(config),
    },
    {
      key: 'copy',
      label: t('common.copy'),
      icon: <CopyOutlined />,
      onClick: () => onCopy(config),
    },
    // Hide delete button for __local__ config
    ...(config.id !== '__local__' ? [
      {
        type: 'divider' as const,
      },
      {
        key: 'delete',
        label: t('common.delete'),
        icon: <DeleteOutlined />,
        danger: true,
        onClick: () => onDelete(config),
      },
    ] : []),
  ].filter(Boolean) as MenuProps['items'];

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <Card
        size="small"
        style={{
          marginBottom: 8,
          borderColor: isSelected ? '#1890ff' : 'var(--color-border-secondary)',
          backgroundColor: isSelected ? 'var(--color-bg-selected)' : 'var(--color-bg-container)',
          transition: 'opacity 0.3s ease, border-color 0.2s ease',
        }}
        styles={{ body: { padding: '8px 12px' } }}
      >
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
          {/* 拖拽手柄 */}
          <div
            {...attributes}
            {...listeners}
            style={{
              cursor: isDragging ? 'grabbing' : 'grab',
              color: '#999',
              touchAction: 'none',
              padding: '4px 0',
            }}
          >
            <HolderOutlined />
          </div>

          {/* 右侧内容区 */}
          <div style={{ flex: 1 }}>
            {/* 第一行：配置名称、标签和操作按钮 */}
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
                <Text strong style={{ fontSize: 14, whiteSpace: 'nowrap' }}>{config.name}</Text>

                {/* __local__ 配置提示 */}
                {config.id === '__local__' && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    ({t('opencode.ohMyOpenCode.localConfigHint')})
                  </Text>
                )}

                <Tag color="blue" style={{ margin: 0 }}>
                  {configuredCount}/{totalAgents} Agent
                  {customAgentsCount > 0 && ` +${customAgentsCount}`}
                </Tag>

                {customCategoriesCount > 0 && (
                  <Tag color="purple" style={{ margin: 0 }}>
                    +{customCategoriesCount} Category
                  </Tag>
                )}

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
                    disabled={disabled || config.isDisabled}
                  >
                    {t('opencode.ohMyOpenCode.apply')}
                  </Button>
                )}
                <Dropdown menu={{ items: menuItems }} trigger={['click']}>
                  <Button
                    type="text"
                    size="small"
                    icon={<MoreOutlined />}
                    disabled={disabled}
                  />
                </Dropdown>
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
                      <Text strong style={{ color: item.isCustom ? '#722ed1' : '#1890ff', fontSize: 12 }}>{item.name}</Text>
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
          </div>
        </div>
    </Card>
    </div>
  );
};

export default OhMyOpenCodeConfigCard;
