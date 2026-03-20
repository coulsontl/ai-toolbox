import { invoke } from "@tauri-apps/api/core";
import type { TFunction } from "i18next";
import type {
	OhMyOpenCodeConfig,
	OhMyOpenCodeGlobalConfig,
} from "@/types/ohMyOpenCode";
import {
	OH_MY_OPENCODE_AGENTS,
	OH_MY_OPENCODE_CATEGORIES,
} from "@/types/ohMyOpenCode";

const AGENT_KEYS = new Set(OH_MY_OPENCODE_AGENTS.map((agent) => agent.key));
const CATEGORY_KEYS = new Set(
	OH_MY_OPENCODE_CATEGORIES.map((category) => category.key),
);

function getMetaText(t: TFunction, key: string): string | undefined {
	const value = t(key);
	return value === key ? undefined : value;
}

// ============================================================================
// Oh My OpenCode API
// ============================================================================

/**
 * List all omo configurations
 */
export const listOhMyOpenCodeConfigs = async (): Promise<
	OhMyOpenCodeConfig[]
> => {
	return await invoke<OhMyOpenCodeConfig[]>("list_oh_my_opencode_configs");
};

/**
 * Create a new oh-my-opencode configuration
 */
export const createOhMyOpenCodeConfig = async (
	config: OhMyOpenCodeConfigInput,
): Promise<OhMyOpenCodeConfig> => {
	return await invoke<OhMyOpenCodeConfig>("create_oh_my_opencode_config", {
		input: config,
	});
};

/**
 * Update an existing oh-my-opencode configuration
 */
export const updateOhMyOpenCodeConfig = async (
	config: OhMyOpenCodeConfigInput,
): Promise<OhMyOpenCodeConfig> => {
	return await invoke<OhMyOpenCodeConfig>("update_oh_my_opencode_config", {
		input: config,
	});
};

/**
 * Delete an oh-my-opencode configuration
 */
export const deleteOhMyOpenCodeConfig = async (id: string): Promise<void> => {
	await invoke("delete_oh_my_opencode_config", { id });
};

/**
 * Apply a configuration to the oh-my-opencode.json file
 */
export const applyOhMyOpenCodeConfig = async (
	configId: string,
): Promise<void> => {
	await invoke("apply_oh_my_opencode_config", { configId });
};

/**
 * Reorder configurations
 */
export const reorderOhMyOpenCodeConfigs = async (
	ids: string[],
): Promise<void> => {
	await invoke("reorder_oh_my_opencode_configs", { ids });
};

/**
 * Toggle is_disabled status for a config
 */
export async function toggleOhMyOpenCodeConfigDisabled(
	configId: string,
	isDisabled: boolean,
): Promise<void> {
	return invoke("toggle_oh_my_opencode_config_disabled", {
		configId,
		isDisabled,
	});
}

/**
 * Get config file path info
 */
export const getOhMyOpenCodeConfigPathInfo = async (): Promise<{
	path: string;
	source: string;
}> => {
	return await invoke("get_oh_my_opencode_config_path_info");
};

/**
 * Check if local oh-my-opencode config file exists
 * Returns true if ~/.config/opencode/oh-my-opencode.jsonc or .json exists
 */
export const checkOhMyOpenCodeConfigExists = async (): Promise<boolean> => {
	return await invoke<boolean>("check_oh_my_opencode_config_exists");
};

// ============================================================================
// Oh My OpenCode Global Config API
// ============================================================================

/**
 * Get global config (从 oh_my_opencode_global_config 表读取)
 */
export const getOhMyOpenCodeGlobalConfig =
	async (): Promise<OhMyOpenCodeGlobalConfig> => {
		return await invoke<OhMyOpenCodeGlobalConfig>(
			"get_oh_my_opencode_global_config",
		);
	};

/**
 * Save global config (保存到 oh_my_opencode_global_config 表)
 */
export const saveOhMyOpenCodeGlobalConfig = async (
	config: OhMyOpenCodeGlobalConfigInput,
): Promise<OhMyOpenCodeGlobalConfig> => {
	return await invoke<OhMyOpenCodeGlobalConfig>(
		"save_oh_my_opencode_global_config",
		{ input: config },
	);
};

/**
 * Save local config (both Agents Profile and Global Config) into database
 * This is used when saving __local__ temporary config to database
 */
export const saveOhMyOpenCodeLocalConfig = async (
	input: OhMyOpenCodeLocalConfigInput,
): Promise<void> => {
	await invoke("save_oh_my_opencode_local_config", { input });
};

// ============================================================================
// Types for API
// ============================================================================

export interface OhMyOpenCodeConfigInput {
	id?: string; // Optional - will be generated if not provided
	name: string;
	isApplied?: boolean;
	agents: Record<string, Record<string, unknown>> | null;
	categories?: Record<string, Record<string, unknown>> | null;
	otherFields?: Record<string, unknown>;
}

/**
 * Global Config Input Type - all nested configs are generic JSON
 */
export interface OhMyOpenCodeGlobalConfigInput {
	schema?: string;
	sisyphusAgent?: Record<string, unknown> | null;
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
 * Local Config Input Type - for saving __local__ temporary config to database
 */
export interface OhMyOpenCodeLocalConfigInput {
	config?: OhMyOpenCodeConfigInput;
	globalConfig?: OhMyOpenCodeGlobalConfigInput;
}

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Get all agent definitions
 */
export const getAllAgents = () => {
	return OH_MY_OPENCODE_AGENTS;
};

/**
 * Create a default config input with preset values
 * Note: id is NOT passed - backend will generate it automatically
 */
export const createDefaultOhMyOpenCodeConfig = (
	name: string,
): OhMyOpenCodeConfigInput => {
	return {
		name,
		agents: {
			Sisyphus: { model: "opencode/minimax-m2.1-free" },
			"Prometheus (Planner)": { model: "" },
			Atlas: { model: "" },
			oracle: { model: "" },
			librarian: { model: "" },
			explore: { model: "" },
			"multimodal-looker": { model: "" },
			"frontend-ui-ux-engineer": { model: "" },
			"document-writer": { model: "" },
			"Sisyphus-Junior": { model: "" },
			"Metis (Plan Consultant)": { model: "" },
			"Momus (Plan Reviewer)": { model: "" },
			"OpenCode-Builder": { model: "" },
		},
	};
};

/**
 * Get display name for an agent type
 */
export const getAgentDisplayName = (
	agentType: string,
	t: TFunction,
): string => {
	if (!AGENT_KEYS.has(agentType)) {
		return agentType;
	}
	return (
		getMetaText(t, `opencode.ohMyOpenCode.agentsMeta.${agentType}.name`) ??
		agentType
	);
};

/** Get localized agent description. */
export const getAgentDescription = (
	agentType: string,
	t: TFunction,
): string => {
	if (!AGENT_KEYS.has(agentType)) {
		return "";
	}
	return (
		getMetaText(
			t,
			`opencode.ohMyOpenCode.agentsMeta.${agentType}.description`,
		) ?? ""
	);
};

/**
 * Get localized recommended model for an agent type
 */
export const getAgentRecommendedModel = (
	agentType: string,
	t: TFunction,
): string | undefined => {
	if (!AGENT_KEYS.has(agentType)) {
		return undefined;
	}
	return getMetaText(
		t,
		`opencode.ohMyOpenCode.agentsMeta.${agentType}.recommendedModel`,
	);
};

/**
 * Get display name for a category key
 */
export const getCategoryDisplayName = (
	categoryKey: string,
	t: TFunction,
): string => {
	if (!CATEGORY_KEYS.has(categoryKey)) {
		return categoryKey;
	}
	return (
		getMetaText(
			t,
			`opencode.ohMyOpenCode.categoriesMeta.${categoryKey}.name`,
		) ?? categoryKey
	);
};

/** Get localized category description. */
export const getCategoryDescription = (
	categoryKey: string,
	t: TFunction,
): string => {
	if (!CATEGORY_KEYS.has(categoryKey)) {
		return "";
	}
	return (
		getMetaText(
			t,
			`opencode.ohMyOpenCode.categoriesMeta.${categoryKey}.description`,
		) ?? ""
	);
};

/**
 * Get localized recommended model for a category key
 */
export const getCategoryRecommendedModel = (
	categoryKey: string,
	t: TFunction,
): string | undefined => {
	if (!CATEGORY_KEYS.has(categoryKey)) {
		return undefined;
	}
	return getMetaText(
		t,
		`opencode.ohMyOpenCode.categoriesMeta.${categoryKey}.recommendedModel`,
	);
};
