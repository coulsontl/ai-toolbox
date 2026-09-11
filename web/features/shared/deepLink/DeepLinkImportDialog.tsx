import React from 'react';
import type { DeepLinkImportRequest } from '@/services/deeplinkApi';
import ProviderTransferModal from './ProviderTransferModal';

export interface DeepLinkImportDialogProps {
  request: DeepLinkImportRequest | null;
  onDismiss: () => void;
}

const DeepLinkImportDialog: React.FC<DeepLinkImportDialogProps> = ({ request, onDismiss }) =>
  <ProviderTransferModal mode="import" request={request} onClose={onDismiss} />;

export default DeepLinkImportDialog;
