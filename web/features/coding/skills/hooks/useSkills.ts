import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { useSkillsStore } from '../stores/skillsStore';
import * as api from '../services/skillsApi';
import { normalizeGitUrlToHttps, parseGitRepo } from '../utils/gitUrl';
import type { ManagedSkill } from '../types';

export function useSkills() {
  const store = useSkillsStore();
  const { t } = useTranslation();

  // Initialize on mount
  React.useEffect(() => {
    if (store.isModalOpen) {
      store.refresh();
    }
  }, [store.isModalOpen]);

  // Listen for skills-changed events from the tray and the background
  // auto-discover pass. The auto pass stays silent (no toast): it only refreshes
  // the list so newly distributed Skills appear without the user asking.
  React.useEffect(() => {
    const unlisten = listen<string>('skills-changed', (event) => {
      if (event.payload === 'tray' || event.payload === 'auto') {
        store.loadSkills();
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [store.loadSkills]);

  // Format relative time
  const formatRelative = React.useCallback((ms: number | null | undefined) => {
    if (!ms) return '—';

    const now = Date.now();
    const diff = now - ms;
    const minutes = Math.floor(diff / 60000);
    const hours = Math.floor(diff / 3600000);
    const days = Math.floor(diff / 86400000);

    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${minutes}m ago`;
    if (hours < 24) return `${hours}h ago`;
    return `${days}d ago`;
  }, []);

  // Resolve a Git remote URL into a display label and HTTPS web URL.
  // Works for GitHub and any self-hosted / custom Git host (HTTPS, SSH, SCP).
  const getRepoInfo = React.useCallback((url: string | null | undefined) => {
    const parsed = parseGitRepo(url);
    if (!parsed) return null;

    return {
      label: `${parsed.owner}/${parsed.repo}`,
      href: normalizeGitUrlToHttps(url) as string,
    };
  }, []);

  // Get skill source label
  const getSkillSourceLabel = React.useCallback((skill: ManagedSkill) => {
    if (skill.source_type === 'git') {
      const info = getRepoInfo(skill.source_ref);
      return info ? info.label : skill.source_ref || 'Git';
    }
    if (skill.source_type === 'local') {
      // Return just the folder name
      const path = skill.source_ref || '';
      const parts = path.split(/[\/\\]/);
      return parts[parts.length - 1] || 'Local';
    }
    if (skill.source_type === 'central') {
      return t('skills.sourceCentral');
    }
    return skill.source_type;
  }, [getRepoInfo, t]);

  // Update skill
  const updateSkill = React.useCallback(
    async (skill: ManagedSkill) => {
      try {
        await api.updateManagedSkill(skill.id);
        await store.loadSkills();
      } catch (error) {
        console.error('Failed to update skill:', error);
        throw error;
      }
    },
    [store]
  );

  // Delete skill
  const deleteSkill = React.useCallback(
    async (skillId: string) => {
      try {
        await api.deleteManagedSkill(skillId);
        await store.loadSkills();
      } catch (error) {
        console.error('Failed to delete skill:', error);
        throw error;
      }
    },
    [store]
  );

  return {
    ...store,
    formatRelative,
    getRepoInfo,
    getSkillSourceLabel,
    updateSkill,
    deleteSkill,
  };
}
