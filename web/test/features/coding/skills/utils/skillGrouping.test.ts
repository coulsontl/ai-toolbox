import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildSkillGroups,
  filterSkillsBySearch,
  getSkillGroupOptions,
  normalizeSkillMetadataText,
} from '../../../../../features/coding/skills/utils/skillGrouping.ts';
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
    user_note: null,
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

test('getSkillGroupOptions returns sorted non-empty custom groups', () => {
  const skills = [
    makeSkill({ id: 'a', user_group: 'Reverse' }),
    makeSkill({ id: 'b', user_group: 'Frontend' }),
    makeSkill({ id: 'c', user_group: ' Reverse ' }),
    makeSkill({ id: 'd', user_group: '' }),
  ];

  assert.deepEqual(getSkillGroupOptions(skills), ['Frontend', 'Reverse']);
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
    makeSkill({ id: 'a', user_group: 'Reverse' }),
    makeSkill({ id: 'b', user_group: null }),
    makeSkill({ id: 'c', user_group: 'Reverse' }),
  ];

  const groups = buildSkillGroups(skills, 'custom', labels, () => null);

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
