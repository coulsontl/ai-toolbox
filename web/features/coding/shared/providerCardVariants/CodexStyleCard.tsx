import React from 'react';
import { Button, Dropdown, Space, Tag, Tooltip, Typography } from 'antd';
import { ApiOutlined, MoreOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import CardShell from './CardShell';
import ProviderModelSection from './ProviderModelSection';
import { renderProviderPrimaryAction } from './providerCardActions';
import {
  buildProviderCardMenuItems,
  dispatchProviderCardMenuClick,
} from './providerCardMenu';
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
  namePrefix,
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
    selectDisabledReason,
    dimmed = false,
    draggable = false,
    sortableId,
    accent,
  } = providerState ?? {};

  const primary = actions?.primaryAction;

  /** The "more" menu — see `buildProviderCardMenuItems` for the fixed order. */
  const menuItems = buildProviderCardMenuItems({
    t,
    actions,
    isDisabled,
    onToggleDisabled,
  });

  return (
    <CardShell
      sortableId={sortableId}
      draggable={draggable}
      selectable={selectable}
      selected={selected}
      onSelectChange={onSelectChange}
      selectDisabledReason={selectDisabledReason}
      dimmed={dimmed}
      accent={accent}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', gap: 12 }}>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <ProviderConnectivityStatus item={connectivityStatus} />
            {namePrefix}
            <ProviderNameLink
              name={provider.name}
              baseUrl={provider.baseUrl}
              style={{ fontSize: 14, fontWeight: 600 }}
            />
            {nameTags}
          </div>

          {(metaEntries?.length || inlineActions) && (
            // One line, never two — same rule as the Claude style: the facts
            // shrink and the trailing action keeps its size, so a long
            // endpoint compresses itself instead of wrapping 模型测试 onto the
            // next row (user-reported 2026-10-21).
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'nowrap', marginTop: 4 }}>
              {metaEntries?.map((entry, index) => (
                // `title` is the untruncated value, for the same reason as the
                // Claude style: the line never wraps, so a compressed endpoint
                // has to stay readable on hover.
                <span
                  key={index}
                  title={entry.title ?? (typeof entry.value === 'string' ? entry.value : undefined)}
                  style={{ display: 'inline-flex', alignItems: 'center', gap: 4, minWidth: 0 }}
                >
                  {entry.label && (
                    <Text type="secondary" style={{ fontSize: 12, flexShrink: 0 }}>
                      {entry.label}
                    </Text>
                  )}
                  {entry.kind === 'tag' ? (
                    <Tag color={entry.color ?? 'blue'} style={{ fontSize: 11, margin: 0, flexShrink: 0 }}>
                      {entry.value}
                    </Tag>
                  ) : entry.kind === 'code' ? (
                    <Text code ellipsis style={{ fontSize: 11, padding: '0 4px', minWidth: 0 }}>
                      {entry.value}
                    </Text>
                  ) : (
                    <Text type="secondary" ellipsis style={{ fontSize: 12, minWidth: 0 }}>
                      {entry.value}
                    </Text>
                  )}
                </span>
              ))}
              {inlineActions && (
                // Only rendered when there is an action: an empty flex item
                // would still cost the row one 8px gap at its end.
                <span style={{ flexShrink: 0, display: 'inline-flex', alignItems: 'center' }}>
                  {inlineActions}
                </span>
              )}
            </div>
          )}
        </div>

        <Space size={0} style={{ whiteSpace: 'nowrap' }}>
          {actions?.gatewayActions}
          {renderProviderPrimaryAction(primary)}
          {menuItems.length > 0 && (
            <Dropdown
              trigger={['click']}
              menu={{
                items: menuItems,
                onClick: ({ key }) => dispatchProviderCardMenuClick(key, actions),
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

      <ProviderModelSection providerId={provider.id} modelSection={modelSection} />
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
