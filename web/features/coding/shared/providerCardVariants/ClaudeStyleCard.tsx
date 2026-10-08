import React from 'react';
import { Button, Dropdown, Space, Switch, Tooltip, Typography } from 'antd';
import type { MenuProps } from 'antd';
import {
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  MoreOutlined,
} from '@ant-design/icons';
import { Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import CardShell from './CardShell';
import type { ProviderCardVariantProps } from './types';

const { Text } = Typography;

/**
 * The Claude Code-style provider card.
 *
 * ```
 * ⠿  Name  [official] [applied]                    [应用] [更多 ▾]
 *    默认: claude-sonnet-4-5  Haiku: …  Sonnet: …  | notes  [连通性测试]
 * ```
 *
 * The distinguishing traits, against the other two styles:
 *
 * - **No model section.** These CLIs do not own a model catalog — the models
 *   live in the config the provider writes, so the card states the bindings
 *   (default / per-role) on the meta line instead of listing rows. A card that
 *   rendered a collapsible list here would imply an editor that does not exist.
 * - The **second line is a set of labelled bindings**, not a summary: each entry
 *   reads `Label: value` with the value monospaced. The Codex style uses the
 *   same line for free-form facts, and the OpenCode style for id/SDK/endpoint.
 * - The **primary action is a text link**, like the Codex style, because these
 *   CLIs do have a single active provider.
 */
const ClaudeStyleCard: React.FC<ProviderCardVariantProps> = ({
  provider,
  providerState,
  actions,
  nameTags,
  metaEntries,
  inlineActions,
  footer,
}) => {
  const { t } = useTranslation();
  const {
    isDisabled,
    onToggleDisabled,
    connectivityStatus,
    selectable = false,
    selected = false,
    onSelectChange,
    dimmed = false,
    draggable = false,
    sortableId,
    accent,
  } = providerState ?? {};

  const primary = actions?.primaryAction;

  /**
   * The "more" menu, in the order the bespoke cards used:
   * enable → edit → copy → share → ─── → delete.
   *
   * Every entry carries its icon, and delete sits behind a divider — the
   * bespoke cards had both, and a menu that silently loses them looks like a
   * different product while still working.
   */
  const menuItems: MenuProps['items'] = [
    // The enable/disable switch lives in the menu, not the header: it is a
    // rarely-used, stateful toggle, and a Switch in the header row would sit
    // beside the primary action and compete with it.
    ...(onToggleDisabled
      ? [{
          key: 'toggle',
          label: (
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <span>{t('common.enable', { defaultValue: '启用' })}</span>
                {actions?.enabledStateLabel && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    {actions.enabledStateLabel}
                  </Text>
                )}
              </div>
              <Switch checked={!isDisabled} onChange={onToggleDisabled} size="small" />
            </div>
          ),
        }]
      : []),
    actions?.onEdit && {
      key: 'edit',
      label: t('common.edit', { defaultValue: '编辑' }),
      icon: <EditOutlined />,
    },
    actions?.onCopy && {
      key: 'copy',
      label: t('common.copy', { defaultValue: '复制' }),
      icon: <CopyOutlined />,
    },
    actions?.onShare && {
      key: 'share',
      label: t('common.share'),
      icon: <Share2 size={14} />,
    },
    ...(actions?.onDelete
      ? [
          { type: 'divider' as const },
          {
            key: 'delete',
            label: t('common.delete', { defaultValue: '删除' }),
            icon: <DeleteOutlined />,
            danger: true,
          },
        ]
      : []),
  ].filter(Boolean) as NonNullable<MenuProps['items']>;

  return (
    <CardShell
      sortableId={sortableId}
      draggable={draggable}
      selectable={selectable}
      selected={selected}
      onSelectChange={onSelectChange}
      dimmed={dimmed}
      accent={accent}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', gap: 12 }}>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <ProviderConnectivityStatus item={connectivityStatus} />
            <ProviderNameLink
              name={provider.name}
              baseUrl={provider.baseUrl}
              style={{ fontSize: 14, fontWeight: 600 }}
            />
            {nameTags}
          </div>

          {metaEntries && metaEntries.length > 0 && (
            // 8, not 4: the Claude-style cards always had a 4px stack gap
            // *plus* a 4px offset on this row (`Space direction="vertical"
            // size={4}` around a row that carried its own `marginTop: 4`), in
            // Claude Code and in the still-bespoke Claude Desktop card. The
            // Codex-style cards stack on 4 alone, which is why this cannot be
            // one value shared by both variants — the migration flattened the
            // two and Claude Code silently lost half its title-to-subtitle
            // gap (2026-10-08 report).
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px 16px', flexWrap: 'wrap', marginTop: 8 }}>
              {metaEntries.map((entry, index) => (
                // One entry renders as one flex item, so the gap separates
                // bindings but never splits a label from its value, and
                // `alignItems: center` lines the label up with the code box.
                <span key={index} style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                  {entry.label && (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {entry.label}
                    </Text>
                  )}
                  {entry.kind === 'code' ? (
                    <Text code style={{ fontSize: 12 }}>
                      {entry.value}
                    </Text>
                  ) : (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {entry.value}
                    </Text>
                  )}
                </span>
              ))}
              {inlineActions}
            </div>
          )}

          {footer && <div style={{ marginTop: 8 }}>{footer}</div>}
        </div>

        <Space size={0} style={{ whiteSpace: 'nowrap' }}>
          {actions?.gatewayActions}
          {primary && (primary.locked ? (
            <Tooltip title={primary.tooltip}>
              <span>
                <Button type="link" size="small" icon={primary.icon} disabled>
                  {primary.label}
                </Button>
              </span>
            </Tooltip>
          ) : (
            <Tooltip title={primary.tooltip}>
              <span>
                <Button
                  type="link"
                  size="small"
                  icon={primary.icon}
                  onClick={primary.onClick}
                  disabled={primary.disabled}
                  loading={primary.loading}
                >
                  {primary.label}
                </Button>
              </span>
            </Tooltip>
          ))}
          {actions?.extraActions}
          {menuItems.length > 0 && (
            <Dropdown
              trigger={['click']}
              menu={{
                items: menuItems,
                onClick: ({ key }) => {
                  if (key === 'edit') actions?.onEdit?.();
                  else if (key === 'copy') actions?.onCopy?.();
                  else if (key === 'share') actions?.onShare?.();
                  else if (key === 'delete') actions?.onDelete?.();
                },
              }}
            >
              <Button type="text" size="small" icon={<MoreOutlined />} />
            </Dropdown>
          )}
        </Space>
      </div>
    </CardShell>
  );
};

export default ClaudeStyleCard;
