import React from 'react';
import { Alert, App, Button, Input, Modal, Select } from 'antd';
import { Copy } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { previewDeepLinkImport, type DeepLinkImportPreview, type DeepLinkImportRequest, type ImportConflictPolicy } from '@/services/deeplinkApi';
import { buildProviderShareUrl } from './providerShareUrl';
import { importDeepLinkRequest } from './deeplinkImportAction';
import { PROVIDER_SHARE_TOOLS, type SharedApiFormat, type SharedModel } from './providerTransfer';
import styles from './ProviderTransferModal.module.less';

interface Props {
  request: DeepLinkImportRequest | null;
  mode: 'share' | 'import';
  credentialUnavailable?: boolean;
  sourceOptions?: Array<{ value: string; label: string }>;
  selectedSource?: string;
  onSourceChange?: (value: string) => void;
  onClose: () => void;
}

const API_FORMATS: Array<{ value: SharedApiFormat; label: string }> = [
  { value: 'anthropic_messages', label: 'Anthropic Messages' },
  { value: 'openai_responses', label: 'OpenAI Responses' },
  { value: 'openai_chat', label: 'OpenAI Chat' },
  { value: 'gemini_native', label: 'Gemini Native' },
  { value: 'ollama/chat', label: 'Ollama' },
];

const ProviderTransferModal: React.FC<Props> = ({ request, mode, credentialUnavailable = false, sourceOptions, selectedSource, onSourceChange, onClose }) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [draft, setDraft] = React.useState<DeepLinkImportRequest | null>(null);
  const [conflictPolicy, setConflictPolicy] = React.useState<ImportConflictPolicy>('skip');
  const [preview, setPreview] = React.useState<DeepLinkImportPreview | null>(null);
  const [checking, setChecking] = React.useState(false);
  const [importing, setImporting] = React.useState(false);
  const [error, setError] = React.useState('');
  const fieldId = React.useId();
  const activeRequest = React.useRef(request);
  activeRequest.current = request;

  React.useEffect(() => {
    setDraft(request ? { ...request, models: request.models?.length ? request.models : request.model ? [{ id: request.model }] : [] } : null);
    setConflictPolicy('skip');
    setPreview(null);
    setError('');
    setImporting(false);
  }, [request]);

  React.useEffect(() => {
    if (!draft) return;
    let active = true;
    setChecking(true);
    setPreview(null);
    setError('');
    const timer = setTimeout(() => {
      void previewDeepLinkImport(draft).then((result) => {
        if (active) setPreview(result);
      }).catch((reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : String(reason));
      }).finally(() => { if (active) setChecking(false); });
    }, 180);
    return () => { active = false; clearTimeout(timer); };
  }, [draft]);

  const updateDraft = (fields: Partial<DeepLinkImportRequest>) => setDraft((current) => current ? { ...current, ...fields } : current);
  const legacyConfig = Boolean(draft?.config || draft?.extra);
  const missingCredential = credentialUnavailable && !draft?.apiKey?.trim();
  const missingFormat = mode === 'share' && !draft?.apiFormat && !draft?.gatewayProfile;
  const disabled = importing || checking || !preview || missingCredential || missingFormat;
  const shareUrl = draft ? buildProviderShareUrl(draft) : '';
  // Windows scheme handlers may truncate long command lines. Local imports do
  // not use a URL and remain available for large model catalogs.
  const linkTooLong = shareUrl.length > 8000;

  const updateModels = (ids: string[]) => {
    if (!draft) return;
    const available = [...(draft.models ?? []), ...(request?.models ?? [])];
    const models = ids.map((id): SharedModel => available.find((model) => model.id === id) ?? { id });
    updateDraft({
      models,
      model: draft.model && ids.includes(draft.model) ? draft.model : undefined,
      modelRoles: Object.fromEntries(Object.entries(draft.modelRoles ?? {}).filter(([, model]) => ids.includes(model))),
    });
  };

  const handleImport = async () => {
    if (!draft || disabled) return;
    setImporting(true);
    setError('');
    const submittedRequest = request;
    try {
      const result = await importDeepLinkRequest(draft, conflictPolicy);
      if (activeRequest.current !== submittedRequest) return;
      if (result.status === 'skipped') message.info(t('common.deepLink.importSkipped', { name: result.name }));
      else message.success(t('common.deepLink.importSuccess'));
      onClose();
    } catch (reason) {
      if (activeRequest.current === submittedRequest) setError(reason instanceof Error ? reason.message : String(reason));
    } finally { if (activeRequest.current === submittedRequest) setImporting(false); }
  };

  const handleCopy = async () => {
    try { await navigator.clipboard.writeText(shareUrl); message.success(t('common.copied')); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
  };

  return (
    <Modal open={request !== null} title={t(mode === 'share' ? 'common.deepLink.shareTitle' : 'common.deepLink.title')}
      width={640} centered style={{ maxWidth: '100%' }} onCancel={onClose} footer={null} closable={!importing} mask={{ closable: !importing }} keyboard={!importing}>
      <p className={styles.description}>{t('common.deepLink.transferDescription')}</p>
      {draft && <>
        <div className={styles.form}>
          {sourceOptions && <div className={styles.field}>
            <label htmlFor={`${fieldId}-source`}>{t('common.deepLink.sourceConnection')}</label>
            <Select id={`${fieldId}-source`} value={selectedSource} options={sourceOptions} disabled={importing} onChange={onSourceChange} />
          </div>}
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-target`}>{t('common.deepLink.shareTargetTool')}</label>
            <Select id={`${fieldId}-target`} value={draft.app} disabled={importing || legacyConfig}
              options={PROVIDER_SHARE_TOOLS.map((tool) => ({ value: tool.app, label: tool.label }))}
              onChange={(app) => updateDraft({ app })} />
          </div>
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-name`}>{t('common.deepLink.fieldName')}</label>
            <Input id={`${fieldId}-name`} value={draft.name} disabled={importing} onChange={(event) => updateDraft({ name: event.target.value })} />
          </div>
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-format`}>{t('common.deepLink.fieldApiFormat')}</label>
            <Select id={`${fieldId}-format`} value={draft.gatewayProfile ? preview?.apiFormat ?? draft.apiFormat : draft.apiFormat ?? (mode === 'import' ? preview?.apiFormat : undefined)} disabled={importing || legacyConfig}
              placeholder={t('common.deepLink.selectApiFormat')} options={API_FORMATS}
              onChange={(apiFormat) => updateDraft({ apiFormat: apiFormat as SharedApiFormat, gatewayProfile: undefined, providerType: undefined, apiKeyField: undefined })} />
          </div>
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-key`}>{t('common.deepLink.fieldApiKey')}</label>
            <Input.Password id={`${fieldId}-key`} value={draft.apiKey ?? ''} disabled={importing || legacyConfig} autoComplete="off"
              onChange={(event) => updateDraft({ apiKey: event.target.value || undefined, category: 'custom' })} />
          </div>
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-url`}>{t('common.deepLink.fieldBaseUrl')}</label>
            <Input id={`${fieldId}-url`} value={draft.baseUrl ?? ''} disabled={importing || legacyConfig}
              onChange={(event) => updateDraft({ baseUrl: event.target.value || undefined })} />
          </div>
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-models`}>{t('common.deepLink.fieldModels')}</label>
            <Select id={`${fieldId}-models`} mode="tags" value={(draft.models ?? []).map((model) => model.id)} disabled={importing || legacyConfig}
              options={(draft.models ?? []).map((model) => ({ value: model.id, label: model.name ? `${model.name} (${model.id})` : model.id }))}
              onChange={updateModels} tokenSeparators={[',']} maxTagCount="responsive" />
          </div>
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-model`}>{t('common.deepLink.fieldDefaultModel')}</label>
            <Select id={`${fieldId}-model`} allowClear showSearch value={draft.model} disabled={importing || legacyConfig}
              options={(draft.models ?? []).map((model) => ({ value: model.id, label: model.id }))}
              onChange={(model) => updateDraft({ model })} />
          </div>
          <div className={styles.field}>
            <label htmlFor={`${fieldId}-conflict`}>{t('common.deepLink.conflictPolicy')}</label>
            <Select id={`${fieldId}-conflict`} value={conflictPolicy} disabled={importing}
              options={['skip', 'copy'].map((value) => ({ value, label: t(`common.deepLink.conflict_${value}`) }))}
              onChange={(value) => setConflictPolicy(value as ImportConflictPolicy)} />
          </div>
          <p className={styles.hint}>{t('common.deepLink.transferScope')}</p>
        </div>
        <div className={styles.feedback} aria-live="polite">
          {legacyConfig && <Alert type="info" showIcon title={t('common.deepLink.legacyConfigHint')} />}
          {legacyConfig && <details>
            <summary>{t('common.deepLink.legacyConfigPreview')}</summary>
            <Input.TextArea aria-label={t('common.deepLink.legacyConfigPreview')} value={[draft.config, draft.extra].filter(Boolean).join('\n\n')} readOnly autoSize={{ minRows: 3, maxRows: 12 }} />
          </details>}
          {missingCredential && <Alert type="warning" showIcon title={t('common.deepLink.credentialUnavailable')} />}
          {missingFormat && <Alert type="info" showIcon title={t('common.deepLink.selectApiFormat')} />}
          {preview?.requiresGateway && <Alert type="info" showIcon title={t('common.deepLink.requiresGateway')} />}
          {preview?.baseUrl && preview.baseUrl !== draft.baseUrl && <p className={styles.hint}>{t('common.deepLink.adaptedBaseUrl', { url: preview.baseUrl })}</p>}
          {preview && <p className={styles.hint}>{t(preview.writesRuntimeFiles ? 'common.deepLink.runtimeImportHint' : 'common.deepLink.savedImportHint')}</p>}
          {draft.gatewayProfile && preview && !preview.profilePreserved && <p className={styles.hint}>{t('common.deepLink.genericProfileHint')}</p>}
          {error && <Alert type="error" showIcon title={error} />}
        </div>
        {mode === 'share' && <div className={styles.shareLink}>
          <Input.TextArea aria-label={t('common.deepLink.shareLink')} value={shareUrl} readOnly rows={3} />
          <p className={styles.hint}>{t('common.deepLink.shareWarning')}</p>
          {linkTooLong && <Alert type="warning" showIcon title={t('common.deepLink.linkTooLong')} />}
        </div>}
        <div className={styles.footer}>
          <Button disabled={importing} onClick={onClose}>{t('common.cancel')}</Button>
          <Button type="primary" loading={importing || checking} disabled={disabled} onClick={() => void handleImport()}>
            {t(mode === 'share' ? 'common.deepLink.shareImportToLocal' : 'common.deepLink.import')}
          </Button>
          {mode === 'share' && <Button icon={<Copy size={14} />} disabled={disabled || linkTooLong} onClick={() => void handleCopy()}>{t('common.deepLink.copyLink')}</Button>}
        </div>
      </>}
    </Modal>
  );
};

export default ProviderTransferModal;
