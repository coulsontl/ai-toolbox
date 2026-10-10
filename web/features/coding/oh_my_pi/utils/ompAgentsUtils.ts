import type {
  OmpAgentConfig,
  OmpModelRoleConfig,
  OmpRuntimeProviderView,
} from '../../../../types/ohMyPi.ts';
import {
  OMP_THINKING_LEVELS,
  getOmpModelDefaultThinkingLevel,
  getOmpModelThinkingLevelOptions,
} from '../../../../utils/ompModelMetadata.ts';
import { getOmpRuntimeModelRecords } from './ompRuntimeModels.ts';

/**
 * OMP 原生核心模型角色定义(modelRoles)。
 * 镜像 upstream `packages/coding-agent/src/config/model-roles.ts` 中的 9 大内置角色。
 */
export interface OmpCoreModelRoleDef {
  key: string;
  tag: string;
  name: string;
  descriptionKey: string;
  color?: string;
  defaultThinkingLevel?: string;
  isPrimary?: boolean;
}

export const OMP_CORE_MODEL_ROLES: OmpCoreModelRoleDef[] = [
  // 核心功能与主代理
  {
    key: 'default',
    tag: 'DEFAULT',
    name: 'Default',
    descriptionKey: 'ohMyPi.subagents.roles.default',
    color: '#52c41a',
    isPrimary: true,
  },
  {
    key: 'plan',
    tag: 'PLAN',
    name: 'Plan (Architect)',
    descriptionKey: 'ohMyPi.subagents.roles.plan',
    color: '#8c8c8c',
    defaultThinkingLevel: 'auto',
  },
  {
    key: 'task',
    tag: 'TASK',
    name: 'Task (Subtask)',
    descriptionKey: 'ohMyPi.subagents.roles.task',
    color: '#1677ff',
    defaultThinkingLevel: 'auto',
  },
  {
    key: 'advisor',
    tag: 'ADVISOR',
    name: 'Advisor (Watchdog)',
    descriptionKey: 'ohMyPi.subagents.roles.advisor',
    color: '#722ed1',
    defaultThinkingLevel: 'high',
  },
  {
    key: 'commit',
    tag: 'COMMIT',
    name: 'Commit',
    descriptionKey: 'ohMyPi.subagents.roles.commit',
    color: '#fa8c16',
  },
  {
    key: 'tiny',
    tag: 'TINY',
    name: 'Tiny (Lightweight)',
    descriptionKey: 'ohMyPi.subagents.roles.tiny',
    color: '#13c2c2',
  },
  // 辅助能力角色
  {
    key: 'smol',
    tag: 'SMOL',
    name: 'Fast (Smol)',
    descriptionKey: 'ohMyPi.subagents.roles.smol',
    color: '#faad14',
    defaultThinkingLevel: 'medium',
  },
  {
    key: 'slow',
    tag: 'SLOW',
    name: 'Thinking (Slow)',
    descriptionKey: 'ohMyPi.subagents.roles.slow',
    color: '#eb2f96',
    defaultThinkingLevel: 'high',
  },
  {
    key: 'vision',
    tag: 'VISION',
    name: 'Vision',
    descriptionKey: 'ohMyPi.subagents.roles.vision',
    color: '#f5222d',
  },
];

export const OMP_CORE_MODEL_ROLE_KEYS = new Set<string>(
  OMP_CORE_MODEL_ROLES.map((role) => role.key),
);

/**
 * 解析单个 modelRole 的存储值(支持对象格式或 "provider/model:level" 字符串)。
 */
export const parseOmpModelRoleEntry = (
  raw: unknown,
): { model?: string; thinkingLevel?: string } => {
  if (!raw) return {};
  if (typeof raw === 'string') {
    const trimmed = raw.trim();
    const lastColon = trimmed.lastIndexOf(':');
    if (lastColon > 0 && !trimmed.slice(0, lastColon).endsWith('/')) {
      const level = trimmed.slice(lastColon + 1);
      // 只有后缀是合法思考级别才当作 `:level`(与后端 parse_role_string、上游
      // splitThinkingSuffix 一致)。Ollama tag(`ollama/qwen2.5:14b`)、OpenRouter
      // `:free` 这类字面 model id 必须整串保留,否则保存后模型会被截断。
      if (OMP_THINKING_LEVELS.has(level)) {
        return { model: trimmed.slice(0, lastColon), thinkingLevel: level };
      }
    }
    return { model: trimmed };
  }
  if (typeof raw === 'object' && raw !== null) {
    const obj = raw as Record<string, unknown>;
    return {
      model: typeof obj.model === 'string' && obj.model.trim() ? obj.model.trim() : undefined,
      thinkingLevel:
        typeof obj.thinkingLevel === 'string' && obj.thinkingLevel.trim()
          ? obj.thinkingLevel.trim()
          : undefined,
    };
  }
  return {};
};

/**
 * 将核心角色编辑值转换为规范对象存储。
 */
export const formatOmpModelRoleEntry = (
  entry: { model?: string; thinkingLevel?: string } | undefined,
): OmpModelRoleConfig | undefined => {
  if (!entry || !entry.model || !entry.model.trim()) return undefined;
  const model = entry.model.trim();
  const thinkingLevel =
    entry.thinkingLevel && entry.thinkingLevel.trim() ? entry.thinkingLevel.trim() : undefined;
  return { model, thinkingLevel };
};

/**
 * 获取某个 role 在方案中的显示文本(用于卡片预览)。
 */
export const getOmpModelRoleDisplay = (
  entry: unknown,
): { model?: string; thinkingLevel?: string } => {
  return parseOmpModelRoleEntry(entry);
};

export interface OmpThinkingOptionsResult {
  supported: boolean;
  options: Array<{ value: string; label: string }>;
  defaultLevel?: string;
  placeholderKey?: string;
  isGeneric?: boolean;
}

export const GENERAL_THINKING_OPTIONS: Array<{ value: string; label: string }> = [
  { value: 'off', label: 'off' },
  { value: 'auto', label: 'auto' },
  { value: 'low', label: 'low' },
  { value: 'medium', label: 'medium' },
  { value: 'high', label: 'high' },
];

/**
 * 根据所选模型和当前 provider 列表计算适用的思考等级选项。
 * 严格贴合 YAML 或原生运行时目录中声明的 reasoning / thinking.efforts 支持。
 */
export const getOmpThinkingOptionsForModel = (
  modelName: string | undefined,
  providers?: OmpRuntimeProviderView[],
): OmpThinkingOptionsResult => {
  if (!modelName || !modelName.trim()) {
    return {
      supported: false,
      options: [],
      placeholderKey: 'ohMyPi.subagents.selectModelFirst',
    };
  }

  const trimmed = modelName.trim();

  // 1. 角色别名(例如 @task, @smol, @default)
  if (trimmed.startsWith('@')) {
    return {
      supported: true,
      options: GENERAL_THINKING_OPTIONS,
      isGeneric: true,
    };
  }

  // 2. 尝试按 providerKey / modelId 解析
  let foundModelRecord: Record<string, unknown> | undefined;
  let hasMatchingModel = false;

  const slashIdx = trimmed.indexOf('/');
  if (slashIdx > 0 && slashIdx < trimmed.length - 1) {
    const providerKey = trimmed.slice(0, slashIdx);
    const modelId = trimmed.slice(slashIdx + 1);
    const provider = providers?.find((p) => p.providerKey === providerKey);
    if (provider) {
      const records = getOmpRuntimeModelRecords(provider);
      const match = records.find((m) => m.id === modelId);
      if (match) {
        hasMatchingModel = true;
        foundModelRecord = match.model;
      }
    }
  }

  // 3. 若没带 provider 前缀或未找到，在所有 provider 中搜索 modelId
  if (!hasMatchingModel && providers && providers.length > 0) {
    for (const provider of providers) {
      const records = getOmpRuntimeModelRecords(provider);
      const match = records.find((m) => m.id === trimmed);
      if (match) {
        hasMatchingModel = true;
        foundModelRecord = match.model;
        break;
      }
    }
  }

  // 4. 如果找到了具体模型记录，按模型的 thinking 属性严格计算
  if (hasMatchingModel) {
    const options = getOmpModelThinkingLevelOptions(foundModelRecord);
    if (options.length === 0) {
      // 该模型不具备思考/推理能力(reasoning === false 或无 thinking 支持)
      return {
        supported: false,
        options: [],
        placeholderKey: 'ohMyPi.subagents.thinkingNotSupported',
      };
    }
    return {
      supported: true,
      options,
      defaultLevel: getOmpModelDefaultThinkingLevel(foundModelRecord),
    };
  }

  // 5. 若为未在 provider 中登记的外部/自定义模型，允许使用通用级别
  return {
    supported: true,
    options: GENERAL_THINKING_OPTIONS,
    isGeneric: true,
  };
};

/**
 * OMP task bundled agents 清单(上游 task 子系统自带的 5 个 markdown 模板)。
 */
export interface OmpBuiltinAgentDefault {
  descriptionKey: string;
  model?: string;
  thinkingLevel?: string;
}

export const OMP_BUILTIN_AGENT_DEFAULTS: Record<string, OmpBuiltinAgentDefault> = {
  task: {
    descriptionKey: 'ohMyPi.subagents.builtin.task',
    model: '@task',
    thinkingLevel: 'auto',
  },
  sonic: {
    descriptionKey: 'ohMyPi.subagents.builtin.sonic',
    model: '@smol',
    thinkingLevel: 'medium',
  },
  scout: { descriptionKey: 'ohMyPi.subagents.builtin.scout' },
  reviewer: { descriptionKey: 'ohMyPi.subagents.builtin.reviewer' },
  'security-reviewer': { descriptionKey: 'ohMyPi.subagents.builtin.securityReviewer' },
};

export const OMP_BUILTIN_AGENTS = Object.keys(OMP_BUILTIN_AGENT_DEFAULTS) as Array<
  keyof typeof OMP_BUILTIN_AGENT_DEFAULTS
>;

export const OMP_BUILTIN_AGENT_NAMES = new Set<string>(OMP_BUILTIN_AGENTS);

/** OMP 保留名:main / sub 是会话 sentinel,不可用作自定义 agent。 */
export const OMP_RESERVED_AGENT_NAMES = new Set<string>(['main', 'sub']);

/** 合法 agent 文件名(与后端 is_valid_agent_file_name 一致)。 */
export const isValidOmpAgentFileName = (name: string): boolean => {
  if (!name || name === '.' || name === '..') return false;
  return /^[A-Za-z0-9._-]+$/.test(name);
};

/** 显示 agent 的主模型(供卡片预览)。OMP 用 `model`(字符串或数组)。 */
export const getOmpAgentModelDisplay = (
  agent: OmpAgentConfig | undefined,
): { primaryModel?: string; fallbackCount: number } => {
  if (!agent) return { primaryModel: undefined, fallbackCount: 0 };
  const model = agent.model;
  if (Array.isArray(model)) {
    const strings = model.filter((entry): entry is string => typeof entry === 'string' && !!entry);
    return { primaryModel: strings[0], fallbackCount: Math.max(0, strings.length - 1) };
  }
  if (typeof model === 'string' && model) {
    return { primaryModel: model, fallbackCount: 0 };
  }
  return { primaryModel: undefined, fallbackCount: 0 };
};

/** 从 frontmatter 配置里取字符串字段(带默认)。 */
export const getOmpAgentFieldString = (
  agent: OmpAgentConfig | undefined,
  key: string,
): string | undefined => {
  if (!agent) return undefined;
  const value = agent[key];
  return typeof value === 'string' && value ? value : undefined;
};

/** 从 frontmatter 配置里取布尔字段。 */
export const getOmpAgentFieldBool = (
  agent: OmpAgentConfig | undefined,
  key: string,
): boolean | undefined => {
  if (!agent) return undefined;
  const value = agent[key];
  return typeof value === 'boolean' ? value : undefined;
};

/** 从 frontmatter 配置里取字符串数组字段(兼容单字符串)。 */
export const getOmpAgentFieldStringArray = (
  agent: OmpAgentConfig | undefined,
  key: string,
): string[] => {
  if (!agent) return [];
  const value = agent[key];
  if (Array.isArray(value)) {
    return value.filter((entry): entry is string => typeof entry === 'string' && !!entry);
  }
  if (typeof value === 'string' && value.trim()) {
    return value
      .split(',')
      .map((entry) => entry.trim())
      .filter(Boolean);
  }
  return [];
};

export interface OmpAgentFormDraft {
  /** 方案内 agent 名(文件名,不可含路径)。 */
  name: string;
  description?: string;
  model?: string;
  thinkingLevel?: string;
  tools: string[];
  blocking?: boolean;
  readSummarize?: boolean;
  prewalk?: string;
  advisor?: string;
  spawns?: string;
  autoloadSkills: string[];
  prompt?: string;
}

/** 把 frontmatter 配置转为表单草案。 */
export const ompAgentConfigToDraft = (
  agent: OmpAgentConfig | undefined,
): OmpAgentFormDraft => {
  const model = getOmpAgentFieldString(agent, 'model');
  const prewalkValue = agent?.prewalk;
  const advisorValue = agent?.advisor;
  return {
    name: agent?.name ? String(agent.name) : '',
    description: getOmpAgentFieldString(agent, 'description'),
    model,
    thinkingLevel:
      getOmpAgentFieldString(agent, 'thinkingLevel') ?? getOmpAgentFieldString(agent, 'thinking'),
    tools: getOmpAgentFieldStringArray(agent, 'tools'),
    blocking: getOmpAgentFieldBool(agent, 'blocking'),
    readSummarize: getOmpAgentFieldBool(agent, 'readSummarize'),
    prewalk:
      typeof prewalkValue === 'boolean'
        ? prewalkValue
          ? 'true'
          : 'false'
        : typeof prewalkValue === 'string'
          ? prewalkValue
          : undefined,
    advisor:
      typeof advisorValue === 'boolean'
        ? advisorValue
          ? 'true'
          : 'false'
        : typeof advisorValue === 'string'
          ? advisorValue
          : undefined,
    spawns: getOmpAgentFieldString(agent, 'spawns'),
    autoloadSkills: getOmpAgentFieldStringArray(agent, 'autoloadSkills'),
    prompt: getOmpAgentFieldString(agent, 'prompt'),
  };
};

const omitEmpty = (value?: string): string | undefined =>
  typeof value === 'string' && value.trim() ? value.trim() : undefined;

/** 判断一个 agent 配置是否包含「显式配置」——即除内置默认外的任何字段。
 *  内置 agent 若只回填了默认 model/thinkingLevel,应视为「未覆盖」,写入方案时
 *  应剔除(保持 OMP bundled 默认),不生成覆盖文件。 */
const BUILTIN_DEFAULT_KEYS = new Set(['name', 'description', 'model', 'thinkingLevel']);

export const hasExplicitOmpAgentConfig = (agent: OmpAgentConfig): boolean => {
  const keys = Object.keys(agent);
  if (keys.some((key) => !BUILTIN_DEFAULT_KEYS.has(key))) {
    return true;
  }
  // 只有 builtin 默认字段时,字段值必须与默认一致才算「未覆盖」。
  const builtinDefault = OMP_BUILTIN_AGENT_DEFAULTS[String(agent.name ?? '')];
  if (!builtinDefault) {
    return true;
  }
  const modelMatches = agent.model === undefined || agent.model === builtinDefault.model;
  const thinkingMatches =
    agent.thinkingLevel === undefined || agent.thinkingLevel === builtinDefault.thinkingLevel;
  return !(modelMatches && thinkingMatches);
};

/** 构造一个内置 agent 的默认 frontmatter 配置(供编辑弹窗回填默认值)。 */
export const buildOmpBuiltinAgentConfig = (name: string): OmpAgentConfig => {
  const builtin = OMP_BUILTIN_AGENT_DEFAULTS[name];
  if (!builtin) {
    return {};
  }
  const config: OmpAgentConfig = { name };
  if (builtin.descriptionKey) {
    config.description = '';
  }
  if (builtin.model) {
    config.model = builtin.model;
  }
  if (builtin.thinkingLevel) {
    config.thinkingLevel = builtin.thinkingLevel;
  }
  return config;
};

/** 把表单草案合并进 frontmatter 配置(仅更新受管字段,未知字段原样保留)。 */
export const ompAgentDraftToConfig = (
  draft: OmpAgentFormDraft,
  existing: OmpAgentConfig,
): OmpAgentConfig => {
  const next: OmpAgentConfig = { ...existing };
  next.name = draft.name;
  const setOptional = (key: string, value: string | undefined) => {
    const cleaned = omitEmpty(value);
    if (cleaned !== undefined) {
      next[key] = cleaned;
    } else {
      delete next[key];
    }
  };
  setOptional('description', draft.description);
  setOptional('model', draft.model);
  setOptional('thinkingLevel', draft.thinkingLevel);
  if (draft.thinkingLevel === undefined) {
    delete next.thinking;
  }
  if (draft.tools.length > 0) {
    next.tools = draft.tools;
  } else {
    delete next.tools;
  }
  const setBool = (key: string, value?: boolean) => {
    if (value === undefined) {
      delete next[key];
    } else {
      next[key] = value;
    }
  };
  setBool('blocking', draft.blocking);
  setBool('readSummarize', draft.readSummarize);
  const setBoolOrString = (key: string, value?: string) => {
    const cleaned = omitEmpty(value);
    if (cleaned === undefined) {
      delete next[key];
    } else if (cleaned === 'true') {
      next[key] = true;
    } else if (cleaned === 'false') {
      next[key] = false;
    } else {
      next[key] = cleaned;
    }
  };
  setBoolOrString('prewalk', draft.prewalk);
  setBoolOrString('advisor', draft.advisor);
  setOptional('spawns', draft.spawns === '*' ? '*' : draft.spawns);
  if (draft.autoloadSkills.length > 0) {
    next.autoloadSkills = draft.autoloadSkills;
  } else {
    delete next.autoloadSkills;
  }
  setOptional('prompt', draft.prompt);
  return next;
};