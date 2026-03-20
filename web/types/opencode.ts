/**
 * OpenCode Configuration Types
 * 
 * Type definitions for OpenCode configuration management.
 */

export interface OpenCodeModelLimit {
  context?: number;
  output?: number;
}

export interface OpenCodeModelVariant {
  reasoningEffort?: 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh';
  textVerbosity?: 'low' | 'medium' | 'high';
  disabled?: boolean;
  [key: string]: unknown;
}

export interface OpenCodeModelModalities {
  input?: string[];
  output?: string[];
}

export interface OpenCodeModel {
  name?: string;
  limit?: OpenCodeModelLimit;
  modalities?: OpenCodeModelModalities;
  attachment?: boolean;
  reasoning?: boolean;
  tool_call?: boolean;
  temperature?: boolean;
  options?: Record<string, unknown>;
  variants?: Record<string, OpenCodeModelVariant>;
}

export interface OpenCodeProviderOptions {
  baseURL?: string;
  apiKey?: string;
  headers?: Record<string, string>;
  timeout?: number | false;
  setCacheKey?: boolean;
  // 允许额外的自定义参数
  [key: string]: unknown;
}

export interface OpenCodeProvider {
  npm?: string;
  name?: string;
  options?: OpenCodeProviderOptions;
  models: Record<string, OpenCodeModel>;
  whitelist?: string[];
  blacklist?: string[];
}

/**
 * MCP Server Configuration
 */
export interface McpServerConfig {
  type: 'local' | 'remote';
  command?: string[];
  url?: string;
  enabled?: boolean;
}

export interface OpenCodeConfig {
  $schema?: string;
  provider: Record<string, OpenCodeProvider>;
  /**
   * List of provider IDs that are disabled.
   * When present, OpenCode should not use these providers for model availability.
   */
  disabled_providers?: string[];
  model?: string;
  small_model?: string;
  plugin?: string[];
  mcp?: Record<string, McpServerConfig>;
  // Preserve unknown fields from config file
  [key: string]: unknown;
}
