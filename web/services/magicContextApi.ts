import { invoke } from '@tauri-apps/api/core';

import type {
  MagicContextCommandResult,
  MagicContextConfigFile,
  MagicContextConfigRequest,
  MagicContextDoctorInput,
  MagicContextSaveInput,
} from '@/types/magicContext';

export const readMagicContextConfig = async (
  request: MagicContextConfigRequest,
): Promise<MagicContextConfigFile> => {
  return await invoke<MagicContextConfigFile>('read_magic_context_config', { request });
};

export const saveMagicContextConfig = async (
  input: MagicContextSaveInput,
): Promise<MagicContextConfigFile> => {
  return await invoke<MagicContextConfigFile>('save_magic_context_config', { input });
};

export const createMagicContextConfig = async (
  request: MagicContextConfigRequest,
): Promise<MagicContextConfigFile> => {
  return await invoke<MagicContextConfigFile>('create_magic_context_config', { request });
};

export const runMagicContextDoctor = async (
  input: MagicContextDoctorInput,
): Promise<MagicContextCommandResult> => {
  return await invoke<MagicContextCommandResult>('run_magic_context_doctor', { input });
};
