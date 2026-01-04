/**
 * Shared types for ProviderCard and ModelItem components
 */

/**
 * Unified provider display data interface
 */
export interface ProviderDisplayData {
  id: string;
  name: string;
  sdkName: string;
  baseUrl: string;
}

/**
 * Unified model display data interface
 */
export interface ModelDisplayData {
  id: string;
  name: string;
  contextLimit?: number;
  outputLimit?: number;
}

/**
 * i18n prefix type for different pages
 */
export type I18nPrefix = 'settings' | 'opencode';
