import { invoke } from '@tauri-apps/api/core';

/** Result of persisting one "sync models to Paseo" switch. */
export interface PaseoModelSyncToggleResult {
  enabled: boolean;
  /**
   * Stable backend warning code when the setting was saved but the immediate
   * local sync could not run:
   * `no_applied_provider` | `no_models` | `paseo_config_missing` | `sync_failed`.
   */
  syncWarning?: string;
}

export const setClaudePaseoModelSyncEnabled = async (
  enabled: boolean,
): Promise<PaseoModelSyncToggleResult> => {
  return await invoke<PaseoModelSyncToggleResult>('set_claude_paseo_model_sync_enabled', {
    input: { enabled },
  });
};

export const setCodexPaseoModelSyncEnabled = async (
  enabled: boolean,
): Promise<PaseoModelSyncToggleResult> => {
  return await invoke<PaseoModelSyncToggleResult>('set_codex_paseo_model_sync_enabled', {
    input: { enabled },
  });
};
