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
  title,
  hint,
  applyHint,
  emptyText,
  accounts,
  loginAction,
  leadingAction,
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

  const titleLine = (
    <Space size={6}>
      {collapsible && (isCollapsed ? <RightOutlined /> : <DownOutlined />)}
      {leadingAction}
      {/* The link glyph belongs to the section, not to any one host. Two of the
          three original implementations drew one on the title line (Kimi and
          ZCode) and it was lost when the section was extracted from Codex's,
          which happened to draw its own on the login button instead. */}
      <LinkOutlined style={{ color: 'var(--color-text-secondary)' }} />
      <Text strong style={{ fontSize: 13 }}>
        {title}
      </Text>
      {accounts.length > 0 && (
        <Text type="secondary" style={{ fontSize: 12 }}>
          ({accounts.length})
        </Text>
      )}
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
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
          flexWrap: 'wrap',
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
            {titleLine}
          </Button>
        ) : (
          titleLine
        )}
        {loginAction}
      </div>

      {/* The section's explanation is the title's subtitle, so it sits directly
          under the title — not after the rows, where it read as a stray
          footnote and, in the empty state, ended up below the empty
          illustration rather than beside the title it explains. */}
      {hint && (
        <div style={{ marginTop: 4, fontSize: 12, color: 'var(--color-text-secondary)' }}>
          {hint}
        </div>
      )}

      {!isCollapsed && (
        <div
          style={{
            marginTop: hint ? 8 : embedded ? 0 : 8,
            display: 'flex',
            flexDirection: 'column',
            gap: embedded ? 8 : 0,
            // Embedded rows line up with the collapsible title's text rather
            // than with its arrow.
            paddingLeft: embedded ? 18 : 0,
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