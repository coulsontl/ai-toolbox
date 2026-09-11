import React from 'react';
import { App } from 'antd';
import { DEEP_LINK_IMPORT_COMPLETED } from '@/constants/configEvents';
import { getProviderShareDefaults } from '@/services/deeplinkApi';
import type { ProviderShareApp, ShareableProvider } from '@/features/shared/deepLink/providerTransfer';
import ShareProviderModal from './ShareProviderModal';

export function useProviderSharing(sourceApp: ProviderShareApp, onImported: () => unknown) {
  const { message } = App.useApp();
  const [provider, setProvider] = React.useState<ShareableProvider | null>(null);
  const shareAttempt = React.useRef(0);
  const onImportedRef = React.useRef(onImported);
  onImportedRef.current = onImported;
  React.useEffect(() => {
    const refresh = (event: Event) => {
      if ((event as CustomEvent<{ app: string }>).detail?.app === sourceApp) {
        void Promise.resolve().then(() => onImportedRef.current()).catch((error) => console.error('Failed to refresh imported provider:', error));
      }
    };
    window.addEventListener(DEEP_LINK_IMPORT_COMPLETED, refresh);
    return () => window.removeEventListener(DEEP_LINK_IMPORT_COMPLETED, refresh);
  }, [sourceApp]);
  const shareProvider = async (selected: ShareableProvider) => {
    const attempt = ++shareAttempt.current;
    if (selected.id && ['opencode', 'openclaw', 'pi', 'omp', 'hermes', 'dsh'].includes(sourceApp)) {
      try {
        const { credentialUnavailable, ...connectionDefaults } = await getProviderShareDefaults(sourceApp, selected.id);
        if (attempt !== shareAttempt.current) return;
        selected = { ...selected, connectionDefaults, credentialUnavailable: selected.credentialUnavailable || credentialUnavailable };
      } catch (error) {
        if (attempt !== shareAttempt.current) return;
        message.error(error instanceof Error ? error.message : String(error));
      }
    }
    if (attempt === shareAttempt.current) setProvider(selected);
  };
  React.useEffect(() => () => { shareAttempt.current += 1; }, []);
  return {
    shareProvider,
    shareModal: <ShareProviderModal open={provider !== null} sourceApp={sourceApp} provider={provider} onClose={() => { shareAttempt.current += 1; setProvider(null); }} />,
  };
}
