import type { ManagedSkill, SkillGroup, SkillGroupRecord } from '../types';

export type SkillGroupingMode = 'custom' | 'source';

export const CUSTOM_UNGROUPED_GROUP_KEY = 'custom:__ungrouped__';

export interface SkillGroupLabels {
  groupLocal: string;
  groupImport: string;
  groupUngrouped: string;
}

export type GithubInfoResolver = (
  url: string | null | undefined,
) => { label: string; href: string } | null;

export function normalizeSkillMetadataText(value: string | null | undefined): string | null {
  const trimmed = value?.trim() ?? '';
  return trimmed ? trimmed : null;
}

export function getSkillGroupOptions(groups: SkillGroupRecord[]): Array<{ id: string; name: string }> {
  return [...groups]
    .sort((left, right) => left.sort_index - right.sort_index || left.name.localeCompare(right.name))
    .map((group) => ({ id: group.id, name: group.name }));
}

export function findSkillGroupOptionByName(
  groups: Array<{ id: string; name: string }>,
  name: string | null | undefined,
): { id: string; name: string } | undefined {
  const normalizedName = normalizeSkillMetadataText(name)?.toLowerCase();
  if (!normalizedName) {
    return undefined;
  }
  return groups.find((group) => group.name.trim().toLowerCase() === normalizedName);
}

export function filterSkillsBySearch(skills: ManagedSkill[], searchText: string): ManagedSkill[] {
  const keyword = searchText.trim().toLowerCase();
  if (!keyword) {
    return skills;
  }

  return skills.filter((skill) => {
    const searchableValues = [
      skill.name,
      skill.source_ref,
      skill.description,
      skill.user_group,
      skill.user_note,
    ];

    return searchableValues.some((value) => value?.toLowerCase().includes(keyword));
  });
}

export function buildSkillGroups(
  skills: ManagedSkill[],
  mode: SkillGroupingMode,
  labels: SkillGroupLabels,
  getGithubInfo: GithubInfoResolver,
  registryGroups: SkillGroupRecord[] = [],
): SkillGroup[] {
  const groupMap = new Map<string, SkillGroup>();

  if (mode === 'custom') {
    for (const group of registryGroups) {
      groupMap.set(`custom:${group.id}`, {
        key: `custom:${group.id}`,
        id: group.id,
        label: group.name,
        note: group.note,
        sort_index: group.sort_index,
        sourceType: 'custom',
        skills: [],
      });
    }
  }

  for (const skill of skills) {
    const group = mode === 'custom'
      ? buildCustomGroup(skill, labels, registryGroups)
      : buildSourceGroup(skill, labels, getGithubInfo);

    const existing = groupMap.get(group.key);
    if (existing) {
      existing.skills.push(skill);
    } else {
      groupMap.set(group.key, { ...group, skills: [skill] });
    }
  }

  return Array.from(groupMap.values()).sort((left, right) => {
    if (mode !== 'custom') return 0;
    if (left.key === CUSTOM_UNGROUPED_GROUP_KEY) return 1;
    if (right.key === CUSTOM_UNGROUPED_GROUP_KEY) return -1;
    return (left.sort_index ?? 0) - (right.sort_index ?? 0) || left.label.localeCompare(right.label);
  });
}

function buildCustomGroup(
  skill: ManagedSkill,
  labels: SkillGroupLabels,
  registryGroups: SkillGroupRecord[],
): Omit<SkillGroup, 'skills'> {
  const registryGroup = skill.group_id
    ? registryGroups.find((group) => group.id === skill.group_id)
    : undefined;
  if (registryGroup) {
    return {
      key: `custom:${registryGroup.id}`,
      id: registryGroup.id,
      label: registryGroup.name,
      note: registryGroup.note,
      sort_index: registryGroup.sort_index,
      sourceType: 'custom',
    };
  }

  if (!normalizeSkillMetadataText(skill.user_group)) {
    return {
      key: CUSTOM_UNGROUPED_GROUP_KEY,
      id: null,
      label: labels.groupUngrouped,
      sourceType: 'custom',
    };
  }

  return {
    key: `custom:legacy:${skill.user_group}`,
    id: null,
    label: skill.user_group ?? labels.groupUngrouped,
    sourceType: 'custom',
  };
}

function buildSourceGroup(
  skill: ManagedSkill,
  labels: SkillGroupLabels,
  getGithubInfo: GithubInfoResolver,
): Omit<SkillGroup, 'skills'> {
  if (skill.source_type === 'git' && skill.source_ref) {
    const github = getGithubInfo(skill.source_ref);
    if (github) {
      return {
        key: `git:${github.href}`,
        id: null,
        label: github.label,
        sourceType: 'git',
      };
    }

    const baseUrl = skill.source_ref.replace(/\/tree\/.*$/, '');
    return {
      key: `git:${baseUrl}`,
      id: null,
      label: baseUrl,
      sourceType: 'git',
    };
  }

  if (skill.source_type === 'local') {
    const path = skill.source_ref || '';
    const parts = path.split(/[\/\\]/).filter(Boolean);
    const parentPath = parts.slice(0, -1).join('/');
    return {
      key: `local:${parentPath || path}`,
      id: null,
      label: parts[parts.length - 2] || parts[parts.length - 1] || labels.groupLocal,
      sourceType: 'local',
    };
  }

  return {
    key: 'import',
    id: null,
    label: labels.groupImport,
    sourceType: 'import',
  };
}

export function getSkillToolIds(skill: ManagedSkill): string[] {
  return [...new Set(skill.targets.map((target) => target.tool))];
}

export function isSkillUngroupedCustomGroup(group: SkillGroup): boolean {
  return group.key === CUSTOM_UNGROUPED_GROUP_KEY;
}

export function getSkillGroupToolIds(group: SkillGroup): string[] {
  const toolIds = new Set<string>();
  for (const skill of getManagementEnabledSkills(group)) {
    for (const toolId of getSkillToolIds(skill)) {
      toolIds.add(toolId);
    }
  }
  return [...toolIds];
}

export function isSkillGroupToolsAligned(group: SkillGroup): boolean {
  const enabledSkills = getManagementEnabledSkills(group);
  if (enabledSkills.length <= 1) {
    return true;
  }

  const [firstSkill, ...restSkills] = enabledSkills;
  const firstToolKey = createToolSetKey(getSkillToolIds(firstSkill));
  return restSkills.every((skill) => createToolSetKey(getSkillToolIds(skill)) === firstToolKey);
}

export function getSkillIdsMissingTool(group: SkillGroup, toolId: string): string[] {
  return getManagementEnabledSkills(group)
    .filter((skill) => !skill.targets.some((target) => target.tool === toolId))
    .map((skill) => skill.id);
}

export function getSkillIdsWithTool(group: SkillGroup, toolId: string): string[] {
  return getManagementEnabledSkills(group)
    .filter((skill) => skill.targets.some((target) => target.tool === toolId))
    .map((skill) => skill.id);
}

function getManagementEnabledSkills(group: SkillGroup): ManagedSkill[] {
  return group.skills.filter((skill) => skill.management_enabled);
}

function createToolSetKey(toolIds: string[]): string {
  return [...new Set(toolIds)].sort().join('\u0000');
}
