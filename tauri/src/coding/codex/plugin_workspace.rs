use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::plugin_types::CodexPluginWorkspaceRoot;
use crate::coding::cli_resolver::apply_create_no_window;
use crate::coding::runtime_location::{self, RuntimeLocationInfo, RuntimeLocationMode};
use crate::coding::skills::git_fetcher::{self, GitProxyMode};
use crate::db::helpers::{db_get, db_put};
use crate::db::schema::{Cli, CliConfigKind, DbTable};
use crate::http_client;

const MARKETPLACE_RELATIVE_PATH: &str = ".agents/plugins/marketplace.json";
const WORKSPACE_SETTINGS_ID: &str = "settings";
const MANAGED_CACHE_ROOT_SEGMENT: &str = ".tmp";
const MANAGED_CACHE_SUBDIR: &str = "plugin-marketplaces";
const GIT_CLONE_TIMEOUT_SECS: u64 = 300;
const GIT_FETCH_TIMEOUT_SECS: u64 = 180;
const GIT_RESET_TIMEOUT_SECS: u64 = 60;
const MKDIR_TIMEOUT_SECS: u64 = 30;
const JSON_DOWNLOAD_TIMEOUT_SECS: u64 = 60;
const JSON_WRITE_TIMEOUT_SECS: u64 = 30;

/// Classification of a marketplace source the user can type into the add modal.
///
/// Codex has no CLI to delegate `plugin marketplace add` to (unlike Grok/Claude),
/// so remote sources are downloaded/cloned by the backend itself into a managed
/// cache directory under `<codex_root>/.tmp/plugin-marketplaces/<id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MarketplaceSource {
    /// A local absolute/relative directory path (existing behavior).
    Local,
    /// A git repository URL or GitHub `owner/repo` shorthand. Cloned whole so
    /// the plugin `source: { Local { path } }` directories remain installable.
    Git(String),
    /// A direct `marketplace.json` URL. Only the JSON file is downloaded; listed
    /// plugins show up but cannot be installed (their source directories do not
    /// exist locally).
    Json(String),
}

fn classify_marketplace_source(raw: &str) -> MarketplaceSource {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return MarketplaceSource::Local;
    }

    let lower = trimmed.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let is_windows_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let is_local_path = trimmed.starts_with('/')
        || is_windows_drive
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.starts_with('~');
    if is_local_path {
        return MarketplaceSource::Local;
    }

    if trimmed.starts_with("git@") {
        return MarketplaceSource::Git(trimmed.to_string());
    }

    if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    {
        let path_part = rest.split(['#', '?']).next().unwrap_or(rest);
        if path_part.to_ascii_lowercase().ends_with(".json") {
            return MarketplaceSource::Json(trimmed.to_string());
        }
        return MarketplaceSource::Git(trimmed.to_string());
    }

    if is_github_shorthand(trimmed) {
        let cleaned = trimmed.trim_end_matches(".git");
        return MarketplaceSource::Git(format!("https://github.com/{cleaned}"));
    }

    // Bare tokens that match no remote scheme fall back to local handling, which
    // will surface the existing "must be an absolute path" error.
    MarketplaceSource::Local
}

fn is_github_shorthand(value: &str) -> bool {
    let Some((owner, repo)) = value.split_once('/') else {
        return false;
    };
    if repo.contains('/') || owner.is_empty() || repo.is_empty() {
        return false;
    }
    let repo_name = repo.trim_end_matches(".git");
    if repo_name.is_empty() {
        return false;
    }
    let is_segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '.' || c == '_')
    };
    is_segment(owner) && is_segment(repo_name)
}

/// Stable FNV-1a 64-bit hash rendered as lower-hex. Used for cache directory
/// names so the same source URL always maps to the same directory across runs
/// (and re-adding refreshes instead of creating duplicates).
fn fnv1a_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

fn managed_cache_root(host_path: &Path) -> PathBuf {
    host_path
        .join(MANAGED_CACHE_ROOT_SEGMENT)
        .join(MANAGED_CACHE_SUBDIR)
}

fn marketplace_cache_id(url: &str) -> String {
    let base = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("marketplace");
    let stripped = base
        .trim_end_matches(".git")
        .trim_end_matches(".json")
        .trim_matches(|c| c == '?' || c == '#' || c == '&');
    let base = if stripped.is_empty() {
        "marketplace"
    } else {
        stripped
    };
    let sanitized: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let sanitized = if sanitized.is_empty() {
        "marketplace".to_string()
    } else {
        sanitized
    };
    format!("{}-{}", sanitized, &fnv1a_hex(url)[..8])
}

fn normalize_workspace_root_path(raw_path: &str) -> Result<String, String> {
    let trimmed_path = raw_path.trim();
    if trimmed_path.is_empty() {
        return Err("Workspace directory is required".to_string());
    }

    let workspace_path = PathBuf::from(trimmed_path);
    if !workspace_path.is_absolute() {
        return Err(format!(
            "Workspace directory must be an absolute path: {trimmed_path}"
        ));
    }

    let metadata = fs::metadata(&workspace_path)
        .map_err(|error| format!("Failed to read workspace directory {trimmed_path}: {error}"))?;
    if !metadata.is_dir() {
        return Err(format!("Workspace path is not a directory: {trimmed_path}"));
    }

    Ok(trimmed_path.to_string())
}

fn path_strings_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn find_git_repo_root(start_path: &Path) -> Option<PathBuf> {
    let mut current_path = Some(start_path);
    while let Some(path) = current_path {
        if path.join(".git").exists() {
            return Some(path.to_path_buf());
        }
        current_path = path.parent();
    }
    None
}

fn resolve_workspace_marketplace_path(
    workspace_path: &Path,
) -> Result<(PathBuf, Option<PathBuf>, String), String> {
    let direct_marketplace_path = workspace_path.join(MARKETPLACE_RELATIVE_PATH);
    if direct_marketplace_path.is_file() {
        return Ok((direct_marketplace_path, None, "direct".to_string()));
    }

    if let Some(repo_root) = find_git_repo_root(workspace_path) {
        let repo_marketplace_path = repo_root.join(MARKETPLACE_RELATIVE_PATH);
        if repo_marketplace_path.is_file() {
            return Ok((
                repo_marketplace_path,
                Some(repo_root),
                "gitRepo".to_string(),
            ));
        }
    }

    Err(format!(
        "No marketplace.json found under {} or its Git repo root",
        workspace_path.display()
    ))
}

fn from_db_value_workspace_root_paths(value: Value) -> Vec<String> {
    value
        .get("workspace_roots")
        .or_else(|| value.get("workspaceRoots"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn to_db_value_workspace_root_paths(workspace_root_paths: &[String]) -> Value {
    json!({
        "workspace_roots": workspace_root_paths,
    })
}

async fn get_stored_workspace_root_paths(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<String>, String> {
    db.with_conn(|conn| {
        Ok(db_get(
            conn,
            DbTable::CliConfig(Cli::Codex, CliConfigKind::WorkspaceRoots),
            WORKSPACE_SETTINGS_ID,
        )?
        .map(from_db_value_workspace_root_paths)
        .unwrap_or_default())
    })
}

async fn save_workspace_root_paths(
    db: &crate::db::SqliteDbState,
    workspace_root_paths: &[String],
) -> Result<(), String> {
    let payload = to_db_value_workspace_root_paths(workspace_root_paths);
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CliConfig(Cli::Codex, CliConfigKind::WorkspaceRoots),
            WORKSPACE_SETTINGS_ID,
            &payload,
        )
    })
}

async fn register_workspace_root_path(
    db: &crate::db::SqliteDbState,
    path: &str,
) -> Result<(), String> {
    let mut workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    if workspace_root_paths
        .iter()
        .any(|existing_path| path_strings_equal(existing_path, path))
    {
        return Ok(());
    }
    workspace_root_paths.push(path.to_string());
    save_workspace_root_paths(db, &workspace_root_paths).await
}

async fn resolve_proxy_for_git(db: &crate::db::SqliteDbState) -> (http_client::ProxyMode, String) {
    http_client::get_proxy_from_settings(db)
        .await
        .unwrap_or((http_client::ProxyMode::System, String::new()))
}

/// Run a synchronous [`Command`] with a hard timeout, killing the child if it
/// exceeds the deadline. Mirrors the loop used by `skills/git_fetcher` so the
/// Codex marketplace clone path does not hang on stalled HTTPS transfers.
fn run_command_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    context: String,
) -> Result<std::process::Output, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|error| format!("{context}: {error}"))?;
    let start = Instant::now();
    loop {
        if start.elapsed() > timeout {
            let _ = child.kill();
            let stderr = child
                .wait_with_output()
                .map(|output| String::from_utf8_lossy(&output.stderr).trim().to_string())
                .unwrap_or_default();
            return Err(format!(
                "{context}: timed out after {}s: {stderr}",
                timeout.as_secs()
            ));
        }

        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("{context}: {error}"))
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(error) => return Err(format!("{context}: {error}")),
        }
    }
}

fn command_error(context: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stderr.is_empty() {
        format!("{context}: {stdout}")
    } else {
        format!("{context}: {stderr}")
    }
}

/// Run a plain command inside WSL (e.g. `mkdir -p`) without the git/proxy env
/// prefix.
fn run_wsl_plain(
    distro: &str,
    args: &[&str],
    timeout: Duration,
    context: &str,
) -> Result<(), String> {
    let mut cmd = Command::new("wsl");
    cmd.args(["-d", distro]).args(args.iter());
    apply_create_no_window(&mut cmd);
    let output = run_command_with_timeout(cmd, timeout, context.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    Err(command_error(context, &output))
}

/// Run `git` inside WSL with the proxy env prefix derived from app settings.
/// `git_args` is the argument list after `git` (e.g. `["clone", url, target]`).
fn run_wsl_git(
    distro: &str,
    git_args: &[&str],
    proxy: &(http_client::ProxyMode, String),
    timeout: Duration,
    context: &str,
) -> Result<(), String> {
    let mut argv: Vec<String> = vec!["--exec".to_string(), "env".to_string()];
    match proxy {
        (http_client::ProxyMode::Direct, _) => {
            for var in [
                "-u",
                "HTTP_PROXY",
                "-u",
                "HTTPS_PROXY",
                "-u",
                "http_proxy",
                "-u",
                "https_proxy",
            ] {
                argv.push(var.to_string());
            }
            argv.push("GIT_TERMINAL_PROMPT=0".to_string());
        }
        (http_client::ProxyMode::Custom, proxy_url) if !proxy_url.is_empty() => {
            argv.push("GIT_TERMINAL_PROMPT=0".to_string());
            for var in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                argv.push(format!("{var}={proxy_url}"));
            }
        }
        _ => {
            argv.push("GIT_TERMINAL_PROMPT=0".to_string());
        }
    }
    argv.push("git".to_string());
    for arg in git_args {
        argv.push((*arg).to_string());
    }
    let arg_refs: Vec<&str> = argv.iter().map(String::as_str).collect();

    let mut cmd = Command::new("wsl");
    cmd.args(["-d", distro]).args(&arg_refs);
    apply_create_no_window(&mut cmd);
    let output = run_command_with_timeout(cmd, timeout, context.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    Err(command_error(context, &output))
}

async fn clone_remote_git(
    db: &crate::db::SqliteDbState,
    location: &RuntimeLocationInfo,
    url: &str,
    cache_dir: &Path,
    linux_cache_dir: Option<&str>,
) -> Result<(), String> {
    let proxy = resolve_proxy_for_git(db).await;

    match location.mode {
        RuntimeLocationMode::LocalWindows => {
            let proxy_mode = match proxy {
                (http_client::ProxyMode::Direct, _) => GitProxyMode::Direct,
                (http_client::ProxyMode::Custom, proxy_url) if !proxy_url.is_empty() => {
                    GitProxyMode::Custom(proxy_url)
                }
                _ => GitProxyMode::System,
            };
            let url_owned = url.to_string();
            let cache_owned = cache_dir.to_path_buf();
            let join_result = tauri::async_runtime::spawn_blocking(move || {
                git_fetcher::set_proxy(proxy_mode);
                git_fetcher::clone_or_pull(&url_owned, &cache_owned, None)
            })
            .await
            .map_err(|error| format!("git clone task failed: {error}"))?;
            join_result.map_err(|error| {
                format!("Failed to clone marketplace repository {url}: {error:#}")
            })?;
            Ok(())
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = location.wsl.as_ref().ok_or_else(|| {
                "Missing WSL runtime metadata for marketplace git clone".to_string()
            })?;
            let linux_target = linux_cache_dir.ok_or_else(|| {
                "Missing WSL linux cache dir for marketplace git clone".to_string()
            })?;
            let distro = wsl.distro.as_str();

            if cache_dir.exists() {
                run_wsl_git(
                    distro,
                    &["-C", linux_target, "fetch", "--prune", "origin"],
                    &proxy,
                    Duration::from_secs(GIT_FETCH_TIMEOUT_SECS),
                    "git fetch marketplace",
                )?;
                run_wsl_git(
                    distro,
                    &["-C", linux_target, "reset", "--hard", "FETCH_HEAD"],
                    &proxy,
                    Duration::from_secs(GIT_RESET_TIMEOUT_SECS),
                    "git reset marketplace",
                )?;
            } else {
                let parent_linux = linux_target
                    .rsplit_once('/')
                    .map(|(parent, _)| parent)
                    .unwrap_or("/");
                run_wsl_plain(
                    distro,
                    &["--exec", "mkdir", "-p", parent_linux],
                    Duration::from_secs(MKDIR_TIMEOUT_SECS),
                    "create marketplace cache dir",
                )?;
                run_wsl_git(
                    distro,
                    &[
                        "clone",
                        "--depth",
                        "1",
                        "--filter=blob:none",
                        "--no-tags",
                        url,
                        linux_target,
                    ],
                    &proxy,
                    Duration::from_secs(GIT_CLONE_TIMEOUT_SECS),
                    &format!("git clone {url}"),
                )?;
            }
            Ok(())
        }
    }
}

async fn download_marketplace_json(
    db: &crate::db::SqliteDbState,
    url: &str,
    cache_dir: &Path,
) -> Result<(), String> {
    let client = http_client::client_with_timeout(db, JSON_DOWNLOAD_TIMEOUT_SECS)
        .await
        .map_err(|error| {
            format!("Failed to create HTTP client for marketplace download: {error}")
        })?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Failed to download marketplace.json from {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download marketplace.json from {url}: HTTP {}",
            response.status()
        ));
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("Failed to read marketplace.json body from {url}: {error}"))?;

    let marketplace_dir = cache_dir.join(".agents").join("plugins");
    let target = marketplace_dir.join("marketplace.json");
    let dir_path = marketplace_dir.clone();
    let target_path = target.clone();
    let body_bytes = body.to_vec();

    // Write on a blocking thread with a hard timeout. For WSL Direct the cache
    // dir is a `\\wsl.localhost\...` UNC path; a sync `fs::write` on the async
    // thread would block the runtime if WSL is unreachable, so mirror the
    // file_io-with-timeout rule used for Codex config reads.
    let write_result = tokio::time::timeout(
        Duration::from_secs(JSON_WRITE_TIMEOUT_SECS),
        tauri::async_runtime::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::create_dir_all(&dir_path)?;
            std::fs::write(&target_path, body_bytes)
        }),
    )
    .await;
    match write_result {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(format!("Failed to write {}: {error}", target.display())),
        Ok(Err(error)) => Err(format!("marketplace.json write task failed: {error}")),
        Err(_) => Err(format!(
            "Timed out writing {} (WSL unreachable?)",
            target.display()
        )),
    }
}

pub(crate) fn describe_workspace_root(path: &str) -> CodexPluginWorkspaceRoot {
    let trimmed_path = path.trim();
    let workspace_path = PathBuf::from(trimmed_path);

    if trimmed_path.is_empty() {
        return CodexPluginWorkspaceRoot {
            path: path.to_string(),
            status: "missing".to_string(),
            resolution_source: None,
            resolved_marketplace_path: None,
            resolved_repo_root: None,
            error: Some("Workspace directory is empty".to_string()),
        };
    }

    match fs::metadata(&workspace_path) {
        Ok(metadata) if metadata.is_dir() => {
            match resolve_workspace_marketplace_path(&workspace_path) {
                Ok((marketplace_path, repo_root, resolution_source)) => CodexPluginWorkspaceRoot {
                    path: trimmed_path.to_string(),
                    status: "ready".to_string(),
                    resolution_source: Some(resolution_source),
                    resolved_marketplace_path: Some(marketplace_path.to_string_lossy().to_string()),
                    resolved_repo_root: repo_root.map(|item| item.to_string_lossy().to_string()),
                    error: None,
                },
                Err(error) => CodexPluginWorkspaceRoot {
                    path: trimmed_path.to_string(),
                    status: "missing".to_string(),
                    resolution_source: None,
                    resolved_marketplace_path: None,
                    resolved_repo_root: None,
                    error: Some(error),
                },
            }
        }
        Ok(_) => CodexPluginWorkspaceRoot {
            path: trimmed_path.to_string(),
            status: "missing".to_string(),
            resolution_source: None,
            resolved_marketplace_path: None,
            resolved_repo_root: None,
            error: Some(format!("Workspace path is not a directory: {trimmed_path}")),
        },
        Err(error) => CodexPluginWorkspaceRoot {
            path: trimmed_path.to_string(),
            status: "missing".to_string(),
            resolution_source: None,
            resolved_marketplace_path: None,
            resolved_repo_root: None,
            error: Some(format!(
                "Failed to read workspace directory {trimmed_path}: {error}"
            )),
        },
    }
}

pub async fn list_codex_plugin_workspace_roots(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<CodexPluginWorkspaceRoot>, String> {
    let workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    Ok(workspace_root_paths
        .into_iter()
        .map(|path| describe_workspace_root(&path))
        .collect())
}

pub async fn list_ready_codex_workspace_marketplace_paths(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<PathBuf>, String> {
    let workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    let mut marketplace_paths = Vec::new();

    for workspace_root_path in workspace_root_paths {
        let workspace_status = describe_workspace_root(&workspace_root_path);
        if let Some(marketplace_path) = workspace_status.resolved_marketplace_path {
            let marketplace_path = PathBuf::from(&marketplace_path);
            if !marketplace_paths.iter().any(|existing_path: &PathBuf| {
                path_strings_equal(
                    &existing_path.to_string_lossy(),
                    &marketplace_path.to_string_lossy(),
                )
            }) {
                marketplace_paths.push(marketplace_path);
            }
        }
    }

    Ok(marketplace_paths)
}

pub async fn add_codex_plugin_workspace_root(
    db: &crate::db::SqliteDbState,
    raw_path: &str,
) -> Result<(), String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("Workspace directory is required".to_string());
    }

    let source = classify_marketplace_source(trimmed);
    match &source {
        MarketplaceSource::Local => {
            let normalized_path = normalize_workspace_root_path(trimmed)?;
            let workspace_path = PathBuf::from(&normalized_path);
            let _ = resolve_workspace_marketplace_path(&workspace_path)?;
            register_workspace_root_path(db, &normalized_path).await
        }
        MarketplaceSource::Git(url) | MarketplaceSource::Json(url) => {
            let location = runtime_location::get_codex_runtime_location_async(db).await?;
            let cache_root = managed_cache_root(&location.host_path);
            let id = marketplace_cache_id(url);
            let cache_dir = cache_root.join(&id);
            let linux_cache_dir = location.wsl.as_ref().map(|wsl| {
                format!(
                    "{}/{}/{}/{}",
                    wsl.linux_path.trim_end_matches('/'),
                    MANAGED_CACHE_ROOT_SEGMENT,
                    MANAGED_CACHE_SUBDIR,
                    id
                )
            });

            match &source {
                MarketplaceSource::Git(url) => {
                    clone_remote_git(db, &location, url, &cache_dir, linux_cache_dir.as_deref())
                        .await?;
                }
                MarketplaceSource::Json(url) => {
                    download_marketplace_json(db, url, &cache_dir).await?;
                }
                MarketplaceSource::Local => unreachable!(),
            }

            let _ = resolve_workspace_marketplace_path(&cache_dir)?;
            register_workspace_root_path(db, &cache_dir.to_string_lossy()).await
        }
    }
}

pub async fn remove_codex_plugin_workspace_root(
    db: &crate::db::SqliteDbState,
    raw_path: &str,
) -> Result<(), String> {
    let normalized_path = raw_path.trim();
    if normalized_path.is_empty() {
        return Err("Workspace directory is required".to_string());
    }

    // Clean up managed remote-source cache directories so removing a remote
    // marketplace does not leave a cloned repo / downloaded JSON behind. Local
    // user-picked directories are never under the managed cache root, so they
    // are left untouched (matching the previous behavior).
    if let Ok(location) = runtime_location::get_codex_runtime_location_async(db).await {
        let cache_root = managed_cache_root(&location.host_path);
        let path_buf = PathBuf::from(normalized_path);
        if path_buf.starts_with(&cache_root) && path_buf.exists() {
            let _ = fs::remove_dir_all(&path_buf);
        }
    }

    let mut workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    workspace_root_paths
        .retain(|existing_path| !path_strings_equal(existing_path, normalized_path));
    save_workspace_root_paths(db, &workspace_root_paths).await
}

#[cfg(test)]
mod tests {
    use super::{
        classify_marketplace_source, marketplace_cache_id, resolve_workspace_marketplace_path,
        MarketplaceSource,
    };
    use tempfile::tempdir;

    #[test]
    fn resolve_workspace_marketplace_path_prefers_direct_marketplace() {
        let temp_dir = tempdir().expect("create temp dir");
        let workspace_root = temp_dir.path().join("workspace");
        std::fs::create_dir_all(workspace_root.join(".agents/plugins"))
            .expect("create marketplace dir");
        std::fs::write(
            workspace_root.join(".agents/plugins/marketplace.json"),
            r#"{"name":"demo","plugins":[]}"#,
        )
        .expect("write marketplace");

        let (marketplace_path, repo_root, resolution_source) =
            resolve_workspace_marketplace_path(&workspace_root)
                .expect("resolve direct marketplace");

        assert_eq!(
            marketplace_path,
            workspace_root.join(".agents/plugins/marketplace.json")
        );
        assert_eq!(repo_root, None);
        assert_eq!(resolution_source, "direct");
    }

    #[test]
    fn describe_workspace_root_resolves_git_repo_marketplace() {
        use super::describe_workspace_root;
        let temp_dir = tempdir().expect("create temp dir");
        let repo_root = temp_dir.path().join("repo");
        let workspace_root = repo_root.join("nested/project");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create git dir");
        std::fs::create_dir_all(repo_root.join(".agents/plugins")).expect("create marketplace dir");
        std::fs::create_dir_all(&workspace_root).expect("create workspace dir");
        std::fs::write(
            repo_root.join(".agents/plugins/marketplace.json"),
            r#"{"name":"demo","plugins":[]}"#,
        )
        .expect("write marketplace");

        let workspace_status = describe_workspace_root(&workspace_root.to_string_lossy());

        assert_eq!(workspace_status.status, "ready");
        assert_eq!(
            workspace_status.resolution_source.as_deref(),
            Some("gitRepo")
        );
        assert_eq!(
            workspace_status.resolved_marketplace_path.as_deref(),
            Some(
                repo_root
                    .join(".agents/plugins/marketplace.json")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            workspace_status.resolved_repo_root.as_deref(),
            Some(repo_root.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn classify_marketplace_source_detects_local_git_and_json_sources() {
        assert_eq!(
            classify_marketplace_source("C:/Users/foo/market"),
            MarketplaceSource::Local
        );
        assert_eq!(
            classify_marketplace_source(r"C:\Users\foo\market"),
            MarketplaceSource::Local
        );
        assert_eq!(
            classify_marketplace_source("/home/foo/.codex"),
            MarketplaceSource::Local
        );
        assert_eq!(
            classify_marketplace_source("./relative/path"),
            MarketplaceSource::Local
        );
        assert_eq!(
            classify_marketplace_source("~/marketplaces/demo"),
            MarketplaceSource::Local
        );

        assert_eq!(
            classify_marketplace_source("https://github.com/owner/repo"),
            MarketplaceSource::Git("https://github.com/owner/repo".to_string())
        );
        assert_eq!(
            classify_marketplace_source("https://github.com/owner/repo.git"),
            MarketplaceSource::Git("https://github.com/owner/repo.git".to_string())
        );
        assert_eq!(
            classify_marketplace_source("git@github.com:owner/repo.git"),
            MarketplaceSource::Git("git@github.com:owner/repo.git".to_string())
        );
        assert_eq!(
            classify_marketplace_source("owner/repo"),
            MarketplaceSource::Git("https://github.com/owner/repo".to_string())
        );

        assert_eq!(
            classify_marketplace_source("https://example.com/path/marketplace.json"),
            MarketplaceSource::Json("https://example.com/path/marketplace.json".to_string())
        );
        assert_eq!(
            classify_marketplace_source("https://example.com/m.json?raw=1"),
            MarketplaceSource::Json("https://example.com/m.json?raw=1".to_string())
        );
    }

    #[test]
    fn marketplace_cache_id_is_stable_and_sanitized() {
        let id = marketplace_cache_id("https://github.com/owner/my-marketplace");
        assert!(
            id.starts_with("my-marketplace-"),
            "expected readable base, got {id}"
        );

        // Same URL must map to the same id.
        assert_eq!(
            id,
            marketplace_cache_id("https://github.com/owner/my-marketplace")
        );

        // Different URLs must not collide.
        assert_ne!(
            id,
            marketplace_cache_id("https://github.com/owner/other-marketplace")
        );
    }
}
