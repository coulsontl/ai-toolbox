import { invoke } from "@tauri-apps/api/core";
import type { TFunction } from "i18next";
import type {
	OhMyOpenAgentConfig,
	OhMyOpenAgentGlobalConfig,
	OhMyOpenAgentLegacyUpgradeResult,
	OhMyOpenAgentLegacyUpgradeStatus,
} from "@/types/ohMyOpenAgent";
import {
	OH_MY_OPENAGENT_AGENTS,
	OH_MY_OPENAGENT_CATEGORIES,
} from "@/types/ohMyOpenAgent";

const AGENT_KEYS = new Set(OH_MY_OPENAGENT_AGENTS.map((agent) => agent.key));
const CATEGORY_KEYS = new Set(
	OH_MY_OPENAGENT_CATEGORIES.map((category) => category.key),
);

function getMetaText(t: TFunction, key: string): string | undefined {
	const value = t(key);
	return value === key ? undefined : value;
}

// ============================================================================
// Oh My OpenAgent API
// ============================================================================

/**
 * List all omo configurations
 */
export const listOhMyOpenAgentConfigs = async (): Promise<
	OhMyOpenAgentConfig[]
> => {
	return await invoke<OhMyOpenAgentConfig[]>("list_oh_my_openagent_configs");
};

/**
 * Create a new Oh My OpenAgent configuration
 */
export const createOhMyOpenAgentConfig = async (
	config: OhMyOpenAgentConfigInput,
): Promise<OhMyOpenAgentConfig> => {
	return await invoke<OhMyOpenAgentConfig>("create_oh_my_openagent_config", {
		input: config,
	});
};

/**
 * Update an existing Oh My OpenAgent configuration
 */
export const updateOhMyOpenAgentConfig = async (
	config: OhMyOpenAgentConfigInput,
): Promise<OhMyOpenAgentConfig> => {
	return await invoke<OhMyOpenAgentConfig>("update_oh_my_openagent_config", {
		input: config,
	});
};

/**
 * Delete an existing Oh My OpenAgent configuration
 */
export const deleteOhMyOpenAgentConfig = async (id: string): Promise<void> => {
	await invoke("delete_oh_my_openagent_config", { id });
};

/**
 * Clear the applied Oh My OpenAgent runtime config file without deleting the saved profile.
 */
export const clearOhMyOpenAgentAppliedConfig = async (
	configId: string,
): Promise<void> => {
	await invoke("clear_oh_my_openagent_applied_config", { configId });
};

/**
 * Apply a configuration to the Oh My OpenAgent config file
 */
export const applyOhMyOpenAgentConfig = async (
	configId: string,
): Promise<void> => {
	await invoke("apply_oh_my_openagent_config", { configId });
};

/**
 * Reorder configurations
 */
export const reorderOhMyOpenAgentConfigs = async (
	ids: string[],
): Promise<void> => {
	await invoke("reorder_oh_my_openagent_configs", { ids });
};

/**
 * Toggle is_disabled status for a config
 */
export async function toggleOhMyOpenAgentConfigDisabled(
	configId: string,
	isDisabled: boolean,
): Promise<void> {
	return invoke("toggle_oh_my_openagent_config_disabled", {
		configId,
		isDisabled,
	});
}

/**
 * Get config file path info
 */
export const getOhMyOpenAgentConfigPathInfo = async (): Promise<{
	path: string;
	source: string;
}> => {
	return await invoke("get_oh_my_openagent_config_path_info");
};

/**
 * Check if a local Oh My OpenAgent config file exists
 * Returns true if either the canonical or legacy config filename exists
 */
export const checkOhMyOpenAgentConfigExists = async (): Promise<boolean> => {
	return await invoke<boolean>("check_oh_my_openagent_config_exists");
};

// ============================================================================
// Oh My OpenAgent Global Config API
// ============================================================================

/**
 * Get global config (从 oh_my_openagent_global_config 表读取)
 */
export const getOhMyOpenAgentGlobalConfig =
	async (): Promise<OhMyOpenAgentGlobalConfig> => {
		return await invoke<OhMyOpenAgentGlobalConfig>(
			"get_oh_my_openagent_global_config",
		);
	};

/**
 * Save global config (保存到 oh_my_openagent_global_config 表)
 */
export const saveOhMyOpenAgentGlobalConfig = async (
	config: OhMyOpenAgentGlobalConfigInput,
): Promise<OhMyOpenAgentGlobalConfig> => {
	return await invoke<OhMyOpenAgentGlobalConfig>(
		"save_oh_my_openagent_global_config",
		{ input: config },
	);
};

/**
 * Save local config (both Agents Profile and Global Config) into database
 * This is used when saving __local__ temporary config to database
 */
export const saveOhMyOpenAgentLocalConfig = async (
	input: OhMyOpenAgentLocalConfigInput,
): Promise<void> => {
	await invoke("save_oh_my_openagent_local_config", { input });
};

/**
 * Get legacy Oh My OpenAgent upgrade status.
 */
export const getOhMyOpenAgentUpgradeStatus =
	async (): Promise<OhMyOpenAgentLegacyUpgradeStatus> => {
		return await invoke<OhMyOpenAgentLegacyUpgradeStatus>(
			"get_oh_my_openagent_upgrade_status",
		);
	};

/**
 * Upgrade legacy Oh My OpenCode plugin/file naming to Oh My OpenAgent.
 */
export const upgradeOhMyOpenAgentLegacySetup =
	async (): Promise<OhMyOpenAgentLegacyUpgradeResult> => {
		return await invoke<OhMyOpenAgentLegacyUpgradeResult>(
			"upgrade_oh_my_openagent_legacy_setup",
		);
	};

// ============================================================================
// Types for API
// ============================================================================

export interface OhMyOpenAgentConfigInput {
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
export interface OhMyOpenAgentGlobalConfigInput {
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
export interface OhMyOpenAgentLocalConfigInput {
	config?: OhMyOpenAgentConfigInput;
	globalConfig?: OhMyOpenAgentGlobalConfigInput;
}

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Get all agent definitions
 */
export const getAllAgents = () => {
	return OH_MY_OPENAGENT_AGENTS;
};

/**
 * Create a default config input with preset values
 * Note: id is NOT passed - backend will generate it automatically
 */
export const createDefaultOhMyOpenAgentConfig = (
	name: string,
): OhMyOpenAgentConfigInput => {
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
export const getOpenAgentDisplayName = (
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
export const getOpenAgentDescription = (
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
export const getOpenAgentRecommendedModel = (
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
export const getOpenAgentCategoryDisplayName = (
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
export const getOpenAgentCategoryDescription = (
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
export const getOpenAgentCategoryRecommendedModel = (
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
