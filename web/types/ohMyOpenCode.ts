/**
 * Oh My OpenCode Configuration Types
 *
 * Type definitions for oh-my-opencode plugin configuration management.
 * All nested config objects are generic JSON to allow flexibility.
 */

/**
 * Agent configuration - generic JSON structure
 */
export type OhMyOpenCodeAgentConfig = Record<string, unknown>;

/**
 * Sisyphus agent specific configuration - generic JSON structure
 */
export type OhMyOpenCodeSisyphusConfig = Record<string, unknown>;

/**
 * LSP Server configuration - generic JSON structure
 */
export type OhMyOpenCodeLspServer = Record<string, unknown>;

/**
 * Experimental features configuration - generic JSON structure
 */
export type OhMyOpenCodeExperimental = Record<string, unknown>;

/**
 * Agent definition for oh-my-opencode
 */
export interface OhMyOpenCodeAgentDefinition {
  /** Agent key used in configuration */
  key: string;
  /** Display name shown in UI */
  display: string;
  /** Chinese description */
  descZh: string;
  /** English description */
  descEn: string;
  /** Recommended model for this agent */
  recommendedModel?: string;
}

/**
 * Category definition for oh-my-opencode
 */
export interface OhMyOpenCodeCategoryDefinition {
  /** Category key used in configuration */
  key: string;
  /** Display name shown in UI */
  display: string;
  /** Chinese description */
  descZh: string;
  /** English description */
  descEn: string;
  /** Recommended model for this category */
  recommendedModel?: string;
}

/**
 * Centralized agent definitions for oh-my-opencode
 * Order defines UI display and should be updated intentionally
 */
export const OH_MY_OPENCODE_AGENTS: OhMyOpenCodeAgentDefinition[] = [
  // ===== 主 Agents：用户的直接入口，负责协调和决策（你主动找他们）=====
  {
    key: '__main_agents_separator__',
    display: '─ 主 Agents ─',
    descZh: '主 Agents：用户的直接入口，负责协调和决策（你主动找他们）',
    descEn: 'Main Agents: Direct entry points for users, responsible for coordination and decision-making (you actively seek them)',
  },
  {
    key: 'Sisyphus',
    display: 'Sisyphus',
    descZh: '主编排者 - 你的"第一接待员"，接收请求后决定自己干还是派专家干，负责任务分类、委派和全局协调',
    descEn: 'Primary orchestrator - Your "first receptionist", receives requests and decides to handle or delegate, responsible for task classification, delegation and global coordination',
    recommendedModel: 'claude-opus-4-5 (variant: max)',
  },
  {
    key: 'hephaestus',
    display: 'Hephaestus',
    descZh: '自主深度工作者 - 给目标就行的"资深工匠"，会花5-15分钟先研究再动手，端到端自主完成复杂任务不需要逐步指挥',
    descEn: 'Autonomous deep worker - A "senior craftsman" who just needs goals, spends 5-15 minutes researching before acting, completes complex tasks end-to-end without step-by-step guidance',
    recommendedModel: 'gpt-5.2-codex (variant: medium，必需)',
  },
  {
    key: 'Prometheus',
    display: 'Prometheus',
    descZh: '战略规划者 - 只做计划不写代码的"产品经理"，把大任务拆成小步骤，生成依赖图和执行计划',
    descEn: 'Strategic planner - A "product manager" who only plans without coding, breaks big tasks into small steps, generates dependency graphs and execution plans',
    recommendedModel: 'claude-opus-4-5 (variant: max)',
  },
  {
    key: 'Atlas',
    display: 'Atlas',
    descZh: '任务管理者 - 持有todo清单的"项目经理"，跟踪多步骤任务进度，确保每个步骤有序完成不遗漏（系统会自动调用）',
    descEn: 'Task manager - A "project manager" with todo list, tracks multi-step task progress, ensures each step is completed in order without omission (automatically invoked by system)',
    recommendedModel: 'kimi-k2.5',
  },
  // ===== 子 Agents：专业领域专家，被主Agent或系统调用（他们被动工作）=====
  {
    key: '__sub_agents_separator__',
    display: '─ 子 Agents ─',
    descZh: '子 Agents：专业领域专家，被主Agent或系统调用（他们被动工作）',
    descEn: 'Sub Agents: Domain experts, invoked by main agents or system (they work passively)',
  },
  {
    key: 'oracle',
    display: 'Oracle',
    descZh: '战略顾问 - 只看不动手的"CTO顾问"，专门分析架构、审查代码、调试疑难，给建议但不改代码',
    descEn: 'Strategic advisor - A "CTO consultant" who only observes, specializes in architecture analysis, code review, debugging, gives advice but does not modify code',
    recommendedModel: 'gpt-5.2 (variant: high)',
  },
  {
    key: 'librarian',
    display: 'Librarian',
    descZh: '多仓库研究员 - 专查外部资料的"图书馆员"，搜GitHub代码、读官方文档、找开源实现示例',
    descEn: 'Multi-repo researcher - A "librarian" for external resources, searches GitHub code, reads official docs, finds open source implementation examples',
    recommendedModel: 'glm-4.7',
  },
  {
    key: 'explore',
    display: 'Explore',
    descZh: '快速代码库搜索 - 项目内的"Ctrl+Shift+F"，快速定位"这个功能在哪"、"谁调用了这个函数"，可并行多个',
    descEn: 'Fast codebase search - Project-level "Ctrl+Shift+F", quickly locates "where is this feature", "who calls this function", can run multiple in parallel',
    recommendedModel: 'grok-code-fast-1',
  },
  {
    key: 'multimodal-looker',
    display: 'Multimodal-Looker',
    descZh: '媒体分析器 - 有"眼睛"的助手，专门看图片、PDF、设计稿，提取视觉信息转成文字描述',
    descEn: 'Media analyzer - An assistant with "eyes", specializes in viewing images, PDFs, design drafts, extracts visual information into text descriptions',
    recommendedModel: 'gemini-3-flash',
  },
  {
    key: 'Metis',
    display: 'Metis',
    descZh: '规划前分析顾问 - Prometheus的"前置分析师"，在做计划前先帮你想清楚"你到底要什么"，发现隐藏需求和潜在坑',
    descEn: 'Pre-planning analyst - Prometheus\'s "pre-analyst", helps clarify "what you really want" before planning, discovers hidden requirements and potential pitfalls',
    recommendedModel: 'claude-opus-4-5 (variant: max)',
  },
  {
    key: 'Momus',
    display: 'Momus',
    descZh: '计划审查者 - Prometheus的"质检员"，专挑计划的阻塞性问题，确保计划可执行、引用有效、没有遗漏',
    descEn: 'Plan reviewer - Prometheus\'s "QA inspector", identifies blocking issues in plans, ensures plans are executable, references are valid, nothing is missed',
    recommendedModel: 'gpt-5.2 (variant: medium)',
  },
  {
    key: 'Sisyphus-Junior',
    display: 'Sisyphus-Junior',
    descZh: '委托任务执行器 - 穿上Category"马甲"的"实习生"，根据任务类型动态调整模型和风格去执行具体工作',
    descEn: 'Delegated task executor - An "intern" wearing Category "vest", dynamically adjusts model and style based on task type to execute specific work',
    recommendedModel: '无需配置，动态继承Category的配置',
  },
];

/**
 * Centralized category definitions for oh-my-opencode
 * Order follows the default categories in 3.1
 */
export const OH_MY_OPENCODE_CATEGORIES: OhMyOpenCodeCategoryDefinition[] = [
  {
    key: 'visual-engineering',
    display: 'Visual Engineering',
    descZh: '前端UI模式 - 做界面的"设计师模式"，强调大胆审美、独特排版、高冲击力动画，拒绝千篇一律',
    descEn: 'Frontend UI mode - "Designer mode" for interfaces, emphasizes bold aesthetics, unique layouts, high-impact animations, rejects cookie-cutter designs',
    recommendedModel: 'gemini-3-pro',
  },
  {
    key: 'ultrabrain',
    display: 'Ultrabrain',
    descZh: '超级大脑模式 - 深度思考的"架构师模式"，只给目标不给步骤，用于真正困难的逻辑推理和架构设计',
    descEn: 'Super brain mode - Deep thinking "architect mode", give goals not steps, for truly difficult logical reasoning and architecture design',
    recommendedModel: 'gpt-5.2-codex (variant: xhigh，必需)',
  },
  {
    key: 'deep',
    display: 'Deep',
    descZh: '深度自主模式 - 自主研究的"资深工程师模式"，会花很长时间先探索再动手，用于疑难杂症和混乱代码重构',
    descEn: 'Deep autonomous mode - "Senior engineer mode" for autonomous research, spends long time exploring before acting, for tricky issues and messy code refactoring',
    recommendedModel: 'gpt-5.2-codex (variant: medium，必需)',
  },
  {
    key: 'artistry',
    display: 'Artistry',
    descZh: '艺术创意模式 - 打破常规的"创意总监模式"，鼓励大胆非传统方向，用于需要惊喜和创新的任务',
    descEn: 'Artistic creativity mode - "Creative director mode" that breaks conventions, encourages bold unconventional directions, for tasks needing surprise and innovation',
    recommendedModel: 'gemini-3-pro (variant: max，必需)',
  },
  {
    key: 'quick',
    display: 'Quick',
    descZh: '快速模式 - 改typo的"实习生模式"，用便宜快速的小模型，但prompt必须写得非常详细明确',
    descEn: 'Quick mode - "Intern mode" for fixing typos, uses cheap fast small models, but prompts must be very detailed and explicit',
    recommendedModel: 'claude-haiku-4-5',
  },
  {
    key: 'unspecified-low',
    display: 'Unspecified (Low)',
    descZh: '通用中等模式 - 不知道归哪类的"中等杂活模式"，工作量不大、范围有限的常规开发任务',
    descEn: 'General medium mode - "Medium chores mode" for uncategorized tasks, regular development with limited scope and workload',
    recommendedModel: 'claude-sonnet-4-5',
  },
  {
    key: 'unspecified-high',
    display: 'Unspecified (High)',
    descZh: '通用高级模式 - 不知道归哪类的"大型杂活模式"，跨多模块、影响范围大的高工作量任务',
    descEn: 'General high mode - "Large chores mode" for uncategorized tasks, cross-module high-workload tasks with large impact scope',
    recommendedModel: 'claude-opus-4-5 (variant: max)',
  },
  {
    key: 'writing',
    display: 'Writing',
    descZh: '写作模式 - 写文档的"技术作家模式"，专注清晰流畅的散文表达，用于README、文档、技术文章',
    descEn: 'Writing mode - "Technical writer mode" for documentation, focuses on clear fluent prose, for README, docs, technical articles',
    recommendedModel: 'gemini-3-flash',
  },
];

/**
 * Agent types supported by oh-my-opencode
 * Auto-generated from OH_MY_OPENCODE_AGENTS
 */
export type OhMyOpenCodeAgentType = typeof OH_MY_OPENCODE_AGENTS[number]['key'];

/**
 * Oh My OpenCode Agents Profile (子 Agents 配置方案)
 * 只包含各 Agent 的模型配置，可以有多个方案供切换
 */
export interface OhMyOpenCodeAgentsProfile {
  id: string;
  name: string;
  isApplied: boolean;
  isDisabled: boolean;
  agents: Record<string, OhMyOpenCodeAgentConfig> | null; // Generic JSON
  categories?: Record<string, OhMyOpenCodeAgentConfig> | null; // Generic JSON
  otherFields?: Record<string, unknown>;
  createdAt?: string;
  updatedAt?: string;
}

/**
 * Oh My OpenCode Global Config (全局通用配置)
 * 全局唯一配置，存储在数据库中，固定 ID 为 "global"
 * 当从本地文件加载时，ID 为 "__local__"
 */
export interface OhMyOpenCodeGlobalConfig {
  id: string; // "global" or "__local__"
  schema?: string;
  sisyphusAgent: Record<string, unknown> | null; // Generic JSON
  disabledAgents?: string[];
  disabledMcps?: string[];
  disabledHooks?: string[];
  disabledSkills?: string[];
  lsp: Record<string, unknown> | null; // Generic JSON
  experimental: Record<string, unknown> | null; // Generic JSON
  backgroundTask?: Record<string, unknown> | null;
  browserAutomationEngine?: Record<string, unknown> | null;
  claudeCode?: Record<string, unknown> | null;
  otherFields?: Record<string, unknown>;
  updatedAt?: string;
}

/**
 * @deprecated 使用 OhMyOpenCodeAgentsProfile 代替
 * 保留用于向后兼容
 */
export type OhMyOpenCodeConfig = OhMyOpenCodeAgentsProfile;

/**
 * Form values for Agents Profile modal (简化版)
 */
export interface OhMyOpenCodeAgentsProfileFormValues {
  id: string;
  name: string;
  isDisabled?: boolean;
  agents: Record<string, OhMyOpenCodeAgentConfig> | null;
  categories?: Record<string, OhMyOpenCodeAgentConfig> | null;
  otherFields?: Record<string, unknown>;
}

/**
 * Form values for Global Config modal
 */
export interface OhMyOpenCodeGlobalConfigFormValues {
  schema?: string;
  sisyphusAgent: Record<string, unknown> | null;
  disabledAgents?: string[];
  disabledMcps?: string[];
  disabledHooks?: string[];
  disabledSkills?: string[];
  lsp?: Record<string, unknown> | null;
  experimental?: Record<string, unknown> | null;
  backgroundTask?: Record<string, unknown> | null;
  browserAutomationEngine?: Record<string, unknown> | null;
  claudeCode?: Record<string, unknown> | null;
  otherFields?: Record<string, unknown>;
}

/**
 * @deprecated 使用 OhMyOpenCodeAgentsProfileFormValues 代替
 */
export type OhMyOpenCodeConfigFormValues = OhMyOpenCodeAgentsProfileFormValues & OhMyOpenCodeGlobalConfigFormValues;

/**
 * Oh My OpenCode JSON file structure
 */
export interface OhMyOpenCodeJsonConfig {
  $schema?: string;
  agents?: Record<string, OhMyOpenCodeAgentConfig>;
  categories?: Record<string, OhMyOpenCodeAgentConfig>;
  sisyphus_agent?: OhMyOpenCodeSisyphusConfig;
  disabled_agents?: string[];
  disabled_mcps?: string[];
  disabled_hooks?: string[];
  disabled_skills?: string[];
  lsp?: Record<string, OhMyOpenCodeLspServer>;
  experimental?: OhMyOpenCodeExperimental;
  background_task?: Record<string, unknown>;
  browser_automation_engine?: Record<string, unknown>;
  claude_code?: Record<string, unknown>;
}
