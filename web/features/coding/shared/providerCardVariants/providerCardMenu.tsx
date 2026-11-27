import { Typography } from 'antd';
import type { MenuProps } from 'antd';
import {
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  SyncOutlined,
  TeamOutlined,
} from '@ant-design/icons';
import { Share2 } from 'lucide-react';
import type { TFunction } from 'i18next';
import { ProviderEnabledSwitch } from './providerCardActions';
import type { ProviderCardActions } from './types';

const { Text } = Typography;

interface BuildProviderCardMenuItemsOptions {
  t: TFunction;
  actions?: ProviderCardActions;
  isDisabled?: boolean;
  onToggleDisabled?: (enabled: boolean) => void;
  /**
   * Extra entries for this CLI, inserted after 分享 and **before** the divider
   * that precedes 删除 — so a tool-specific action reads as part of the "acts
   * on this provider" group rather than as part of the destructive one.
   *
   * Used by the compact card, which folds the list card's inline 连通性测试
   * here: the grid cell has room for the two primary actions only.
   */
  extraItems?: NonNullable<MenuProps['items']>;
  /** Prefix the plugin entries with a divider when the menu has earlier items. */
  leadingDivider?: boolean;
}

/**
 * The plugin actions appended to the "more" menu (and the only menu the
 * OpenCode style opens).
 *
 * They sit after a divider at the very end, after 删除: the menu reads as
 * "what this card can do" first, then "where this provider came from". The
 * separator is suppressed when there are no plugin actions, so an ordinary
 * provider's menu keeps its exact old shape.
 */
export function buildPluginProviderMenuItems({
  t,
  actions,
  leadingDivider = false,
}: BuildProviderCardMenuItemsOptions): NonNullable<MenuProps['items']> {
  const items: NonNullable<MenuProps['items']> = [
    ...(actions?.onSyncPluginModels
      ? [{
          key: 'syncPluginModels',
          label: t('plugins.providerCard.syncModels'),
          icon: <SyncOutlined />,
        }]
      : []),
    ...(actions?.onManagePluginAccounts
      ? [{
          key: 'managePluginAccounts',
          label: t('plugins.providerCard.manageAccounts'),
          icon: <TeamOutlined />,
        }]
      : []),
  ];
  if (items.length === 0) {
    return [];
  }
  return leadingDivider ? [{ type: 'divider' as const }, ...items] : items;
}

/**
 * The provider card's "more" menu: **a contract, not decoration**.
 *
 * The order and furniture are fixed — enable (with its state sub-label) → edit
 * → copy → share → extra → ─── → delete → ─── → plugin actions, every entry
 * carrying its icon, delete behind a divider. These were in the bespoke cards
 * before the styles were extracted; a migration that dropped the icons or
 * flattened the divider produces a menu that looks like a different product
 * while still working, and nothing fails (see `providerCardVariants/AGENTS.md`,
 * "「更多」菜单是契约").
 *
 * Shared by the two Claude/Codex-style list cards, the compact card view and
 * the OpenCode style (which opens the plugin-only builder directly). Built on
 * antd `Dropdown` rather than the Skills page's `ManagementMenu` because the
 * enable row contains a real `<Switch>`; `ManagementMenu` renders its entries
 * as `<button>`, and nesting an interactive control inside one is invalid
 * markup.
 */
export function buildProviderCardMenuItems({
  t,
  actions,
  isDisabled,
  onToggleDisabled,
  extraItems,
}: BuildProviderCardMenuItemsOptions): NonNullable<MenuProps['items']> {
  const baseItems = [
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
              {/* No tooltip: the item names the control on its own line. */}
              <ProviderEnabledSwitch
                isDisabled={isDisabled}
                onToggleDisabled={onToggleDisabled}
                tooltip={false}
              />
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
    ...(extraItems ?? []),
    ...(actions?.onDelete
      ? [
          { type: 'divider' as const },
          {
            key: 'delete',
            label: t('common.delete', { defaultValue: '删除' }),
            icon: <DeleteOutlined />,
            danger: true,
            // The list cards grey out their delete button when the record is
            // protected (a default provider, a bridge record), and the compact
            // card's detail sidebar does the same. The menu is the third entry
            // point and used to be the way around both: a live 删除 here deleted
            // what the other two refused to. `title` is antd's tooltip, which
            // is where the reason is already shown on the other two surfaces.
            disabled: Boolean(actions.deleteDisabledReason),
            title: actions.deleteDisabledReason,
          },
        ]
      : []),
  ].filter(Boolean) as NonNullable<MenuProps['items']>;

  return [
    ...baseItems,
    ...buildPluginProviderMenuItems({
      t,
      actions,
      leadingDivider: baseItems.length > 0,
    }),
  ];
}

/**
 * Dispatch a menu click to the matching action. Kept beside the builder so the
 * keys and the handlers cannot drift apart in one card but not another.
 */
export function dispatchProviderCardMenuClick(
  key: string,
  actions?: ProviderCardActions,
): void {
  if (key === 'edit') actions?.onEdit?.();
  else if (key === 'copy') actions?.onCopy?.();
  else if (key === 'share') actions?.onShare?.();
  else if (key === 'delete') actions?.onDelete?.();
  else if (key === 'syncPluginModels') actions?.onSyncPluginModels?.();
  else if (key === 'managePluginAccounts') actions?.onManagePluginAccounts?.();
}
