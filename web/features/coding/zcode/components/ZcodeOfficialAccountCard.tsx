import React from 'react';
import { Button, Dropdown, Space } from 'antd';
import { LinkOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { CardShell } from '@/features/coding/shared/providerCardVariants';
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
 * sit inside.
 *
 * It is still a peer of the provider cards in the list, and it looks like one:
 * the frame is the shared `CardShell` (drag registration, handle, chrome), the
 * block inside it is `shared/officialAccounts`, and this file only maps ZCode's
 * own account records onto `OfficialAccountRowView`. Both halves used to be
 * written by hand here, and both drifted — the handle lost its hover feedback and
 * the content lost its left edge, because the handle sat *inside* the heading
 * line instead of in its own column.
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

  /**
   * The account list collapses, like Codex's and Kimi's.
   *
   * It starts **open** here: this card's accounts have always been on screen,
   * and the heading above them is what a reader lands on — Codex starts closed
   * only because its card already carries a model section. The ability, not the
   * starting state, is what the three cards share.
   */
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(false);

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
    <CardShell
      sortableId={sortableId}
      draggable={Boolean(sortableId) && !dragDisabled}
    >
      <OfficialAccountsSection
        variant="standalone"
        headingTitle={t('common.officialAccount.headingTitle')}
        listTitle={t('common.officialAccount.listTitle')}
        hint={t('zcode.officialAccount.hint')}
        collapsed={accountsCollapsed}
        onToggleCollapsed={() => setAccountsCollapsed((current) => !current)}
        applyHint={t('zcode.officialAccount.applyHint')}
        emptyText={t('zcode.officialAccount.empty')}
        accounts={accountRows}
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
    </CardShell>
  );
};

export default ZcodeOfficialAccountCard;