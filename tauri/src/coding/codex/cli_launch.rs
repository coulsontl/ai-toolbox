//! Launch Codex CLI with a temporary provider profile.
//!
//! Codex's `--profile <name>` layers `$CODEX_HOME/<name>.config.toml` on top of
//! the user's base `config.toml`. We write a throwaway profile containing only
//! this provider's settings, point `CODEX_HOME` at the current runtime root and
//! delete the file when the CLI exits. The base `config.toml`, `auth.json` and
//! `is_applied` are never touched.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
use std::process::Stdio;

use serde_json::Value;

use crate::coding::cli_launch_terminal::{
    launch_local, launch_wsl, CliLaunchSpec,
};
use crate::coding::cli_resolver::resolve_local_codex_program;
use crate::coding::runtime_location::{RuntimeLocationInfo, RuntimeLocationMode};

use super::commands::CodexCatalogToolProfile;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A provider-scoped profile ready to be written next to `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexLaunchProfile {
    pub profile_name: String,
    pub toml: String,
    /// Launch-scoped model catalog, written next to the profile and named by
    /// the profile's `model_catalog_json` pointer. Without it Codex inherits
    /// the applied provider's catalog, so `/model` lists the wrong models.
    pub catalog_json: Option<String>,
}

fn sanitize_provider_id_for_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn managed_api_key(settings_config: &Value) -> Option<&str> {
    settings_config
        .get("auth")
        .and_then(|auth| auth.get("OPENAI_API_KEY"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

/// Build the temp profile TOML for a provider.
///
/// Custom providers get a fresh provider id (`aitb-launch-*`) so the profile
/// cannot inherit fields such as `env_key` / `requires_openai_auth` from the
/// base config's same-named table. Official providers point at Codex's built-in
/// `openai` provider and keep using the global `auth.json` login.
pub(crate) fn build_launch_profile(
    provider_id: &str,
    provider_name: &str,
    category: &str,
    settings_config: &str,
    catalog_tool_profile: CodexCatalogToolProfile,
) -> Result<CodexLaunchProfile, String> {
    let parsed: Value = serde_json::from_str(settings_config)
        .map_err(|error| format!("Failed to parse provider settings: {error}"))?;
    let config_toml = parsed
        .get("config")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();

    // A per-launch suffix keeps two concurrent launches of the same provider
    // from overwriting each other's profile (and deleting the other's file on
    // exit).
    let random_suffix = uuid::Uuid::new_v4().simple().to_string();
    let profile_name = format!(
        "aitb-launch-{}-{}",
        sanitize_provider_id_for_filename(provider_id),
        &random_suffix[..8]
    );

    let mut root: toml::Value = if config_toml.is_empty() {
        toml::Value::Table(Default::default())
    } else {
        toml::from_str(config_toml)
            .map_err(|error| format!("Failed to parse provider config: {error}"))?
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| "Provider config is not a TOML table".to_string())?;

    if category == "official" {
        table.insert(
            "model_provider".to_string(),
            toml::Value::String("openai".to_string()),
        );
        // The built-in provider is used; any provider tables in this config are
        // irrelevant and must not shadow the base config's tables.
        table.remove("model_providers");
        return Ok(CodexLaunchProfile {
            profile_name,
            toml: toml::to_string(&root)
                .map_err(|error| format!("Failed to serialize launch profile: {error}"))?,
            catalog_json: None,
        });
    }

    let old_provider_id = table
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            table
                .get("model_providers")
                .and_then(toml::Value::as_table)
                .and_then(|providers| providers.keys().next().cloned())
        })
        .ok_or_else(|| {
            "This provider has no model provider configuration to launch with".to_string()
        })?;

    let mut providers = table
        .remove("model_providers")
        .and_then(|value| match value {
            toml::Value::Table(providers) => Some(providers),
            _ => None,
        })
        .ok_or_else(|| {
            "This provider has no model provider configuration to launch with".to_string()
        })?;

    let mut provider_table = providers.remove(&old_provider_id).ok_or_else(|| {
        format!("Provider table '{old_provider_id}' is missing from this provider's config")
    })?;

    if let toml::Value::Table(provider_fields) = &mut provider_table {
        // Codex decides `ModelProviderInfo::is_openai()` by exact name match.
        // Leaving the template's `name = "OpenAI"` makes a third-party provider
        // take the official OpenAI path (internal metadata passthrough and
        // other fields the upstream may reject). Use the provider's display
        // name, falling back when it would still collide with the built-in one.
        let trimmed_name = provider_name.trim();
        let provider_display_name = if trimmed_name.is_empty()
            || trimmed_name.eq_ignore_ascii_case("OpenAI")
        {
            "AI Toolbox"
        } else {
            trimmed_name
        };
        provider_fields.insert(
            "name".to_string(),
            toml::Value::String(provider_display_name.to_string()),
        );

        // AI Toolbox supplies the credential (bearer token) below. Codex
        // resolves `env_key` before the bearer token and fails hard when the
        // variable is missing, so the launch profile must not carry it.
        if let Some(api_key) = managed_api_key(&parsed) {
            provider_fields.insert(
                "experimental_bearer_token".to_string(),
                toml::Value::String(api_key.to_string()),
            );
            provider_fields.remove("env_key");
        }
    }

    let mut launch_providers = toml::map::Map::new();
    launch_providers.insert(profile_name.clone(), provider_table);
    table.insert(
        "model_providers".to_string(),
        toml::Value::Table(launch_providers),
    );
    table.insert(
        "model_provider".to_string(),
        toml::Value::String(profile_name.clone()),
    );

    // Publish the launched provider's own models to Codex. The profile is an
    // overlay on the base config, so without this pointer `/model` would show
    // the applied provider's catalog instead of the models this provider
    // serves. The catalog file is cleaned up with the profile.
    let catalog_json =
        super::commands::build_codex_launch_catalog(&parsed, config_toml, catalog_tool_profile)
        .map(|catalog| {
            table.insert(
                "model_catalog_json".to_string(),
                toml::Value::String(format!("{profile_name}.catalog.json")),
            );
            serde_json::to_string_pretty(&catalog)
                .map_err(|error| format!("Failed to serialize launch model catalog: {error}"))
        })
        .transpose()?;

    Ok(CodexLaunchProfile {
        profile_name,
        toml: toml::to_string(&root)
            .map_err(|error| format!("Failed to serialize launch profile: {error}"))?,
        catalog_json,
    })
}

fn write_local_profile_file(path: &Path, content: &str) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("Failed to create launch profile: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Failed to set launch profile permissions: {error}"))?;
    }
    file.write_all(content.as_bytes())
        .map_err(|error| format!("Failed to write launch profile: {error}"))
}

#[cfg(target_os = "windows")]
fn write_wsl_profile_file(distro: &str, linux_path: &str, content: &str) -> Result<(), String> {
    let mut child = Command::new("wsl")
        .args([
            "-d",
            distro,
            "--exec",
            "sh",
            "-c",
            "umask 077 && cat > \"$1\" && chmod 600 \"$1\"",
            "sh",
            linux_path,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("Failed to create WSL launch profile: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to open WSL launch profile stdin".to_string())?;
    if let Err(error) = stdin.write_all(content.as_bytes()) {
        let _ = child.kill();
        return Err(format!("Failed to write WSL launch profile: {error}"));
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|error| format!("Failed to finish WSL launch profile write: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to write WSL launch profile: {stderr}"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn remove_wsl_file(distro: &str, linux_path: &str) {
    let _ = Command::new("wsl")
        .args(["-d", distro, "--exec", "rm", "-f", linux_path])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

/// Convert the user-selected folder into a Linux path for the WSL shell.
/// The folder picker returns a Windows path, but `cd` runs inside the distro.
fn wsl_launch_cwd(cwd: Option<&Path>) -> Option<PathBuf> {
    cwd.map(|path| {
        let windows_path = path.to_string_lossy();
        let linux_path = crate::coding::wsl::windows_to_wsl_path(&windows_path)
            .unwrap_or_else(|_| {
                crate::coding::runtime_location::parse_wsl_unc_path(&windows_path)
                    .map(|info| info.linux_path)
                    .unwrap_or_else(|| windows_path.to_string())
            });
        PathBuf::from(linux_path)
    })
}

fn build_args(profile_name: &str, full_access: bool) -> Vec<String> {
    let mut args = vec!["--profile".to_string(), profile_name.to_string()];
    if full_access {
        args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }
    args
}

/// Launch a Codex session for one provider without touching the live config.
pub(crate) fn launch_codex_provider_cli_session(
    runtime_location: &RuntimeLocationInfo,
    provider_id: &str,
    provider_name: &str,
    category: &str,
    settings_config: &str,
    catalog_tool_profile: CodexCatalogToolProfile,
    full_access: bool,
    cwd: Option<&Path>,
    preferred_terminal: Option<&str>,
) -> Result<(), String> {
    let profile = build_launch_profile(
        provider_id,
        provider_name,
        category,
        settings_config,
        catalog_tool_profile,
    )?;
    let args = build_args(&profile.profile_name, full_access);
    let banner = format!(
        "Using temporary Codex profile '{}' (does not change applied config)",
        profile.profile_name
    );

    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            let root = &runtime_location.host_path;
            let profile_path = root.join(format!("{}.config.toml", profile.profile_name));
            write_local_profile_file(&profile_path, &profile.toml)?;

            let catalog_path = match profile.catalog_json.as_deref() {
                Some(catalog_json) => {
                    let path = root.join(format!("{}.catalog.json", profile.profile_name));
                    if let Err(error) = write_local_profile_file(&path, catalog_json) {
                        let _ = fs::remove_file(&profile_path);
                        return Err(error);
                    }
                    Some(path)
                }
                None => None,
            };

            let mut cleanup_paths = vec![profile_path.clone()];
            if let Some(path) = &catalog_path {
                cleanup_paths.push(path.clone());
            }

            let program = resolve_local_codex_program().path.to_string_lossy().to_string();
            let spec = CliLaunchSpec {
                program,
                args,
                env: vec![("CODEX_HOME".to_string(), root.to_string_lossy().to_string())],
                cwd: cwd.map(Path::to_path_buf),
                cleanup_paths,
                banner,
                safe_id: sanitize_provider_id_for_filename(provider_id),
                preferred_terminal: preferred_terminal.map(str::to_string),
            };
            let result = launch_local(&spec);
            if result.is_err() {
                // The launcher never ran, so nothing will delete the files.
                let _ = fs::remove_file(&profile_path);
                if let Some(path) = &catalog_path {
                    let _ = fs::remove_file(path);
                }
            }
            result
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = runtime_location
                .wsl
                .as_ref()
                .ok_or_else(|| "Missing WSL runtime metadata for Codex CLI launch".to_string())?;
            let profile_path = format!(
                "{}/{}.config.toml",
                wsl.linux_path.trim_end_matches('/'),
                profile.profile_name
            );

            #[cfg(target_os = "windows")]
            write_wsl_profile_file(&wsl.distro, &profile_path, &profile.toml)?;
            #[cfg(not(target_os = "windows"))]
            {
                let _ = &profile_path;
                return Err("WSL Direct Codex CLI launch is only supported on Windows".to_string());
            }

            #[cfg(target_os = "windows")]
            let catalog_path = match profile.catalog_json.as_deref() {
                Some(catalog_json) => {
                    let path = format!(
                        "{}/{}.catalog.json",
                        wsl.linux_path.trim_end_matches('/'),
                        profile.profile_name
                    );
                    if let Err(error) = write_wsl_profile_file(&wsl.distro, &path, catalog_json) {
                        remove_wsl_file(&wsl.distro, &profile_path);
                        return Err(error);
                    }
                    Some(path)
                }
                None => None,
            };
            #[cfg(not(target_os = "windows"))]
            let catalog_path: Option<String> = None;

            let mut cleanup_paths = vec![PathBuf::from(&profile_path)];
            if let Some(path) = &catalog_path {
                cleanup_paths.push(PathBuf::from(path));
            }

            let spec = CliLaunchSpec {
                program: "codex".to_string(),
                args,
                env: vec![("CODEX_HOME".to_string(), wsl.linux_path.clone())],
                cwd: wsl_launch_cwd(cwd),
                cleanup_paths,
                banner,
                safe_id: sanitize_provider_id_for_filename(provider_id),
                preferred_terminal: preferred_terminal.map(str::to_string),
            };
            let result = launch_wsl(&wsl.distro, &spec);
            if result.is_err() {
                // The inner script never ran, so it will not clean up either.
                #[cfg(target_os = "windows")]
                {
                    remove_wsl_file(&wsl.distro, &profile_path);
                    if let Some(path) = &catalog_path {
                        remove_wsl_file(&wsl.distro, path);
                    }
                }
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_launch_profile, sanitize_provider_id_for_filename, CodexCatalogToolProfile};

    fn settings(config: &str, api_key: Option<&str>) -> String {
        let auth = match api_key {
            Some(key) => format!(r#""auth": {{"OPENAI_API_KEY": "{key}"}}"#),
            None => r#""auth": {}"#.to_string(),
        };
        format!(
            r#"{{"config": {}, {auth}}}"#,
            serde_json::to_string(config).unwrap()
        )
    }

    fn settings_with_catalog(config: &str, models: serde_json::Value) -> String {
        serde_json::json!({
            "config": config,
            "auth": {},
            "modelCatalog": { "models": models }
        })
        .to_string()
    }

    #[test]
    fn custom_provider_gets_fresh_id_and_bearer_token() {
        let config = r#"
model = "gpt-5"
model_provider = "custom"

[model_providers.custom]
name = "Relay"
base_url = "https://relay.example/v1"
wire_api = "responses"
env_key = "OPENAI_API_KEY"
"#;
        let profile = build_launch_profile(
            "p1",
            "Relay",
            "custom",
            &settings(config, Some("sk-test")),
            CodexCatalogToolProfile::Full,
        )
        .unwrap();
        assert!(profile.profile_name.starts_with("aitb-launch-p1-"));
        let parsed: toml::Value = toml::from_str(&profile.toml).unwrap();
        let table = parsed.as_table().unwrap();
        assert_eq!(
            table.get("model_provider").and_then(toml::Value::as_str),
            Some(profile.profile_name.as_str())
        );
        assert_eq!(table.get("model").and_then(toml::Value::as_str), Some("gpt-5"));
        let providers = table["model_providers"].as_table().unwrap();
        assert_eq!(providers.len(), 1);
        let provider = providers[&profile.profile_name].as_table().unwrap();
        assert_eq!(
            provider.get("experimental_bearer_token").and_then(toml::Value::as_str),
            Some("sk-test")
        );
        assert!(provider.get("env_key").is_none());
    }

    #[test]
    fn custom_provider_launch_carries_its_own_model_catalog() {
        let config = r#"
model = "deepseek-v4.1-flash"
model_provider = "custom"

[model_providers.custom]
name = "Relay"
base_url = "https://relay.example/v1"
wire_api = "responses"
"#;
        let settings = settings_with_catalog(
            config,
            serde_json::json!([
                { "model": "deepseek-v4.1-flash", "displayName": "DeepSeek V4.1 Flash" },
                { "model": "glm-5.3", "displayName": "GLM-5.3" }
            ]),
        );
        let profile = build_launch_profile(
            "p1",
            "Relay",
            "custom",
            &settings,
            CodexCatalogToolProfile::Full,
        )
        .unwrap();
        let parsed: toml::Value = toml::from_str(&profile.toml).unwrap();
        let pointer = parsed
            .as_table()
            .unwrap()
            .get("model_catalog_json")
            .and_then(toml::Value::as_str);
        let expected_pointer = format!("{}.catalog.json", profile.profile_name);
        assert_eq!(pointer, Some(expected_pointer.as_str()));

        let catalog: serde_json::Value =
            serde_json::from_str(profile.catalog_json.as_deref().unwrap()).unwrap();
        let slugs: Vec<&str> = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["slug"].as_str())
            .collect();
        assert_eq!(slugs, vec!["deepseek-v4.1-flash", "glm-5.3"]);
    }

    #[test]
    fn custom_provider_launch_seeds_session_model_without_mapping() {
        let config = r#"
model = "gpt-5"
model_provider = "custom"

[model_providers.custom]
name = "Relay"
base_url = "https://relay.example/v1"
wire_api = "responses"
"#;
        let profile = build_launch_profile(
            "p1",
            "Relay",
            "custom",
            &settings(config, None),
            CodexCatalogToolProfile::Full,
        )
        .unwrap();
        let catalog: serde_json::Value =
            serde_json::from_str(profile.catalog_json.as_deref().unwrap()).unwrap();
        let slugs: Vec<&str> = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["slug"].as_str())
            .collect();
        assert_eq!(slugs, vec!["gpt-5"]);
    }

    #[test]
    fn official_provider_does_not_write_a_launch_catalog() {
        let config = "model = \"gpt-5.5\"\n";
        let profile = build_launch_profile(
            "official",
            "OpenAI",
            "official",
            &settings(config, None),
            CodexCatalogToolProfile::Full,
        )
        .unwrap();
        assert!(profile.catalog_json.is_none());
        let parsed: toml::Value = toml::from_str(&profile.toml).unwrap();
        assert!(parsed
            .as_table()
            .unwrap()
            .get("model_catalog_json")
            .is_none());
    }

    #[test]
    fn official_provider_targets_builtin_openai_provider() {
        let config = "model = \"gpt-5.5\"\n";
        let profile = build_launch_profile(
            "official",
            "OpenAI",
            "official",
            &settings(config, None),
            CodexCatalogToolProfile::Full,
        )
        .unwrap();
        let parsed: toml::Value = toml::from_str(&profile.toml).unwrap();
        let table = parsed.as_table().unwrap();
        assert_eq!(
            table.get("model_provider").and_then(toml::Value::as_str),
            Some("openai")
        );
        assert!(table.get("model_providers").is_none());
    }

    #[test]
    fn custom_provider_without_table_is_rejected() {
        let config = "model = \"gpt-5\"\n";
        let error = build_launch_profile(
            "p1",
            "Relay",
            "custom",
            &settings(config, None),
            CodexCatalogToolProfile::Full,
        )
        .unwrap_err();
        assert!(error.contains("no model provider configuration"));
    }

    #[test]
    fn custom_provider_uses_display_name_instead_of_openai_placeholder() {
        let config = r#"
model = "gpt-5"
model_provider = "custom"

[model_providers.custom]
name = "OpenAI"
base_url = "https://ark.cn-beijing.volces.com/api/v3"
wire_api = "responses"
"#;
        let profile = build_launch_profile(
            "p1",
            "Volcengine Ark",
            "custom",
            &settings(config, None),
            CodexCatalogToolProfile::Full,
        )
        .unwrap();
        let parsed: toml::Value = toml::from_str(&profile.toml).unwrap();
        let provider = parsed["model_providers"][&profile.profile_name]
            .as_table()
            .unwrap();
        assert_eq!(
            provider.get("name").and_then(toml::Value::as_str),
            Some("Volcengine Ark")
        );
    }

    #[test]
    fn openai_display_name_falls_back_to_ai_toolbox() {
        let config = r#"
model = "gpt-5"
model_provider = "custom"

[model_providers.custom]
name = "OpenAI"
base_url = "https://relay.example/v1"
wire_api = "responses"
"#;
        let profile = build_launch_profile(
            "p1",
            "openai",
            "custom",
            &settings(config, None),
            CodexCatalogToolProfile::Full,
        )
        .unwrap();
        let parsed: toml::Value = toml::from_str(&profile.toml).unwrap();
        let provider = parsed["model_providers"][&profile.profile_name]
            .as_table()
            .unwrap();
        assert_eq!(
            provider.get("name").and_then(toml::Value::as_str),
            Some("AI Toolbox")
        );
    }

    #[test]
    fn native_safe_launch_catalog_omits_private_tool_types() {
        let config = r#"
model = "doubao-seed-2-1-pro"
model_provider = "custom"

[model_providers.custom]
name = "OpenAI"
base_url = "https://ark.cn-beijing.volces.com/api/v3"
wire_api = "responses"
"#;
        let settings = settings_with_catalog(
            config,
            serde_json::json!([
                { "model": "doubao-seed-2-1-pro", "displayName": "Doubao Seed 2.1 Pro" }
            ]),
        );
        let profile = build_launch_profile(
            "p1",
            "Doubao",
            "custom",
            &settings,
            CodexCatalogToolProfile::NativeSafe,
        )
        .unwrap();
        let catalog: serde_json::Value =
            serde_json::from_str(profile.catalog_json.as_deref().unwrap()).unwrap();
        let model = &catalog["models"][0];
        assert!(model.get("apply_patch_tool_type").is_none());
        assert!(model.get("web_search_tool_type").is_none());
        assert_eq!(model["supports_search_tool"], serde_json::json!(false));
        assert_eq!(model["multi_agent_version"], serde_json::json!("disabled"));
        assert_eq!(model["shell_type"], serde_json::json!("unified_exec"));
        assert_eq!(model["support_verbosity"], serde_json::json!(false));
        assert_eq!(
            model["supports_parallel_tool_calls"],
            serde_json::json!(false)
        );
        assert_eq!(
            model["supports_image_detail_original"],
            serde_json::json!(false)
        );
        assert_eq!(
            model["supports_reasoning_summaries"],
            serde_json::json!(true)
        );
        assert_eq!(
            model["default_reasoning_summary"],
            serde_json::json!("none")
        );
    }

    #[test]
    fn provider_id_is_sanitized_for_file_names() {
        assert_eq!(sanitize_provider_id_for_filename("a/b:c d"), "a_b_c_d");
    }
}
