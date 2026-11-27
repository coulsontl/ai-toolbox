import type { ManagedSkill } from '../types';

/**
 * Built-in Skills ship with the app: their content is versioned by the binary,
 * so editing, deleting, disabling, and bulk metadata operations are disabled.
 * Tool sync remains fully user-controlled.
 */
export function isBuiltinSkill(skill: Pick<ManagedSkill, 'source_type'>): boolean {
  return skill.source_type === 'builtin';
}
