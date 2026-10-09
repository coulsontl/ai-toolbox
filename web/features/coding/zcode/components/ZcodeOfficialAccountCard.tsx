import React from 'react';
import { Button, Card, Dropdown, Space } from 'antd';
import { HolderOutlined, LinkOutlined } from '@ant-design/icons';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useTranslation } from 'react-i18next';
import { OfficialAccountsSection } from '@/features/coding/shared/officialAccounts';
import type { OfficialAccountRowView } from '@/features/coding/shared/officialAccounts';
import type { ZcodeOfficialAccount } from '@/types/zcode';

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
 * The official-account card of the ZCode provider list.
 *
 * ZCode needs its own card because an official login is not a provider: ZCode
 * keeps it in `credentials.json`, outside the provider registry, and the two are
 * switched independently — so there is no provider row for the shared section to
 * sit inside. It is still a peer of the provider cards in the list, though, so
 * it carries the same drag handle and can be moved down among them. That is the
 * whole of what this file adds: the card shell and the drag registration. The
 * section itself is `shared/officialAccounts`, the same one Codex renders inside
 * its official provider card.
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

  const labelForProvider = (providerId: string) =>
    loginProviders.find((provider) => provider.value === providerId)?.label ?? providerId;

  const accountRows = React.useMemo<OfficialAccountRowView[]>(
    () =>
      accounts.map((account) => ({
        id: account.id,
        label: account.email || account.name || t('zcode.officialAccount.unnamed'),
        // The virtual entry is the live login rather than one of the CLI's own
        // providers, so it says "current" instead of naming a provider.
        kindTag: account.isVirtual
          ? t('zcode.officialAccount.currentTag')
          : labelForProvider(account.providerId),
        isApplied: account.isApplied,
        isVirtual: account.isVirtual,
      })),
    // `labelForProvider` reads `loginProviders`, so that is a dependency too.
    [accounts, loginProviders, t],
  );

  // Only the live-but-unsaved row can be saved, and there is at most one.
  const pendingSaveAccountId = accounts.find((account) => account.isVirtual)?.id ?? null;
  const pending = savingCurrent && pendingSaveAccountId
    ? { accountId: pendingSaveAccountId, action: 'save' as const }
    : applyingAccountId
      ? { accountId: applyingAccountId, action: 'apply' as const }
      : null;

  const accountById = (id: string) => accounts.find((account) => account.id === id);

  const loginAction = (
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
          style={{ fontSize: 12 }}
          loading={loginPending}
        >
          {t('zcode.officialAccount.login')}
        </Button>
      </Dropdown>
    </Space>
  );

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
        <OfficialAccountsSection
          variant="standalone"
          title={t('zcode.officialAccount.title')}
          hint={t('zcode.officialAccount.hint')}
          applyHint={t('zcode.officialAccount.applyHint')}
          emptyText={t('zcode.officialAccount.empty')}
          accounts={accountRows}
          leadingAction={
            <>
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
            </>
          }
          loginAction={loginAction}
          actionsDisabled={loginPending}
          pending={pending}
          onSaveLocal={() => onSaveCurrent()}
          onApply={(row) => {
            const account = accountById(row.id);
            if (account) {
              onApply(account);
            }
          }}
          onDelete={(row) => {
            const account = accountById(row.id);
            if (account) {
              onDelete(account);
            }
          }}
        />
      </Card>
    </div>
  );
};

export default ZcodeOfficialAccountCard;