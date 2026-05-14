use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tokio::process::Command as TokioCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCliProgram {
    pub path: PathBuf,
}

pub fn resolve_local_claude_program() -> LocalCliProgram {
    let mut candidates = Vec::new();

    if let Some(home_dir) = dirs::home_dir() {
        push_command_candidate(
            &mut candidates,
            home_dir.join(".local").join("bin"),
            "claude",
        );
        push_command_candidate(
            &mut candidates,
            home_dir.join(".claude").join("local"),
            "claude",
        );
        push_command_candidate(
            &mut candidates,
            home_dir.join(".claude").join("bin"),
            "claude",
        );
    }

    push_command_candidate(&mut candidates, "/opt/homebrew/bin", "claude");
    push_command_candidate(&mut candidates, "/usr/local/bin", "claude");
    append_node_global_candidates(&mut candidates, "claude");

    resolve_local_cli_program("claude", candidates)
}

pub fn resolve_local_opencode_program() -> LocalCliProgram {
    let mut candidates = Vec::new();

    if let Some(home_dir) = dirs::home_dir() {
        push_command_candidate(
            &mut candidates,
            home_dir.join(".opencode").join("bin"),
            "opencode",
        );
        push_command_candidate(
            &mut candidates,
            home_dir.join(".local").join("bin"),
            "opencode",
        );
        push_command_candidate(
            &mut candidates,
            home_dir.join(".cache").join("opencode").join("bin"),
            "opencode",
        );
    }

    push_command_candidate(&mut candidates, "/opt/homebrew/bin", "opencode");
    push_command_candidate(&mut candidates, "/usr/local/bin", "opencode");
    append_node_global_candidates(&mut candidates, "opencode");

    resolve_local_cli_program("opencode", candidates)
}

pub fn build_local_std_command(program_path: &Path) -> Command {
    build_local_std_command_impl(program_path)
}

pub fn build_local_tokio_command(program_path: &Path) -> TokioCommand {
    build_local_tokio_command_impl(program_path)
}

pub fn local_cli_missing_hint(command_name: &str) -> String {
    format!(
        "未找到 `{command_name}` CLI。AI Toolbox 已检查当前 PATH、常见安装路径，以及 nvm、volta、fnm、nvm-windows 管理的 Node 全局 bin；macOS 从 Dock/Finder/Spotlight 启动时不会继承终端 shell PATH。请确认 CLI 已安装。"
    )
}

fn resolve_local_cli_program(command_name: &str, candidate_paths: Vec<PathBuf>) -> LocalCliProgram {
    if let Some(path) = resolve_cli_from_path(command_name) {
        return LocalCliProgram { path };
    }

    if let Some(path) = select_existing_command_path(&candidate_paths) {
        return LocalCliProgram { path };
    }

    LocalCliProgram {
        path: PathBuf::from(command_name),
    }
}

fn resolve_cli_from_path(command_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let lookup_command = "where";

    #[cfg(not(target_os = "windows"))]
    let lookup_command = "which";

    let output = Command::new(lookup_command)
        .arg(command_name)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let paths = parse_lookup_command_output(&stdout);
    let existing_paths = paths
        .into_iter()
        .filter(|path| is_existing_command_path(path))
        .collect::<Vec<_>>();

    select_command_path(&existing_paths)
}

fn parse_lookup_command_output(stdout: &str) -> Vec<PathBuf> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn select_existing_command_path(paths: &[PathBuf]) -> Option<PathBuf> {
    let existing_paths = paths
        .iter()
        .filter(|path| is_existing_command_path(path))
        .cloned()
        .collect::<Vec<_>>();

    select_command_path(&existing_paths)
}

#[cfg(target_os = "windows")]
fn select_command_path(paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .min_by_key(|path| windows_command_path_priority(path))
        .cloned()
}

#[cfg(not(target_os = "windows"))]
fn select_command_path(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.first().cloned()
}

#[cfg(target_os = "windows")]
fn windows_command_path_priority(path: &Path) -> usize {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("exe") => 0,
        Some("cmd") => 1,
        Some("bat") => 2,
        Some("com") => 3,
        Some("ps1") => 4,
        _ => 5,
    }
}

fn append_node_global_candidates(candidates: &mut Vec<PathBuf>, command_name: &str) {
    if let Some(home_dir) = dirs::home_dir() {
        append_nvm_candidates_from_dir(candidates, &home_dir.join(".nvm"), command_name);

        if let Some(volta_home) = env_path("VOLTA_HOME") {
            push_command_candidate(candidates, volta_home.join("bin"), command_name);
        } else {
            push_command_candidate(
                candidates,
                home_dir.join(".volta").join("bin"),
                command_name,
            );
        }

        for fnm_base_dir in default_fnm_base_dirs(&home_dir) {
            append_fnm_candidates_from_dir(candidates, &fnm_base_dir, command_name);
        }
    }

    if let Some(nvm_dir) = env_path("NVM_DIR") {
        append_nvm_candidates_from_dir(candidates, &nvm_dir, command_name);
    }

    if let Some(fnm_dir) = env_path("FNM_DIR") {
        append_fnm_candidates_from_dir(candidates, &fnm_dir, command_name);
    }

    append_windows_node_candidates(candidates, command_name);
}

fn append_nvm_candidates_from_dir(
    candidates: &mut Vec<PathBuf>,
    nvm_dir: &Path,
    command_name: &str,
) {
    for path in collect_nvm_candidates(nvm_dir, command_name) {
        push_unique_candidate(candidates, path);
    }
}

fn collect_nvm_candidates(nvm_dir: &Path, command_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(default_version) = read_default_node_version_alias(nvm_dir) {
        push_command_candidate(
            &mut candidates,
            nvm_dir
                .join("versions")
                .join("node")
                .join(default_version)
                .join("bin"),
            command_name,
        );
    }

    append_node_version_bins(
        &mut candidates,
        &nvm_dir.join("versions").join("node"),
        command_name,
    );

    candidates
}

fn read_default_node_version_alias(nvm_dir: &Path) -> Option<String> {
    let default_alias = fs::read_to_string(nvm_dir.join("alias").join("default")).ok()?;
    let alias_value = default_alias.lines().next()?.trim();
    normalize_node_version_dir(alias_value)
}

fn normalize_node_version_dir(alias_value: &str) -> Option<String> {
    let value = alias_value.trim();
    if value.is_empty() {
        return None;
    }

    let value = value.strip_prefix("node/").unwrap_or(value);
    if value.starts_with('v')
        && value
            .chars()
            .nth(1)
            .is_some_and(|character| character.is_ascii_digit())
    {
        return Some(value.to_string());
    }

    if value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        return Some(format!("v{value}"));
    }

    None
}

fn append_fnm_candidates_from_dir(
    candidates: &mut Vec<PathBuf>,
    fnm_dir: &Path,
    command_name: &str,
) {
    for path in collect_fnm_candidates(fnm_dir, command_name) {
        push_unique_candidate(candidates, path);
    }
}

fn collect_fnm_candidates(fnm_dir: &Path, command_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    let default_alias = fnm_dir.join("aliases").join("default");
    push_command_candidate(&mut candidates, default_alias.join("bin"), command_name);
    push_command_candidate(
        &mut candidates,
        default_alias.join("installation").join("bin"),
        command_name,
    );

    append_fnm_version_bins(
        &mut candidates,
        &fnm_dir.join("node-versions"),
        command_name,
    );
    append_fnm_version_bins(&mut candidates, &fnm_dir.join("versions"), command_name);

    candidates
}

fn append_fnm_version_bins(candidates: &mut Vec<PathBuf>, version_root: &Path, command_name: &str) {
    for version_dir in sorted_child_dirs_desc(version_root) {
        push_command_candidate(
            candidates,
            version_dir.join("installation").join("bin"),
            command_name,
        );
        push_command_candidate(candidates, version_dir.join("bin"), command_name);
    }
}

fn append_node_version_bins(
    candidates: &mut Vec<PathBuf>,
    version_root: &Path,
    command_name: &str,
) {
    for version_dir in sorted_child_dirs_desc(version_root) {
        push_command_candidate(candidates, version_dir.join("bin"), command_name);
    }
}

fn sorted_child_dirs_desc(root: &Path) -> Vec<PathBuf> {
    let mut dirs = fs::read_dir(root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();

    dirs.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    dirs
}

fn default_fnm_base_dirs(home_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        dirs.push(
            home_dir
                .join("Library")
                .join("Application Support")
                .join("fnm"),
        );
        dirs.push(home_dir.join(".local").join("share").join("fnm"));
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(xdg_data_home) = env_path("XDG_DATA_HOME") {
            dirs.push(xdg_data_home.join("fnm"));
        }
        dirs.push(home_dir.join(".local").join("share").join("fnm"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(app_data) = env_path("APPDATA") {
            dirs.push(app_data.join("fnm"));
            dirs.push(app_data.join("npm"));
        }
        if let Some(local_app_data) = env_path("LOCALAPPDATA") {
            dirs.push(local_app_data.join("fnm"));
        }
        dirs.push(home_dir.join("AppData").join("Roaming").join("fnm"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        dirs.push(home_dir.join(".local").join("share").join("fnm"));
    }

    dirs
}

#[cfg(target_os = "windows")]
fn append_windows_node_candidates(candidates: &mut Vec<PathBuf>, command_name: &str) {
    if let Some(app_data) = env_path("APPDATA") {
        push_command_candidate(candidates, app_data.join("npm"), command_name);
        append_nvm_windows_versions(candidates, &app_data.join("nvm"), command_name);
    }

    if let Some(local_app_data) = env_path("LOCALAPPDATA") {
        append_nvm_windows_versions(candidates, &local_app_data.join("nvm"), command_name);
        push_command_candidate(
            candidates,
            local_app_data.join("Volta").join("bin"),
            command_name,
        );
    }

    if let Some(nvm_home) = env_path("NVM_HOME") {
        append_nvm_windows_versions(candidates, &nvm_home, command_name);
    }

    if let Some(nvm_symlink) = env_path("NVM_SYMLINK") {
        push_command_candidate(candidates, nvm_symlink, command_name);
    }
}

#[cfg(not(target_os = "windows"))]
fn append_windows_node_candidates(_candidates: &mut Vec<PathBuf>, _command_name: &str) {}

#[cfg(target_os = "windows")]
fn append_nvm_windows_versions(candidates: &mut Vec<PathBuf>, nvm_root: &Path, command_name: &str) {
    for version_dir in sorted_child_dirs_desc(nvm_root) {
        push_command_candidate(candidates, version_dir, command_name);
    }
}

fn push_command_candidate(
    candidates: &mut Vec<PathBuf>,
    bin_dir: impl AsRef<Path>,
    command_name: &str,
) {
    let base_path = bin_dir.as_ref().join(command_name);
    push_unique_candidate(candidates, base_path.clone());

    #[cfg(target_os = "windows")]
    {
        for extension in ["exe", "cmd", "bat", "com", "ps1"] {
            push_unique_candidate(candidates, base_path.with_extension(extension));
        }
    }
}

fn push_unique_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !candidates.iter().any(|candidate| candidate == &path) {
        candidates.push(path);
    }
}

fn is_existing_command_path(path: &Path) -> bool {
    path.is_file()
}

fn env_path(name: &str) -> Option<PathBuf> {
    let value = env::var_os(name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(target_os = "windows")]
fn build_local_std_command_impl(program_path: &Path) -> Command {
    match command_extension(program_path).as_deref() {
        Some("cmd") | Some("bat") => {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(program_path);
            command
        }
        Some("ps1") => {
            let mut command = Command::new("powershell");
            command
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(program_path);
            command
        }
        _ => Command::new(program_path),
    }
}

#[cfg(not(target_os = "windows"))]
fn build_local_std_command_impl(program_path: &Path) -> Command {
    Command::new(program_path)
}

#[cfg(target_os = "windows")]
fn build_local_tokio_command_impl(program_path: &Path) -> TokioCommand {
    match command_extension(program_path).as_deref() {
        Some("cmd") | Some("bat") => {
            let mut command = TokioCommand::new("cmd");
            command.arg("/C").arg(program_path);
            command
        }
        Some("ps1") => {
            let mut command = TokioCommand::new("powershell");
            command
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(program_path);
            command
        }
        _ => TokioCommand::new(program_path),
    }
}

#[cfg(not(target_os = "windows"))]
fn build_local_tokio_command_impl(program_path: &Path) -> TokioCommand {
    TokioCommand::new(program_path)
}

#[cfg(target_os = "windows")]
fn command_extension(program_path: &Path) -> Option<String> {
    program_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{collect_fnm_candidates, collect_nvm_candidates, normalize_node_version_dir};

    use std::fs;
    use std::path::{Path, PathBuf};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ai-toolbox-cli-resolver-{label}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&path).expect("failed to create test directory");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn normalize_node_version_dir_accepts_nvm_version_aliases() {
        assert_eq!(
            normalize_node_version_dir("v22.18.0"),
            Some("v22.18.0".to_string())
        );
        assert_eq!(
            normalize_node_version_dir("22.18.0"),
            Some("v22.18.0".to_string())
        );
        assert_eq!(normalize_node_version_dir("stable"), None);
        assert_eq!(normalize_node_version_dir("lts/iron"), None);
    }

    #[test]
    fn nvm_default_alias_candidate_precedes_scanned_versions() {
        let test_dir = TestDir::new("nvm-default");
        let nvm_dir = test_dir.path().join(".nvm");
        let alias_dir = nvm_dir.join("alias");
        let default_bin = nvm_dir
            .join("versions")
            .join("node")
            .join("v22.18.0")
            .join("bin");
        let older_bin = nvm_dir
            .join("versions")
            .join("node")
            .join("v20.10.0")
            .join("bin");

        fs::create_dir_all(&alias_dir).expect("failed to create alias dir");
        fs::create_dir_all(&default_bin).expect("failed to create default bin dir");
        fs::create_dir_all(&older_bin).expect("failed to create older bin dir");
        fs::write(alias_dir.join("default"), "22.18.0\n").expect("failed to write default alias");

        let candidates = collect_nvm_candidates(&nvm_dir, "claude");

        assert_eq!(candidates.first(), Some(&default_bin.join("claude")));
        assert!(candidates.contains(&older_bin.join("claude")));
    }

    #[test]
    fn fnm_default_alias_candidate_precedes_scanned_versions() {
        let test_dir = TestDir::new("fnm-default");
        let fnm_dir = test_dir.path().join("fnm");
        let default_bin = fnm_dir.join("aliases").join("default").join("bin");
        let version_bin = fnm_dir
            .join("node-versions")
            .join("v22.18.0")
            .join("installation")
            .join("bin");

        fs::create_dir_all(&default_bin).expect("failed to create default alias bin dir");
        fs::create_dir_all(&version_bin).expect("failed to create fnm version bin dir");

        let candidates = collect_fnm_candidates(&fnm_dir, "opencode");

        assert_eq!(candidates.first(), Some(&default_bin.join("opencode")));
        assert!(candidates.contains(&version_bin.join("opencode")));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_windows_command_path_prefers_cmd_over_extensionless() {
        let selected = super::select_command_path(&[
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode"),
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode.cmd"),
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode.ps1"),
        ])
        .expect("expected selected path");

        assert_eq!(
            selected,
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode.cmd")
        );
    }
}
