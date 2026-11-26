// Skills Feature
// Entry point for the skills management feature

// Pages
export { default as SkillsPage } from './pages/SkillsPage';

// Components
export { SkillsButton } from './components/SkillsButton';
export { SkillsModal } from './components/SkillsModal';
export { SkillCard } from './components/SkillCard';
export { SkillsList } from './components/SkillsList';
export { ToolBadge } from './components/ToolBadge';

// Modals
export { AddSkillModal } from './components/modals/AddSkillModal';
export { GitPickModal } from './components/modals/GitPickModal';
export { DeleteConfirmModal } from './components/modals/DeleteConfirmModal';
export { ImportModal } from './components/modals/ImportModal';
export { NewToolsModal } from './components/modals/NewToolsModal';
export { SkillsSettingsModal } from './components/modals/SkillsSettingsModal';

// Hooks
export { useSkills } from './hooks/useSkills';
export { useSkillActions } from './hooks/useSkillActions';
export { useToolStatus } from './hooks/useToolStatus';

// Utils
export {
  isSkillExistsError,
  extractSkillName,
  parseTargetExistsError,
  showGitError,
  confirmTargetOverwrite,
} from './utils/errorHandlers';
export { syncSkillToTools } from './utils/syncHelpers';

// Store
export { useSkillsStore } from './stores/skillsStore';

// Types
export type {
  ManagedSkill,
  SkillTarget,
  ToolInfo,
  ToolStatus,
  GitSkillCandidate,
  OnboardingPlan,
  OnboardingGroup,
  OnboardingVariant,
} from './types';

// API (for direct access if needed)
export * as skillsApi from './services/skillsApi';
