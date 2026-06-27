use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;

use crate::coding::cli_resolver::{
    build_local_tokio_command, local_cli_missing_hint, resolve_local_npx_program,
};
use crate::coding::runtime_location::{
    self, build_windows_unc_path, expand_home_from_user_root, RuntimeLocationInfo,
    RuntimeLocationMode,
};
use crate::db::SqliteDbState;

const MAGIC_CONTEXT_CONFIG_FILE: &str = "magic-context.jsonc";
const MAGIC_CONTEXT_SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/cortexkit/magic-context/master/assets/magic-context.schema.json";
const MAGIC_CONTEXT_PACKAGE: &str = "@cortexkit/magic-context@latest";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MagicContextHarness {
    Opencode,
    Pi,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MagicContextConfigRequest {
    pub harness: MagicContextHarness,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MagicContextSaveInput {
    pub harness: MagicContextHarness,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MagicContextDoctorInput {
    pub harness: MagicContextHarness,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MagicContextConfigFile {
    pub harness: MagicContextHarness,
    pub path: String,
    pub directory: String,
    pub exists: bool,
    pub content: String,
    pub parsed: Option<Value>,
    pub parse_error: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MagicContextCommandResult {
    pub command: String,
    pub output: String,
}

struct MagicContextResolvedPath {
    path: PathBuf,
    warnings: Vec<String>,
}

struct MagicContextCommandInvocation {
    command: Command,
    display_command: String,
    local_program_label: Option<String>,
}

impl MagicContextHarness {
    fn as_cli_value(self) -> &'static str {
        match self {
            MagicContextHarness::Opencode => "opencode",
            MagicContextHarness::Pi => "pi",
        }
    }
}

fn default_config_content() -> String {
    format!(
        r#"{{
  "$schema": "{MAGIC_CONTEXT_SCHEMA_URL}",
  "enabled": true,
  "ctx_reduce_enabled": true,
  "temporal_awareness": true,
  "smart_drops": false,
  "memory": {{
    "enabled": true
  }}
}}
"#
    )
}

async fn runtime_location_for_harness(
    db: &SqliteDbState,
    harness: MagicContextHarness,
) -> Result<RuntimeLocationInfo, String> {
    match harness {
        MagicContextHarness::Opencode => {
            runtime_location::get_opencode_runtime_location_async(db).await
        }
        MagicContextHarness::Pi => runtime_location::get_pi_runtime_location_async(db).await,
    }
}

fn local_user_config_path() -> Result<PathBuf, String> {
    let home_dir =
        dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    Ok(local_user_config_path_from_home(home_dir, xdg_config_home))
}

fn local_user_config_path_from_home(
    home_dir: PathBuf,
    xdg_config_home: Option<PathBuf>,
) -> PathBuf {
    #[cfg(unix)]
    if let Some(config_home) = xdg_config_home {
        if !config_home.as_os_str().is_empty() {
            return config_home
                .join("cortexkit")
                .join(MAGIC_CONTEXT_CONFIG_FILE);
        }
    }

    home_dir
        .join(".config")
        .join("cortexkit")
        .join(MAGIC_CONTEXT_CONFIG_FILE)
}

fn wsl_user_config_path(location: &RuntimeLocationInfo) -> Option<PathBuf> {
    let wsl = location.wsl.as_ref()?;
    let linux_path = expand_home_from_user_root(
        wsl.linux_user_root.as_deref(),
        "~/.config/cortexkit/magic-context.jsonc",
    );
    if linux_path.starts_with('~') {
        return None;
    }
    Some(build_windows_unc_path(&wsl.distro, &linux_path))
}

fn resolve_user_config_path(
    location: &RuntimeLocationInfo,
) -> Result<MagicContextResolvedPath, String> {
    match location.mode {
        RuntimeLocationMode::WslDirect => {
            if let Some(path) = wsl_user_config_path(location) {
                return Ok(MagicContextResolvedPath {
                    path,
                    warnings: Vec::new(),
                });
            }
            Ok(MagicContextResolvedPath {
                path: local_user_config_path()?,
                warnings: vec![
                    "当前 WSL Direct 路径无法推导 WSL 用户 home，已回退到本机用户配置路径。"
                        .to_string(),
                ],
            })
        }
        RuntimeLocationMode::LocalWindows => Ok(MagicContextResolvedPath {
            path: local_user_config_path()?,
            warnings: Vec::new(),
        }),
    }
}

fn read_config_file(
    harness: MagicContextHarness,
    resolved: MagicContextResolvedPath,
) -> Result<MagicContextConfigFile, String> {
    let directory = resolved
        .path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Failed to resolve Magic Context config directory".to_string())?;

    if !resolved.path.exists() {
        return Ok(MagicContextConfigFile {
            harness,
            path: resolved.path.to_string_lossy().to_string(),
            directory: directory.to_string_lossy().to_string(),
            exists: false,
            content: String::new(),
            parsed: None,
            parse_error: None,
            warnings: resolved.warnings,
        });
    }

    let content = fs::read_to_string(&resolved.path).map_err(|error| {
        format!(
            "Failed to read Magic Context config {}: {error}",
            resolved.path.display()
        )
    })?;

    let (parsed, parse_error) = match json5::from_str::<Value>(&content) {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error.to_string())),
    };

    Ok(MagicContextConfigFile {
        harness,
        path: resolved.path.to_string_lossy().to_string(),
        directory: directory.to_string_lossy().to_string(),
        exists: true,
        content,
        parsed,
        parse_error,
        warnings: resolved.warnings,
    })
}

fn validate_config_content(content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("Magic Context config content cannot be empty".to_string());
    }

    let parsed: Value = json5::from_str(trimmed)
        .map_err(|error| format!("Invalid Magic Context JSONC config: {error}"))?;
    if !parsed.is_object() {
        return Err("Magic Context config must be a JSON object".to_string());
    }
    Ok(())
}

fn write_config_file(path: &Path, content: &str) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| "Failed to resolve Magic Context config directory".to_string())?;
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "Failed to create Magic Context config directory {}: {error}",
            directory.display()
        )
    })?;
    fs::write(path, content).map_err(|error| {
        format!(
            "Failed to write Magic Context config {}: {error}",
            path.display()
        )
    })
}

fn build_doctor_command(
    location: &RuntimeLocationInfo,
    harness: MagicContextHarness,
) -> MagicContextCommandInvocation {
    let harness_value = harness.as_cli_value();
    match location.mode {
        RuntimeLocationMode::WslDirect => {
            if let Some(wsl) = location.wsl.as_ref() {
                let mut command = Command::new("wsl");
                command.args([
                    "-d",
                    &wsl.distro,
                    "--exec",
                    "npx",
                    "-y",
                    MAGIC_CONTEXT_PACKAGE,
                    "doctor",
                    "--harness",
                    harness_value,
                ]);
                return MagicContextCommandInvocation {
                    command,
                    display_command: format!(
                        "wsl -d {} --exec npx -y {} doctor --harness {}",
                        wsl.distro, MAGIC_CONTEXT_PACKAGE, harness_value
                    ),
                    local_program_label: None,
                };
            }
        }
        RuntimeLocationMode::LocalWindows => {}
    }

    let npx_program = resolve_local_npx_program();
    let local_program_label = npx_program.path.display().to_string();
    let mut command = build_local_tokio_command(&npx_program.path);
    command.args([
        "-y",
        MAGIC_CONTEXT_PACKAGE,
        "doctor",
        "--harness",
        harness_value,
    ]);
    MagicContextCommandInvocation {
        command,
        display_command: format!("npx -y {MAGIC_CONTEXT_PACKAGE} doctor --harness {harness_value}"),
        local_program_label: Some(local_program_label),
    }
}

fn build_doctor_spawn_error(error: &std::io::Error, local_program_label: Option<&str>) -> String {
    let base_message = format!("Failed to run Magic Context doctor: {error}");
    if error.kind() == std::io::ErrorKind::NotFound {
        if let Some(label) = local_program_label {
            return format!(
                "{base_message}. attempted_program={label}. {}",
                local_cli_missing_hint("npx")
            );
        }
    }
    base_message
}

async fn run_doctor_command(
    location: &RuntimeLocationInfo,
    harness: MagicContextHarness,
) -> Result<MagicContextCommandResult, String> {
    let MagicContextCommandInvocation {
        mut command,
        display_command,
        local_program_label,
    } = build_doctor_command(location, harness);

    let output = command
        .output()
        .await
        .map_err(|error| build_doctor_spawn_error(&error, local_program_label.as_deref()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined = [stdout.as_str(), stderr.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if output.status.success() {
        return Ok(MagicContextCommandResult {
            command: display_command,
            output: combined,
        });
    }

    Err(if combined.is_empty() {
        "Magic Context doctor failed without output".to_string()
    } else {
        combined
    })
}

#[tauri::command]
pub async fn read_magic_context_config(
    state: tauri::State<'_, SqliteDbState>,
    request: MagicContextConfigRequest,
) -> Result<MagicContextConfigFile, String> {
    let db = state.db();
    let location = runtime_location_for_harness(&db, request.harness).await?;
    let resolved = resolve_user_config_path(&location)?;
    read_config_file(request.harness, resolved)
}

#[tauri::command]
pub async fn save_magic_context_config(
    state: tauri::State<'_, SqliteDbState>,
    input: MagicContextSaveInput,
) -> Result<MagicContextConfigFile, String> {
    validate_config_content(&input.content)?;

    let db = state.db();
    let location = runtime_location_for_harness(&db, input.harness).await?;
    let resolved = resolve_user_config_path(&location)?;
    write_config_file(&resolved.path, &input.content)?;
    read_config_file(input.harness, resolved)
}

#[tauri::command]
pub async fn create_magic_context_config(
    state: tauri::State<'_, SqliteDbState>,
    request: MagicContextConfigRequest,
) -> Result<MagicContextConfigFile, String> {
    let db = state.db();
    let location = runtime_location_for_harness(&db, request.harness).await?;
    let resolved = resolve_user_config_path(&location)?;
    if resolved.path.exists() {
        return read_config_file(request.harness, resolved);
    }
    write_config_file(&resolved.path, &default_config_content())?;
    read_config_file(request.harness, resolved)
}

#[tauri::command]
pub async fn run_magic_context_doctor(
    state: tauri::State<'_, SqliteDbState>,
    input: MagicContextDoctorInput,
) -> Result<MagicContextCommandResult, String> {
    let db = state.db();
    let location = runtime_location_for_harness(&db, input.harness).await?;
    run_doctor_command(&location, input.harness).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_does_not_enable_smart_drops() {
        let config = json5::from_str::<Value>(&default_config_content()).expect("default config");
        assert_eq!(config["smart_drops"], Value::Bool(false));
    }

    #[cfg(unix)]
    #[test]
    fn local_user_config_path_uses_xdg_config_home_on_unix() {
        let path = local_user_config_path_from_home(
            PathBuf::from("/home/alice"),
            Some(PathBuf::from("/custom/config")),
        );
        assert_eq!(
            path,
            PathBuf::from("/custom/config/cortexkit/magic-context.jsonc")
        );
    }

    #[test]
    fn local_user_config_path_falls_back_to_home_config() {
        let path = local_user_config_path_from_home(PathBuf::from("/home/alice"), None);
        assert_eq!(
            path,
            PathBuf::from("/home/alice/.config/cortexkit/magic-context.jsonc")
        );
    }
}
