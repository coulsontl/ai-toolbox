use std::fmt;

/// Every CLI that keeps preset records in the database.
///
/// **This enum is the only place a new CLI has to be registered.** Providers,
/// official accounts and config presets live in three shared tables told apart
/// by the `cli_key` column this enum spells. Before this, each CLI added four
/// tables of its own plus a `migrate_vNN` step; now it is one line here and no
/// schema change at all.
///
/// The strings deliberately match the `app_type` spellings the gateway and the
/// usage tables already use (`gemini`, not `gemini_cli`), so a row's CLI can be
/// matched against request logs without a translation table.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Cli {
    Claude,
    ClaudeDesktop,
    Codex,
    Grok,
    Kimi,
    Gemini,
    Antigravity,
    Zcode,
    Workbuddy,
    Codebuddy,
    Hermes,
    Dsh,
    Pi,
    OhMyPi,
    OmoNative,
    OpenCode,
    OpenClaw,
    OhMyOpenAgent,
    OhMyOpenCodeSlim,
}

impl Cli {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ClaudeDesktop => "claude_desktop",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Kimi => "kimi",
            Self::Gemini => "gemini",
            Self::Antigravity => "antigravity",
            Self::Zcode => "zcode",
            Self::Workbuddy => "workbuddy",
            Self::Codebuddy => "codebuddy",
            Self::Hermes => "hermes",
            Self::Dsh => "dsh",
            Self::Pi => "pi",
            Self::OhMyPi => "oh_my_pi",
            Self::OmoNative => "omo_native",
            Self::OpenCode => "opencode",
            Self::OpenClaw => "openclaw",
            Self::OhMyOpenAgent => "oh_my_openagent",
            Self::OhMyOpenCodeSlim => "oh_my_opencode_slim",
        }
    }

    pub const ALL: &'static [Cli] = &[
        Self::Claude,
        Self::ClaudeDesktop,
        Self::Codex,
        Self::Grok,
        Self::Kimi,
        Self::Gemini,
        Self::Antigravity,
        Self::Zcode,
        Self::Workbuddy,
        Self::Codebuddy,
        Self::Hermes,
        Self::Dsh,
        Self::Pi,
        Self::OhMyPi,
        Self::OmoNative,
        Self::OpenCode,
        Self::OpenClaw,
        Self::OhMyOpenAgent,
        Self::OhMyOpenCodeSlim,
    ];
}

/// Which of the CLI's config namespaces a `cli_config` row belongs to.
///
/// `common` / `prompt` / `settings` are the structured presets the pages edit;
/// `agents` / `global` / the namespaces below are whole JSON blobs with no
/// fixed shape. `favorite_plugin` / `favorite_provider` / `workspace_roots` used
/// to live in per-CLI tables; they are user data keyed by CLI, not by preset
/// kind, so they belong in this shared table with a `cli_key` scope like every
/// other CLI-owned row.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CliConfigKind {
    Common,
    Prompt,
    Settings,
    Agents,
    Global,
    FavoritePlugin,
    FavoriteProvider,
    WorkspaceRoots,
}

impl CliConfigKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Prompt => "prompt",
            Self::Settings => "settings",
            Self::Agents => "agents",
            Self::Global => "global",
            Self::FavoritePlugin => "favorite_plugin",
            Self::FavoriteProvider => "favorite_provider",
            Self::WorkspaceRoots => "workspace_roots",
        }
    }
}

/// Which kind of record a `relay_entity` row holds.
///
/// The 2API tab's channel/model/plugin/check-in records are all low-frequency
/// user configuration with identical JSONB columns, so they share one table.
/// Quota snapshots and quota points are *not* in here: they are rewritten or
/// appended on every poll and would put themselves in the same pages as a
/// handful of channel rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum RelayKind {
    Channel,
    Model,
    Plugin,
    PluginProvider,
    PluginAccount,
    AccessKey,
    CheckinSchedule,
    CheckinRecord,
    ClaimRecord,
}

impl RelayKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Model => "model",
            Self::Plugin => "plugin",
            Self::PluginProvider => "plugin_provider",
            Self::PluginAccount => "plugin_account",
            Self::AccessKey => "access_key",
            Self::CheckinSchedule => "checkin_schedule",
            Self::CheckinRecord => "checkin_record",
            Self::ClaimRecord => "claim_record",
        }
    }
}

/// Which kind of record a `skill_entity` row holds.
///
/// The Skills feature owns several entity types (managed skills, groups, source
/// repos, and two singleton records). They have different JSON shapes but the
/// same lifecycle and query pattern, so they share one JSONB table. `kind` is a
/// real column rather than a JSON field because every payload struct is
/// `#[serde(default)]`; without the column a repo payload would deserialize
/// happily as a skill.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SkillEntityKind {
    Skill,
    Group,
    Repo,
    Preferences,
    Settings,
}

impl SkillEntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Group => "group",
            Self::Repo => "repo",
            Self::Preferences => "preferences",
            Self::Settings => "settings",
        }
    }
}

/// Which kind of record an `mcp_entity` row holds.
///
/// Servers, favourites, managed groups and the singleton preferences record were
/// four separate tables before v35. They share the JSONB row shape and are all
/// low-frequency user configuration, so they now share one scoped table.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum McpEntityKind {
    Server,
    Preferences,
    Favorite,
    Group,
}

impl McpEntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Preferences => "preferences",
            Self::Favorite => "favorite",
            Self::Group => "group",
        }
    }
}

/// Which kind of record a `sync_entity` row holds.
///
/// WSL and SSH are the two remote-sync transports. Their config singletons,
/// default-version markers, file mappings and SSH connections all live in one
/// table, but each gets a distinct kind: the two file-mapping tables reuse the
/// same ids (`opencode-main`, `claude-config`, …) and both config tables use
/// the id `config`, so collapsing them into one kind would overwrite rows. The
/// `defaults_version` rows get their own kinds for the same reason — they are a
/// different record shape from the `config` row and must not appear in its list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SyncEntityKind {
    WslConfig,
    WslDefaultsVersion,
    WslFileMapping,
    SshConfig,
    SshDefaultsVersion,
    SshConnection,
    SshFileMapping,
}

impl SyncEntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WslConfig => "wsl_config",
            Self::WslDefaultsVersion => "wsl_defaults_version",
            Self::WslFileMapping => "wsl_file_mapping",
            Self::SshConfig => "ssh_config",
            Self::SshDefaultsVersion => "ssh_defaults_version",
            Self::SshConnection => "ssh_connection",
            Self::SshFileMapping => "ssh_file_mapping",
        }
    }
}

/// Which kind of record an `image_entity` row holds.
///
/// Channels, generation jobs and image assets are all user-scoped rows in the
/// Image Studio feature. Jobs and assets are history rather than high-frequency
/// state: they are created only when the user starts a generation, so they do
/// not need the separate-table treatment the relay quota tables use.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ImageEntityKind {
    Channel,
    Job,
    Asset,
}

impl ImageEntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Job => "job",
            Self::Asset => "asset",
        }
    }
}

/// The physical row scope a table reads and writes.
///
/// A consolidated table holds rows for many CLIs (or many relay kinds); the
/// scope is what keeps one CLI from seeing another's. It is carried by the
/// [`DbTable`] variant rather than passed alongside it so that **every** helper
/// applies it — `db_update_applied_status` clears `is_applied` across the whole
/// scope before setting the target, and a forgotten `cli_key` there would wipe
/// every other CLI's applied flag without any error.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TableScope {
    pub cli_key: Option<&'static str>,
    pub kind: Option<&'static str>,
}

impl TableScope {
    pub const NONE: Self = Self {
        cli_key: None,
        kind: None,
    };

    pub const fn cli(cli_key: &'static str) -> Self {
        Self {
            cli_key: Some(cli_key),
            kind: None,
        }
    }

    pub const fn cli_kind(cli_key: &'static str, kind: &'static str) -> Self {
        Self {
            cli_key: Some(cli_key),
            kind: Some(kind),
        }
    }

    pub const fn kind(kind: &'static str) -> Self {
        Self {
            cli_key: None,
            kind: Some(kind),
        }
    }

    /// `(column, value)` pairs the scope binds to, in binding order.
    ///
    /// Returned as an iterator rather than a slice because a slice would have to
    /// borrow from a temporary — and copying the pair list into a `Vec` at every
    /// SQL build site is noise for at most two entries.
    pub fn bindings(&self) -> impl Iterator<Item = (&'static str, &'static str)> {
        let cli = self.cli_key.map(|value| ("cli_key", value));
        let kind = self.kind.map(|value| ("kind", value));
        cli.into_iter().chain(kind)
    }

    /// Column names the scope binds to, in binding order.
    pub fn columns(&self) -> impl Iterator<Item = &'static str> {
        self.bindings().map(|(column, _)| column)
    }

    /// Values bound to [`Self::columns`], in the same order.
    pub fn values(&self) -> impl Iterator<Item = &'static str> {
        self.bindings().map(|(_, value)| value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DbTable {
    Settings,

    // ---- per-CLI tables, consolidated ----------------------------------------
    //
    // Three tables instead of fifty-five. `cli_key` is a real column, not a JSON
    // field: every preset struct is `#[serde(default)]`, so a codex provider
    // payload deserializes happily into a kimi provider — the scope has to be
    // enforced by SQL, not by the shape of the data.
    //
    // Every CLI-owned row lives here; the only way to add a new kind of CLI data
    // is a new `CliConfigKind`, never a per-CLI table.
    CliProvider(Cli),
    CliOfficialAccount(Cli),
    CliConfig(Cli, CliConfigKind),

    // ---- feature-domain tables, consolidated ---------------------------------
    //
    // Four tables instead of seventeen. Each domain's entity types share the
    // same JSONB columns and lifecycle; the `kind` column (carried by the
    // variant) keeps them from reading each other's rows. `custom_tool` stays
    // separate: it is a cross-domain tool registry shared by Skills and MCP,
    // not an entity owned by either.
    SkillEntity(SkillEntityKind),
    McpEntity(McpEntityKind),
    SyncEntity(SyncEntityKind),
    ImageEntity(ImageEntityKind),

    CustomTool,
    ProxyGatewaySettings,
    UsageSessionState,

    // ---- relay ("2API") -----------------------------------------------------
    RelayEntity(RelayKind),
    /// Current balance/usage per account, rewritten on every poll.
    RelayQuotaSnapshot,
    /// One appended point per poll, for the trend chart.
    RelayQuotaPoint,
}

/// Every physical JSONB table the app creates at startup.
///
/// Kept separate from [`DbTable`] because that enum is a *scoped reference* to a
/// table, not a table: `CliProvider(Codex)` and `CliProvider(Kimi)` are both
/// `cli_provider`, and creating a table per variant would be a no-op at best.
pub const ALL_TABLES: &[DbTable] = &[
    DbTable::Settings,
    DbTable::CliProvider(Cli::Claude),
    DbTable::CliOfficialAccount(Cli::Claude),
    DbTable::CliConfig(Cli::Claude, CliConfigKind::Common),
    DbTable::SkillEntity(SkillEntityKind::Skill),
    DbTable::CustomTool,
    DbTable::McpEntity(McpEntityKind::Server),
    DbTable::SyncEntity(SyncEntityKind::WslConfig),
    DbTable::ProxyGatewaySettings,
    DbTable::UsageSessionState,
    DbTable::ImageEntity(ImageEntityKind::Channel),
    DbTable::RelayEntity(RelayKind::Channel),
    DbTable::RelayQuotaSnapshot,
    DbTable::RelayQuotaPoint,
];

/// Physical table names in creation order, deduplicated.
///
/// Migrations and consistency tests need the table list without inventing a
/// scope just to call [`DbTable::name`].
pub fn all_table_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for table in ALL_TABLES {
        let name = table.name();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

impl DbTable {
    /// Physical table this variant reads and writes.
    pub fn name(self) -> &'static str {
        match self {
            DbTable::Settings => "settings",
            DbTable::CliProvider(_) => "cli_provider",
            DbTable::CliOfficialAccount(_) => "cli_official_account",
            DbTable::CliConfig(..) => "cli_config",
            DbTable::SkillEntity(_) => "skill_entity",
            DbTable::McpEntity(_) => "mcp_entity",
            DbTable::SyncEntity(_) => "sync_entity",
            DbTable::ImageEntity(_) => "image_entity",
            DbTable::CustomTool => "custom_tool",
            DbTable::ProxyGatewaySettings => "proxy_gateway_settings",
            DbTable::UsageSessionState => "usage_session_state",
            DbTable::RelayEntity(_) => "relay_entity",
            DbTable::RelayQuotaSnapshot => "relay_quota_snapshot",
            DbTable::RelayQuotaPoint => "relay_quota_point",
        }
    }

    /// Rows this variant may touch. [`TableScope::NONE`] means the whole table.
    pub fn scope(self) -> TableScope {
        match self {
            DbTable::CliProvider(cli) => TableScope::cli(cli.as_str()),
            DbTable::CliOfficialAccount(cli) => TableScope::cli(cli.as_str()),
            DbTable::CliConfig(cli, kind) => TableScope::cli_kind(cli.as_str(), kind.as_str()),
            DbTable::SkillEntity(kind) => TableScope::kind(kind.as_str()),
            DbTable::McpEntity(kind) => TableScope::kind(kind.as_str()),
            DbTable::SyncEntity(kind) => TableScope::kind(kind.as_str()),
            DbTable::ImageEntity(kind) => TableScope::kind(kind.as_str()),
            DbTable::RelayEntity(kind) => TableScope::kind(kind.as_str()),
            _ => TableScope::NONE,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_physical_table_is_listed_once() {
        let names = all_table_names();
        assert_eq!(
            names.len(),
            14,
            "the consolidated schema has 14 physical JSONB tables; a different \
             count means ALL_TABLES and this expectation drifted apart"
        );
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "ALL_TABLES must name each physical table once"
        );
    }

    #[test]
    fn consolidated_tables_are_named_once_but_scoped_many_times() {
        let provider_scopes: HashSet<_> = Cli::ALL
            .iter()
            .map(|cli| DbTable::CliProvider(*cli).scope().bindings().next().unwrap().1)
            .collect();
        assert_eq!(provider_scopes.len(), Cli::ALL.len());
        assert!(provider_scopes.contains("codex") && provider_scopes.contains("oh_my_pi"));

        // Two scopes, one table: the whole point of the consolidation.
        assert_eq!(
            DbTable::CliProvider(Cli::Codex).name(),
            DbTable::CliProvider(Cli::Kimi).name()
        );
    }

    #[test]
    fn scope_columns_and_values_stay_aligned() {
        let cases = [
            DbTable::Settings,
            DbTable::CliProvider(Cli::Grok),
            DbTable::CliConfig(Cli::Kimi, CliConfigKind::Prompt),
            DbTable::SkillEntity(SkillEntityKind::Group),
            DbTable::McpEntity(McpEntityKind::Favorite),
            DbTable::SyncEntity(SyncEntityKind::SshFileMapping),
            DbTable::ImageEntity(ImageEntityKind::Job),
            DbTable::RelayEntity(RelayKind::Channel),
            DbTable::RelayQuotaSnapshot,
        ];
        for table in cases {
            let bindings: Vec<(&str, &str)> = table.scope().bindings().collect();
            let columns: Vec<&str> = table.scope().columns().collect();
            assert_eq!(bindings.len(), columns.len(), "{table:?} scope arity");
            assert!(bindings
                .iter()
                .zip(columns.iter())
                .all(|((column, _), listed)| column == listed));
        }
        assert_eq!(DbTable::Settings.scope().bindings().count(), 0);
        assert_eq!(
            DbTable::CliConfig(Cli::Kimi, CliConfigKind::Prompt).scope(),
            TableScope::cli_kind("kimi", "prompt")
        );
    }

    #[test]
    fn the_three_cli_tables_cover_every_cli() {
        for cli in Cli::ALL {
            for table in [
                DbTable::CliProvider(*cli),
                DbTable::CliOfficialAccount(*cli),
                DbTable::CliConfig(*cli, CliConfigKind::Common),
            ] {
                assert_eq!(table.scope().cli_key, Some(cli.as_str()));
            }
        }
    }

    #[test]
    fn feature_entity_tables_are_named_once_but_scoped_many_times() {
        let cases: [(&str, Vec<DbTable>); 4] = [
            (
                "skill_entity",
                vec![
                    DbTable::SkillEntity(SkillEntityKind::Skill),
                    DbTable::SkillEntity(SkillEntityKind::Group),
                    DbTable::SkillEntity(SkillEntityKind::Repo),
                    DbTable::SkillEntity(SkillEntityKind::Preferences),
                    DbTable::SkillEntity(SkillEntityKind::Settings),
                ],
            ),
            (
                "mcp_entity",
                vec![
                    DbTable::McpEntity(McpEntityKind::Server),
                    DbTable::McpEntity(McpEntityKind::Preferences),
                    DbTable::McpEntity(McpEntityKind::Favorite),
                    DbTable::McpEntity(McpEntityKind::Group),
                ],
            ),
            (
                "sync_entity",
                vec![
                    DbTable::SyncEntity(SyncEntityKind::WslConfig),
                    DbTable::SyncEntity(SyncEntityKind::WslDefaultsVersion),
                    DbTable::SyncEntity(SyncEntityKind::WslFileMapping),
                    DbTable::SyncEntity(SyncEntityKind::SshConfig),
                    DbTable::SyncEntity(SyncEntityKind::SshDefaultsVersion),
                    DbTable::SyncEntity(SyncEntityKind::SshConnection),
                    DbTable::SyncEntity(SyncEntityKind::SshFileMapping),
                ],
            ),
            (
                "image_entity",
                vec![
                    DbTable::ImageEntity(ImageEntityKind::Channel),
                    DbTable::ImageEntity(ImageEntityKind::Job),
                    DbTable::ImageEntity(ImageEntityKind::Asset),
                ],
            ),
        ];

        for (expected_name, tables) in cases {
            let mut kinds = HashSet::new();
            for table in tables {
                assert_eq!(table.name(), expected_name);
                assert_eq!(table.scope().cli_key, None);
                let kind = table.scope().kind.expect("feature tables are kind-scoped");
                assert!(kinds.insert(kind), "duplicate kind {kind} in {expected_name}");
            }
        }
    }
}
