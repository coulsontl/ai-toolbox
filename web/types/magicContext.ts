export type MagicContextHarness = 'opencode' | 'pi';

export interface MagicContextConfigRequest {
  harness: MagicContextHarness;
}

export interface MagicContextSaveInput extends MagicContextConfigRequest {
  content: string;
}

export interface MagicContextConfigFile {
  harness: MagicContextHarness;
  path: string;
  directory: string;
  exists: boolean;
  content: string;
  parsed?: Record<string, unknown> | null;
  parseError?: string | null;
  warnings: string[];
}

export interface MagicContextDoctorInput {
  harness: MagicContextHarness;
}

export interface MagicContextCommandResult {
  command: string;
  output: string;
}
