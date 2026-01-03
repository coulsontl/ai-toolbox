/**
 * Provider API Service
 *
 * Handles all provider and model related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type { Provider, Model, ProviderWithModels } from '@/types/provider';

// ============================================================================
// Provider APIs
// ============================================================================

/**
 * List all providers
 */
export const listProviders = async (): Promise<Provider[]> => {
  return await invoke<Provider[]>('list_providers');
};

/**
 * Create a new provider
 */
export const createProvider = async (provider: Omit<Provider, 'created_at' | 'updated_at'>): Promise<Provider> => {
  return await invoke<Provider>('create_provider', { provider });
};

/**
 * Update an existing provider
 */
export const updateProvider = async (provider: Provider): Promise<Provider> => {
  return await invoke<Provider>('update_provider', { provider });
};

/**
 * Delete a provider
 */
export const deleteProvider = async (id: string): Promise<void> => {
  await invoke('delete_provider', { id });
};

/**
 * Reorder providers
 */
export const reorderProviders = async (ids: string[]): Promise<void> => {
  await invoke('reorder_providers', { ids });
};

// ============================================================================
// Model APIs
// ============================================================================

/**
 * List models for a specific provider
 */
export const listModels = async (providerId: string): Promise<Model[]> => {
  return await invoke<Model[]>('list_models', { provider_id: providerId });
};

/**
 * Create a new model
 */
export const createModel = async (model: Omit<Model, 'created_at' | 'updated_at'>): Promise<Model> => {
  return await invoke<Model>('create_model', { model });
};

/**
 * Update an existing model
 */
export const updateModel = async (model: Model): Promise<Model> => {
  return await invoke<Model>('update_model', { model });
};

/**
 * Delete a model
 */
export const deleteModel = async (providerId: string, id: string): Promise<void> => {
  await invoke('delete_model', { provider_id: providerId, id });
};

/**
 * Reorder models for a specific provider
 */
export const reorderModels = async (providerId: string, ids: string[]): Promise<void> => {
  await invoke('reorder_models', { provider_id: providerId, ids });
};

// ============================================================================
// Combined APIs
// ============================================================================

/**
 * Get all providers with their models
 */
export const getAllProvidersWithModels = async (): Promise<ProviderWithModels[]> => {
  return await invoke<ProviderWithModels[]>('get_all_providers_with_models');
};
