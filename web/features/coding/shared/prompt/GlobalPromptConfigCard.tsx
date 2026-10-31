import React from 'react';
import { Button, Card, Dropdown, Space, Typography, theme } from 'antd';
import {
  CheckOutlined,
  DeleteOutlined,
  DownOutlined,
  EditOutlined,
  HolderOutlined,
  MoreOutlined,
  StopOutlined,
  UpOutlined,
} from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { useTranslation } from 'react-i18next';
import { LOCAL_CONFIG_ID } from '../localConfig';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import MarkdownPreview from '@/components/common/MarkdownPreview';
import AppliedTag from '@/components/common/AppliedTag';
import type { GlobalPromptConfig } from '@/types/globalPrompt';
import styles from './GlobalPromptSettings.module.less';

const { Text } = Typography;

interface GlobalPromptConfigCardProps {
  config: GlobalPromptConfig;
  /** Name of the runtime prompt file, interpolated into the local-file hint. */
  promptFileName?: string;
  /** Extra note beside the name (the local-file hint, "not created yet"). */
  hint?: React.ReactNode;
  /**
   * Omit an action to hide it.
   *
   * Identity files are fixed built-ins: they can be edited, never applied,
   * disabled, deleted or added, so that section passes `onEdit` alone and this
   * card renders the same list without offering what does not exist there.
   */
  onEdit?: (config: GlobalPromptConfig) => void;
  onDelete?: (config: GlobalPromptConfig) => void;
  onApply?: (config: GlobalPromptConfig) => void;
  onDisable?: (config: GlobalPromptConfig) => void;
  /**
   * Whether this entry takes part in the list's drag ordering.
   *
   * `false` for a fixed set (identity files): the grip would render and drag
   * nothing, since there is no order to write back.
   */
  reorderable?: boolean;
}

const GlobalPromptConfigCard: React.FC<GlobalPromptConfigCardProps> = ({
  config,
  promptFileName,
  hint,
  onEdit,
  onDelete,
  onApply,
  onDisable,
  reorderable = true,
}) => {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const [expanded, setExpanded] = React.useState(false);
  const isLocalConfig = config.id === LOCAL_CONFIG_ID;
  // `__local__` is only a local-file bridge, not a managed applied preset.
  const showAsApplied = config.isApplied && !isLocalConfig;
  const draggable = reorderable && !isLocalConfig;
  // Captured in a local: narrowing a destructured parameter does not survive
  // into the click callback below.
  const applyConfig = onApply;
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: config.id, disabled: !draggable });

  const sortableStyle: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  const menuItems: MenuProps['items'] = [
    ...(onEdit
      ? [{
          key: 'edit',
          label: t('common.edit'),
          icon: <EditOutlined />,
          onClick: () => onEdit(config),
        }]
      : []),
    // Only the managed applied preset can be disabled: `__local__` already
    // represents the "not managed" state, and unapplied presets need no disable.
    ...(showAsApplied && onDisable
      ? [{
          key: 'disable',
          label: t('common.disable'),
          icon: <StopOutlined />,
          onClick: () => onDisable(config),
        }]
      : []),
    ...(onDelete && !isLocalConfig
      ? [{
          type: 'divider' as const,
        }, {
          key: 'delete',
          label: t('common.delete'),
          icon: <DeleteOutlined />,
          danger: true,
          onClick: () => onDelete(config),
        }]
      : []),
  ];

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <Card
        size="small"
        className={styles.card}
        style={{
          marginBottom: 8,
          borderColor: showAsApplied ? token.colorPrimary : 'var(--color-border-secondary)',
          backgroundColor: showAsApplied ? 'var(--color-bg-selected)' : 'var(--color-bg-container)',
          opacity: isLocalConfig ? 0.95 : 1,
          transition: 'opacity 0.3s ease, border-color 0.2s ease',
        }}
        styles={{ body: { padding: '8px 12px' } }}
      >
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
          {draggable && (
            <div
              className={styles.dragHandle}
              {...attributes}
              {...listeners}
              style={{
                cursor: isDragging ? 'grabbing' : 'grab',
                touchAction: 'none',
                padding: '4px 0',
              }}
            >
              <HolderOutlined />
            </div>
          )}
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className={styles.cardHeader}>
              <div className={styles.cardTitleRow}>
                <Text strong className={styles.cardName}>{config.name}</Text>
                {isLocalConfig && (
                  <Text type="secondary" className={styles.cardHint}>
                    ({t('common.prompt.localConfigHint', { file: promptFileName })})
                  </Text>
                )}
                {hint}
                {showAsApplied && (
                  <AppliedTag>
                    {t('common.prompt.applied')}
                  </AppliedTag>
                )}
              </div>
              <Space size={4}>
                {applyConfig && !isLocalConfig && !config.isApplied && (
                  <Button type="link" size="small" icon={<CheckOutlined />} onClick={() => applyConfig(config)}>
                    {t('common.prompt.apply')}
                  </Button>
                )}
                {menuItems.length > 0 && (
                  <Dropdown menu={{ items: menuItems }} trigger={['click']}>
                    <Button type="text" size="small" icon={<MoreOutlined />} />
                  </Dropdown>
                )}
              </Space>
            </div>
            <div className={styles.cardContent}>
              {expanded ? (
                <>
                  <MarkdownPreview
                    content={config.content}
                    className={styles.cardPreview}
                  />
                  <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
                    <Button
                      type="text"
                      size="small"
                      className={styles.expandToggle}
                      icon={<UpOutlined />}
                      aria-label={t('common.prompt.collapse')}
                      title={t('common.prompt.collapse')}
                      onClick={() => setExpanded(false)}
                    />
                  </div>
                </>
              ) : (
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                  <Text
                    ellipsis
                    style={{ flex: 1, minWidth: 0, color: 'inherit' }}
                  >
                    {config.content}
                  </Text>
                  <Button
                    type="text"
                    size="small"
                    className={styles.expandToggle}
                    icon={<DownOutlined />}
                    aria-label={t('common.prompt.expand')}
                    title={t('common.prompt.expand')}
                    onClick={() => setExpanded(true)}
                  />
                </div>
              )}
            </div>
          </div>
        </div>
      </Card>
    </div>
  );
};

export default GlobalPromptConfigCard;
