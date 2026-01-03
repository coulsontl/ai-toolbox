/**
 * Provider and Model Types
 * 
 * Type definitions for AI provider and model management.
 */

export interface Provider {
  id: string;
  name: string;
  provider_type: string;
  base_url: string;
  api_key: string;
  sort_order: number;
  created_at: string;
  updated_at: string;
}

export interface Model {
  id: string;
  provider_id: string;
  name: string;
  context_limit: number;
  output_limit: number;
  options: string;
  sort_order: number;
  created_at: string;
  updated_at: string;
}

export interface ProviderWithModels {
  provider: Provider;
  models: Model[];
}

export interface CreateProviderInput {
  id: string;
  name: string;
  provider_type: string;
  base_url: string;
  api_key: string;
  sort_order: number;
}

export interface CreateModelInput {
  id: string;
  provider_id: string;
  name: string;
  context_limit: number;
  output_limit: number;
  options: string;
  sort_order: number;
}
