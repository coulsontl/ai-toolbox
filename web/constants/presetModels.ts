/**
 * Preset models configuration for different AI SDK types
 */

export interface PresetModel {
  id: string;
  name: string;
  contextLimit?: number;
  outputLimit?: number;
  modalities?: { input: string[]; output: string[] };
  variants?: Record<string, unknown>;
  options?: Record<string, unknown>;
}

/**
 * Preset models grouped by npm SDK type
 */
export const PRESET_MODELS: Record<string, PresetModel[]> = {
  '@ai-sdk/openai-compatible': [
    {
      id: 'MiniMax-M2.5',
      name: 'Minimax M2.5',
      contextLimit: 204800,
      outputLimit: 131072,
      modalities: { input: ['text'], output: ['text'] },
    },
    {
      id: 'glm-5',
      name: 'GLM 5',
      contextLimit: 204800,
      outputLimit: 131072,
      modalities: { input: ['text'], output: ['text'] },
    },
    {
      id: 'qwen3.5-plus',
      name: 'Qwen3.5 Plus',
      contextLimit: 1000000,
      outputLimit: 65536,
      modalities: { input: ['text', 'image', 'video'], output: ['text'] },
    },
    {
      id: 'kimi-k2.5',
      name: 'Kimi K2.5',
      contextLimit: 262144,
      outputLimit: 262144,
      modalities: { input: ['text', 'image', 'video'], output: ['text'] },
    },
    {
      id: 'MiniMax-M2.1',
      name: 'Minimax M2.1',
      contextLimit: 204800,
      outputLimit: 131072,
      modalities: { input: ['text'], output: ['text'] },
    },
    {
      id: 'glm-4.7',
      name: 'GLM 4.7',
      contextLimit: 204800,
      outputLimit: 131072,
      modalities: { input: ['text'], output: ['text'] },
    },
  ],
  '@ai-sdk/google': [
    {
      id: 'gemini-2.5-flash-lite',
      name: 'Gemini 2.5 Flash Lite',
      contextLimit: 1048576,
      outputLimit: 65536,
      modalities: { input: ['text', 'image', 'pdf', 'video', 'audio'], output: ['text'] },
      variants: {
        auto: {
          thinkingConfig: {
            includeThoughts: true,
            thinkingBudget: -1,
          },
        },
        'no-thinking': {
          thinkingConfig: {
            thinkingBudget: 0,
          },
        },
      },
    },
    {
      id: 'gemini-3-flash-preview',
      name: 'Gemini 3 Flash Preview',
      contextLimit: 1048576,
      outputLimit: 65536,
      modalities: { input: ['text', 'image', 'pdf', 'video', 'audio'], output: ['text'] },
      variants: {
        high: {
          thinkingConfig: {
            includeThoughts: true,
            thinkingLevel: 'high',
          },
        },
        low: {
          thinkingConfig: {
            includeThoughts: true,
            thinkingLevel: 'low',
          },
        },
        medium: {
          thinkingConfig: {
            includeThoughts: true,
            thinkingLevel: 'medium',
          },
        },
      },
    },
    {
      id: 'gemini-3-pro-preview',
      name: 'Gemini 3 Pro Preview',
      contextLimit: 1048576,
      outputLimit: 65536,
      modalities: { input: ['text', 'image', 'pdf', 'video', 'audio'], output: ['text'] },
      variants: {
        high: {
          thinkingConfig: {
            includeThoughts: true,
            thinkingLevel: 'high',
          },
        },
        low: {
          thinkingConfig: {
            includeThoughts: true,
            thinkingLevel: 'low',
          },
        },
      },
    },
  ],
  '@ai-sdk/openai': [
    {
      id: 'gpt-5',
      name: 'GPT-5',
      contextLimit: 400000,
      outputLimit: 128000,
      modalities: { input: ['text', 'image'], output: ['text'] },
      variants: {
        high: {
          reasoningEffort: 'high',
          reasoningSummary: 'auto',
          textVerbosity: 'high',
        },
        low: {
          reasoningEffort: 'low',
          reasoningSummary: 'auto',
          textVerbosity: 'low',
        },
        medium: {
          reasoningEffort: 'medium',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
      },
    },
    {
      id: 'gpt-5.1',
      name: 'GPT-5.1',
      contextLimit: 400000,
      outputLimit: 272000,
      modalities: { input: ['text', 'image'], output: ['text'] },
      variants: {
        high: {
          reasoningEffort: 'high',
          reasoningSummary: 'auto',
          textVerbosity: 'high',
        },
        low: {
          reasoningEffort: 'low',
          reasoningSummary: 'auto',
          textVerbosity: 'low',
        },
        medium: {
          reasoningEffort: 'medium',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
      },
    },
    {
      id: 'gpt-5.1-codex',
      name: 'GPT-5.1 Codex',
      contextLimit: 400000,
      outputLimit: 128000,
      modalities: { input: ['text', 'image'], output: ['text'] },
      options: {
        include: ['reasoning.encrypted_content'],
        store: false,
      },
      variants: {
        high: {
          reasoningEffort: 'high',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        low: {
          reasoningEffort: 'low',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        medium: {
          reasoningEffort: 'medium',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
      },
    },
    {
      id: 'gpt-5.1-codex-max',
      name: 'GPT-5.1 Codex Max',
      contextLimit: 400000,
      outputLimit: 128000,
      modalities: { input: ['text', 'image'], output: ['text'] },
      options: {
        include: ['reasoning.encrypted_content'],
        store: false,
      },
      variants: {
        high: {
          reasoningEffort: 'high',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        low: {
          reasoningEffort: 'low',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        medium: {
          reasoningEffort: 'medium',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        xhigh: {
          reasoningEffort: 'xhigh',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
      },
    },
    {
      id: 'gpt-5.2',
      name: 'GPT-5.2',
      contextLimit: 400000,
      outputLimit: 128000,
      modalities: { input: ['text', 'image'], output: ['text'] },
      variants: {
        high: {
          reasoningEffort: 'high',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        low: {
          reasoningEffort: 'low',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        medium: {
          reasoningEffort: 'medium',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        xhigh: {
          reasoningEffort: 'xhigh',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
      },
    },
    {
      id: 'gpt-5.2-codex',
      name: 'GPT-5.2 Codex',
      contextLimit: 400000,
      outputLimit: 128000,
      modalities: { input: ['text', 'image'], output: ['text'] },
      options: {
        include: ['reasoning.encrypted_content'],
        store: false,
      },
      variants: {
        high: {
          reasoningEffort: 'high',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        low: {
          reasoningEffort: 'low',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        medium: {
          reasoningEffort: 'medium',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        xhigh: {
          reasoningEffort: 'xhigh',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
      },
    },
    {
      id: 'gpt-5.3-codex',
      name: 'GPT-5.3 Codex',
      contextLimit: 400000,
      outputLimit: 128000,
      modalities: { input: ['text', 'image'], output: ['text'] },
      options: {
        include: ['reasoning.encrypted_content'],
        store: false,
      },
      variants: {
        high: {
          reasoningEffort: 'high',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        low: {
          reasoningEffort: 'low',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        medium: {
          reasoningEffort: 'medium',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
        xhigh: {
          reasoningEffort: 'xhigh',
          reasoningSummary: 'auto',
          textVerbosity: 'medium',
        },
      },
    },
  ],
  '@ai-sdk/anthropic': [
    {
      id: 'claude-sonnet-4-5-20250929',
      name: 'Claude Sonnet 4.5',
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ['text', 'image', 'pdf'], output: ['text'] },
      variants: {
        high: {
          effort: 'high',
        },
        low: {
          effort: 'low',
        },
        medium: {
          effort: 'medium',
        },
      },
    },
    {
      id: 'claude-opus-4-5-20251101',
      name: 'Claude Opus 4.5',
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ['text', 'image', 'pdf'], output: ['text'] },
      variants: {
        high: {
          thinking: {
            budgetTokens: 18000,
            type: 'enabled',
          },
        },
        low: {
          thinking: {
            budgetTokens: 5000,
            type: 'enabled',
          },
        },
        medium: {
          thinking: {
            budgetTokens: 13000,
            type: 'enabled',
          },
        },
      },
    },
    {
      id: 'claude-sonnet-4-6',
      name: 'Claude Sonnet 4.6',
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ['text', 'image', 'pdf'], output: ['text'] },
      variants: {
        high: {
          effort: 'high',
        },
        low: {
          effort: 'low',
        },
        medium: {
          effort: 'medium',
        },
      },
    },
    {
      id: 'claude-opus-4-6',
      name: 'Claude Opus 4.6',
      contextLimit: 200000,
      outputLimit: 128000,
      modalities: { input: ['text', 'image', 'pdf'], output: ['text'] },
      variants: {
        high: {
          thinking: {
            budgetTokens: 18000,
            type: 'enabled',
          },
        },
        low: {
          thinking: {
            budgetTokens: 5000,
            type: 'enabled',
          },
        },
        medium: {
          thinking: {
            budgetTokens: 13000,
            type: 'enabled',
          },
        },
      },
    },
    {
      id: 'claude-haiku-4-5-20251001',
      name: 'Claude Haiku 4.5',
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ['text', 'image', 'pdf'], output: ['text'] },
    },
    {
      id: 'gemini-claude-opus-4-5-thinking',
      name: 'Antigravity - Claude Opus 4.5',
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ['text', 'image', 'pdf'], output: ['text'] },
      variants: {
        high: {
          effort: 'high',
        },
        low: {
          effort: 'low',
        },
        medium: {
          effort: 'medium',
        },
      },
    },
    {
      id: 'gemini-claude-sonnet-4-5-thinking',
      name: 'Antigravity - Claude Sonnet 4.5',
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ['text', 'image', 'pdf'], output: ['text'] },
      variants: {
        high: {
          thinking: {
            budgetTokens: 18000,
            type: 'enabled',
          },
        },
        low: {
          thinking: {
            budgetTokens: 5000,
            type: 'enabled',
          },
        },
        medium: {
          thinking: {
            budgetTokens: 13000,
            type: 'enabled',
          },
        },
      },
    },
  ],
};
