/**
 * OpenCode Configuration Types
 * 
 * Type definitions for OpenCode configuration management.
 */

export interface OpenCodeModelLimit {
  context?: number;
  output?: number;
}

export interface OpenCodeModel {
  name: string;
  limit?: OpenCodeModelLimit;
  options?: Record<string, unknown>;
}

export interface OpenCodeProviderOptions {
  baseURL: string;
  apiKey?: string;
  headers?: Record<string, string>;
}

export interface OpenCodeProvider {
  npm: string;
  name: string;
  options: OpenCodeProviderOptions;
  models: Record<string, OpenCodeModel>;
}

export interface OpenCodeConfig {
  $schema?: string;
  provider: Record<string, OpenCodeProvider>;
  model?: string;
  small_model?: string;
}
