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
}

/**
 * Category definition for oh-my-opencode
 */
export interface OhMyOpenCodeCategoryDefinition {
	/** Category key used in configuration */
	key: string;
}

/**
 * Centralized agent definitions for oh-my-opencode
 * Order defines UI display and should be updated intentionally
 */
export const OH_MY_OPENCODE_AGENTS: OhMyOpenCodeAgentDefinition[] = [
	// ===== 主 Agents：用户的直接入口，负责协调和决策（你主动找他们）=====
	{
		key: "__main_agents_separator__",
	},
	{
		key: "sisyphus",
	},
	{
		key: "hephaestus",
	},
	{
		key: "prometheus",
	},
	{
		key: "atlas",
	},
	// ===== 子 Agents：专业领域专家，被主Agent或系统调用（他们被动工作）=====
	{
		key: "__sub_agents_separator__",
	},
	{
		key: "oracle",
	},
	{
		key: "librarian",
	},
	{
		key: "explore",
	},
	{
		key: "multimodal-looker",
	},
	{
		key: "metis",
	},
	{
		key: "momus",
	},
	{
		key: "sisyphus-junior",
	},
];

/**
 * Centralized category definitions for oh-my-opencode
 * Order follows the default categories in 3.1
 */
export const OH_MY_OPENCODE_CATEGORIES: OhMyOpenCodeCategoryDefinition[] = [
	{
		key: "visual-engineering",
	},
	{
		key: "ultrabrain",
	},
	{
		key: "deep",
	},
	{
		key: "artistry",
	},
	{
		key: "quick",
	},
	{
		key: "unspecified-low",
	},
	{
		key: "unspecified-high",
	},
	{
		key: "writing",
	},
];

/**
 * Agent types supported by oh-my-opencode
 * Auto-generated from OH_MY_OPENCODE_AGENTS
 */
export type OhMyOpenCodeAgentType =
	(typeof OH_MY_OPENCODE_AGENTS)[number]["key"];

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
	sortIndex?: number; // For manual ordering
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
export type OhMyOpenCodeConfigFormValues = OhMyOpenCodeAgentsProfileFormValues &
	OhMyOpenCodeGlobalConfigFormValues;

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
