/**
 * OpenCode Configuration Types
 * 
 * Type definitions for OpenCode configuration management.
 */

export interface OpenCodeModelLimit {
  context?: number;
  input?: number;
  output?: number;
}

export interface OpenCodeModelVariant {
  reasoningEffort?: 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max';
  textVerbosity?: 'low' | 'medium' | 'high';
  disabled?: boolean;
  [key: string]: unknown;
}

export interface OpenCodeModelModalities {
  input?: string[];
  output?: string[];
}

export interface OpenCodeModel {
  id?: string;
  name?: string;
  family?: string;
  release_date?: string;
  limit?: OpenCodeModelLimit;
  modalities?: OpenCodeModelModalities;
  attachment?: boolean;
  reasoning?: boolean;
  tool_call?: boolean;
  temperature?: boolean;
  options?: Record<string, unknown>;
  variants?: Record<string, OpenCodeModelVariant>;
  [key: string]: unknown;
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
  api?: string;
  env?: unknown;
  id?: string;
  npm?: string;
  name?: string;
  options?: OpenCodeProviderOptions;
  models: Record<string, OpenCodeModel>;
  whitelist?: string[];
  blacklist?: string[];
  [key: string]: unknown;
}

export type OpenCodePluginEntry =
  | string
  | [string, Record<string, unknown>];

export type OpenCodePermissionAction = 'ask' | 'allow' | 'deny';

export type OpenCodePermissionRule =
  | OpenCodePermissionAction
  | Record<string, OpenCodePermissionAction>;

export interface OpenCodeAgentConfig {
  model?: string;
  variant?: string;
  temperature?: number;
  top_p?: number;
  prompt?: string;
  tools?: Record<string, boolean>;
  disable?: boolean;
  description?: string;
  mode?: 'subagent' | 'primary' | 'all';
  hidden?: boolean;
  options?: Record<string, unknown>;
  color?: string;
  steps?: number;
  maxSteps?: number;
  permission?: Record<string, OpenCodePermissionRule> | OpenCodePermissionAction;
  [key: string]: unknown;
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
  default_agent?: string;
  agent?: Record<string, OpenCodeAgentConfig>;
  plugin?: OpenCodePluginEntry[];
  mcp?: Record<string, McpServerConfig>;
  // Preserve unknown fields from config file
  [key: string]: unknown;
}
