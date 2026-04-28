import { invoke } from '@tauri-apps/api/core';

export interface ImageChannelModel {
  id: string;
  name?: string | null;
  supports_text_to_image: boolean;
  supports_image_to_image: boolean;
  enabled: boolean;
}

export interface ImageChannel {
  id: string;
  name: string;
  provider_kind: 'openai_compatible';
  base_url: string;
  api_key: string;
  generation_path?: string | null;
  edit_path?: string | null;
  timeout_seconds?: number | null;
  enabled: boolean;
  sort_order: number;
  models: ImageChannelModel[];
  created_at: number;
  updated_at: number;
}

export interface ImageTaskParams {
  size: string;
  quality: string;
  output_format: string;
  output_compression?: number | null;
  moderation?: string | null;
}

export interface ImageReferenceInput {
  file_name: string;
  mime_type: string;
  base64_data: string;
}

export interface CreateImageJobInput {
  mode: 'text_to_image' | 'image_to_image';
  prompt: string;
  channel_id: string;
  model_id: string;
  params: ImageTaskParams;
  references: ImageReferenceInput[];
}

export interface UpsertImageChannelInput {
  id?: string | null;
  name: string;
  provider_kind: 'openai_compatible';
  base_url: string;
  api_key: string;
  generation_path?: string | null;
  edit_path?: string | null;
  timeout_seconds?: number | null;
  enabled: boolean;
  models: ImageChannelModel[];
}

export interface ImageAsset {
  id: string;
  job_id?: string | null;
  role: string;
  mime_type: string;
  file_name: string;
  relative_path: string;
  bytes: number;
  width?: number | null;
  height?: number | null;
  created_at: number;
  file_path: string;
}

export interface ImageJob {
  id: string;
  mode: 'text_to_image' | 'image_to_image';
  prompt: string;
  channel_id: string;
  channel_name_snapshot: string;
  model_id: string;
  model_name_snapshot: string;
  params_json: string;
  status: 'running' | 'done' | 'error';
  error_message?: string | null;
  request_url?: string | null;
  request_headers_json?: string | null;
  request_body_json?: string | null;
  input_assets: ImageAsset[];
  output_assets: ImageAsset[];
  created_at: number;
  finished_at?: number | null;
  elapsed_ms?: number | null;
}

export interface ImageWorkspace {
  channels: ImageChannel[];
  jobs: ImageJob[];
}

export interface DeleteImageJobInput {
  id: string;
  delete_local_assets: boolean;
}

export const getImageWorkspace = async (): Promise<ImageWorkspace> => {
  return invoke<ImageWorkspace>('image_get_workspace');
};

export const listImageChannels = async (limit = 200): Promise<ImageChannel[]> => {
  return invoke<ImageChannel[]>('image_list_channels', { input: { limit } });
};

export const updateImageChannel = async (
  input: UpsertImageChannelInput
): Promise<ImageChannel> => {
  return invoke<ImageChannel>('image_update_channel', { input });
};

export const deleteImageChannel = async (id: string): Promise<void> => {
  return invoke<void>('image_delete_channel', { input: { id } });
};

export const reorderImageChannels = async (orderedIds: string[]): Promise<ImageChannel[]> => {
  return invoke<ImageChannel[]>('image_reorder_channels', {
    input: { ordered_ids: orderedIds },
  });
};

export const listImageJobs = async (limit = 50): Promise<ImageJob[]> => {
  return invoke<ImageJob[]>('image_list_jobs', { input: { limit } });
};

export const createImageJob = async (input: CreateImageJobInput): Promise<ImageJob> => {
  return invoke<ImageJob>('image_create_job', { input });
};

export const deleteImageJob = async (input: DeleteImageJobInput): Promise<void> => {
  return invoke<void>('image_delete_job', { input });
};

export const revealImageAssetsDir = async (): Promise<string> => {
  return invoke<string>('image_reveal_assets_dir');
};
