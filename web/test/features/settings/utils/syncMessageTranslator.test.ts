import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';

import { translateSyncMessage } from '../../../../features/settings/utils/syncMessageTranslator.ts';

const stubT = ((key: string, vars?: Record<string, unknown>) => {
  if (!vars || Object.keys(vars).length === 0) {
    return key;
  }
  return `${key}?${JSON.stringify(vars)}`;
}) as unknown as TFunction;

test('skills foreign path warning is translated with skill, tool, and path', () => {
  assert.equal(
    translateSyncMessage(
      "技能 'git-commit-batcher' 在工具 'Qoder' 的路径 '~/.qoder/skills/git-commit-batcher' 不是 AI Toolbox 管理的链接，已保留原样",
      'wsl',
      stubT,
    ),
    'settings.syncMessages.skillsForeignPathKept?' +
      JSON.stringify({
        skill: 'git-commit-batcher',
        tool: 'Qoder',
        path: '~/.qoder/skills/git-commit-batcher',
      }),
  );
});

test('skills link maintenance failure warning keeps the raw detail', () => {
  assert.equal(
    translateSyncMessage(
      "技能 'git-commit-batcher' 在工具 'Claude Code' 的链接维护失败：Permission denied",
      'ssh',
      stubT,
    ),
    'settings.syncMessages.skillsLinkMaintenanceFailed?' +
      JSON.stringify({
        skill: 'git-commit-batcher',
        tool: 'Claude Code',
        detail: 'Permission denied',
      }),
  );
});

test('skills source-missing warning is translated with skill and path', () => {
  assert.equal(
    translateSyncMessage(
      "技能 'demo' 的源目录不存在，已跳过同步：C:\\missing\\demo",
      'wsl',
      stubT,
    ),
    'settings.syncMessages.skillsSourceMissingSkipped?' +
      JSON.stringify({ skill: 'demo', detail: 'C:\\missing\\demo' }),
  );
});

test('skills remote dir cleanup warning is translated with skill and detail', () => {
  assert.equal(
    translateSyncMessage(
      "技能 'demo' 的远端目录清理失败：rm exited with code 1",
      'ssh',
      stubT,
    ),
    'settings.syncMessages.skillsRemoteDirCleanupFailed?' +
      JSON.stringify({ skill: 'demo', detail: 'rm exited with code 1' }),
  );
});

test('skills hash-write failure warning is translated with skill and detail', () => {
  assert.equal(
    translateSyncMessage(
      "技能 'demo' 的同步哈希写入失败：Permission denied",
      'wsl',
      stubT,
    ),
    'settings.syncMessages.skillsHashWriteFailed?' +
      JSON.stringify({ skill: 'demo', detail: 'Permission denied' }),
  );
});

test('unrecognized sync messages pass through unchanged', () => {
  const raw = 'Skills WSL sync: some internal english log line';
  assert.equal(translateSyncMessage(raw, 'wsl', stubT), raw);
});
