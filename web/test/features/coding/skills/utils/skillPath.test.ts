import assert from 'node:assert/strict';
import test from 'node:test';

import {
  getSkillFolderOpenCandidates,
  getSkillManifestPath,
  joinSkillPath,
  normalizeSkillPath,
} from '../../../../../features/coding/skills/utils/skillPath.ts';

test('normalizeSkillPath trims empty path values', () => {
  assert.equal(normalizeSkillPath('  /tmp/skill  '), '/tmp/skill');
  assert.equal(normalizeSkillPath('   '), '');
  assert.equal(normalizeSkillPath(null), '');
});

test('joinSkillPath uses slash for POSIX-style central paths', () => {
  assert.equal(joinSkillPath('/Users/ralph/skills/reverse', 'SKILL.md'), '/Users/ralph/skills/reverse/SKILL.md');
  assert.equal(joinSkillPath('/Users/ralph/skills/reverse/', '/SKILL.md'), '/Users/ralph/skills/reverse/SKILL.md');
});

test('joinSkillPath preserves backslash-only Windows and UNC paths', () => {
  assert.equal(joinSkillPath('C:\\Users\\ralph\\skills\\reverse', 'SKILL.md'), 'C:\\Users\\ralph\\skills\\reverse\\SKILL.md');
  assert.equal(
    joinSkillPath('\\\\wsl.localhost\\Ubuntu\\home\\ralph\\skills\\reverse\\', 'SKILL.md'),
    '\\\\wsl.localhost\\Ubuntu\\home\\ralph\\skills\\reverse\\SKILL.md',
  );
});

test('getSkillManifestPath returns null when central path is unavailable', () => {
  assert.equal(getSkillManifestPath(null), null);
  assert.equal(getSkillManifestPath('   '), null);
});

test('getSkillFolderOpenCandidates prefers local source and falls back to central path', () => {
  assert.deepEqual(
    getSkillFolderOpenCandidates({
      source_type: 'local',
      source_ref: ' /source/skill ',
      central_path: '/central/skill',
    }),
    ['/source/skill', '/central/skill'],
  );
});

test('getSkillFolderOpenCandidates de-duplicates source and central paths', () => {
  assert.deepEqual(
    getSkillFolderOpenCandidates({
      source_type: 'local',
      source_ref: '/central/skill',
      central_path: '/central/skill',
    }),
    ['/central/skill'],
  );
});

test('getSkillFolderOpenCandidates still exposes central path for non-local skills', () => {
  assert.deepEqual(
    getSkillFolderOpenCandidates({
      source_type: 'git',
      source_ref: 'https://github.com/acme/skills',
      central_path: '/central/git-skill',
    }),
    ['/central/git-skill'],
  );
});
