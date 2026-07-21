use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tokio::process::Command as TokioCommand;

/// Windows CREATE_NO_WINDOW: hide console for short-lived CLI spawns from a GUI process.
/// Prefer this over DETACHED_PROCESS when capturing stdout/stderr via `.output()`.
#[cfg(target_os = "windows")]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply CREATE_NO_WINDOW on Windows so GUI hosts do not flash a console.
pub fn apply_create_no_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

/// Apply CREATE_NO_WINDOW on Windows for tokio process commands.
pub fn apply_create_no_window_tokio(command: &mut TokioCommand) {
    // tokio::process::Command exposes creation_flags as an inherent Windows method.
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

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

pub fn resolve_local_pi_program() -> LocalCliProgram {
    let mut candidates = Vec::new();

    if let Some(home_dir) = dirs::home_dir() {
        push_command_candidate(&mut candidates, home_dir.join(".local").join("bin"), "pi");
    }

    push_command_candidate(&mut candidates, "/opt/homebrew/bin", "pi");
    push_command_candidate(&mut candidates, "/usr/local/bin", "pi");
    append_node_global_candidates(&mut candidates, "pi");

    resolve_local_cli_program("pi", candidates)
}

pub fn resolve_local_grok_program() -> LocalCliProgram {
    let mut candidates = Vec::new();
    if let Some(home_dir) = dirs::home_dir() {
        push_command_candidate(&mut candidates, home_dir.join(".local").join("bin"), "grok");
    }
    push_command_candidate(&mut candidates, "/opt/homebrew/bin", "grok");
    push_command_candidate(&mut candidates, "/usr/local/bin", "grok");
    append_node_global_candidates(&mut candidates, "grok");
    resolve_local_cli_program("grok", candidates)
}

pub fn resolve_local_npx_program() -> LocalCliProgram {
    let mut candidates = Vec::new();

    push_command_candidate(&mut candidates, "/opt/homebrew/bin", "npx");
    push_command_candidate(&mut candidates, "/usr/local/bin", "npx");
    append_node_global_candidates(&mut candidates, "npx");

    resolve_local_cli_program("npx", candidates)
}

pub fn build_local_std_command(program_path: &Path) -> Command {
    build_local_std_command_impl(program_path)
}

pub fn build_local_tokio_command(program_path: &Path) -> TokioCommand {
    build_local_tokio_command_impl(program_path)
}

pub fn local_cli_missing_hint(command_name: &str) -> String {
    format!(
        "未找到 `{command_name}` CLI。AI Toolbox 已检查当前 PATH、常见安装路径，以及 nvm、volta、fnm、nvm-windows、bun、mise、asdf 管理的全局 bin；macOS 从 Dock/Finder/Spotlight 启动时不会继承终端 shell PATH。请确认 CLI 已安装。"
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

    let mut lookup = Command::new(lookup_command);
    lookup.arg(command_name);
    apply_create_no_window(&mut lookup);
    let output = lookup.output().ok()?;
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
    let home_dir = dirs::home_dir();
    append_node_global_candidates_with_home(candidates, command_name, home_dir.as_deref());
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

fn build_local_command_path(program_path: &Path, current_path: Option<&OsStr>) -> Option<OsString> {
    let mut dirs = Vec::new();

    if let Some(program_dir) = program_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        push_existing_dir(&mut dirs, program_dir.to_path_buf());
    }

    append_node_runtime_dirs(&mut dirs);
    append_mise_asdf_runtime_dirs(&mut dirs, dirs::home_dir().as_deref());

    if let Some(current_path) = current_path {
        for path in env::split_paths(current_path) {
            push_unique_dir(&mut dirs, path);
        }
    }

    if dirs.is_empty() {
        return None;
    }

    env::join_paths(dirs).ok()
}

fn append_node_runtime_dirs(dirs: &mut Vec<PathBuf>) {
    append_node_runtime_dirs_with_home(dirs, dirs::home_dir().as_deref());
}

fn append_node_runtime_dirs_with_home(dirs: &mut Vec<PathBuf>, home_dir: Option<&Path>) {
    let mut candidates = Vec::new();

    if let Some(home_dir) = home_dir {
        push_command_candidate(&mut candidates, home_dir.join(".local").join("bin"), "node");
    }
    push_command_candidate(&mut candidates, "/opt/homebrew/bin", "node");
    push_command_candidate(&mut candidates, "/usr/local/bin", "node");
    append_node_global_candidates_with_home(&mut candidates, "node", home_dir);

    for candidate in candidates {
        if !is_existing_command_path(&candidate) {
            continue;
        }
        if let Some(parent) = candidate.parent() {
            push_existing_dir(dirs, parent.to_path_buf());
        }
    }
}

/// Append mise / asdf runtime dirs to the child process PATH.
///
/// mise/asdf shims are thin wrappers that `exec mise` / `exec asdf`, so they need the
/// manager binary itself on PATH. GUI-launched children lack `~/.local/bin` (where mise is
/// curl-installed) and Homebrew prefixes. Inject only when mise/asdf shims actually exist,
/// so non-mise/asdf environments are left untouched.
fn append_mise_asdf_runtime_dirs(dirs: &mut Vec<PathBuf>, home_dir: Option<&Path>) {
    let Some(home) = home_dir else {
        return;
    };
    let mise_data =
        env_path("MISE_DATA_DIR").unwrap_or_else(|| home.join(".local").join("share").join("mise"));
    let asdf_data = env_path("ASDF_DATA_DIR").unwrap_or_else(|| home.join(".asdf"));
    let mise_shims = mise_data.join("shims");
    let asdf_shims = asdf_data.join("shims");

    if mise_shims.is_dir() {
        push_existing_dir(dirs, mise_shims);
        push_existing_dir(dirs, home.join(".local").join("bin"));
        push_existing_dir(dirs, PathBuf::from("/opt/homebrew/bin"));
        push_existing_dir(dirs, PathBuf::from("/usr/local/bin"));
    }
    if asdf_shims.is_dir() {
        push_existing_dir(dirs, asdf_shims);
        push_existing_dir(dirs, home.join(".local").join("bin"));
        push_existing_dir(dirs, PathBuf::from("/opt/homebrew/bin"));
        push_existing_dir(dirs, PathBuf::from("/usr/local/bin"));
    }
}

fn append_node_global_candidates_with_home(
    candidates: &mut Vec<PathBuf>,
    command_name: &str,
    home_dir: Option<&Path>,
) {
    if let Some(home_dir) = home_dir {
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

        for fnm_base_dir in default_fnm_base_dirs(home_dir) {
            append_fnm_candidates_from_dir(candidates, &fnm_base_dir, command_name);
        }
    }

    if let Some(nvm_dir) = env_path("NVM_DIR") {
        append_nvm_candidates_from_dir(candidates, &nvm_dir, command_name);
    }

    if let Some(fnm_dir) = env_path("FNM_DIR") {
        append_fnm_candidates_from_dir(candidates, &fnm_dir, command_name);
    }

    append_bun_candidates(candidates, command_name, home_dir);
    append_mise_asdf_candidates(candidates, command_name, home_dir);
    append_windows_node_candidates(candidates, command_name);
}

fn append_bun_candidates(
    candidates: &mut Vec<PathBuf>,
    command_name: &str,
    home_dir: Option<&Path>,
) {
    for path in collect_bun_candidates(command_name, env_path("BUN_INSTALL").as_deref(), home_dir) {
        push_unique_candidate(candidates, path);
    }
}

fn collect_bun_candidates(
    command_name: &str,
    bun_install: Option<&Path>,
    home_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(bun_install) = bun_install {
        push_command_candidate(&mut candidates, bun_install.join("bin"), command_name);
    }

    if let Some(home_dir) = home_dir {
        push_command_candidate(
            &mut candidates,
            home_dir.join(".bun").join("bin"),
            command_name,
        );
    }

    candidates
}

/// Append mise / asdf managed CLI candidates.
///
/// Covers both the version-pinned node install bin (for `mise use node` + `npm -g` installs)
/// and the shim directory — the stable entry point for mise/asdf backend tools, including
/// `npm:` backend packages whose real bin path embeds the package name and cannot be
/// generalized. Respects `$MISE_DATA_DIR` / `$ASDF_DATA_DIR` with `~/.local/share/mise` /
/// `~/.asdf` fallbacks.
fn append_mise_asdf_candidates(
    candidates: &mut Vec<PathBuf>,
    command_name: &str,
    home_dir: Option<&Path>,
) {
    let mise_roots = env_path("MISE_DATA_DIR")
        .into_iter()
        .chain(home_dir.map(|home| home.join(".local").join("share").join("mise")))
        .collect::<Vec<_>>();
    for root in &mise_roots {
        append_node_version_bins(
            candidates,
            &root.join("installs").join("node"),
            command_name,
        );
        push_command_candidate(candidates, root.join("shims"), command_name);
    }

    let asdf_roots = env_path("ASDF_DATA_DIR")
        .into_iter()
        .chain(home_dir.map(|home| home.join(".asdf")))
        .collect::<Vec<_>>();
    for root in &asdf_roots {
        append_node_version_bins(
            candidates,
            &root.join("installs").join("nodejs"),
            command_name,
        );
        push_command_candidate(candidates, root.join("shims"), command_name);
    }
}

fn push_existing_dir(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if path.is_dir() {
        push_unique_dir(dirs, path);
    }
}

fn push_unique_dir(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if !dirs.iter().any(|existing| existing == &path) {
        dirs.push(path);
    }
}

fn apply_local_std_command_environment(command: &mut Command, program_path: &Path) {
    if let Some(path) = build_local_command_path(program_path, env::var_os("PATH").as_deref()) {
        command.env("PATH", path);
    }
}

fn apply_local_tokio_command_environment(command: &mut TokioCommand, program_path: &Path) {
    if let Some(path) = build_local_command_path(program_path, env::var_os("PATH").as_deref()) {
        command.env("PATH", path);
    }
}

#[cfg(target_os = "windows")]
fn build_local_std_command_impl(program_path: &Path) -> Command {
    let mut command = match command_extension(program_path).as_deref() {
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
    };
    apply_create_no_window(&mut command);
    apply_local_std_command_environment(&mut command, program_path);
    command
}

#[cfg(not(target_os = "windows"))]
fn build_local_std_command_impl(program_path: &Path) -> Command {
    let mut command = Command::new(program_path);
    apply_local_std_command_environment(&mut command, program_path);
    command
}

#[cfg(target_os = "windows")]
fn build_local_tokio_command_impl(program_path: &Path) -> TokioCommand {
    let mut command = match command_extension(program_path).as_deref() {
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
    };
    apply_create_no_window_tokio(&mut command);
    apply_local_tokio_command_environment(&mut command, program_path);
    command
}

#[cfg(not(target_os = "windows"))]
fn build_local_tokio_command_impl(program_path: &Path) -> TokioCommand {
    let mut command = TokioCommand::new(program_path);
    apply_local_tokio_command_environment(&mut command, program_path);
    command
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
    use super::{
        collect_bun_candidates, collect_fnm_candidates, collect_nvm_candidates,
        normalize_node_version_dir,
    };

    use std::env;
    use std::ffi::OsString;
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

    #[test]
    fn bun_install_candidate_precedes_default_home_bun_bin() {
        let test_dir = TestDir::new("bun-install");
        let bun_install = test_dir.path().join("custom-bun");
        let home_dir = test_dir.path().join("home");
        let custom_bin = bun_install.join("bin");
        let default_bin = home_dir.join(".bun").join("bin");

        fs::create_dir_all(&custom_bin).expect("failed to create custom bun bin");
        fs::create_dir_all(&default_bin).expect("failed to create default bun bin");

        let candidates = collect_bun_candidates("pi", Some(&bun_install), Some(&home_dir));

        assert_eq!(candidates.first(), Some(&custom_bin.join("pi")));
        assert!(candidates.contains(&default_bin.join("pi")));
    }

    #[test]
    fn bun_default_home_bin_is_candidate_without_bun_install() {
        let test_dir = TestDir::new("bun-default");
        let home_dir = test_dir.path().join("home");
        let default_bin = home_dir.join(".bun").join("bin");
        fs::create_dir_all(&default_bin).expect("failed to create default bun bin");

        let candidates = collect_bun_candidates("pi", None, Some(&home_dir));

        assert!(candidates.contains(&default_bin.join("pi")));
        assert!(!candidates
            .iter()
            .any(|path| path.to_string_lossy().contains("custom-bun")));
    }

    #[test]
    fn local_command_path_includes_program_dir_and_existing_path() {
        let test_dir = TestDir::new("local-command-path");
        let program_dir = test_dir.path().join("bin");
        let existing_path_dir = test_dir.path().join("existing");
        fs::create_dir_all(&program_dir).expect("failed to create program dir");
        fs::create_dir_all(&existing_path_dir).expect("failed to create existing path dir");
        let program_path = program_dir.join("pi");
        fs::write(&program_path, "#!/usr/bin/env node\n").expect("failed to write program");

        let path = super::build_local_command_path(
            &program_path,
            Some(OsString::from(existing_path_dir.as_os_str()).as_os_str()),
        )
        .expect("expected PATH");
        let dirs = env::split_paths(&path).collect::<Vec<_>>();

        assert_eq!(dirs.first(), Some(&program_dir));
        assert!(dirs.contains(&existing_path_dir));
    }

    #[test]
    fn node_runtime_dirs_include_nvm_default_node_bin() {
        let test_dir = TestDir::new("node-runtime-dirs");
        let home_dir = test_dir.path().join("home");
        let nvm_dir = home_dir.join(".nvm");
        let alias_dir = nvm_dir.join("alias");
        let node_bin = nvm_dir
            .join("versions")
            .join("node")
            .join("v22.18.0")
            .join("bin");

        fs::create_dir_all(&alias_dir).expect("failed to create alias dir");
        fs::create_dir_all(&node_bin).expect("failed to create node bin dir");
        fs::write(alias_dir.join("default"), "22.18.0\n").expect("failed to write default alias");
        fs::write(node_bin.join("node"), "").expect("failed to write node");

        let mut dirs = Vec::new();
        super::append_node_runtime_dirs_with_home(&mut dirs, Some(&home_dir));

        assert!(dirs.contains(&node_bin));
    }

    #[test]
    fn mise_shims_and_node_install_bins_are_candidates() {
        let test_dir = TestDir::new("mise");
        let home_dir = test_dir.path().join("home");
        let shims_dir = home_dir
            .join(".local")
            .join("share")
            .join("mise")
            .join("shims");
        let node_bin = home_dir
            .join(".local")
            .join("share")
            .join("mise")
            .join("installs")
            .join("node")
            .join("22.18.0")
            .join("bin");
        fs::create_dir_all(&shims_dir).expect("failed to create mise shims dir");
        fs::create_dir_all(&node_bin).expect("failed to create mise node bin dir");

        let mut candidates = Vec::new();
        super::append_mise_asdf_candidates(&mut candidates, "pi", Some(&home_dir));

        assert!(candidates.contains(&shims_dir.join("pi")));
        assert!(candidates.contains(&node_bin.join("pi")));
    }

    #[test]
    fn asdf_shims_and_node_install_bins_are_candidates() {
        let test_dir = TestDir::new("asdf");
        let home_dir = test_dir.path().join("home");
        let shims_dir = home_dir.join(".asdf").join("shims");
        let node_bin = home_dir
            .join(".asdf")
            .join("installs")
            .join("nodejs")
            .join("22.18.0")
            .join("bin");
        fs::create_dir_all(&shims_dir).expect("failed to create asdf shims dir");
        fs::create_dir_all(&node_bin).expect("failed to create asdf node bin dir");

        let mut candidates = Vec::new();
        super::append_mise_asdf_candidates(&mut candidates, "pi", Some(&home_dir));

        assert!(candidates.contains(&shims_dir.join("pi")));
        assert!(candidates.contains(&node_bin.join("pi")));
    }

    #[test]
    fn mise_runtime_dirs_added_only_when_shims_present() {
        let test_dir = TestDir::new("mise-runtime");
        let home_dir = test_dir.path().join("home");
        let mise_shims = home_dir
            .join(".local")
            .join("share")
            .join("mise")
            .join("shims");
        let local_bin = home_dir.join(".local").join("bin");

        // No mise shims yet -> nothing injected.
        let mut dirs = Vec::new();
        super::append_mise_asdf_runtime_dirs(&mut dirs, Some(&home_dir));
        assert!(dirs.is_empty());

        // With shims present -> shims + manager bin dirs injected.
        fs::create_dir_all(&mise_shims).expect("failed to create mise shims dir");
        fs::create_dir_all(&local_bin).expect("failed to create local bin dir");
        super::append_mise_asdf_runtime_dirs(&mut dirs, Some(&home_dir));

        assert!(dirs.contains(&mise_shims));
        assert!(dirs.contains(&local_bin));
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
