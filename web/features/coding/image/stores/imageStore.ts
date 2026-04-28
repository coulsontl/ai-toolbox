import { create } from 'zustand';
import type {
  CreateImageJobInput,
  ImageChannel,
  ImageJob,
  UpsertImageChannelInput,
} from '../services/imageApi.ts';
import {
  createImageJob,
  deleteImageJob,
  deleteImageChannel,
  getImageWorkspace,
  listImageJobs,
  reorderImageChannels,
  updateImageChannel,
} from '../services/imageApi.ts';

export type ImageViewKey = 'workbench' | 'history' | 'more';

interface ImageState {
  channels: ImageChannel[];
  jobs: ImageJob[];
  loading: boolean;
  submitting: boolean;
  channelSaving: boolean;
  activeView: ImageViewKey;
  editingChannelId: string | null;
  lastJobId: string | null;
  loadWorkspace: () => Promise<void>;
  refreshJobs: () => Promise<void>;
  saveChannel: (input: UpsertImageChannelInput) => Promise<ImageChannel>;
  removeChannel: (channelId: string) => Promise<void>;
  removeJob: (jobId: string, deleteLocalAssets: boolean) => Promise<void>;
  reorderChannels: (orderedIds: string[]) => Promise<ImageChannel[]>;
  submitJob: (input: CreateImageJobInput) => Promise<ImageJob>;
  setActiveView: (view: ImageViewKey) => void;
  setEditingChannelId: (channelId: string | null) => void;
}

const upsertImageJob = (jobs: ImageJob[], job: ImageJob): ImageJob[] => {
  const nextJobs = jobs.some((currentJob) => currentJob.id === job.id)
    ? jobs.map((currentJob) => (currentJob.id === job.id ? job : currentJob))
    : [job, ...jobs];

  return [...nextJobs].sort((left, right) => right.created_at - left.created_at);
};

interface ImageStoreDependencies {
  createImageJob: typeof createImageJob;
  deleteImageJob: typeof deleteImageJob;
  deleteImageChannel: typeof deleteImageChannel;
  getImageWorkspace: typeof getImageWorkspace;
  listImageJobs: typeof listImageJobs;
  reorderImageChannels: typeof reorderImageChannels;
  updateImageChannel: typeof updateImageChannel;
}

const defaultImageStoreDependencies: ImageStoreDependencies = {
  createImageJob,
  deleteImageJob,
  deleteImageChannel,
  getImageWorkspace,
  listImageJobs,
  reorderImageChannels,
  updateImageChannel,
};

export const createImageStore = (
  dependencies: ImageStoreDependencies = defaultImageStoreDependencies
) => create<ImageState>()((set, get) => ({
  channels: [],
  jobs: [],
  loading: false,
  submitting: false,
  channelSaving: false,
  activeView: 'workbench',
  editingChannelId: null,
  lastJobId: null,

  loadWorkspace: async () => {
    set({ loading: true });
    try {
      const workspace = await dependencies.getImageWorkspace();
      set({
        channels: workspace.channels,
        jobs: workspace.jobs,
        lastJobId: workspace.jobs[0]?.id ?? null,
        editingChannelId:
          get().editingChannelId && workspace.channels.some((channel) => channel.id === get().editingChannelId)
            ? get().editingChannelId
          : workspace.channels[0]?.id ?? null,
      });
    } finally {
      set({ loading: false });
    }
  },

  refreshJobs: async () => {
    const jobs = await dependencies.listImageJobs(50);
    set({
      jobs,
      lastJobId: jobs[0]?.id ?? null,
    });
  },

  saveChannel: async (input) => {
    set({ channelSaving: true });
    try {
      const channel = await dependencies.updateImageChannel(input);
      set((currentState) => {
        const nextChannels = currentState.channels.some((item) => item.id === channel.id)
          ? currentState.channels.map((item) => (item.id === channel.id ? channel : item))
          : [...currentState.channels, channel];

        return {
          channels: nextChannels.sort((left, right) => left.sort_order - right.sort_order),
          editingChannelId: channel.id,
        };
      });
      return channel;
    } finally {
      set({ channelSaving: false });
    }
  },

  removeChannel: async (channelId) => {
    await dependencies.deleteImageChannel(channelId);
    set((currentState) => {
      const nextChannels = currentState.channels.filter((channel) => channel.id !== channelId);
      return {
        channels: nextChannels,
        editingChannelId:
          currentState.editingChannelId === channelId
            ? nextChannels[0]?.id ?? null
            : currentState.editingChannelId,
      };
    });
  },

  removeJob: async (jobId, deleteLocalAssets) => {
    await dependencies.deleteImageJob({ id: jobId, delete_local_assets: deleteLocalAssets });
    set((currentState) => {
      const nextJobs = currentState.jobs.filter((job) => job.id !== jobId);
      const nextLastJobId =
        currentState.lastJobId === jobId
          ? nextJobs[0]?.id ?? null
          : currentState.lastJobId;

      return {
        jobs: nextJobs,
        lastJobId: nextLastJobId,
      };
    });
  },

  reorderChannels: async (orderedIds) => {
    const channels = await dependencies.reorderImageChannels(orderedIds);
    set({ channels });
    return channels;
  },

  submitJob: async (input) => {
    set({ submitting: true });
    try {
      const job = await dependencies.createImageJob(input);
      set((currentState) => ({
        jobs: upsertImageJob(currentState.jobs, job),
        lastJobId: job.id,
      }));
      try {
        const jobs = await dependencies.listImageJobs(50);
        set((currentState) => ({
          jobs: jobs.length > 0 ? jobs : currentState.jobs,
          lastJobId: job.id,
        }));
      } catch (refreshError) {
        console.warn('Image jobs refresh failed after successful submit', refreshError);
      }
      return job;
    } finally {
      set({ submitting: false });
    }
  },

  setActiveView: (view) => set({ activeView: view }),
  setEditingChannelId: (channelId) => set({ editingChannelId: channelId }),
}));

export const useImageStore = createImageStore();
