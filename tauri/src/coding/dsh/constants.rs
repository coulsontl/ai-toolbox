//! DeepSeek Harness (dsh) path / provider constants.

/// Environment variable used by dsh to override its config directory (`~/.dsh`).
pub const DSH_ENV_KEY: &str = "DSH_HOME";
/// The namespaced YAML file dsh reads for its stack configuration.
pub const DSH_SETTINGS_FILE: &str = "settings.yaml";
/// Separate credentials file holding `REF: secret` entries (REF is a POSIX env var name).
pub const DSH_CREDENTIALS_FILE: &str = ".credentials.yaml";
/// Global prompt file authored into the config directory.
pub const DSH_PROMPT_FILE: &str = "AGENTS.md";
/// The Cordis patch DSL file the `mcp::cordis_patch` adapter writes MCP servers into.
/// Each MCP server is one `insert` row in a top-level YAML array within this file.
pub const DSH_MCP_FILE: &str = "cordis.patch.yml";
/// Per-profile patch layer file (`profiles/<profile>/cordis.patch.yml`).
///
/// dsh >= 0.1.7 keeps live configuration here instead of `settings.yaml`;
/// the same file name is used at the home level for MCP rows.
pub const DSH_PATCH_FILE: &str = "cordis.patch.yml";
/// The one-shot import source dsh writes after moving `settings.yaml` into the
/// profile patch layer; read-only history for this app.
pub const DSH_IMPORTED_SETTINGS_FILE: &str = "settings.yaml.imported";
/// Plugin package name of the `llm-pi-ai` row.
pub const DSH_LLM_PI_AI_PACKAGE: &str = "@deepseek-ai/dsh-llm-pi-ai";
/// Plugin package name of the `agent-default-model` row.
pub const DSH_DEFAULT_MODEL_PACKAGE: &str = "@deepseek-ai/dsh-agent-default-model";

/// The settings.yaml section under which llm-pi-ai plugin providers are stored
/// as a dictionary keyed by route id (`settings["llm-pi-ai"]["providers"][route]`).
pub const DSH_LLM_PI_AI_SECTION: &str = "llm-pi-ai";
/// The providers sub-key of the llm-pi-ai section.
pub const DSH_PROVIDERS_KEY: &str = "providers";
/// The top-level default-model section (`{ provider, model, reasoningEffort }`).
pub const DSH_DEFAULT_MODEL_SECTION: &str = "agent-default-model";

/// Versioned `.credentials.yaml` layout (dsh >= 0.1.1-rc.1): top-level `version`
/// marker whose presence moves every ref entry under a nested mapping.
pub const DSH_CREDENTIALS_VERSION_KEY: &str = "version";
/// Document version this app reads and writes (`refs:` nesting + `records:`).
pub const DSH_CREDENTIALS_VERSION: i64 = 1;
/// Nested key holding the ref entries in the versioned credentials layout.
pub const DSH_CREDENTIALS_REFS_KEY: &str = "refs";
/// Nested key holding sign-in credential records (api-key / OAuth grant) that
/// dsh's own login flow writes; managed by dsh, never written by this app.
pub const DSH_CREDENTIALS_RECORDS_KEY: &str = "records";
/// Record scope of the stored sign-in credentials (`<scope>/<provider_id>`
/// record keys), mirroring `RECORD_SCOPE` in upstream `llm-pi-ai/src/auth.ts`.
pub const DSH_CREDENTIAL_RECORD_SCOPE: &str = DSH_LLM_PI_AI_SECTION;

/// Installed pi-ai catalog routes the `llm-pi-ai` adapter serves, with display
/// names.
///
/// Mirrors `catalogProviderIds()` in upstream `llm-pi-ai/src/catalog.ts` for
/// `@earendil-works/pi-ai@1.1.0`: every provider that ships a chat model, plus
/// catalog entries that list no models. `typesafe` is excluded because its
/// catalog holds only a non-chat model, so the adapter does not offer it.
///
/// The list drives display names and the built-in tag only. Removability is not
/// decided by catalog membership: upstream shows remove for a route whose
/// profile exists in the user layer, which here is an
/// `llm-pi-ai.providers.<route>` entry (or a stored credential). The
/// authoritative provider fact source remains that configuration.
pub const DSH_BUILTIN_PROVIDERS: [(&str, &str); 41] = [
    ("amazon-bedrock", "Amazon Bedrock"),
    ("ant-ling", "Ant Ling"),
    ("anthropic", "Anthropic"),
    ("azure", "Azure OpenAI"),
    ("baseten", "Baseten"),
    ("cerebras", "Cerebras"),
    ("cloudflare-ai-gateway", "Cloudflare AI Gateway"),
    ("cloudflare-workers-ai", "Cloudflare Workers AI"),
    ("deepseek", "DeepSeek"),
    ("fireworks", "Fireworks"),
    ("github-copilot", "GitHub Copilot"),
    ("google", "Google"),
    ("google-vertex", "Google Vertex"),
    ("groq", "Groq"),
    ("huggingface", "Hugging Face"),
    ("kimi-coding", "Kimi Coding"),
    ("meta", "Meta"),
    ("minimax", "MiniMax"),
    ("minimax-cn", "MiniMax (CN)"),
    ("mistral", "Mistral"),
    ("moonshotai", "Moonshot AI"),
    ("moonshotai-cn", "Moonshot AI (CN)"),
    ("nvidia", "NVIDIA"),
    ("openai", "OpenAI"),
    ("openai-codex", "OpenAI Codex"),
    ("opencode", "OpenCode"),
    ("opencode-go", "OpenCode Go"),
    ("openrouter", "OpenRouter"),
    ("qwen-token-plan", "Qwen Token Plan"),
    ("qwen-token-plan-cn", "Qwen Token Plan (CN)"),
    ("qwen-token-plan-individual", "Qwen Token Plan (Individual)"),
    ("radius", "Radius"),
    ("together", "Together"),
    ("vercel-ai-gateway", "Vercel AI Gateway"),
    ("xai", "xAI"),
    ("xiaomi", "Xiaomi"),
    ("xiaomi-token-plan-ams", "Xiaomi Token Plan (AMS)"),
    ("xiaomi-token-plan-cn", "Xiaomi Token Plan (CN)"),
    ("xiaomi-token-plan-sgp", "Xiaomi Token Plan (SGP)"),
    ("zai", "Z.ai"),
    ("zai-coding-cn", "Z.ai Coding (CN)"),
];

pub fn is_builtin_provider(provider_key: &str) -> bool {
    DSH_BUILTIN_PROVIDERS
        .iter()
        .any(|(key, _)| *key == provider_key)
}

pub fn builtin_provider_name(provider_key: &str) -> Option<&'static str> {
    DSH_BUILTIN_PROVIDERS
        .iter()
        .find_map(|(key, name)| (*key == provider_key).then_some(*name))
}
