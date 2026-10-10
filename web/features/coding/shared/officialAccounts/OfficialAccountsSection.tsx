import React from 'react';
import { Button, Empty, Space, Tag, Tooltip, Typography } from 'antd';
import {
  CheckOutlined,
  DeleteOutlined,
  DownOutlined,
  EyeOutlined,
  LinkOutlined,
  RightOutlined,
  SwapOutlined,
  SyncOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import AppliedTag from '@/components/common/AppliedTag';
import type {
  OfficialAccountAction,
  OfficialAccountsSectionProps,
  OfficialAccountRowView,
} from './types';

const { Text } = Typography;

/**
 * Borderless, small actions — the same shape the model-list toolbar uses, so
 * the two toolbars in one card do not drift.
 */
const actionButtonStyle: React.CSSProperties = { fontSize: 12, height: 'auto', paddingInline: 4 };

/**
 * The account block's explanation sentence, rendered the one way.
 *
 * It is drawn in two places — under the heading on a `standalone` card, and as
 * an `embedded` card's second line — so it is a component rather than a string
 * with a style copied next to it. The first version drew it with the app's
 * `--color-text-secondary` variable on one host and through antd's Typography
 * on the others; those two names look interchangeable and are not (the variable
 * is alpha 0.65, antd's `secondary` is `colorTextDescription` = alpha 0.45), so
 * the standalone card's sentence came out a shade darker (reported 2026-10-10).
 *
 * `style` is for placement only (block display, margins); the typography is not
 * overridable — that is the point of exporting it.
 */
export const OfficialAccountHint: React.FC<{
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({ children, style }) => (
  <Text type="secondary" style={{ fontSize: 12, ...style }}>
    {children}
  </Text>
);

/**
 * The 🔗 an official-account heading carries.
 *
 * Exported because an `embedded` host draws it on **its own** heading line (the
 * provider card's name row) — the heading of a block is the block's mark, no
 * matter which component owns the line it sits on. Two drawings of the same
 * glyph is exactly how it went missing once already.
 */
export const OfficialAccountHeadingIcon: React.FC = () => (
  <LinkOutlined style={{ color: 'var(--color-text-secondary)' }} />
);

/**
 * The `(n)` beside an official-account heading.
 *
 * A count belongs to the heading, so an embedded host renders it on its name
 * row while the list repeats it on its own title line; both go through here so
 * the two can never drift apart in size or colour.
 */
export const OfficialAccountCount: React.FC<{ count: number }> = ({ count }) =>
  count > 0 ? (
    <Text type="secondary" style={{ fontSize: 12 }}>
      ({count})
    </Text>
  ) : null;

/**
 * The official-account section, shared by every CLI that has an official login.
 *
 * It used to exist three times — inside Codex's provider card, inside Kimi's and
 * ZCode's standalone cards — and the three had grown apart in ways nobody chose:
 * different action labels for the same action, an explanation rendered as a
 * footnote in one and a subtitle in another, a switch button that was hidden in
 * one and shown-disabled in another. The differences that survived are the ones
 * that are real: whether the host card needs a separator, and whether the empty
 * state may spend vertical space on an illustration (`variant`).
 *
 * What the section deliberately does *not* know: any CLI's account type. Callers
 * map their own records to `OfficialAccountRowView` (`types.ts`).
 */
const OfficialAccountsSection: React.FC<OfficialAccountsSectionProps> = ({
  variant,
  headingTitle,
  listTitle,
  hint,
  applyHint,
  emptyText,
  accounts,
  loginAction,
  collapsed,
  onToggleCollapsed,
  pending = null,
  actionsDisabled = false,
  onApply,
  onSaveLocal,
  onRefresh,
  onViewDetails,
  onDelete,
}) => {
  const { t } = useTranslation();
  const embedded = variant === 'embedded';
  // Only opt into collapsing when the host asks for it; a section rendered
  // without the props must never hide its rows.
  const collapsible = Boolean(onToggleCollapsed);
  const isCollapsed = collapsible && Boolean(collapsed);

  const isPending = (account: OfficialAccountRowView, action: OfficialAccountAction) =>
    pending?.accountId === account.id && pending.action === action;

  /**
   * The block's heading — `🔗 官方账号 (n)`.
   *
   * Drawn here only for a `standalone` host, whose card *is* this section, so
   * this is the card's own heading. An `embedded` host owns that line (the
   * provider card's name row) and builds it from the same exports, so the three
   * headings cannot drift apart.
   */
  const headingLine = (
    <Space size={6}>
      <OfficialAccountHeadingIcon />
      <Text strong style={{ fontSize: 13 }}>
        {headingTitle}
      </Text>
      <OfficialAccountCount count={accounts.length} />
    </Space>
  );

  /**
   * The list's own title — `▾ 账号列表 (n)` — which every host carries.
   *
   * The caret lives *here*, not on the heading: what opens and closes is the
   * list, and that reads the same whether the heading above it belongs to this
   * section (ZCode) or to a provider card (Codex, Kimi).
   */
  const listLine = (
    <Space size={6}>
      {collapsible && (isCollapsed ? <RightOutlined /> : <DownOutlined />)}
      <Text strong style={{ fontSize: 13 }}>
        {listTitle}
      </Text>
      <OfficialAccountCount count={accounts.length} />
    </Space>
  );

  return (
    <div
      style={
        embedded
          ? { marginTop: 12, paddingTop: 12, borderTop: '1px solid var(--color-border)' }
          : undefined
      }
    >
      {!embedded && (
        <>
          {headingLine}

          {/* The explanation is the heading's subtitle, so it sits directly
              under the heading — not after the rows, where it read as a stray
              footnote and, in the empty state, ended up below the empty
              illustration rather than beside the line it explains. An embedded
              host renders this same sentence on its own second line instead
              (`hint` is not passed there) — both through `OfficialAccountHint`,
              so the two cannot drift in colour or size. */}
          {hint && (
            <OfficialAccountHint style={{ display: 'block', marginTop: 4 }}>
              {hint}
            </OfficialAccountHint>
          )}
        </>
      )}

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
          flexWrap: 'wrap',
          marginTop: !embedded && hint ? 8 : 0,
          marginBottom: embedded && !isCollapsed ? 10 : 0,
        }}
      >
        {collapsible ? (
          <Button
            type="text"
            size="small"
            onClick={onToggleCollapsed}
            style={{ padding: 0, height: 'auto' }}
          >
            {listLine}
          </Button>
        ) : (
          listLine
        )}
        {/* The sign-in entry belongs to the *list*: it adds a row to it, and it
            sits where the rows it changes are — not up on the card's heading,
            which names the card rather than the list under it. */}
        {loginAction}
      </div>

      {!isCollapsed && (
        <div
          style={{
            marginTop: hint ? 8 : embedded ? 0 : 8,
            display: 'flex',
            flexDirection: 'column',
            gap: embedded ? 8 : 0,
            // Every host indents the rows past the title's arrow, so they line
            // up with the text of `账号列表` rather than with its caret. Only
            // `embedded` used to: the standalone card had no caret to clear, and
            // its rows sat flush with the card edge while everything above them
            // was indented — the misalignment the user circled.
            paddingLeft: 18,
          }}
        >
          {accounts.length === 0 ? (
            embedded ? (
              <Text type="secondary" style={{ fontSize: 12 }}>
                {emptyText}
              </Text>
            ) : (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {emptyText}
                  </Text>
                }
                style={{ margin: '8px 0' }}
              />
            )
          ) : (
            accounts.map((account) => (
              <div
                key={account.id}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 12,
                  padding: '6px 0',
                  borderBottom: '1px solid var(--color-border)',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    flexWrap: 'wrap',
                    gap: 8,
                    minWidth: 0,
                    flex: 1,
                  }}
                >
                  <Text
                    strong={account.isApplied}
                    style={{ fontSize: 12 }}
                    ellipsis={{ tooltip: account.label }}
                  >
                    {account.label}
                  </Text>
                  {account.kindTag && <Tag style={{ margin: 0, fontSize: 10 }}>{account.kindTag}</Tag>}
                  {account.lastError ? (
                    <Text type="danger" style={{ fontSize: 11 }}>
                      {account.lastError}
                    </Text>
                  ) : (
                    (account.metaLines ?? []).map((line) => (
                      <Text key={line} type="secondary" style={{ fontSize: 11 }}>
                        {line}
                      </Text>
                    ))
                  )}
                  {account.isApplied && (
                    <AppliedTag style={{ fontSize: 10 }}>
                      {t('common.officialAccount.default')}
                    </AppliedTag>
                  )}
                </div>

                <Space size={4} wrap>
                  {onRefresh && (
                    <Button
                      type="text"
                      size="small"
                      icon={<SyncOutlined />}
                      style={actionButtonStyle}
                      disabled={actionsDisabled}
                      loading={isPending(account, 'refresh')}
                      onClick={() => onRefresh(account)}
                    >
                      {t('common.officialAccount.refresh')}
                    </Button>
                  )}
                  {onViewDetails && (
                    <Button
                      type="text"
                      size="small"
                      icon={<EyeOutlined />}
                      style={actionButtonStyle}
                      disabled={actionsDisabled}
                      onClick={() => onViewDetails(account)}
                    >
                      {t('common.officialAccount.details')}
                    </Button>
                  )}
                  {/* A live-but-unsaved login can only be saved; a stored one can
                      only be switched to, and only while it is not the one the
                      CLI is already using — a disabled "switch" next to a
                      "default" tag says the same thing twice. */}
                  {account.isVirtual && onSaveLocal && (
                    <Button
                      type="text"
                      size="small"
                      icon={<CheckOutlined />}
                      style={actionButtonStyle}
                      disabled={actionsDisabled}
                      loading={isPending(account, 'save')}
                      onClick={() => onSaveLocal(account)}
                    >
                      {t('common.officialAccount.saveCurrent')}
                    </Button>
                  )}
                  {!account.isVirtual && !account.isApplied && onApply && (
                    <Tooltip title={applyHint}>
                      {/* The Tooltip needs a live child so a disabled button
                          still has something to attach to. */}
                      <span>
                        <Button
                          type="text"
                          size="small"
                          icon={<SwapOutlined />}
                          style={actionButtonStyle}
                          disabled={actionsDisabled}
                          loading={isPending(account, 'apply')}
                          onClick={() => onApply(account)}
                        >
                          {t('common.officialAccount.switch')}
                        </Button>
                      </span>
                    </Tooltip>
                  )}
                  {!account.isVirtual && onDelete && (
                    <Button
                      type="text"
                      danger
                      size="small"
                      icon={<DeleteOutlined />}
                      style={actionButtonStyle}
                      disabled={actionsDisabled}
                      loading={isPending(account, 'delete')}
                      onClick={() => onDelete(account)}
                    >
                      {t('common.delete')}
                    </Button>
                  )}
                </Space>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
};

export default OfficialAccountsSection;