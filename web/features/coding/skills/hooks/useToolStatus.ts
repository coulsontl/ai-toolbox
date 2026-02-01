import React, { useRef } from 'react';
import { useSkillsStore } from '../stores/skillsStore';

export function useToolStatus() {
  const { toolStatus, loadToolStatus } = useSkillsStore();
  const hasLoadedRef = useRef(false);

  React.useEffect(() => {
    // Prevent duplicate loading on re-renders and StrictMode double-mount
    if (hasLoadedRef.current) return;
    hasLoadedRef.current = true;

    loadToolStatus();
  }, [loadToolStatus]);

  return {
    toolStatus,
    installedTools: toolStatus?.installed || [],
    newlyInstalledTools: toolStatus?.newly_installed || [],
    allTools: toolStatus?.tools || [],
  };
}
