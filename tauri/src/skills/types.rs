use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Skill record stored in SurrealDB (wide table pattern)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub source_type: String, // "local" | "git" | "import"
    pub source_ref: Option<String>,
    pub source_revision: Option<String>,
    pub central_path: String,
    pub content_hash: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_sync_at: Option<i64>,
    pub status: String,

    // Sort order for drag-and-drop reordering
    pub sort_index: i32,

    // Enabled tool keys list
    pub enabled_tools: Vec<String>, // ["claude_code", "codex", "opencode"]

    // Sync details JSON (per-tool target_path/mode/status etc.)
    // Structure: { "claude_code": { "target_path": "...", "mode": "...", ... }, ... }
    pub sync_details: Option<Value>,
}

/// Skill target info - used within sync_details (no longer a separate table)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillTarget {
    pub tool: String,
    pub target_path: String,
    pub mode: String, // "symlink" | "copy" | "junction"
    pub status: String,
    pub synced_at: Option<i64>,
    pub error_message: Option<String>,
}

/// Skill repository source - user configured skill source repos
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillRepo {
    pub id: String,     // Format: "owner/name"
    pub owner: String,
    pub name: String,
    pub branch: String, // default: "main"
    pub enabled: bool,  // default: true
    pub created_at: i64,
}

/// Skill preferences - user preference settings (structured wide table)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillPreferences {
    pub id: String, // Fixed "default"
    pub central_repo_path: String,
    pub preferred_tools: Option<Vec<String>>, // User selected preferred tools
    pub git_cache_cleanup_days: i32,
    pub git_cache_ttl_secs: i32,
    pub known_tool_versions: Option<Value>,
    pub installed_tools: Option<Vec<String>>, // Detected installed tools
    pub updated_at: i64,
}

impl Default for SkillPreferences {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            central_repo_path: dirs::home_dir()
                .map(|p| p.join(".skills").to_string_lossy().to_string())
                .unwrap_or_default(),
            preferred_tools: None,
            git_cache_cleanup_days: 30,
            git_cache_ttl_secs: 60,
            known_tool_versions: None,
            installed_tools: None,
            updated_at: 0,
        }
    }
}

/// Tool detection status
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDetection {
    pub tool: String,
    pub installed: bool,
    pub skills_dir: Option<String>,
    pub detected_at: i64,
    pub first_seen_at: Option<i64>,
}

/// DTO for tool status response
#[derive(Debug, Serialize)]
pub struct ToolStatusDto {
    pub tools: Vec<ToolInfoDto>,
    pub installed: Vec<String>,
    pub newly_installed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolInfoDto {
    pub key: String,
    pub label: String,
    pub installed: bool,
    pub skills_dir: String,
}

/// DTO for managed skills (frontend display)
#[derive(Debug, Serialize)]
pub struct ManagedSkillDto {
    pub id: String,
    pub name: String,
    pub source_type: String,
    pub source_ref: Option<String>,
    pub central_path: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_sync_at: Option<i64>,
    pub status: String,
    pub sort_index: i32,
    pub enabled_tools: Vec<String>,
    pub targets: Vec<SkillTargetDto>, // Derived from sync_details
}

#[derive(Debug, Serialize)]
pub struct SkillTargetDto {
    pub tool: String,
    pub mode: String,
    pub status: String,
    pub target_path: String,
    pub synced_at: Option<i64>,
}

/// DTO for install result
#[derive(Debug, Serialize)]
pub struct InstallResultDto {
    pub skill_id: String,
    pub name: String,
    pub central_path: String,
    pub content_hash: Option<String>,
}

/// DTO for sync result
#[derive(Debug, Serialize)]
pub struct SyncResultDto {
    pub mode_used: String,
    pub target_path: String,
}

/// DTO for update result
#[derive(Debug, Serialize)]
pub struct UpdateResultDto {
    pub skill_id: String,
    pub name: String,
    pub content_hash: Option<String>,
    pub source_revision: Option<String>,
    pub updated_targets: Vec<String>,
}

/// Git skill candidate for multi-skill repos
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitSkillCandidate {
    pub name: String,
    pub description: Option<String>,
    pub subpath: String,
}

/// Onboarding plan for discovered skills
#[derive(Clone, Debug, Serialize)]
pub struct OnboardingPlan {
    pub total_tools_scanned: usize,
    pub total_skills_found: usize,
    pub groups: Vec<OnboardingGroup>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OnboardingGroup {
    pub name: String,
    pub variants: Vec<OnboardingVariant>,
    pub has_conflict: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OnboardingVariant {
    pub tool: String,
    pub name: String,
    pub path: String,
    pub fingerprint: Option<String>,
    pub is_link: bool,
    pub link_target: Option<String>,
    /// Tools that have the same skill name but different content (conflicting versions)
    pub conflicting_tools: Vec<String>,
}

/// Internal struct for install operations
pub struct InstallResult {
    pub skill_id: String,
    pub name: String,
    pub central_path: std::path::PathBuf,
    pub content_hash: Option<String>,
}

/// Internal struct for update operations
pub struct UpdateResult {
    pub skill_id: String,
    pub name: String,
    pub central_path: std::path::PathBuf,
    pub content_hash: Option<String>,
    pub source_revision: Option<String>,
    pub updated_targets: Vec<String>,
}

/// Sync mode used for skill syncing
#[derive(Clone, Debug)]
pub enum SyncMode {
    Auto,
    Symlink,
    Junction,
    Copy,
}

impl SyncMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncMode::Auto => "auto",
            SyncMode::Symlink => "symlink",
            SyncMode::Junction => "junction",
            SyncMode::Copy => "copy",
        }
    }
}

/// Sync outcome from sync operations
#[derive(Clone, Debug)]
pub struct SyncOutcome {
    pub mode_used: SyncMode,
    pub target_path: std::path::PathBuf,
    pub replaced: bool,
}

/// Detected skill in a tool directory
#[derive(Clone, Debug)]
pub struct DetectedSkill {
    pub tool: String,
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_link: bool,
    pub link_target: Option<std::path::PathBuf>,
}

/// Custom tool defined by user
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomTool {
    pub key: String,                  // 唯一标识符
    pub display_name: String,         // 显示名称
    pub relative_skills_dir: String,  // Skills 目录相对路径
    pub relative_detect_dir: String,  // 检测目录相对路径
    pub created_at: i64,
}

/// DTO for custom tool
#[derive(Debug, Serialize)]
pub struct CustomToolDto {
    pub key: String,
    pub display_name: String,
    pub relative_skills_dir: String,
    pub relative_detect_dir: String,
    pub created_at: i64,
}

/// DTO for skill repo
#[derive(Debug, Serialize)]
pub struct SkillRepoDto {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub branch: String,
    pub enabled: bool,
    pub created_at: i64,
}

/// Helper function to get current timestamp in milliseconds
pub fn now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
