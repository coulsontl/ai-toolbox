/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  extractOpenCodeOtherConfigFields,
  mergeOpenCodeOtherConfigFields,
} from '../../../../../features/coding/opencode/utils/openCodeOtherConfig.ts';

test('extractOpenCodeOtherConfigFields keeps disabled_providers visible in other config', () => {
  const result = extractOpenCodeOtherConfigFields({
    $schema: 'https://opencode.ai/config.json',
    provider: {
      openai: {
        npm: '@ai-sdk/openai',
        name: 'OpenAI',
        models: {},
      },
    },
    disabled_providers: ['opencode', 'opencode-go'],
    model: 'openai/gpt-5.5',
    small_model: 'openai/gpt-5.4-mini',
    default_agent: 'build',
    agent: {
      explore: {
        model: 'openai/gpt-5.4-mini',
      },
    },
    plugin: ['opencode-ai'],
    mcp: {
      demo: {
        type: 'local',
        command: ['demo'],
      },
    },
    permission: {
      external_directory: {
        '*': 'allow',
      },
    },
  });

  assert.deepEqual(result, {
    disabled_providers: ['opencode', 'opencode-go'],
    permission: {
      external_directory: {
        '*': 'allow',
      },
    },
  });
});

test('extractOpenCodeOtherConfigFields keeps mcp hidden because MCP page owns it', () => {
  const result = extractOpenCodeOtherConfigFields({
    provider: {},
    mcp: {
      demo: {
        type: 'local',
        command: ['demo'],
      },
    },
    permission: true,
  });

  assert.deepEqual(result, {
    permission: true,
  });
});

test('extractOpenCodeOtherConfigFields hides agent fields because Agent settings owns them', () => {
  const result = extractOpenCodeOtherConfigFields({
    provider: {},
    default_agent: 'build',
    agent: {
      explore: {
        model: 'openai/gpt-5.4-mini',
      },
    },
    permission: true,
  });

  assert.deepEqual(result, {
    permission: true,
  });
});

test('mergeOpenCodeOtherConfigFields preserves disabled_providers from other config editor', () => {
  const result = mergeOpenCodeOtherConfigFields(
    {
      provider: {
        openai: {
          npm: '@ai-sdk/openai',
          name: 'OpenAI',
          models: {},
        },
      },
      disabled_providers: ['old-provider'],
      model: 'openai/gpt-5.5',
      default_agent: 'build',
      agent: {
        explore: {
          model: 'openai/gpt-5.4-mini',
          permission: { edit: 'deny' },
        },
      },
    },
    {
      disabled_providers: ['opencode', 'opencode-go'],
      permission: {
        external_directory: {
          '*': 'allow',
        },
      },
    },
  );

  assert.deepEqual(result, {
    $schema: undefined,
    provider: {
      openai: {
        npm: '@ai-sdk/openai',
        name: 'OpenAI',
        models: {},
      },
    },
    model: 'openai/gpt-5.5',
    small_model: undefined,
    default_agent: 'build',
    agent: {
      explore: {
        model: 'openai/gpt-5.4-mini',
        permission: { edit: 'deny' },
      },
    },
    plugin: undefined,
    mcp: undefined,
    disabled_providers: ['opencode', 'opencode-go'],
    permission: {
      external_directory: {
        '*': 'allow',
      },
    },
  });
});

test('mergeOpenCodeOtherConfigFields preserves mcp while saving other config fields', () => {
  const result = mergeOpenCodeOtherConfigFields(
    {
      provider: {},
      mcp: {
        demo: {
          type: 'local',
          command: ['demo'],
        },
      },
    },
    {
      permission: true,
    },
  );

  assert.deepEqual(result, {
    $schema: undefined,
    provider: {},
    model: undefined,
    small_model: undefined,
    default_agent: undefined,
    agent: undefined,
    plugin: undefined,
    mcp: {
      demo: {
        type: 'local',
        command: ['demo'],
      },
    },
    permission: true,
  });
});

test('mergeOpenCodeOtherConfigFields clears disabled_providers when removed from other config editor', () => {
  const result = mergeOpenCodeOtherConfigFields(
    {
      provider: {},
      disabled_providers: ['opencode'],
      permission: true,
    },
    {
      permission: {
        external_directory: {
          '*': 'allow',
        },
      },
    },
  );

  assert.deepEqual(result, {
    $schema: undefined,
    provider: {},
    model: undefined,
    small_model: undefined,
    default_agent: undefined,
    agent: undefined,
    plugin: undefined,
    mcp: undefined,
    permission: {
      external_directory: {
        '*': 'allow',
      },
    },
  });
});
