import assert from 'node:assert/strict';
import test from 'node:test';

import {
  extractRepoNameFromGitUrl,
  suggestSkillAlias,
  validateSkillAlias,
} from '../../../../../features/coding/skills/utils/skillAlias.ts';

test('validateSkillAlias rejects empty, overly long, dot-bounded, invalid-char, and duplicate names', () => {
  const existing = new Set(['my-skill', 'frontend-design']);

  assert.equal(validateSkillAlias('', existing), 'empty');
  assert.equal(validateSkillAlias('   ', existing), 'empty');
  assert.equal(validateSkillAlias('a'.repeat(129), existing), 'tooLong');
  assert.equal(validateSkillAlias('a'.repeat(128), existing), null);

  assert.equal(validateSkillAlias('.', existing), 'startsWithDot');
  assert.equal(validateSkillAlias('..', existing), 'startsWithDot');
  assert.equal(validateSkillAlias('.hidden', existing), 'startsWithDot');
  assert.equal(validateSkillAlias('trailing.', existing), 'endsWithDot');

  for (const badChar of ['/', '\\', '<', '>', ':', '"', '|', '?', '*', '\u0000', '\u001f', '\u007f']) {
    assert.equal(validateSkillAlias(`skill${badChar}name`, existing), 'invalidChars');
  }

  assert.equal(validateSkillAlias('MY-SKILL', existing), 'exists');
  assert.equal(validateSkillAlias('  Frontend-Design  ', existing), 'exists');
  assert.equal(validateSkillAlias('  my-skill-v2  ', existing), null);
});

test('extractRepoNameFromGitUrl handles GitHub, tree subpaths, shorthands, SCP/SSH, and generic URLs', () => {
  assert.equal(extractRepoNameFromGitUrl('https://github.com/owner/repo'), 'repo');
  assert.equal(extractRepoNameFromGitUrl('https://github.com/owner/repo.git'), 'repo');
  assert.equal(extractRepoNameFromGitUrl('https://github.com/owner/repo/tree/main/skills/foo'), 'repo');
  assert.equal(extractRepoNameFromGitUrl('github.com/owner/repo'), 'repo');
  assert.equal(extractRepoNameFromGitUrl('owner/repo'), 'repo');
  assert.equal(extractRepoNameFromGitUrl('git@github.com:owner/my-repo.git'), 'my-repo');
  assert.equal(extractRepoNameFromGitUrl('ssh://git@gitlab.com:2222/group/team-skills.git'), 'team-skills');
  assert.equal(extractRepoNameFromGitUrl('https://example.com/custom-repo.git'), 'custom-repo');
  assert.equal(extractRepoNameFromGitUrl('https://github.com/owner/.repo:bad*.git'), 'repo-bad');
  assert.equal(extractRepoNameFromGitUrl(''), null);
  assert.equal(extractRepoNameFromGitUrl('   '), null);
});

test('suggestSkillAlias uses repoHint when present and falls back to numeric suffixes', () => {
  const existing = new Set(['brainstorming', 'brainstorming-superpowers', 'brainstorming-2']);

  assert.equal(
    suggestSkillAlias('brainstorming', existing, 'superpowers'),
    'brainstorming-superpowers-2',
  );
  assert.equal(
    suggestSkillAlias('brainstorming', new Set(['brainstorming']), 'superpowers'),
    'brainstorming-superpowers',
  );
  assert.equal(
    suggestSkillAlias('brainstorming', existing, null),
    'brainstorming-3',
  );
  assert.equal(
    suggestSkillAlias('  review  ', new Set(['review'])),
    'review-2',
  );
  assert.equal(
    suggestSkillAlias('.hidden:bad/name.', new Set(['hidden-bad-name'])),
    'hidden-bad-name-2',
  );
  const longSuggestion = suggestSkillAlias('a'.repeat(128), new Set(['a'.repeat(128)]), 'my-repo');
  assert.equal(validateSkillAlias(longSuggestion, new Set(['a'.repeat(128)])), null);
  assert.ok(longSuggestion.length <= 128);
});
