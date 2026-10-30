import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';

import { translateDefaultMappingName, translateSyncMessage } from '../../../../features/settings/utils/syncMessageTranslator.ts';

const stubT = ((key: string, vars?: Record<string, unknown>) => {
  if (!vars || Object.keys(vars).length === 0) {
    return key;
  }
  return `${key}?${JSON.stringify(vars)}`;
}) as unknown as TFunction;

test('copied skill targets use the same warning translations as legacy links', () => {
  for (const mode of ['wsl', 'ssh'] as const) {
    for (const warning of [
      "技能 'demo' 在工具 'Antigravity CLI' 的路径 '/tmp/skills/demo' 不是 AI Toolbox 管理的链接，已保留原样",
      "技能 'demo' 在工具 'Antigravity CLI' 的链接维护失败：Permission denied",
    ]) {
      assert.equal(
        translateSyncMessage(warning.replace('链接', '同步目标'), mode, stubT),
        translateSyncMessage(warning, mode, stubT),
      );
    }
  }
});

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

test('skills warnings preserve quotes and semicolons inside skill names and paths', () => {
  const skill = "reviewer's skill; notes";
  const path = `/home/test/.claude/skills/${skill}`;
  for (const mode of ['wsl', 'ssh'] as const) {
    assert.equal(
      translateSyncMessage(
        `技能 '${skill}' 在工具 'Claude Code' 的路径 '${path}' 不是 AI Toolbox 管理的链接，已保留原样`,
        mode,
        stubT,
      ),
      'settings.syncMessages.skillsForeignPathKept?' +
        JSON.stringify({ skill, tool: 'Claude Code', path }),
    );
  }
});

test('skills warning translations retain multiline command diagnostics', () => {
  const detail = "Permission denied\nUnable to update target; retry later";
  const cases = [
    {
      message: `技能 'demo' 在工具 'Claude Code' 的链接维护失败：${detail}`,
      key: 'skillsLinkMaintenanceFailed',
      vars: { skill: 'demo', tool: 'Claude Code', detail },
    },
    {
      message: `技能 'demo' 的远端目录清理失败：${detail}`,
      key: 'skillsRemoteDirCleanupFailed',
      vars: { skill: 'demo', detail },
    },
    {
      message: `技能 'demo' 的同步哈希写入失败：${detail}`,
      key: 'skillsHashWriteFailed',
      vars: { skill: 'demo', detail },
    },
  ];
  for (const mode of ['wsl', 'ssh'] as const) {
    for (const { message, key, vars } of cases) {
      assert.equal(
        translateSyncMessage(message, mode, stubT),
        `settings.syncMessages.${key}?${JSON.stringify(vars)}`,
      );
    }
  }
});

test('Antigravity CLI default mapping names translate to their own i18n keys', () => {
  // The three Antigravity mappings split runtime root (env/settings) from the
  // global rules file, and each has a distinct label key. A regression here would
  // silently fall back to the raw Chinese Rust default name in the sync modal.
  assert.equal(
    translateDefaultMappingName('Antigravity CLI 环境变量', stubT),
    'settings.wsl.defaultMappings.antigravityEnv',
  );
  assert.equal(
    translateDefaultMappingName('Antigravity CLI 设置', stubT),
    'settings.wsl.defaultMappings.antigravitySettings',
  );
  assert.equal(
    translateDefaultMappingName('Antigravity CLI 全局提示词', stubT),
    'settings.wsl.defaultMappings.antigravityPrompt',
  );
});

test('WSL claude.json parse failure names the distro, path, and that the file was untouched', () => {
  const message =
    "Onboarding sync: Failed to parse WSL claude.json (distro 'Ubuntu-24.04', ~/.claude.json): trailing characters at line 308 column 2 (char 8799). The file was left untouched.";

  assert.equal(
    translateSyncMessage(message, 'wsl', stubT),
    'settings.syncMessages.onboardingSyncParseFailed?' +
      JSON.stringify({
        distro: 'Ubuntu-24.04',
        path: '~/.claude.json',
        detail: 'trailing characters at line 308 column 2 (char 8799)',
      }),
  );
});

test('WSL claude.json non-object failure keeps distro and path', () => {
  const message =
    "Onboarding sync: WSL claude.json is not a JSON object (distro 'Debian', ~/.claude.json). The file was left untouched.";

  assert.equal(
    translateSyncMessage(message, 'wsl', stubT),
    'settings.syncMessages.onboardingSyncNotObject?' +
      JSON.stringify({ distro: 'Debian', path: '~/.claude.json' }),
  );
});

test('the MCP chain wrapping is translated too, not left as raw English', () => {
  // The MCP sync wraps the same diagnostic in `Claude Code: ` and then in the
  // older "skipped" sentence. Without accepting that prefix the Chinese UI
  // embeds the whole English message.
  const wrapped =
    "WSL ~/.claude.json 同步已跳过：文件解析失败，请检查该文件格式是否正确。(Claude Code: Failed to parse WSL claude.json (distro 'Ubuntu', ~/.claude.json): trailing characters at line 308 column 2. The file was left untouched.)";

  const translated = translateSyncMessage(wrapped, 'wsl', stubT);

  assert.ok(
    translated.startsWith('settings.syncMessages.wslClaudeJsonSyncSkipped?'),
    translated,
  );
  assert.ok(
    translated.includes('settings.syncMessages.onboardingSyncParseFailed'),
    translated,
  );
  assert.ok(!translated.includes('The file was left untouched.'), translated);
});

test('claude.json diagnostics survive a "; " inside the json5 detail', () => {
  // `translateSyncMessage` splits joined errors on "; " before pattern matching.
  // The distro, path and json5 detail are free-form, so these messages must be
  // parsed as a whole first or they fragment into untranslatable pieces.
  const message =
    "Onboarding sync: Failed to parse WSL claude.json (distro 'Ubuntu', ~/.claude.json): expected comma; found 2 at line 308. The file was left untouched.";

  assert.equal(
    translateSyncMessage(message, 'wsl', stubT),
    'settings.syncMessages.onboardingSyncParseFailed?' +
      JSON.stringify({
        distro: 'Ubuntu',
        path: '~/.claude.json',
        detail: 'expected comma; found 2 at line 308',
      }),
  );
});

test('unsupported remote shell preflight is translated with the detected shell', () => {
  assert.equal(
    translateSyncMessage(
      '远端 Shell 不是 POSIX（Windows cmd.exe），当前版本仅支持 Linux/macOS 远端，已停止同步',
      'ssh',
      stubT,
    ),
    'settings.syncMessages.unsupportedRemoteShell?' +
      JSON.stringify({ shell: 'Windows cmd.exe' }),
  );
});

test('ZCode default mapping names translate to their own i18n keys', () => {
  // A regression here silently falls back to the raw Chinese Rust default name
  // in the sync modal, which is how the missing keys would surface.
  assert.equal(
    translateDefaultMappingName('ZCode 供应商配置', stubT),
    'settings.wsl.defaultMappings.zcodeProviderConfig',
  );
  assert.equal(
    translateDefaultMappingName('ZCode 全局提示词', stubT),
    'settings.wsl.defaultMappings.zcodePrompt',
  );
  assert.equal(
    translateDefaultMappingName('ZCode CLI 配置', stubT),
    'settings.wsl.defaultMappings.zcodeCliConfig',
  );
  assert.equal(
    translateDefaultMappingName('ZCode Skills 目录', stubT),
    'settings.wsl.defaultMappings.zcodeSkills',
  );
});
