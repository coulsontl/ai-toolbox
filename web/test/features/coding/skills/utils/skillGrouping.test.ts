import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildSkillGroups,
  CUSTOM_UNGROUPED_GROUP_KEY,
  findSkillGroupOptionByName,
  filterSkillsBySearch,
  getSkillGroupToolIds,
  getSkillGroupOptions,
  getSkillIdsMissingTool,
  getSkillIdsWithTool,
  isSkillGroupToolsAligned,
  isSkillUngroupedCustomGroup,
  normalizeSkillMetadataText,
} from '../../../../../features/coding/skills/utils/skillGrouping.ts';
import {
  GROUP_TOOL_BATCH_OPTIONS,
  shouldOverwriteExistingTarget,
} from '../../../../../features/coding/skills/utils/batchToolOptions.ts';
import type { ManagedSkill } from '../../../../../features/coding/skills/types/index.ts';

function makeSkill(overrides: Partial<ManagedSkill>): ManagedSkill {
  return {
    id: 'skill-1',
    name: 'default-skill',
    source_type: 'local',
    source_ref: null,
    central_path: 'D:/skills/default-skill',
    created_at: 1,
    updated_at: 1,
    last_sync_at: null,
    status: 'ok',
    sort_index: 0,
    user_group: null,
    group_id: null,
    user_note: null,
    management_enabled: true,
    disabled_previous_tools: [],
    description: null,
    content_hash: null,
    source_health: 'ok',
    source_error: null,
    enabled_tools: [],
    targets: [],
    ...overrides,
  };
}

const labels = {
  groupLocal: 'Local Skills',
  groupImport: 'Imported Skills',
  groupUngrouped: 'Ungrouped',
};

test('normalizeSkillMetadataText trims empty values to null', () => {
  assert.equal(normalizeSkillMetadataText('  Reverse  '), 'Reverse');
  assert.equal(normalizeSkillMetadataText('   '), null);
  assert.equal(normalizeSkillMetadataText(null), null);
});

test('getSkillGroupOptions returns sorted registry groups', () => {
  const groups = [
    { id: 'reverse', name: 'Reverse', note: null, sort_index: 2, created_at: 1, updated_at: 1 },
    { id: 'frontend', name: 'Frontend', note: null, sort_index: 1, created_at: 1, updated_at: 1 },
  ];

  assert.deepEqual(getSkillGroupOptions(groups), [
    { id: 'frontend', name: 'Frontend' },
    { id: 'reverse', name: 'Reverse' },
  ]);
});

test('findSkillGroupOptionByName matches trimmed names case-insensitively', () => {
  const groups = [
    { id: 'reverse', name: 'Reverse' },
    { id: 'frontend', name: 'Frontend' },
  ];

  assert.deepEqual(findSkillGroupOptionByName(groups, ' reverse '), groups[0]);
  assert.deepEqual(findSkillGroupOptionByName(groups, 'FRONTEND'), groups[1]);
  assert.equal(findSkillGroupOptionByName(groups, '   '), undefined);
});

test('filterSkillsBySearch matches custom group and note', () => {
  const skills = [
    makeSkill({ id: 'reverse', name: 'apk-helper', user_group: 'Reverse' }),
    makeSkill({ id: 'note', name: 'misc', user_note: 'Use with Frida scripts' }),
    makeSkill({ id: 'other', name: 'frontend-helper' }),
  ];

  assert.deepEqual(filterSkillsBySearch(skills, 'reverse').map((skill) => skill.id), ['reverse']);
  assert.deepEqual(filterSkillsBySearch(skills, 'frida').map((skill) => skill.id), ['note']);
});

test('buildSkillGroups groups by custom group and keeps ungrouped skills', () => {
  const skills = [
    makeSkill({ id: 'a', group_id: 'reverse', user_group: 'Reverse' }),
    makeSkill({ id: 'b', user_group: null }),
    makeSkill({ id: 'c', group_id: 'reverse', user_group: 'Reverse' }),
  ];

  const groups = buildSkillGroups(skills, 'custom', labels, () => null, [
    { id: 'reverse', name: 'Reverse', note: 'Reverse tools', sort_index: 0, created_at: 1, updated_at: 1 },
  ]);

  assert.equal(groups[1].key, CUSTOM_UNGROUPED_GROUP_KEY);
  assert.equal(isSkillUngroupedCustomGroup(groups[1]), true);
  assert.deepEqual(groups.map((group) => [group.label, group.skills.map((skill) => skill.id)]), [
    ['Reverse', ['a', 'c']],
    ['Ungrouped', ['b']],
  ]);
});

test('buildSkillGroups preserves source grouping behavior for git and local skills', () => {
  const skills = [
    makeSkill({
      id: 'git',
      source_type: 'git',
      source_ref: 'https://github.com/acme/skills/tree/main/reverse',
    }),
    makeSkill({
      id: 'local',
      source_type: 'local',
      source_ref: 'D:/repo/skills/frontend',
    }),
  ];

  const groups = buildSkillGroups(skills, 'source', labels, (url) => (
    url?.startsWith('https://github.com/acme/skills')
      ? { label: 'acme/skills', href: 'https://github.com/acme/skills' }
      : null
  ));

  assert.deepEqual(groups.map((group) => [group.key, group.label]), [
    ['git:https://github.com/acme/skills', 'acme/skills'],
    ['local:D:/repo/skills', 'skills'],
  ]);
});

test('skill group tool helpers use union and detect mixed tool sets', () => {
  const group = {
    key: 'custom:Dev',
    id: 'dev',
    label: 'Dev',
    sourceType: 'custom' as const,
    skills: [
      makeSkill({
        id: 'a',
        targets: [
          { tool: 'claude_code', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
          { tool: 'codex', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
        ],
      }),
      makeSkill({
        id: 'b',
        targets: [
          { tool: 'claude_code', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
        ],
      }),
    ],
  };

  assert.deepEqual(getSkillGroupToolIds(group).sort(), ['claude_code', 'codex']);
  assert.equal(isSkillGroupToolsAligned(group), false);
  assert.deepEqual(getSkillIdsMissingTool(group, 'codex'), ['b']);
  assert.deepEqual(getSkillIdsWithTool(group, 'claude_code'), ['a', 'b']);
});

test('skill group tool helpers treat equal sets in different order as aligned', () => {
  const group = {
    key: 'custom:Dev',
    id: 'dev',
    label: 'Dev',
    sourceType: 'custom' as const,
    skills: [
      makeSkill({
        id: 'a',
        targets: [
          { tool: 'codex', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
          { tool: 'claude_code', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
        ],
      }),
      makeSkill({
        id: 'b',
        targets: [
          { tool: 'claude_code', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
          { tool: 'codex', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
        ],
      }),
    ],
  };

  assert.equal(isSkillGroupToolsAligned(group), true);
});

test('skill group tool helpers ignore disabled skills for group tool mode', () => {
  const group = {
    key: 'custom:Dev',
    id: 'dev',
    label: 'Dev',
    sourceType: 'custom' as const,
    skills: [
      makeSkill({
        id: 'enabled-a',
        targets: [
          { tool: 'claude_code', mode: 'link', status: 'ok', target_path: '', synced_at: 1 },
        ],
      }),
      makeSkill({
        id: 'enabled-b',
        targets: [],
      }),
      makeSkill({
        id: 'disabled',
        management_enabled: false,
        targets: [],
      }),
    ],
  };

  assert.deepEqual(getSkillGroupToolIds(group), ['claude_code']);
  assert.equal(isSkillGroupToolsAligned(group), false);
  assert.deepEqual(getSkillIdsMissingTool(group, 'claude_code'), ['enabled-b']);
  assert.deepEqual(getSkillIdsWithTool(group, 'claude_code'), ['enabled-a']);
});

test('group tool mode writes opt into overwriting existing unmanaged targets', () => {
  assert.equal(shouldOverwriteExistingTarget(undefined), false);
  assert.equal(shouldOverwriteExistingTarget({ quiet: true }), false);
  assert.equal(shouldOverwriteExistingTarget(GROUP_TOOL_BATCH_OPTIONS), true);
  assert.deepEqual(GROUP_TOOL_BATCH_OPTIONS, {
    quiet: true,
    overwriteExisting: true,
  });
});
