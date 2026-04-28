import assert from 'node:assert/strict';
import test from 'node:test';

import type {
  CreateImageJobInput,
  DeleteImageJobInput,
  ImageChannel,
  ImageJob,
  ImageWorkspace,
  UpsertImageChannelInput,
} from '../../../../../features/coding/image/services/imageApi.ts';
import { createImageStore } from '../../../../../features/coding/image/stores/imageStore.ts';

const sampleJobInput: CreateImageJobInput = {
  mode: 'text_to_image',
  prompt: 'test prompt',
  channel_id: 'channel-1',
  model_id: 'gpt-image-2',
  params: {
    size: '1024x1024',
    quality: 'high',
    output_format: 'png',
    output_compression: null,
    moderation: 'low',
  },
  references: [],
};

const sampleJob: ImageJob = {
  id: 'job-1',
  mode: 'text_to_image',
  prompt: 'test prompt',
  channel_id: 'channel-1',
  channel_name_snapshot: 'Channel 1',
  model_id: 'gpt-image-2',
  model_name_snapshot: 'GPT Image 2',
  params_json: '{}',
  status: 'done',
  error_message: null,
  request_url: null,
  request_headers_json: null,
  request_body_json: null,
  input_assets: [],
  output_assets: [],
  created_at: 100,
  finished_at: 120,
  elapsed_ms: 20,
};

const sampleWorkspace: ImageWorkspace = {
  channels: [],
  jobs: [],
};

function createTestDependencies(overrides?: Partial<{
  createImageJob: (input: CreateImageJobInput) => Promise<ImageJob>;
  deleteImageJob: (input: DeleteImageJobInput) => Promise<void>;
  deleteImageChannel: (id: string) => Promise<void>;
  getImageWorkspace: () => Promise<ImageWorkspace>;
  listImageJobs: (limit?: number) => Promise<ImageJob[]>;
  reorderImageChannels: (orderedIds: string[]) => Promise<ImageChannel[]>;
  updateImageChannel: (input: UpsertImageChannelInput) => Promise<ImageChannel>;
}>) {
  return {
    createImageJob: async (_input: CreateImageJobInput) => sampleJob,
    deleteImageJob: async (_input: DeleteImageJobInput) => {},
    deleteImageChannel: async (_id: string) => {},
    getImageWorkspace: async () => sampleWorkspace,
    listImageJobs: async (_limit = 50) => [sampleJob],
    reorderImageChannels: async (_orderedIds: string[]) => [],
    updateImageChannel: async (_input: UpsertImageChannelInput) => {
      throw new Error('not implemented');
    },
    ...overrides,
  };
}

test('submitJob keeps successful job in store when follow-up refresh fails', async () => {
  const imageStore = createImageStore(createTestDependencies({
    listImageJobs: async () => {
      throw new Error('temporary list failure');
    },
  }));

  const originalConsoleWarn = console.warn;
  console.warn = () => {};

  try {
    const result = await imageStore.getState().submitJob(sampleJobInput);
    const state = imageStore.getState();

    assert.equal(result.id, sampleJob.id);
    assert.equal(state.jobs.length, 1);
    assert.equal(state.jobs[0]?.id, sampleJob.id);
    assert.equal(state.lastJobId, sampleJob.id);
    assert.equal(state.submitting, false);
  } finally {
    console.warn = originalConsoleWarn;
  }
});

test('submitJob prefers refreshed jobs list when background refresh succeeds', async () => {
  const refreshedJob: ImageJob = {
    ...sampleJob,
    status: 'error',
    error_message: 'refreshed status',
    created_at: 200,
  };
  const imageStore = createImageStore(createTestDependencies({
    listImageJobs: async () => [refreshedJob],
  }));

  const result = await imageStore.getState().submitJob(sampleJobInput);
  const state = imageStore.getState();

  assert.equal(result.id, sampleJob.id);
  assert.equal(state.jobs.length, 1);
  assert.equal(state.jobs[0]?.id, refreshedJob.id);
  assert.equal(state.jobs[0]?.status, 'error');
  assert.equal(state.jobs[0]?.error_message, 'refreshed status');
  assert.equal(state.lastJobId, sampleJob.id);
});
