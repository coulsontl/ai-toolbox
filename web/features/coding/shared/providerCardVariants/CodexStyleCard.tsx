import React from 'react';
import { Button, Dropdown, Space, Switch, Tag, Tooltip, Typography } from 'antd';
import type { MenuProps } from 'antd';
import {
  ApiOutlined,
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  MoreOutlined,
} from '@ant-design/icons';
import { Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import ModelListSection from '@/features/coding/shared/ModelListSection';
import CardShell from './CardShell';
import type { ProviderCardVariantProps } from './types';

const { Text } = Typography;

/**
 * The Codex-style provider card.
 *
 * ```
 * ⠿  Name  [default] [proxy]                      [应用] [更多 ▾]
 *    `https://…`  [gpt-5]  API Key: sk-…  | notes   [连通性测试]
 *    > 模型列表 (3)   [批量删除] [模型测试] [获取模型] [+ 添加模型]
 * ```
 *
 * The distinguishing traits, against the other two styles:
 *
 * - The **second line is free-form**: endpoint, model tag, masked key, notes and
 *   an inline connectivity action, each optional, in whatever order the CLI
 *   supplies. The OpenCode style fixes that line to id → SDK → endpoint; the
 *   Claude style uses it for role bindings.
 * - The **primary action is a text link** in the header, with secondary actions
 *   folded into a "more" menu. A text link is right here because applying *is*
 *   this card's main job — unlike the OpenCode style, where the equivalent
 *   lives on the model row.
 * - **Model rows are not the only thing below the header**: official/read-only
 *   rows and CLI-specific blocks go through `footer`, above the model section.
 */
const CodexStyleCard: React.FC<ProviderCardVariantProps> = ({
  provider,
  providerState,
  actions,
  modelSection,
  nameTags,
  footer,
  metaEntries,
  inlineActions,
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

          {(metaEntries?.length || inlineActions) && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', marginTop: 4 }}>
              {metaEntries?.map((entry, index) => (
                <span key={index} style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                  {entry.label && (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {entry.label}
                    </Text>
                  )}
                  {entry.kind === 'tag' ? (
                    <Tag color={entry.color ?? 'blue'} style={{ fontSize: 11, margin: 0 }}>
                      {entry.value}
                    </Tag>
                  ) : entry.kind === 'code' ? (
                    <Text code style={{ fontSize: 11, padding: '0 4px' }}>
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

      {/* Below the header row, not inside it. The header row is two columns —
          content beside the action links — so a block rendered inside its left
          column stops short of the card's right edge by the width of those
          links. Everything that is a *section* of the card (the model list
          here, the official accounts through `footer`) spans the card, so its
          right-aligned actions line up with each other and with the card edge.
          The 2026-10-08 report of "the account list leaves a big empty chunk on
          the right" was exactly this: the account rows' buttons stopped ~160px
          short of the model toolbar's. */}
      {footer && <div style={{ marginTop: 8 }}>{footer}</div>}

      {modelSection && (
        <ModelListSection
          models={modelSection.models}
          rowKeyOf={modelSection.rowKeyOf}
          sectionKey={`provider-models-${provider.id}`}
          className={modelSection.className}
          bodyStyle={modelSection.bodyStyle}
          transparentRows
          modelsDraggable={modelSection.modelsDraggable}
          onReorderModels={modelSection.onReorderModels}
          selectionMode={modelSection.modelSelectionMode}
          selectedIds={modelSection.selectedModelIds}
          onToggleSelection={modelSection.onToggleModelSelection}
          onToggleBatchDeleteMode={modelSection.onToggleBatchDeleteMode}
          onBatchDelete={modelSection.onBatchDeleteModels}
          onTest={modelSection.onTestModels}
          testDisabled={modelSection.testModelsDisabled}
          testDisabledTooltip={modelSection.testModelsDisabledTooltip}
          onFetchModels={modelSection.onFetchModels}
          fetchDisabled={modelSection.fetchDisabled}
          fetchDisabledTooltip={modelSection.fetchDisabledTooltip}
          onAddModel={modelSection.onAddModel}
          onEditModel={modelSection.onEditModel}
          onCopyModel={modelSection.onCopyModel}
          onDeleteModel={modelSection.onDeleteModel}
          onSetPrimaryModel={modelSection.onSetPrimaryModel}
          renderModelExtraActions={modelSection.renderModelExtraActions}
          aboveList={modelSection.aboveList}
        />
      )}
    </CardShell>
  );
};

export default CodexStyleCard;

/** The inline connectivity action both the Codex and Claude styles put on the meta line. */
export const InlineConnectivityButton: React.FC<{
  onClick: () => void;
  disabled?: boolean;
  tooltip?: string;
}> = ({ onClick, disabled, tooltip }) => {
  const { t } = useTranslation();
  return (
    <Tooltip title={disabled ? tooltip : ''}>
      <span>
        <Button
          type="text"
          size="small"
          icon={<ApiOutlined />}
          onClick={onClick}
          disabled={disabled}
          style={{ fontSize: 11, padding: '0 4px', height: 'auto', flexShrink: 0 }}
        >
          {t('opencode.connectivity.button')}
        </Button>
      </span>
    </Tooltip>
  );
};
