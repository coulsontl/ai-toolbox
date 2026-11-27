import React from 'react';
import { App, Input, Modal, Select, Spin } from 'antd';
import {
  ChevronRight,
  ExternalLink,
  FileUp,
  Fingerprint,
  KeyRound,
  Loader2,
  RotateCcw,
  Upload,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { readTextFile } from '@tauri-apps/plugin-fs';
import {
  cancelPluginLogin,
  loginPluginApiKey,
  loginPluginAuthorize,
  loginPluginCallback,
  loginPluginImportFiles,
  loginPluginPrompt,
  loginPluginValidate,
  openExternalUrl,
  type PluginImportResult,
  type PluginLoginAuthorizeResult,
  type PluginLoginPrompt,
  type PluginProviderMethod,
  type PluginProviderRow,
} from '@/services';
import { formatPluginError, pickLocalizedText } from '../utils/pluginFormatters';
import styles from './PluginModals.module.less';

type LoginMode = 'methods' | 'prompt' | 'oauth' | 'apikey' | 'import';

interface PluginLoginModalProps {
  open: boolean;
  provider: PluginProviderRow | null;
  onClose: () => void;
  onSignedIn: () => void;
}

const MAX_IMPORT_FILES = 20;
const MAX_IMPORT_BYTES = 512 * 1024;

const PluginLoginModal: React.FC<PluginLoginModalProps> = ({
  open,
  provider,
  onClose,
  onSignedIn,
}) => {
  const { t, i18n } = useTranslation();
  const { message } = App.useApp();

  const [mode, setMode] = React.useState<LoginMode>('methods');
  const [methodIndex, setMethodIndex] = React.useState<number | null>(null);
  const [inputs, setInputs] = React.useState<Record<string, unknown>>({});
  const [prompt, setPrompt] = React.useState<PluginLoginPrompt | null>(null);
  const [promptValue, setPromptValue] = React.useState('');
  const [validationError, setValidationError] = React.useState<string | null>(null);
  const [oauthState, setOauthState] = React.useState<PluginLoginAuthorizeResult | null>(null);
  const [oauthCode, setOauthCode] = React.useState('');
  const [waitingForCallback, setWaitingForCallback] = React.useState(false);
  const [apiKeyValue, setApiKeyValue] = React.useState('');
  const [importResults, setImportResults] = React.useState<PluginImportResult[]>([]);
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setMode('methods');
    setMethodIndex(null);
    setInputs({});
    setPrompt(null);
    setPromptValue('');
    setValidationError(null);
    setOauthState(null);
    setOauthCode('');
    setWaitingForCallback(false);
    setApiKeyValue('');
    setImportResults([]);
    setSubmitting(false);
    setError(null);
  }, [open, provider?.id]);

  const methodLabel = (method: PluginProviderMethod | undefined) => {
    if (!method) return '';
    return pickLocalizedText(method.labels, method.label || method.type, i18n.language);
  };

  const currentMethod = methodIndex == null
    ? undefined
    : provider?.methods[methodIndex];

  const resetToMethods = () => {
    setMode('methods');
    setMethodIndex(null);
    setInputs({});
    setPrompt(null);
    setPromptValue('');
    setValidationError(null);
    setOauthState(null);
    setOauthCode('');
    setWaitingForCallback(false);
    setApiKeyValue('');
    setImportResults([]);
    setError(null);
  };

  const handleClose = () => {
    if (submitting || waitingForCallback) {
      void cancelPluginLogin().catch(() => undefined);
    }
    onClose();
  };

  const finishLogin = () => {
    message.success(t('plugins.loginModal.loginSuccess'));
    onSignedIn();
  };

  const runApiKey = async (
    index: number,
    currentInputs: Record<string, unknown>,
    key: string,
    keyless: boolean,
  ) => {
    if (!provider) return;
    setSubmitting(true);
    setError(null);
    try {
      const completion = await loginPluginApiKey(
        provider.id,
        String(index),
        currentInputs,
        keyless ? null : key || null,
        null,
        null,
      );
      if (completion.ok) {
        finishLogin();
      } else {
        setError(completion.error ?? t('plugins.loginModal.loginFailedNoReason'));
      }
    } catch (loginError) {
      setError(formatPluginError(loginError));
    } finally {
      setSubmitting(false);
    }
  };

  const runAuthorize = async (index: number, currentInputs: Record<string, unknown>) => {
    if (!provider) return;
    setMode('oauth');
    setOauthState(null);
    setOauthCode('');
    setError(null);
    setSubmitting(true);
    try {
      const result = await loginPluginAuthorize(
        provider.id,
        String(index),
        currentInputs,
        null,
        null,
      );
      if (!result.ok) {
        setError(result.error ?? t('plugins.loginModal.oauthUnsupported'));
        return;
      }
      setOauthState(result);
      if (result.method !== 'code') {
        if (!result.session) {
          setError(t('plugins.loginModal.oauthUnsupported'));
          return;
        }
        setWaitingForCallback(true);
        const completion = await loginPluginCallback(result.session, null);
        if (completion.ok) {
          finishLogin();
        } else {
          setError(completion.error ?? t('plugins.loginModal.loginFailedNoReason'));
        }
      }
    } catch (loginError) {
      setError(formatPluginError(loginError));
    } finally {
      setWaitingForCallback(false);
      setSubmitting(false);
    }
  };

  /** Ask the next plugin question; when none is left, enter the real step. */
  const advancePrompt = async (index: number, currentInputs: Record<string, unknown>) => {
    if (!provider) return;
    setSubmitting(true);
    setError(null);
    try {
      const response = await loginPluginPrompt(provider.id, String(index), currentInputs);
      const method = provider.methods[index];
      if (response.prompt) {
        setPrompt(response.prompt);
        setPromptValue('');
        setValidationError(null);
        setMode('prompt');
        setSubmitting(false);
        return;
      }

      setPrompt(null);
      if (method?.type === 'oauth') {
        await runAuthorize(index, currentInputs);
        return;
      }
      if (method?.type === 'api' && !method.keyAsked && !method.keyless) {
        setMode('apikey');
        setSubmitting(false);
        return;
      }
      await runApiKey(index, currentInputs, '', method?.keyless === true);
    } catch (promptError) {
      setError(formatPluginError(promptError));
      setSubmitting(false);
    }
  };

  const startMethod = (index: number) => {
    setMethodIndex(index);
    setInputs({});
    setPrompt(null);
    setPromptValue('');
    setValidationError(null);
    setOauthState(null);
    setOauthCode('');
    setApiKeyValue('');
    setImportResults([]);
    setError(null);
    setMode('prompt');
    void advancePrompt(index, {});
  };

  const handlePromptContinue = async () => {
    if (!provider || methodIndex == null || !prompt) return;
    if (!promptValue.trim()) {
      setValidationError(t('plugins.loginModal.promptRequired'));
      return;
    }
    setValidationError(null);
    setSubmitting(true);
    setError(null);
    try {
      const validation = await loginPluginValidate(
        provider.id,
        String(methodIndex),
        prompt.key,
        promptValue,
      );
      if (validation.error) {
        setValidationError(validation.error);
        return;
      }
      const nextInputs = { ...inputs, [prompt.key]: promptValue };
      setInputs(nextInputs);
      await advancePrompt(methodIndex, nextInputs);
    } catch (promptError) {
      setError(formatPluginError(promptError));
    } finally {
      setSubmitting(false);
    }
  };

  const handleOAuthCode = async () => {
    if (!oauthState?.session) {
      setError(t('plugins.loginModal.oauthUnsupported'));
      return;
    }
    if (!oauthCode.trim()) {
      setError(t('plugins.loginModal.codeRequired'));
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const completion = await loginPluginCallback(oauthState.session, oauthCode.trim());
      if (completion.ok) {
        finishLogin();
      } else {
        setError(completion.error ?? t('plugins.loginModal.loginFailedNoReason'));
      }
    } catch (callbackError) {
      setError(formatPluginError(callbackError));
    } finally {
      setSubmitting(false);
    }
  };

  const handleApiKeySubmit = async () => {
    if (methodIndex == null) return;
    if (!apiKeyValue.trim()) {
      setError(t('plugins.loginModal.apiKeyRequired'));
      return;
    }
    await runApiKey(methodIndex, inputs, apiKeyValue.trim(), false);
  };

  const handleImport = async () => {
    if (!provider) return;
    setMode('import');
    setError(null);
    setImportResults([]);
    let selection: string | string[] | null = null;
    try {
      selection = await openDialog({
        multiple: true,
        directory: false,
        filters: [{ name: 'Account files', extensions: ['json', 'txt'] }],
      });
    } catch (dialogError) {
      setError(formatPluginError(dialogError));
      return;
    }
    const files = typeof selection === 'string'
      ? [selection]
      : Array.isArray(selection)
        ? selection
        : [];
    if (files.length === 0) {
      return;
    }

    setSubmitting(true);
    try {
      const texts: string[] = [];
      for (const file of files.slice(0, MAX_IMPORT_FILES)) {
        const text = await readTextFile(file);
        if (!text.trim()) continue;
        if (new TextEncoder().encode(text).length > MAX_IMPORT_BYTES) {
          setError(t('plugins.loginModal.importTooLarge', { file }));
          return;
        }
        texts.push(text);
      }
      if (texts.length === 0) {
        setError(t('plugins.loginModal.importEmpty'));
        return;
      }
      const response = await loginPluginImportFiles(provider.id, texts);
      setImportResults(response.results);
      if (response.results.length === 0) {
        setError(t('plugins.loginModal.importNoResults'));
      }
    } catch (importError) {
      setError(formatPluginError(importError));
    } finally {
      setSubmitting(false);
    }
  };

  const importStatusText = (status: string) => {
    if (status === 'added') return t('plugins.loginModal.importStatusAdded');
    if (status === 'updated') return t('plugins.loginModal.importStatusUpdated');
    return t('plugins.loginModal.importStatusFailed');
  };

  const importStatusClass = (status: string) => {
    if (status === 'added') return styles.resultAdded;
    if (status === 'updated') return styles.resultUpdated;
    return styles.resultFailed;
  };

  const importFrom = provider
    ? pickLocalizedText(provider.importFroms, provider.importFrom ?? '', i18n.language)
    : '';
  const methodCount = provider?.methods.length ?? 0;

  return (
    <Modal
      open={open}
      title={
        provider
          ? t('plugins.loginModal.title', { name: provider.name || provider.id })
          : ''
      }
      width={620}
      footer={null}
      onCancel={handleClose}
      mask={{ closable: !submitting && !waitingForCallback }}
    >
      <div className={styles.content}>
        {mode === 'methods' ? (
          <div className={styles.sectionCard}>
            <h3 className={styles.sectionTitle}>
              {t('plugins.loginModal.chooseMethod')}
            </h3>
            <p className={styles.sectionHint}>{t('plugins.loginModal.methodHint')}</p>
            <div className={styles.methodList}>
              {provider?.methods.map((method, index) => (
                <button
                  key={`${method.type}-${method.label ?? index}`}
                  type="button"
                  className={styles.methodRow}
                  onClick={() => startMethod(index)}
                >
                  <span className={styles.methodIcon}>
                    {method.type === 'oauth' ? (
                      <Fingerprint size={18} aria-hidden="true" />
                    ) : (
                      <KeyRound size={18} aria-hidden="true" />
                    )}
                  </span>
                  <span className={styles.methodBody}>
                    <span className={styles.methodName}>{methodLabel(method)}</span>
                    <span className={styles.methodMeta}>
                      {method.type === 'oauth'
                        ? t('plugins.loginModal.oauthMeta')
                        : method.keyless
                          ? t('plugins.loginModal.keylessMeta')
                          : t('plugins.loginModal.apiKeyMeta')}
                    </span>
                  </span>
                  <ChevronRight
                    size={16}
                    className={styles.methodChevron}
                    aria-hidden="true"
                  />
                </button>
              ))}
              {provider?.importable ? (
                <button
                  type="button"
                  className={styles.methodRow}
                  onClick={() => void handleImport()}
                >
                  <span className={styles.methodIcon}>
                    <Upload size={18} aria-hidden="true" />
                  </span>
                  <span className={styles.methodBody}>
                    <span className={styles.methodName}>
                      {t('plugins.loginModal.import')}
                    </span>
                    <span className={styles.methodMeta}>
                      {importFrom || t('plugins.loginModal.importHint')}
                    </span>
                  </span>
                  <ChevronRight
                    size={16}
                    className={styles.methodChevron}
                    aria-hidden="true"
                  />
                </button>
              ) : null}
            </div>
            {methodCount === 0 && !provider?.importable ? (
              <div className={styles.emptyText}>
                {t('plugins.loginModal.noMethods')}
              </div>
            ) : null}
            <div className={styles.footerActions}>
              <button type="button" className={styles.actionButton} onClick={handleClose}>
                {t('common.cancel')}
              </button>
            </div>
          </div>
        ) : null}

        {mode === 'prompt' ? (
          <div className={styles.sectionCard}>
            <div className={styles.stepHeader}>
              <span className={styles.stepLabel}>{methodLabel(currentMethod)}</span>
              <button type="button" className={styles.linkButton} onClick={resetToMethods}>
                {t('common.back')}
              </button>
            </div>
            {prompt ? (
              <>
                <p className={styles.promptMessage}>
                  {pickLocalizedText(prompt.messages, prompt.message, i18n.language)}
                </p>
                <div className={styles.formFieldRow}>
                  <span className={styles.formLabel}>{prompt.key}</span>
                  <div className={styles.formControl}>
                    {prompt.type === 'select' ? (
                      <Select
                        value={promptValue || undefined}
                        onChange={(value: string) => setPromptValue(value)}
                        placeholder={prompt.placeholder || t('plugins.loginModal.promptSelectPlaceholder')}
                        options={(prompt.options ?? []).map((option) => ({
                          value: option.value,
                          label: pickLocalizedText(option.labels, option.label, i18n.language),
                        }))}
                        className={styles.formControlInput}
                      />
                    ) : prompt.type === 'password' ? (
                      <Input.Password
                        value={promptValue}
                        onChange={(event) => setPromptValue(event.target.value)}
                        placeholder={prompt.placeholder}
                        onPressEnter={() => void handlePromptContinue()}
                        className={styles.formControlInput}
                      />
                    ) : (
                      <Input
                        value={promptValue}
                        onChange={(event) => setPromptValue(event.target.value)}
                        placeholder={prompt.placeholder}
                        onPressEnter={() => void handlePromptContinue()}
                        className={styles.formControlInput}
                      />
                    )}
                  </div>
                </div>
                {validationError ? (
                  <div className={styles.formError}>{validationError}</div>
                ) : null}
                <div className={styles.footerActions}>
                  <button
                    type="button"
                    className={styles.actionButton}
                    onClick={handleClose}
                  >
                    {t('common.cancel')}
                  </button>
                  <button
                    type="button"
                    className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                    disabled={submitting}
                    onClick={() => void handlePromptContinue()}
                  >
                    {submitting ? (
                      <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                    ) : null}
                    <span>{t('plugins.loginModal.continue')}</span>
                  </button>
                </div>
              </>
            ) : (
              <div className={styles.waitingRow}>
                <Spin size="small" />
                <span>{t('plugins.loginModal.starting')}</span>
              </div>
            )}
            {error ? <div className={styles.formError}>{error}</div> : null}
          </div>
        ) : null}

        {mode === 'oauth' ? (
          <div className={styles.sectionCard}>
            <div className={styles.stepHeader}>
              <span className={styles.stepLabel}>{methodLabel(currentMethod)}</span>
              <button
                type="button"
                className={styles.linkButton}
                onClick={() => {
                  if (methodIndex != null) startMethod(methodIndex);
                }}
              >
                <RotateCcw size={12} aria-hidden="true" />
                {t('plugins.loginModal.restart')}
              </button>
            </div>

            {oauthState ? (
              <>
                {oauthState.url ? (
                  <div className={styles.oauthBox}>
                    <div className={styles.oauthUrl}>{oauthState.url}</div>
                    <div className={styles.oauthActions}>
                      <button
                        type="button"
                        className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                        onClick={() => void openExternalUrl(oauthState.url as string)}
                      >
                        <ExternalLink size={14} aria-hidden="true" />
                        <span>{t('plugins.loginModal.openUrl')}</span>
                      </button>
                    </div>
                  </div>
                ) : null}
                {oauthState.instructions ? (
                  <p className={styles.promptHint}>{oauthState.instructions}</p>
                ) : null}
                {oauthState.method === 'code' ? (
                  <>
                    <div className={styles.formFieldRow}>
                      <span className={styles.formLabel}>
                        {t('plugins.loginModal.codeLabel')}
                      </span>
                      <div className={styles.formControl}>
                        <Input
                          value={oauthCode}
                          onChange={(event) => setOauthCode(event.target.value)}
                          placeholder={t('plugins.loginModal.codePlaceholder')}
                          onPressEnter={() => void handleOAuthCode()}
                          className={styles.formControlInput}
                        />
                      </div>
                    </div>
                    <div className={styles.footerActions}>
                      <button
                        type="button"
                        className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                        disabled={submitting}
                        onClick={() => void handleOAuthCode()}
                      >
                        {submitting ? (
                          <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                        ) : null}
                        <span>{t('plugins.loginModal.complete')}</span>
                      </button>
                    </div>
                  </>
                ) : waitingForCallback ? (
                  <div className={styles.waitingRow}>
                    <Spin size="small" />
                    <span>{t('plugins.loginModal.waiting')}</span>
                  </div>
                ) : null}
              </>
            ) : (
              <div className={styles.waitingRow}>
                <Spin size="small" />
                <span>{t('plugins.loginModal.starting')}</span>
              </div>
            )}

            {error ? <div className={styles.formError}>{error}</div> : null}
            <div className={styles.footerActions}>
              <button type="button" className={styles.actionButton} onClick={handleClose}>
                {t('common.cancel')}
              </button>
            </div>
          </div>
        ) : null}

        {mode === 'apikey' ? (
          <div className={styles.sectionCard}>
            <div className={styles.stepHeader}>
              <span className={styles.stepLabel}>
                {methodLabel(currentMethod)} · {t('plugins.loginModal.apiKeyLabel')}
              </span>
              <button type="button" className={styles.linkButton} onClick={resetToMethods}>
                {t('common.back')}
              </button>
            </div>
            <Input.Password
              value={apiKeyValue}
              onChange={(event) => setApiKeyValue(event.target.value)}
              placeholder={
                currentMethod?.placeholder || t('plugins.loginModal.apiKeyPlaceholder')
              }
              onPressEnter={() => void handleApiKeySubmit()}
            />
            {error ? <div className={styles.formError}>{error}</div> : null}
            <div className={styles.footerActions}>
              <button type="button" className={styles.actionButton} onClick={handleClose}>
                {t('common.cancel')}
              </button>
              <button
                type="button"
                className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                disabled={submitting}
                onClick={() => void handleApiKeySubmit()}
              >
                {submitting ? (
                  <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                ) : null}
                <span>{t('plugins.loginModal.submit')}</span>
              </button>
            </div>
          </div>
        ) : null}

        {mode === 'import' ? (
          <div className={styles.sectionCard}>
            <div className={styles.stepHeader}>
              <span className={styles.stepLabel}>{t('plugins.loginModal.import')}</span>
              <button type="button" className={styles.linkButton} onClick={resetToMethods}>
                {t('common.back')}
              </button>
            </div>

            {submitting ? (
              <div className={styles.waitingRow}>
                <Spin size="small" />
                <span>{t('plugins.loginModal.importReading')}</span>
              </div>
            ) : null}

            {error ? <div className={styles.formError}>{error}</div> : null}

            {importResults.length > 0 ? (
              <>
                <div className={styles.importSummary}>
                  <span className={styles.resultAdded}>
                    {t('plugins.loginModal.importStatusAddedCount', {
                      count: importResults.filter((result) => result.status === 'added').length,
                    })}
                  </span>
                  <span className={styles.resultUpdated}>
                    {t('plugins.loginModal.importStatusUpdatedCount', {
                      count: importResults.filter((result) => result.status === 'updated').length,
                    })}
                  </span>
                  <span className={styles.resultFailed}>
                    {t('plugins.loginModal.importStatusFailedCount', {
                      count: importResults.filter((result) => result.status === 'failed').length,
                    })}
                  </span>
                </div>
                <div className={styles.resultList}>
                  {importResults.map((result, index) => (
                    <div key={`${result.account ?? result.user}-${index}`} className={styles.resultRow}>
                      <span className={importStatusClass(result.status)}>
                        {importStatusText(result.status)}
                      </span>
                      <span className={styles.resultUser}>
                        {result.user || result.account || '—'}
                      </span>
                      {result.error ? (
                        <span className={styles.resultError}>{result.error}</span>
                      ) : null}
                    </div>
                  ))}
                </div>
                <div className={styles.footerActions}>
                  <button
                    type="button"
                    className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                    onClick={onSignedIn}
                  >
                    {t('plugins.loginModal.done')}
                  </button>
                </div>
              </>
            ) : !submitting ? (
              <>
                <p className={styles.promptHint}>
                  {importFrom || t('plugins.loginModal.importHint')}
                </p>
                <div className={styles.footerActions}>
                  <button type="button" className={styles.actionButton} onClick={handleClose}>
                    {t('common.cancel')}
                  </button>
                  <button
                    type="button"
                    className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                    onClick={() => void handleImport()}
                  >
                    <FileUp size={14} aria-hidden="true" />
                    <span>{t('plugins.loginModal.importPick')}</span>
                  </button>
                </div>
              </>
            ) : null}
          </div>
        ) : null}
      </div>
    </Modal>
  );
};

export default PluginLoginModal;
