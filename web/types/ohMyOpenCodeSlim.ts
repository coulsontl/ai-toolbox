/**
 * Type definitions for oh-my-opencode-slim plugin configuration management.
 *
 * Oh My OpenCode Slim is a lightweight alternative to oh-my-opencode with fewer agents.
 * Configuration structure is similar but focuses on essential agents only:
 * - orchestrator
 * - oracle
 * - librarian
 * - explorer
 * - designer
 * - fixer
 */

/**
 * Agent configuration for oh-my-opencode-slim
 */
export interface OhMyOpenCodeSlimAgent {
	model?: string;
	variant?: string;
	fallback_models?: string[] | string;
	skills?: string[];
	[key: string]: any; // Allow additional custom fields
}

export interface OhMyOpenCodeSlimFallbackConfig {
	enabled?: boolean;
	timeoutMs?: number;
	retryDelayMs?: number;
	retry_on_empty?: boolean;
	chains?: Record<string, string[] | string>;
	[key: string]: unknown;
}

/**
 * Centralized agent definitions for oh-my-opencode-slim
 */
export interface OhMyOpenCodeSlimAgents {
	orchestrator?: OhMyOpenCodeSlimAgent;
	oracle?: OhMyOpenCodeSlimAgent;
	librarian?: OhMyOpenCodeSlimAgent;
	explorer?: OhMyOpenCodeSlimAgent;
	designer?: OhMyOpenCodeSlimAgent;
	fixer?: OhMyOpenCodeSlimAgent;
	[key: string]: any; // Allow additional custom agents
}

/**
 * Oh My OpenCode Slim Agents Profile (stored in database)
 */
export interface OhMyOpenCodeSlimConfig {
	id: string;
	name: string;
	isApplied: boolean;
	isDisabled: boolean;
	agents?: OhMyOpenCodeSlimAgents;
	council?: Record<string, unknown> | null;
	fallback?: OhMyOpenCodeSlimFallbackConfig | null;
	otherFields?: Record<string, any>; // For extra configuration fields
	sortIndex?: number; // For manual ordering
	createdAt?: string;
	updatedAt?: string;
}

/**
 * Input type for creating/updating Agents Profile
 */
export interface OhMyOpenCodeSlimConfigInput {
	id?: string;
	name: string;
	isDisabled?: boolean;
	agents?: OhMyOpenCodeSlimAgents;
	council?: Record<string, unknown> | null;
	fallback?: OhMyOpenCodeSlimFallbackConfig | null;
	otherFields?: Record<string, any>;
}

/**
 * Oh My OpenCode Slim Global Config
 * When loaded from local file, id is "__local__"
 */
export interface OhMyOpenCodeSlimGlobalConfig {
	id: string; // "global" or "__local__"
	sisyphusAgent?: Record<string, unknown>;
	disabledAgents?: string[];
	disabledMcps?: string[];
	disabledHooks?: string[];
	lsp?: Record<string, unknown>;
	experimental?: Record<string, unknown>;
	council?: Record<string, unknown> | null;
	otherFields?: Record<string, unknown> | null;
	updatedAt?: string;
}

/**
 * Input type for Global Config
 */
export interface OhMyOpenCodeSlimGlobalConfigInput {
	sisyphusAgent?: Record<string, unknown> | null;
	disabledAgents?: string[];
	disabledMcps?: string[];
	disabledHooks?: string[];
	lsp?: Record<string, unknown> | null;
	experimental?: Record<string, unknown> | null;
	council?: Record<string, unknown> | null;
	otherFields?: Record<string, unknown> | null;
}

export type OhMyOpenCodeSlimCouncilExecutionMode = "parallel" | "serial";

/**
 * Config path info
 */
export interface ConfigPathInfo {
	path: string;
	source: string;
}

/**
 * Agent types supported by oh-my-opencode-slim
 */
export const SLIM_AGENT_TYPES = [
	"orchestrator",
	"oracle",
	"librarian",
	"explorer",
	"designer",
	"fixer",
] as const;

export type SlimAgentType = (typeof SLIM_AGENT_TYPES)[number];

const SLIM_AGENT_I18N_SUFFIX: Record<SlimAgentType, string> = {
	orchestrator: "orchestrator",
	oracle: "oracle",
	librarian: "librarian",
	explorer: "explorer",
	designer: "designer",
	fixer: "fixer",
};

export const getSlimAgentDisplayNameKey = (agentType: SlimAgentType) =>
	`opencode.ohMyOpenCodeSlim.agents.${SLIM_AGENT_I18N_SUFFIX[agentType]}.name`;

export const getSlimAgentDescriptionKey = (agentType: SlimAgentType) =>
	`opencode.ohMyOpenCodeSlim.agents.${SLIM_AGENT_I18N_SUFFIX[agentType]}.description`;
