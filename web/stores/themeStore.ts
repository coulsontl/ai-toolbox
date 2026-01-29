import { create } from 'zustand';
import { getSettings, updateSettings } from '@/services/settingsApi';

export type ThemeMode = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

interface ThemeState {
  mode: ThemeMode;
  resolvedTheme: ResolvedTheme;
  isInitialized: boolean;
  initTheme: () => Promise<void>;
  setMode: (mode: ThemeMode) => Promise<void>;
  updateResolvedTheme: (theme: ResolvedTheme) => void;
}

const getSystemTheme = (): ResolvedTheme => {
  if (typeof window !== 'undefined' && window.matchMedia) {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return 'light';
};

const resolveTheme = (mode: ThemeMode): ResolvedTheme => {
  if (mode === 'system') {
    return getSystemTheme();
  }
  return mode;
};

export const useThemeStore = create<ThemeState>()((set, get) => ({
  mode: 'system',
  resolvedTheme: 'light',
  isInitialized: false,

  initTheme: async () => {
    try {
      const settings = await getSettings();
      const mode = (settings.theme as ThemeMode) || 'system';
      const resolvedTheme = resolveTheme(mode);
      set({ mode, resolvedTheme, isInitialized: true });
    } catch (error) {
      console.error('Failed to init theme:', error);
      set({ mode: 'system', resolvedTheme: getSystemTheme(), isInitialized: true });
    }
  },

  setMode: async (mode: ThemeMode) => {
    const resolvedTheme = resolveTheme(mode);
    set({ mode, resolvedTheme });

    try {
      await updateSettings({ theme: mode });
    } catch (error) {
      console.error('Failed to save theme:', error);
    }
  },

  updateResolvedTheme: (theme: ResolvedTheme) => {
    const { mode } = get();
    if (mode === 'system') {
      set({ resolvedTheme: theme });
    }
  },
}));
