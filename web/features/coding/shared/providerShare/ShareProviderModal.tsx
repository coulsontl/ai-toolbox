import React from 'react';
import ProviderTransferModal from '@/features/shared/deepLink/ProviderTransferModal';
import { sanitizeShareHomepage } from '@/features/shared/deepLink/providerShareUrl';
import { extractProviderShareVariants, hasUnresolvedShareCredential, normalizeSharedApiFormat, type ProviderShareApp, type ShareableProvider } from '@/features/shared/deepLink/providerTransfer';
import type { DeepLinkImportRequest } from '@/services/deeplinkApi';
import { findGatewayProviderEndpointByReference, getGatewayProviderProfileReferenceFromMeta } from '../gateway/providerProfiles';

export type { ShareableProvider } from '@/features/shared/deepLink/providerTransfer';

export interface ShareProviderModalProps {
  open: boolean;
  sourceApp: ProviderShareApp;
  provider: ShareableProvider | null;
  onClose: () => void;
}

const ShareProviderModal: React.FC<ShareProviderModalProps> = ({ open, sourceApp, provider, onClose }) => {
  const [variantId, setVariantId] = React.useState('');
  const variants = React.useMemo(() => {
    if (!provider) return [];
    const resolved = findGatewayProviderEndpointByReference(getGatewayProviderProfileReferenceFromMeta(provider.meta));
    const effectiveProvider = resolved ? {
      ...provider,
      connectionDefaults: { ...provider.connectionDefaults, apiFormat: normalizeSharedApiFormat(resolved.endpoint.apiFormat) },
    } : provider;
    return extractProviderShareVariants(sourceApp, effectiveProvider);
  }, [sourceApp, provider]);
  const selectedVariant = variants.find((variant) => variant.id === variantId) ?? variants[0];
  React.useEffect(() => setVariantId(''), [provider]);
  const request = React.useMemo<DeepLinkImportRequest | null>(() => {
    if (!open || !provider || !selectedVariant) return null;
    const fields = { ...selectedVariant.fields };
    const reference = getGatewayProviderProfileReferenceFromMeta(provider.meta);
    const resolved = findGatewayProviderEndpointByReference(reference);
    if (resolved) {
      fields.apiFormat = normalizeSharedApiFormat(resolved.endpoint.apiFormat);
      fields.baseUrl ??= resolved.endpoint.baseUrl;
    }
    if (fields.gatewayProfile && !fields.gatewayProfile.tool) {
      fields.gatewayProfile = { ...fields.gatewayProfile, tool: sourceApp === 'claudedesktop' ? 'claude_desktop' : sourceApp };
    }
    return {
      resource: 'provider', app: sourceApp, name: variants.length > 1 ? `${provider.name} (${selectedVariant.id})` : provider.name,
      category: provider.category === 'third_party' ? 'third_party' : 'custom',
      ...fields, homepage: sanitizeShareHomepage(provider.websiteUrl), notes: provider.notes, icon: provider.icon, iconColor: provider.iconColor,
      sourceProviderId: provider.id && provider.id !== '__local__' ? `share:${sourceApp}:${provider.id}${selectedVariant.id ? `:${selectedVariant.id}` : ''}` : undefined,
      rawUrl: '',
    };
  }, [open, sourceApp, provider, selectedVariant, variants.length]);
  const credential = provider?.credential;
  const oauth = credential && typeof credential === 'object' && 'type' in credential && credential.type === 'oauth';
  return <ProviderTransferModal mode="share" request={request} onClose={onClose}
    sourceOptions={variants.length > 1 ? variants.map((variant) => ({ value: variant.id, label: variant.label })) : undefined}
    selectedSource={selectedVariant?.id} onSourceChange={setVariantId}
    credentialUnavailable={Boolean((provider?.category === 'official' || provider?.credentialUnavailable || oauth || (provider && hasUnresolvedShareCredential(provider))) && !request?.apiKey)} />;
};

export default ShareProviderModal;
