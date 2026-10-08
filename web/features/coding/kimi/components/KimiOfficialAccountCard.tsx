import React from 'react';
import { Button, Card, Empty, Space, Tooltip, Typography } from 'antd';
import {
  DeleteOutlined,
  HolderOutlined,
  LinkOutlined,
  LoginOutlined,
  SwapOutlined,
} from '@ant-design/icons';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useTranslation } from 'react-i18next';
import AppliedTag from '@/components/common/AppliedTag';
import type { KimiOfficialAccount } from '@/types/kimi';

const { Text, Link } = Typography;

/** Matches the borderless actions on the model list toolbar. */
const actionButtonStyle: React.CSSProperties = { fontSize: 12 };

/**
 * Sortable id of the official-account card.
 *
 * The card is not a provider row, so it has no id of its own to sort by; this
 * sentinel stands in for it in the one ordering that mixes the two. The backend
 * stores the position as the number of provider cards above it.
 */
export const KIMI_OFFICIAL_ACCOUNT_CARD_ID = 'kimi-official-account';

export interface KimiOfficialAccountCardProps {
  accounts: KimiOfficialAccount[];
  /** A device-auth login is waiting; its entry is withheld until it resolves. */
  loginPending?: boolean;
  /** A saved account's switch action is in flight. */
  applyingAccountId?: string | null;
  /**
   * Sortable id, so the card can be dragged among the provider cards. Omit it
   * when dragging is off (search, non-custom sort, batch selection) — the
   * card then renders without a handle, exactly like its provider siblings.
   */
  sortableId?: string;
  onLogin: () => void;
  onApply: (account: KimiOfficialAccount) => void;
  onDelete: (account: KimiOfficialAccount) => void;
  /** Opens the Kimi console — the same「查看额度」link the old block carried. */
  onViewUsage: () => void;
}

/**
 * The official-account section of the Kimi provider list.
 *
 * A peer of the provider cards rather than a block above or below them: a Kimi
 * official login is not a provider (it lives in the credentials store and is
 * switched independently), but it is the same *kind* of thing to the user —
 * something in this list they pick between. So it carries the same drag handle
 * and shares the one ordering with them.
 *
 * It also owns the「登录」entry. That entry used to sit in the list toolbar,
 * which is where a list-level action goes — but it acts on *this* section, and
 * a list-level entry for a section that is one of the list's own members is
 * both harder to find and impossible to explain. The button that adds to a
 * section belongs on the section.
 */
const KimiOfficialAccountCard: React.FC<KimiOfficialAccountCardProps> = ({
  accounts,
  loginPending = false,
  applyingAccountId = null,
  sortableId,
  onLogin,
  onApply,
  onDelete,
  onViewUsage,
}) => {
  const { t } = useTranslation();

  // `useSortable` must run on every render, so a card rendered without an id
  // registers under the sentinel id and is disabled instead.
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: sortableId ?? KIMI_OFFICIAL_ACCOUNT_CARD_ID,
    disabled: !sortableId,
  });

  const accountLabel = (account: KimiOfficialAccount) =>
    account.email || account.name || account.id;

  return (
    <div
      // Attached unconditionally: dnd-kit measures the node to compute
      // transforms, so a card that skips registration cannot move once
      // dragging comes back on.
      ref={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
        opacity: isDragging ? 0.5 : 1,
      }}
    >
      {/* Bottom margin on the Card, like every sibling CLI card. Each card
          spaces itself, so the gap survives being reordered among the
          provider cards. */}
      <Card size="small" style={{ marginBottom: 12 }} styles={{ body: { padding: 12 } }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
            flexWrap: 'wrap',
          }}
        >
          <Space size="small" wrap>
            {sortableId && (
              <span
                {...attributes}
                {...listeners}
                style={{
                  cursor: isDragging ? 'grabbing' : 'grab',
                  color: '#999',
                  touchAction: 'none',
                }}
              >
                <HolderOutlined />
              </span>
            )}
            <LinkOutlined style={{ color: 'var(--color-text-secondary)' }} />
            <Text strong>{t('kimi.officialAccounts')}</Text>
            {accounts.length > 0 && (
              <Text type="secondary" style={{ fontSize: 12 }}>
                ({accounts.length})
              </Text>
            )}
            <Link
              type="secondary"
              style={{ fontSize: 12 }}
              onClick={(event) => {
                event.stopPropagation();
                onViewUsage();
              }}
            >
              {t('kimi.viewUsage')}
            </Link>
          </Space>

          <Button
            type="text"
            size="small"
            icon={<LoginOutlined />}
            style={actionButtonStyle}
            loading={loginPending}
            disabled={loginPending}
            onClick={onLogin}
          >
            {t('kimi.officialAccount.login')}
          </Button>
        </div>

        {/* The section's own explanation is its subtitle, so it sits directly
            under the title — not after the rows. At the bottom it read as a
            stray footnote, and in the empty state it ended up below a large
            empty illustration, further from the title it explains. */}
        <div style={{ marginTop: 4, fontSize: 12, color: 'var(--color-text-secondary)' }}>
          {t('kimi.officialAccount.hint')}
        </div>

        <div style={{ marginTop: 8 }}>
          {accounts.length === 0 ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('kimi.officialAccount.empty')}
                </Text>
              }
              style={{ margin: '8px 0' }}
            />
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
                <Space size={8} wrap style={{ minWidth: 0 }}>
                  <Text strong={account.isApplied} style={{ fontSize: 13 }}>
                    {accountLabel(account)}
                  </Text>
                  {account.isApplied && (
                    <AppliedTag style={{ fontSize: 10 }}>
                      {t('kimi.account.applied')}
                    </AppliedTag>
                  )}
                </Space>

                <Space size={0}>
                  <Tooltip title={t('kimi.officialAccount.applyHint')}>
                    {/* The Tooltip needs a live child so a disabled button still
                        has something to attach to. */}
                    <span>
                      <Button
                        type="text"
                        size="small"
                        icon={<SwapOutlined />}
                        style={actionButtonStyle}
                        disabled={account.isApplied || loginPending}
                        loading={applyingAccountId === account.id}
                        onClick={() => onApply(account)}
                      >
                        {t('kimi.account.apply')}
                      </Button>
                    </span>
                  </Tooltip>
                  <Button
                    type="text"
                    size="small"
                    danger
                    icon={<DeleteOutlined />}
                    style={actionButtonStyle}
                    disabled={account.isApplied || loginPending}
                    onClick={() => onDelete(account)}
                  >
                    {t('common.delete')}
                  </Button>
                </Space>
              </div>
            ))
          )}
        </div>
      </Card>
    </div>
  );
};

export default KimiOfficialAccountCard;