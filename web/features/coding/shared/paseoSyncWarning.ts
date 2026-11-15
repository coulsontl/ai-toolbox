/**
 * Backend warning codes for the "sync models to Paseo" switch. The suffix is
 * appended to `common.paseoSync.warning`; keeping the mapping total prevents a
 * new backend code from rendering a literal i18n key.
 */
const PASEO_SYNC_WARNING_SUFFIX: Record<string, string> = {
  no_applied_provider: 'NoAppliedProvider',
  no_models: 'NoModels',
  paseo_config_missing: 'ConfigMissing',
  sync_failed: 'Failed',
};

export function paseoSyncWarningKey(code: string | undefined): string {
  const suffix = code ? (PASEO_SYNC_WARNING_SUFFIX[code] ?? 'Failed') : 'Failed';
  return `common.paseoSync.warning${suffix}`;
}
