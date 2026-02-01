import { create } from 'zustand';
import type {
  ManagedSkill,
  ToolStatus,
  ToolOption,
  OnboardingPlan,
} from '../types';
import * as api from '../services/skillsApi';

interface SkillsState {
  // Data
  skills: ManagedSkill[];
  toolStatus: ToolStatus | null;
  onboardingPlan: OnboardingPlan | null;
  centralRepoPath: string;

  // UI state
  loading: boolean;
  error: string | null;

  // Modal state
  isModalOpen: boolean;
  isAddModalOpen: boolean;
  isImportModalOpen: boolean;
  isSettingsModalOpen: boolean;
  isNewToolsModalOpen: boolean;

  // Actions
  setModalOpen: (open: boolean) => void;
  setAddModalOpen: (open: boolean) => void;
  setImportModalOpen: (open: boolean) => void;
  setSettingsModalOpen: (open: boolean) => void;
  setNewToolsModalOpen: (open: boolean) => void;

  // Data actions
  loadToolStatus: () => Promise<void>;
  loadSkills: () => Promise<void>;
  loadOnboardingPlan: () => Promise<void>;
  loadCentralRepoPath: () => Promise<void>;
  refresh: () => Promise<void>;
  setSkills: (skills: ManagedSkill[]) => void;

  // Computed
  getInstalledTools: () => ToolOption[];
  getAllTools: () => ToolOption[];
}

export const useSkillsStore = create<SkillsState>()((set, get) => ({
  // Data
  skills: [],
  toolStatus: null,
  onboardingPlan: null,
  centralRepoPath: '',

  // UI state
  loading: false,
  error: null,

  // Modal state
  isModalOpen: false,
  isAddModalOpen: false,
  isImportModalOpen: false,
  isSettingsModalOpen: false,
  isNewToolsModalOpen: false,

  // Actions
  setModalOpen: (open) => set({ isModalOpen: open }),
  setAddModalOpen: (open) => set({ isAddModalOpen: open }),
  setImportModalOpen: (open) => set({ isImportModalOpen: open }),
  setSettingsModalOpen: (open) => set({ isSettingsModalOpen: open }),
  setNewToolsModalOpen: (open) => set({ isNewToolsModalOpen: open }),

  // Data actions
  loadToolStatus: async () => {
    try {
      const status = await api.getToolStatus();
      set({ toolStatus: status });
    } catch (error) {
      console.error('Failed to load tool status:', error);
      set({ error: String(error) });
    }
  },

  loadSkills: async () => {
    set({ loading: true, error: null });
    try {
      const skills = await api.getManagedSkills();
      set({ skills, loading: false });
    } catch (error) {
      console.error('Failed to load skills:', error);
      set({ error: String(error), loading: false });
    }
  },

  loadOnboardingPlan: async () => {
    try {
      const plan = await api.getOnboardingPlan();
      set({ onboardingPlan: plan });
    } catch (error) {
      console.error('Failed to load onboarding plan:', error);
    }
  },

  loadCentralRepoPath: async () => {
    try {
      const path = await api.getCentralRepoPath();
      set({ centralRepoPath: path });
    } catch (error) {
      console.error('Failed to load central repo path:', error);
    }
  },

  refresh: async () => {
    // Note: loadOnboardingPlan is NOT called here to avoid automatic scanning.
    // It should only be triggered manually from the ImportModal.
    const { loadToolStatus, loadSkills, loadCentralRepoPath } = get();
    await Promise.all([
      loadToolStatus(),
      loadSkills(),
      loadCentralRepoPath(),
    ]);
  },

  setSkills: (skills) => set({ skills }),

  // Computed
  getInstalledTools: () => {
    const { toolStatus } = get();
    if (!toolStatus) return [];
    return toolStatus.tools
      .filter((t) => t.installed)
      .map((t) => ({
        id: t.key,
        label: t.label,
        installed: t.installed,
      }));
  },

  getAllTools: () => {
    const { toolStatus } = get();
    if (!toolStatus) return [];
    return toolStatus.tools.map((t) => ({
      id: t.key,
      label: t.label,
      installed: t.installed,
    }));
  },
}));
