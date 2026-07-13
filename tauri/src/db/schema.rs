use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DbTable {
    Settings,
    AppMigration,
    ClaudeProvider,
    ClaudeCommonConfig,
    ClaudePromptConfig,
    CodexProvider,
    CodexCommonConfig,
    CodexPromptConfig,
    CodexOfficialAccount,
    CodexPluginWorkspaceRoots,
    GrokProvider,
    GrokOfficialAccount,
    GrokCommonConfig,
    GrokPromptConfig,
    GeminiCliProvider,
    GeminiCliCommonConfig,
    GeminiCliPromptConfig,
    GeminiCliOfficialAccount,
    PiSettingsConfig,
    PiPromptConfig,
    OpenCodeCommonConfig,
    OpenCodePromptConfig,
    OpenCodeFavoritePlugin,
    OpenCodeFavoriteProvider,
    OpenClawCommonConfig,
    OhMyOpenAgentConfig,
    OhMyOpenAgentGlobalConfig,
    OhMyOpenCodeSlimConfig,
    OhMyOpenCodeSlimGlobalConfig,
    Skill,
    SkillGroup,
    SkillRepo,
    SkillPreferences,
    SkillSettings,
    CustomTool,
    McpServer,
    McpPreferences,
    FavoriteMcp,
    WslSyncConfig,
    WslFileMapping,
    SshSyncConfig,
    SshConnection,
    SshFileMapping,
    ProxyGatewaySettings,
    ImageChannel,
    ImageJob,
    ImageAsset,
}

pub const ALL_TABLES: &[DbTable] = &[
    DbTable::Settings,
    DbTable::AppMigration,
    DbTable::ClaudeProvider,
    DbTable::ClaudeCommonConfig,
    DbTable::ClaudePromptConfig,
    DbTable::CodexProvider,
    DbTable::CodexCommonConfig,
    DbTable::CodexPromptConfig,
    DbTable::CodexOfficialAccount,
    DbTable::CodexPluginWorkspaceRoots,
    DbTable::GrokProvider,
    DbTable::GrokOfficialAccount,
    DbTable::GrokCommonConfig,
    DbTable::GrokPromptConfig,
    DbTable::GeminiCliProvider,
    DbTable::GeminiCliCommonConfig,
    DbTable::GeminiCliPromptConfig,
    DbTable::GeminiCliOfficialAccount,
    DbTable::PiSettingsConfig,
    DbTable::PiPromptConfig,
    DbTable::OpenCodeCommonConfig,
    DbTable::OpenCodePromptConfig,
    DbTable::OpenCodeFavoritePlugin,
    DbTable::OpenCodeFavoriteProvider,
    DbTable::OpenClawCommonConfig,
    DbTable::OhMyOpenAgentConfig,
    DbTable::OhMyOpenAgentGlobalConfig,
    DbTable::OhMyOpenCodeSlimConfig,
    DbTable::OhMyOpenCodeSlimGlobalConfig,
    DbTable::Skill,
    DbTable::SkillGroup,
    DbTable::SkillRepo,
    DbTable::SkillPreferences,
    DbTable::SkillSettings,
    DbTable::CustomTool,
    DbTable::McpServer,
    DbTable::McpPreferences,
    DbTable::FavoriteMcp,
    DbTable::WslSyncConfig,
    DbTable::WslFileMapping,
    DbTable::SshSyncConfig,
    DbTable::SshConnection,
    DbTable::SshFileMapping,
    DbTable::ProxyGatewaySettings,
    DbTable::ImageChannel,
    DbTable::ImageJob,
    DbTable::ImageAsset,
];

impl DbTable {
    pub fn name(self) -> &'static str {
        match self {
            DbTable::Settings => "settings",
            DbTable::AppMigration => "app_migration",
            DbTable::ClaudeProvider => "claude_provider",
            DbTable::ClaudeCommonConfig => "claude_common_config",
            DbTable::ClaudePromptConfig => "claude_prompt_config",
            DbTable::CodexProvider => "codex_provider",
            DbTable::CodexCommonConfig => "codex_common_config",
            DbTable::CodexPromptConfig => "codex_prompt_config",
            DbTable::CodexOfficialAccount => "codex_official_account",
            DbTable::CodexPluginWorkspaceRoots => "codex_plugin_workspace_roots",
            DbTable::GrokProvider => "grok_provider",
            DbTable::GrokOfficialAccount => "grok_official_account",
            DbTable::GrokCommonConfig => "grok_common_config",
            DbTable::GrokPromptConfig => "grok_prompt_config",
            DbTable::GeminiCliProvider => "gemini_cli_provider",
            DbTable::GeminiCliCommonConfig => "gemini_cli_common_config",
            DbTable::GeminiCliPromptConfig => "gemini_cli_prompt_config",
            DbTable::GeminiCliOfficialAccount => "gemini_cli_official_account",
            DbTable::PiSettingsConfig => "pi_settings_config",
            DbTable::PiPromptConfig => "pi_prompt_config",
            DbTable::OpenCodeCommonConfig => "opencode_common_config",
            DbTable::OpenCodePromptConfig => "opencode_prompt_config",
            DbTable::OpenCodeFavoritePlugin => "opencode_favorite_plugin",
            DbTable::OpenCodeFavoriteProvider => "opencode_favorite_provider",
            DbTable::OpenClawCommonConfig => "openclaw_common_config",
            DbTable::OhMyOpenAgentConfig => "oh_my_openagent_config",
            DbTable::OhMyOpenAgentGlobalConfig => "oh_my_openagent_global_config",
            DbTable::OhMyOpenCodeSlimConfig => "oh_my_opencode_slim_config",
            DbTable::OhMyOpenCodeSlimGlobalConfig => "oh_my_opencode_slim_global_config",
            DbTable::Skill => "skill",
            DbTable::SkillGroup => "skill_group",
            DbTable::SkillRepo => "skill_repo",
            DbTable::SkillPreferences => "skill_preferences",
            DbTable::SkillSettings => "skill_settings",
            DbTable::CustomTool => "custom_tool",
            DbTable::McpServer => "mcp_server",
            DbTable::McpPreferences => "mcp_preferences",
            DbTable::FavoriteMcp => "favorite_mcp",
            DbTable::WslSyncConfig => "wsl_sync_config",
            DbTable::WslFileMapping => "wsl_file_mapping",
            DbTable::SshSyncConfig => "ssh_sync_config",
            DbTable::SshConnection => "ssh_connection",
            DbTable::SshFileMapping => "ssh_file_mapping",
            DbTable::ProxyGatewaySettings => "proxy_gateway_settings",
            DbTable::ImageChannel => "image_channel",
            DbTable::ImageJob => "image_job",
            DbTable::ImageAsset => "image_asset",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ValidatedTableName(String);

impl ValidatedTableName {
    pub fn new(name: &str) -> Result<Self, String> {
        validate_identifier(name)?;
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ValidatedTableName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct JsonFieldPath {
    segments: Vec<String>,
}

impl JsonFieldPath {
    pub fn new(path: &str) -> Result<Self, String> {
        let segments: Vec<String> = path
            .split('.')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .map(|segment| {
                validate_identifier(segment)?;
                Ok(segment.to_string())
            })
            .collect::<Result<_, String>>()?;

        if segments.is_empty() {
            return Err("JSON field path cannot be empty".to_string());
        }

        Ok(Self { segments })
    }

    pub fn from_segments(segments: &[&str]) -> Result<Self, String> {
        if segments.is_empty() {
            return Err("JSON field path cannot be empty".to_string());
        }

        let mut validated_segments = Vec::with_capacity(segments.len());
        for segment in segments {
            validate_identifier(segment)?;
            validated_segments.push((*segment).to_string());
        }

        Ok(Self {
            segments: validated_segments,
        })
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn to_sql_path(&self) -> String {
        format!("$.{}", self.segments.join("."))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderDirection {
    Asc,
    Desc,
}

impl OrderDirection {
    fn sql(self) -> &'static str {
        match self {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonValueKind {
    Text,
    Integer,
    Real,
    Boolean,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrderTarget {
    Column(&'static str),
    Json {
        path: JsonFieldPath,
        kind: JsonValueKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderField {
    target: OrderTarget,
    direction: OrderDirection,
}

impl OrderField {
    pub fn id(direction: OrderDirection) -> Self {
        Self {
            target: OrderTarget::Column("id"),
            direction,
        }
    }

    pub fn created_at(direction: OrderDirection) -> Self {
        Self {
            target: OrderTarget::Column("created_at"),
            direction,
        }
    }

    pub fn updated_at(direction: OrderDirection) -> Self {
        Self {
            target: OrderTarget::Column("updated_at"),
            direction,
        }
    }

    pub fn json_text(path: &str, direction: OrderDirection) -> Result<Self, String> {
        Self::json(path, JsonValueKind::Text, direction)
    }

    pub fn json_integer(path: &str, direction: OrderDirection) -> Result<Self, String> {
        Self::json(path, JsonValueKind::Integer, direction)
    }

    pub fn json_bool(path: &str, direction: OrderDirection) -> Result<Self, String> {
        Self::json(path, JsonValueKind::Boolean, direction)
    }

    fn json(path: &str, kind: JsonValueKind, direction: OrderDirection) -> Result<Self, String> {
        Ok(Self {
            target: OrderTarget::Json {
                path: JsonFieldPath::new(path)?,
                kind,
            },
            direction,
        })
    }

    pub fn to_sql(&self) -> String {
        let expression = match &self.target {
            OrderTarget::Column(column) => (*column).to_string(),
            OrderTarget::Json { path, kind } => {
                let json_path = sql_string_literal(&path.to_sql_path());
                match kind {
                    JsonValueKind::Text => format!("json_extract(data, {json_path})"),
                    JsonValueKind::Integer | JsonValueKind::Boolean => {
                        format!("CAST(json_extract(data, {json_path}) AS INTEGER)")
                    }
                    JsonValueKind::Real => {
                        format!("CAST(json_extract(data, {json_path}) AS REAL)")
                    }
                }
            }
        };

        format!("{} {}", expression, self.direction.sql())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OrderSpec {
    fields: Vec<OrderField>,
}

impl OrderSpec {
    pub fn new(fields: Vec<OrderField>) -> Self {
        Self { fields }
    }

    pub fn single(field: OrderField) -> Self {
        Self {
            fields: vec![field],
        }
    }

    pub fn to_sql(&self) -> String {
        if self.fields.is_empty() {
            String::new()
        } else {
            format!(
                " ORDER BY {}",
                self.fields
                    .iter()
                    .map(OrderField::to_sql)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

pub fn validate_identifier(identifier: &str) -> Result<(), String> {
    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        return Err("Identifier cannot be empty".to_string());
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!(
            "Invalid identifier '{}': must start with a letter or underscore",
            identifier
        ));
    }

    if chars.any(|char| !(char.is_ascii_alphanumeric() || char == '_')) {
        return Err(format!(
            "Invalid identifier '{}': only letters, numbers, and underscores are allowed",
            identifier
        ));
    }

    Ok(())
}

pub fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
