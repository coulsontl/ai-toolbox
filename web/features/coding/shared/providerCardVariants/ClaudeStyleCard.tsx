import React from 'react';
import { Button, Dropdown, Space, Tag, Typography } from 'antd';
import { MoreOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import CardShell from './CardShell';
import { renderProviderPrimaryAction } from './providerCardActions';
import {
  buildProviderCardMenuItems,
  dispatchProviderCardMenuClick,
} from './providerCardMenu';
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
  namePrefix,
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
            // 8, not 4: the Claude-style cards always had a 4px stack gap
            // *plus* a 4px offset on this row (`Space direction="vertical"
            // size={4}` around a row that carried its own `marginTop: 4`), in
            // Claude Code and in the still-bespoke Claude Desktop card. The
            // Codex-style cards stack on 4 alone, which is why this cannot be
            // one value shared by both variants — the migration flattened the
            // two and Claude Code silently lost half its title-to-subtitle
            // gap (2026-10-08 report).
            //
            // One line, never two. The entries shrink (each value ellipsises)
            // and the trailing `inlineActions` keeps its size, so a long
            // endpoint or a fifth role compresses the facts *before* the
            // action instead of pushing 模型测试 onto a second row
            // (user-reported 2026-10-21). Same rule as the compact cell's
            // subtitle: the action is what the user came for.
            <div style={{ display: 'flex', alignItems: 'center', gap: 16, flexWrap: 'nowrap', marginTop: 8 }}>
              {metaEntries?.map((entry, index) => (
                // One entry renders as one flex item, so the gap separates
                // bindings but never splits a label from its value, and
                // `alignItems: center` lines the label up with the code box.
                //
                // `title` carries the untruncated value: the row is one line by
                // design, and a compressed endpoint must still be readable
                // somewhere. It is a native tooltip (not antd's ellipsis
                // tooltip) so the flex row keeps its own measurement.
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
                    <Text code ellipsis style={{ fontSize: 12, minWidth: 0 }}>
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
                // would still cost the row one 16px gap at its end.
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

      {/* Below the header row, not inside it — the same rule the Codex style
          follows. The header is two columns (content beside the action links),
          so a block rendered inside its left column stops short of the card's
          right edge by the width of those links. The official-account list is
          the block that exposed this on the Codex style (2026-10-08, ~160px of
          empty space), and Gemini's account footer would reproduce it here. */}
      {footer && <div style={{ marginTop: 8 }}>{footer}</div>}
    </CardShell>
  );
};

export default ClaudeStyleCard;
