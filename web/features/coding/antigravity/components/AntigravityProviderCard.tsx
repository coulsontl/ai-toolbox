import React from 'react';
import { Button } from 'antd';
import { LinkOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  OfficialAccountsSection,
  type OfficialAccountPendingAction,
  type OfficialAccountRowView,
} from '@/features/coding/shared/officialAccounts';
import type { AntigravityOfficialAccount, AntigravityProvider } from '@/types/antigravity';

interface AntigravityProviderCardProps {
  provider: AntigravityProvider;
  officialAccounts?: AntigravityOfficialAccount[];
  onOfficialAccountLogin?: (provider: AntigravityProvider) => void;
  onOfficialAccountSaveLocal?: (provider: AntigravityProvider) => void;
  onOfficialAccountApply?: (provider: AntigravityProvider, account: AntigravityOfficialAccount) => void;
  onOfficialAccountDelete?: (provider: AntigravityProvider, account: AntigravityOfficialAccount) => void;
  onOfficialAccountRefresh?: (provider: AntigravityProvider, account: AntigravityOfficialAccount) => void;
  onOfficialAccountViewDetails?: (provider: AntigravityProvider, account: AntigravityOfficialAccount) => void;
  pendingAction?: OfficialAccountPendingAction | null;
  oauthPending?: boolean;
}

const AntigravityProviderCard: React.FC<AntigravityProviderCardProps> = ({
  provider,
  officialAccounts = [],
  onOfficialAccountLogin,
  onOfficialAccountSaveLocal,
  onOfficialAccountApply,
  onOfficialAccountDelete,
  onOfficialAccountRefresh,
  onOfficialAccountViewDetails,
  pendingAction = null,
  oauthPending = false,
}) => {
  const { t } = useTranslation();
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(false);

  if (provider.category !== 'official') {
    return null;
  }

  const accountById = (id: string) => officialAccounts.find((account) => account.id === id);

  const getPlanTag = (planType?: string): OfficialAccountRowView['planTag'] => {
    const trimmed = planType?.trim();
    if (!trimmed) return undefined;
    const upper = trimmed.toUpperCase();
    if (upper.includes('ULTRA')) {
      return { label: trimmed, color: 'purple' };
    }
    if (upper.includes('PRO')) {
      return { label: trimmed, color: 'blue' };
    }
    return { label: trimmed };
  };

  const accountRows: OfficialAccountRowView[] = officialAccounts.map((account) => {
    const metaLines: string[] = [];
    if (account.limit5hText) {
      metaLines.push(`5h: ${account.limit5hText}`);
    }
    if (account.limitWeeklyText) {
      metaLines.push(`Weekly: ${account.limitWeeklyText}`);
    } else if (!account.limit5hText && account.limitShortLabel) {
      metaLines.push(account.limitShortLabel);
    }

    return {
      id: account.id,
      label:
        account.email
        || (account.isVirtual
          ? t('antigravity.provider.officialAccountLocal')
          : account.name),
      kindTag: account.isVirtual
        ? t('antigravity.provider.officialAccountLocalTag')
        : undefined,
      planTag: getPlanTag(account.planType),
      metaLines,
      isApplied: account.isApplied,
      isVirtual: account.isVirtual,
      lastError: account.lastError || undefined,
    };
  });

  const loginAction = onOfficialAccountLogin ? (
    <Button
      type="text"
      size="small"
      icon={<LinkOutlined />}
      style={{ fontSize: 12 }}
      loading={oauthPending}
      disabled={oauthPending || provider.isDisabled}
      onClick={() => onOfficialAccountLogin(provider)}
    >
      {t('antigravity.officialAccounts.loginOauth')}
    </Button>
  ) : undefined;

  return (
    <OfficialAccountsSection
      variant="standalone"
      listTitle={t('common.officialAccount.listTitle')}
      hint={t('antigravity.officialAccounts.hint')}
      applyHint={t('antigravity.officialAccounts.applyHint')}
      emptyText={t('antigravity.officialAccounts.empty')}
      accounts={accountRows}
      loginAction={loginAction}
      collapsed={accountsCollapsed}
      onToggleCollapsed={() => setAccountsCollapsed((current) => !current)}
      pending={pendingAction}
      actionsDisabled={oauthPending || provider.isDisabled}
      onSaveLocal={
        onOfficialAccountSaveLocal
          ? () => onOfficialAccountSaveLocal(provider)
          : undefined
      }
      onApply={
        onOfficialAccountApply
          ? (row) => {
              const account = accountById(row.id);
              if (account) {
                onOfficialAccountApply(provider, account);
              }
            }
          : undefined
      }
      onRefresh={
        onOfficialAccountRefresh
          ? (row) => {
              const account = accountById(row.id);
              if (account) {
                onOfficialAccountRefresh(provider, account);
              }
            }
          : undefined
      }
      onViewDetails={
        onOfficialAccountViewDetails
          ? (row) => {
              const account = accountById(row.id);
              if (account) {
                onOfficialAccountViewDetails(provider, account);
              }
            }
          : undefined
      }
      onDelete={
        onOfficialAccountDelete
          ? (row) => {
              const account = accountById(row.id);
              if (account) {
                onOfficialAccountDelete(provider, account);
              }
            }
          : undefined
      }
    />
  );
};

export default AntigravityProviderCard;
