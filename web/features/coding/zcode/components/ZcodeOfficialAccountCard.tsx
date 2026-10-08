import React from 'react';
import { Button, Card, Dropdown, Empty, Space, Tag, Tooltip, Typography } from 'antd';
import {
  DeleteOutlined,
  HolderOutlined,
  LinkOutlined,
  SaveOutlined,
  SwapOutlined,
} from '@ant-design/icons';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useTranslation } from 'react-i18next';
import AppliedTag from '@/components/common/AppliedTag';
import type { ZcodeOfficialAccount } from '@/types/zcode';

const { Text } = Typography;

/** Matches the borderless actions on the model list toolbar. */
const actionButtonStyle: React.CSSProperties = { fontSize: 12 };

export interface ZcodeOfficialAccountCardProps {
  accounts: ZcodeOfficialAccount[];
  /** A saved account's switch action is in flight. */
  applyingAccountId?: string | null;
  /** The "save current login" action is in flight. */
  savingCurrent?: boolean;
  /** A browser login is waiting; every action is withheld until it resolves. */
  loginPending?: boolean;
  /** The login providers, in the order the menu lists them. */
  loginProviders: readonly { value: string; label: string }[];
  /**
   * Sortable id, so the card can be dragged among the provider cards. Omit to
   * render it pinned with no drag handle.
   */
  sortableId?: string;
  /** Renders the handle inert while batch selection owns the pointer. */
  dragDisabled?: boolean;
  onLogin: (providerId: string) => void;
  onApply: (account: ZcodeOfficialAccount) => void;
  onDelete: (account: ZcodeOfficialAccount) => void;
  onSaveCurrent: () => void;
}

/**
 * The official-account section of the ZCode provider list.
 *
 * Separate from the provider cards because an official login is not a provider:
 * ZCode keeps it in `credentials.json`, outside the provider registry, and the
 * two are switched independently. It is still a peer of those cards in the
 * list, though, so it carries the same drag handle and can be moved down among
 * them.
 *
 * The entry whose `isVirtual` is set mirrors whatever is logged in right now
 * but was never captured — showing it is what keeps the list honest about what
 * is actually applied.
 */
const ZcodeOfficialAccountCard: React.FC<ZcodeOfficialAccountCardProps> = ({
  accounts,
  applyingAccountId = null,
  savingCurrent = false,
  loginPending = false,
  loginProviders,
  sortableId,
  dragDisabled = false,
  onLogin,
  onApply,
  onDelete,
  onSaveCurrent,
}) => {
  const { t } = useTranslation();

  // `useSortable` must run on every render, so a card rendered without an id
  // registers under a placeholder and is disabled instead.
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: sortableId ?? 'zcode-official-account',
    disabled: !sortableId || dragDisabled,
  });

  const accountLabel = (account: ZcodeOfficialAccount) =>
    account.email || account.name || t('zcode.officialAccount.unnamed');

  const labelForProvider = (providerId: string) =>
    loginProviders.find((provider) => provider.value === providerId)?.label ?? providerId;

  return (
    <div
      ref={sortableId ? setNodeRef : undefined}
      style={
        sortableId
          ? {
              transform: CSS.Transform.toString(transform),
              transition,
              opacity: isDragging ? 0.5 : 1,
            }
          : undefined
      }
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
                  cursor: dragDisabled ? 'default' : isDragging ? 'grabbing' : 'grab',
                  color: '#999',
                  touchAction: 'none',
                }}
              >
                <HolderOutlined />
              </span>
            )}
            <LinkOutlined style={{ color: 'var(--color-text-secondary)' }} />
            <Text strong>{t('zcode.officialAccount.title')}</Text>
            {accounts.length > 0 && (
              <Text type="secondary" style={{ fontSize: 12 }}>
                ({accounts.length})
              </Text>
            )}
          </Space>

          <Space size={0}>
            <Dropdown
              menu={{
                items: loginProviders.map((provider) => ({
                  key: provider.value,
                  label: provider.label,
                })),
                onClick: ({ key }) => onLogin(key),
              }}
              disabled={loginPending}
            >
              <Button
                type="text"
                size="small"
                icon={<LinkOutlined />}
                style={actionButtonStyle}
                loading={loginPending}
              >
                {t('zcode.officialAccount.login')}
              </Button>
            </Dropdown>
          </Space>
        </div>

        {/* The section's own explanation is its subtitle, so it sits directly
          under the title — not after the rows, where it read as a stray
          footnote and, in the empty state, ended up below the empty
          illustration rather than beside the title it explains. */}
        <div style={{ marginTop: 4, fontSize: 12, color: 'var(--color-text-secondary)' }}>
          {t('zcode.officialAccount.hint')}
        </div>

        <div style={{ marginTop: 8 }}>
          {accounts.length === 0 ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('zcode.officialAccount.empty')}
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
                  <Tag style={{ margin: 0, fontSize: 10 }}>
                    {account.isVirtual
                      ? t('zcode.officialAccount.currentTag')
                      : labelForProvider(account.providerId)}
                  </Tag>
                  {account.isApplied && (
                    <AppliedTag style={{ fontSize: 10 }}>
                      {t('zcode.provider.applied', { defaultValue: '默认' })}
                    </AppliedTag>
                  )}
                </Space>

                <Space size={0}>
                  {/* Only the unsaved live login can be saved, so the button
                      lives on that row rather than on the card: the entry it
                      acts on is the entry that carries it. The backend lists a
                      virtual entry while a live login has no snapshot and none
                      once it is stored, so its presence is the whole condition
                      — keeping the button around after saving invites the user
                      to save the same login twice, and it does nothing. */}
                  {account.isVirtual && (
                    <Button
                      type="text"
                      size="small"
                      icon={<SaveOutlined />}
                      style={actionButtonStyle}
                      loading={savingCurrent}
                      disabled={loginPending}
                      onClick={onSaveCurrent}
                    >
                      {t('zcode.officialAccount.saveCurrent')}
                    </Button>
                  )}
                  <Tooltip title={t('zcode.officialAccount.applyHint')}>
                    {/* The Tooltip needs a live child so a disabled button still
                        has something to attach to. */}
                    <span>
                      <Button
                        type="text"
                        size="small"
                        icon={<SwapOutlined />}
                        style={actionButtonStyle}
                        disabled={account.isApplied || account.isVirtual || loginPending}
                        loading={applyingAccountId === account.id}
                        onClick={() => onApply(account)}
                      >
                        {t('zcode.officialAccount.apply')}
                      </Button>
                    </span>
                  </Tooltip>
                  <Button
                    type="text"
                    size="small"
                    danger
                    icon={<DeleteOutlined />}
                    style={actionButtonStyle}
                    disabled={account.isVirtual || loginPending}
                    onClick={() => onDelete(account)}
                  >
                    {t('common.delete', { defaultValue: '删除' })}
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

export default ZcodeOfficialAccountCard;