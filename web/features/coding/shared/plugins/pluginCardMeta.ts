import React from 'react';
import { App } from 'antd';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { syncPluginCliProviderModels } from '@/services';
import {
  extractPluginProviderIdFromUrl,
  getPluginProviderRef,
} from '@/features/coding/shared/gateway/providerProtocol';
import type {
  ProviderCardActions,
  ProviderCardState,
} from '@/features/coding/shared/providerCardVariants/types';

/**
 * What the provider list knows about the CLI it is rendering.
 *
 * Supplied once by `ProviderListSection` rather than threaded through every
 * `*ProviderCard.tsx`: a card already gets its provider record, and the CLI key
 * and list-reload callback are properties of the *list*, not of the row.
 */
export interface PluginProviderCardContextValue {
  cliKey: string;
  /** Re-read the CLI's provider list after a plugin sync / import. */
  onChanged?: () => void | Promise<void>;
}

export const PluginProviderCardContext =
  React.createContext<PluginProviderCardContextValue | null>(null);

export interface PluginProviderCardMeta {
  pluginBadge?: NonNullable<ProviderCardState['pluginBadge']>;
  pluginActions: Pick<
    ProviderCardActions,
    'onSyncPluginModels' | 'onManagePluginAccounts'
  >;
}

const asMetaRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
};

/** The same shape the plugin feature's own error formatter returns. */
const pluginErrorMessage = (error: unknown): string => {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === 'string') {
    return error;
  }
  if (error && typeof error === 'object') {
    try {
      return JSON.stringify(error);
    } catch {
      return String(error);
    }
  }
  return String(error ?? '');
};

/** Deep-link used by both the card menu and the plugin center. */
export const pluginAccountsPath = (providerId: string): string =>
  `/plugins/accounts?provider=${encodeURIComponent(providerId)}`;

/**
 * Derive the plugin badge and the two plugin actions from a provider row.
 *
 * A provider added from a plugin can be recognized two ways: a `meta` reference
 * (the gateway takeover CLIs) or the direct `/plugin/<id>/...` endpoint written
 * into a file-config CLI. Both paths end at the same `providerId`, which is
 * what the sync command and the account manager need.
 *
 * The hook is intentionally forgiving outside a `ProviderListSection`: the
 * badge and "manage accounts" still work, while "sync models" is withheld
 * because there is no CLI key to sync into.
 */
export const usePluginProviderCardMeta = (input: {
  meta?: unknown;
  baseUrl?: string | null;
  /** The CLI-side provider id, which differs from the plugin provider id. */
  providerId: string;
}): PluginProviderCardMeta => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const navigate = useNavigate();
  const context = React.useContext(PluginProviderCardContext);

  const pluginRef = getPluginProviderRef(asMetaRecord(input.meta));
  const pluginProviderId = pluginRef?.providerId
    ?? extractPluginProviderIdFromUrl(input.baseUrl);

  if (!pluginProviderId) {
    return { pluginActions: {} };
  }

  const pluginActions: PluginProviderCardMeta['pluginActions'] = {
    onManagePluginAccounts: () => navigate(pluginAccountsPath(pluginProviderId)),
  };

  if (context?.cliKey) {
    pluginActions.onSyncPluginModels = () => {
      void syncPluginCliProviderModels(
        context.cliKey,
        input.providerId,
        pluginProviderId,
      ).then(async (result) => {
        message.success?.(t('plugins.providerCard.syncSuccess', {
          count: result.modelCount,
          added: result.added,
          updated: result.updated,
        }));
        try {
          await context.onChanged?.();
        } catch {
          // The sync itself succeeded; a failed re-read only leaves the list
          // stale until its next load.
        }
      }).catch((error) => {
        message.error?.(t('plugins.providerCard.syncFailed', {
          error: pluginErrorMessage(error),
        }));
      });
    };
  }

  return {
    pluginBadge: {
      label: t('plugins.providerCard.badge'),
      providerId: pluginProviderId,
      tooltip: t('plugins.providerCard.badgeTooltip', {
        provider: pluginProviderId,
      }),
    },
    pluginActions,
  };
};
