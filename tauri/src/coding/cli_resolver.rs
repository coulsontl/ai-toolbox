use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::process::Command as TokioCommand;

/// In-memory registry of user-specified manual CLI paths, keyed by command name
/// (e.g. `claude`, `opencode`, `grok`, `kimi`, `pi`, `omp`, `hermes`, `dsh`, `openclaw`).
///
/// When set, these paths take priority over PATH / candidate-dir resolution in
/// `resolve_local_cli_program`. The map is refreshed whenever application
/// settings are loaded or saved (see `crate::settings::store`). If a configured
/// path no longer exists on disk, resolution silently falls back to the normal
/// discovery logic.
fn manual_cli_override_registry() -> &'static Mutex<HashMap<String, String>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Replace the manual CLI override registry with the given map (command name -> path).
pub fn set_manual_cli_overrides(overrides: HashMap<String, String>) {
    let mut registry = manual_cli_override_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Drop empty / whitespace-only entries.
    *registry = overrides
        .into_iter()
        .filter(|(_, path)| !path.trim().is_empty())
        .collect();
}

/// Look up a user-specified explicit path for a command name. Returns the path
/// only when it still exists on disk (mirrors the "file not found → fallback"
/// requirement).
fn manual_cli_override_path(command_name: &str) -> Option<PathBuf> {
    let registry = manual_cli_override_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = registry.get(command_name)?;
    let path_buf = PathBuf::from(path);
    if is_existing_command_path(&path_buf) {
        Some(path_buf)
    } else {
        None
    }
}

/// Whether a user has explicitly registered a manual CLI path for `command_name`.
pub fn has_manual_cli_override(command_name: &str) -> bool {
    manual_cli_override_path(command_name).is_some()
}

/// Whether a manual CLI path is configured for `command_name`, regardless of
/// whether the file currently exists on disk. Used to tell users their saved
/// override is present but currently unusable.
pub fn manual_cli_path_configured(command_name: &str) -> bool {
    let registry = manual_cli_override_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry
        .get(command_name)
        .is_some_and(|path| !path.trim().is_empty())
}

/// Human-readable note to append when a configured manual CLI path exists but
/// cannot be used, guiding the user back to the tab's "More Options".
pub fn manual_cli_config_hint(command_name: &str) -> String {
    if manual_cli_path_configured(command_name) && manual_cli_override_path(command_name).is_none()
    {
        format!(
            "检测到你已在“更多选项”中手动指定了 `{command_name}` 的 CLI 路径，但该路径当前不存在或不可用；请到对应 tab 的“更多选项”中确认路径是否正确。"
        )
    } else {
        String::new()
    }
}

/// Private alias so `local_cli_missing_hint` can reuse the same copy.
fn manual_cli_override_hint(command_name: &str) -> String {
    manual_cli_config_hint(command_name)
}

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

/// Whether a CLI command can be resolved on the current PATH (via `where`/`which`).
///
/// Used before spawning a terminal that runs a long-lived CLI such as
/// `hermes dashboard` / `openclaw gateway`: those launches use `cmd /C start`
/// which always returns Ok even when the inner CLI is missing, so without this
/// check the UI would show a success toast for a service that never started.
pub fn cli_resolved_on_path(command_name: &str) -> bool {
    resolve_cli_from_path(command_name).is_some()
}

/// Resolve a local CLI program by name, checking PATH first then extra candidate dirs.
pub fn resolve_named_cli_program(
    command_name: &str,
    candidate_paths: Vec<PathBuf>,
) -> LocalCliProgram {
    resolve_local_cli_program(command_name, candidate_paths)
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

pub fn resolve_local_codex_program() -> LocalCliProgram {
    let mut candidates = Vec::new();

    if let Some(home_dir) = dirs::home_dir() {
        push_command_candidate(
            &mut candidates,
            home_dir.join(".local").join("bin"),
            "codex",
        );
        push_command_candidate(&mut candidates, home_dir.join(".codex").join("bin"), "codex");
    }

    // The Codex desktop app bundles its own CLI, and the standalone installer
    // keeps one under the user's local programs dir. Neither is on the GUI
    // process PATH when the app is started from the dock / start menu.
    #[cfg(target_os = "windows")]
    if let Some(local_data_dir) = dirs::data_local_dir() {
        push_command_candidate(
            &mut candidates,
            local_data_dir
                .join("Programs")
                .join("OpenAI")
                .join("Codex")
                .join("bin"),
            "codex",
        );
        // Desktop bundles live in `OpenAI\Codex\bin\<build-hash>\codex.exe`.
        append_versioned_command_candidates(
            &mut candidates,
            &local_data_dir.join("OpenAI").join("Codex").join("bin"),
            "codex",
        );
    }

    push_command_candidate(&mut candidates, "/opt/homebrew/bin", "codex");
    push_command_candidate(&mut candidates, "/usr/local/bin", "codex");
    append_node_global_candidates(&mut candidates, "codex");

    resolve_local_cli_program("codex", candidates)
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

/// Common candidate bin dirs shared by every npm-installed CLI: user `~/.local/bin`,
/// Homebrew, `/usr/local/bin`, plus every Node version manager's global bin
/// (nvm/volta/fnm/bun/mise/asdf). Lets `resolve_local_cli_by_name` find a CLI the
/// GUI process PATH does not inherit (macOS Dock/Finder/Spotlight launch).
fn default_npm_global_candidates(command_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(home_dir) = dirs::home_dir() {
        push_command_candidate(
            &mut candidates,
            home_dir.join(".local").join("bin"),
            command_name,
        );
    }

    push_command_candidate(&mut candidates, "/opt/homebrew/bin", command_name);
    push_command_candidate(&mut candidates, "/usr/local/bin", command_name);
    append_node_global_candidates(&mut candidates, command_name);

    candidates
}

/// Generic CLI resolver: PATH (`where`/`which`) then the common candidate bin dirs
/// above. Returns `None` when nothing resolves — `resolve_local_cli_program`'s bare
/// command-name fallback is the "not found" signal. Prefer this over
/// `cli_resolved_on_path` (PATH-only) so a CLI installed via a version manager is
/// detected even when the GUI host lacks that PATH.
pub fn resolve_local_cli_by_name(command_name: &str) -> Option<LocalCliProgram> {
    let program =
        resolve_named_cli_program(command_name, default_npm_global_candidates(command_name));
    (program.path.as_os_str() != OsStr::new(command_name)).then_some(program)
}

/// Resolve a CLI program by the command name used across the app (e.g. `claude`,
/// `opencode`, `grok`, `kimi`, `pi`, `omp`, `hermes`, `dsh`, `openclaw`). Uses each
/// tool's dedicated resolver when one exists so tool-specific install locations
/// are also honored.
pub fn resolve_local_cli_by_command_name(command_name: &str) -> Option<LocalCliProgram> {
    let program = match command_name {
        "claude" => Some(resolve_local_claude_program()),
        "codex" => Some(resolve_local_codex_program()),
        "opencode" => Some(resolve_local_opencode_program()),
        "grok" => Some(resolve_local_grok_program()),
        "kimi" => Some(resolve_local_kimi_program()),
        "pi" => Some(resolve_local_pi_program()),
        "omp" => Some(resolve_local_omp_program()),
        _ => resolve_local_cli_by_name(command_name),
    }?;
    (program.path.as_os_str() != OsStr::new(command_name)).then_some(program)
}

pub fn resolve_local_pi_program() -> LocalCliProgram {
    resolve_named_cli_program("pi", default_npm_global_candidates("pi"))
}

pub fn resolve_local_omp_program() -> LocalCliProgram {
    resolve_named_cli_program("omp", default_npm_global_candidates("omp"))
}

pub fn resolve_local_grok_program() -> LocalCliProgram {
    resolve_named_cli_program("grok", default_npm_global_candidates("grok"))
}

/// Resolve the Kimi Code CLI (`kimi`, installed by `@moonshot-ai/kimi-code`).
/// Uses the shared npm global candidate paths so GUI-launched processes can
/// discover installs managed by npm, nvm, volta, fnm, bun, mise, or asdf.
pub fn resolve_local_kimi_program() -> LocalCliProgram {
    resolve_named_cli_program("kimi", default_npm_global_candidates("kimi"))
}

pub fn resolve_local_npx_program() -> LocalCliProgram {
    resolve_named_cli_program("npx", default_npm_global_candidates("npx"))
}

pub fn build_local_std_command(program_path: &Path) -> Command {
    build_local_std_command_impl(program_path)
}

pub fn build_local_tokio_command(program_path: &Path) -> TokioCommand {
    build_local_tokio_command_impl(program_path)
}

pub fn local_cli_missing_hint(command_name: &str) -> String {
    let manual_hint = manual_cli_override_hint(command_name);
    let mut message = format!(
        "未找到 `{command_name}` CLI。AI Toolbox 已检查当前 PATH、常见安装路径，以及 nvm、volta、fnm、nvm-windows、bun、mise、asdf 管理的全局 bin；macOS 从 Dock/Finder/Spotlight 启动时不会继承终端 shell PATH。请确认 CLI 已安装。"
    );
    if !manual_hint.is_empty() {
        message.push(' ');
        message.push_str(&manual_hint);
    }
    message
}

/// Probe a CLI's version by trying `--version`, `-v`, then `version`, and
/// returning the first non-empty line of the matched output.
///
/// Used to validate a manually-configured CLI path before saving, and to show
/// the installed version next to a saved path in the "More Options" modal.
const CLI_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(4);

pub async fn probe_cli_version(readable_path: &str) -> Result<String, String> {
    probe_cli_version_with_timeout(readable_path, CLI_VERSION_PROBE_TIMEOUT).await
}

async fn probe_cli_version_with_timeout(
    readable_path: &str,
    probe_timeout: Duration,
) -> Result<String, String> {
    let path = PathBuf::from(readable_path.trim());
    if path.as_os_str().is_empty() {
        return Err("CLI 路径不能为空".to_string());
    }
    if !is_existing_command_path(&path) {
        return Err(format!("未找到 CLI 文件: {}", path.display()));
    }

    let mut failure_reason = String::new();
    for flag in ["--version", "-v", "version"] {
        let mut command = build_local_tokio_command(&path);
        command.arg(flag).kill_on_drop(true);
        let output = match tokio::time::timeout(probe_timeout, command.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => {
                failure_reason = format!("{flag}: {error}");
                continue;
            }
            Err(_) => {
                failure_reason = format!("{flag}: 版本探测超时");
                continue;
            }
        };
        if !output.status.success() {
            failure_reason = format!("{flag}: 退出码 {:?}", output.status.code());
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}\n{stderr}");
        let first_line = combined
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_string();
        if !first_line.is_empty() {
            return Ok(first_line);
        }
        failure_reason = format!("{flag}: CLI 未输出版本信息");
    }

    Err(format!(
        "无法通过 `--version` / `-v` / `version` 探测 CLI 版本: {failure_reason}"
    ))
}

/// Validate that a manual CLI path is usable: must exist and produce a version.
pub async fn validate_manual_cli_path(readable_path: &str) -> Result<String, String> {
    probe_cli_version(readable_path).await
}

fn resolve_local_cli_program(command_name: &str, candidate_paths: Vec<PathBuf>) -> LocalCliProgram {
    // User-specified manual path takes priority (only when it still exists).
    if let Some(path) = manual_cli_override_path(command_name) {
        return LocalCliProgram { path };
    }

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

    let paths = decode_lookup_lines(&output.stdout);
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

/// Decode the stdout of `where` / `which` into candidate paths.
///
/// On Windows, `where` may emit bytes in the system OEM or ANSI code page
/// rather than UTF-8 (e.g. a Chinese locale uses CP936/GBK). A path containing
/// non-ASCII characters would then fail `String::from_utf8` and silently drop
/// the PATH lookup. We first try UTF-8, then fall back to the OEM and ANSI
/// code pages so non-ASCII install paths are still discovered.
fn decode_lookup_lines(stdout: &[u8]) -> Vec<PathBuf> {
    if let Ok(text) = std::str::from_utf8(stdout) {
        return parse_lookup_command_output(text);
    }

    #[cfg(target_os = "windows")]
    {
        let mut paths = Vec::new();
        for code_page in [windows_oem_code_page(), windows_ansi_code_page()] {
            if let Some(text) = windows_decode_code_page(code_page, stdout) {
                paths.extend(parse_lookup_command_output(&text));
            }
        }
        return paths;
    }

    #[cfg(not(target_os = "windows"))]
    {
        parse_lookup_command_output(&String::from_utf8_lossy(stdout))
    }
}

#[cfg(target_os = "windows")]
fn windows_oem_code_page() -> u32 {
    extern "system" {
        fn GetOEMCP() -> u32;
    }
    // SAFETY: GetOEMCP is a leaf query with no inputs and is always safe.
    unsafe { GetOEMCP() }
}

#[cfg(target_os = "windows")]
fn windows_ansi_code_page() -> u32 {
    extern "system" {
        fn GetACP() -> u32;
    }
    // SAFETY: GetACP is a leaf query with no inputs and is always safe.
    unsafe { GetACP() }
}

#[cfg(target_os = "windows")]
fn windows_decode_code_page(code_page: u32, bytes: &[u8]) -> Option<String> {
    extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            multi_byte_str: *const u8,
            cb_multi_byte: i32,
            wide_char_str: *mut u16,
            cch_wide_char: i32,
        ) -> i32;
    }
    if bytes.is_empty() {
        return Some(String::new());
    }
    // SAFETY: we pass a valid pointer + length for the input; the first call only
    // computes the required wide length (null output buffer). We then allocate
    // exactly that capacity for the second call.
    unsafe {
        let wide_len = MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if wide_len <= 0 {
            return None;
        }
        let mut wide = vec![0u16; wide_len as usize];
        let written = MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            wide_len,
        );
        if written <= 0 {
            return None;
        }
        String::from_utf16(&wide[..written as usize]).ok()
    }
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
    // Windows: npm 全局可执行文件直接位于 Node 安装根目录，无 `bin` 子文件夹
    // （`{prefix}/bin` 是 Unix 约定，Windows 上直接链接进 `{prefix}`）。
    #[cfg(target_os = "windows")]
    {
        push_command_candidate(&mut candidates, &default_alias, command_name);
        push_command_candidate(
            &mut candidates,
            &default_alias.join("installation"),
            command_name,
        );
    }
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
        // Windows: 可执行文件直接在 `installation` 根目录，无 `bin` 子文件夹。
        #[cfg(target_os = "windows")]
        push_command_candidate(candidates, &version_dir.join("installation"), command_name);
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

/// Append `<root>/<version-or-hash>/<command>` candidates. Desktop bundles keep
/// one directory per build, so the CLI is one level below the known root.
fn append_versioned_command_candidates(
    candidates: &mut Vec<PathBuf>,
    root: &Path,
    command_name: &str,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            push_command_candidate(candidates, entry.path(), command_name);
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
    append_mise_asdf_runtime_dirs_from_env(&mut dirs, dirs::home_dir().as_deref());

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

/// Production entry: resolve mise/asdf data dirs from env + defaults, then inject runtime PATH.
fn append_mise_asdf_runtime_dirs_from_env(dirs: &mut Vec<PathBuf>, home_dir: Option<&Path>) {
    let mise_data_dir = env_path("MISE_DATA_DIR");
    let asdf_data_dir = env_path("ASDF_DATA_DIR");
    let xdg_data_home = env_path("XDG_DATA_HOME");
    let local_app_data = env_path("LOCALAPPDATA");
    let mise_roots = collect_mise_data_dirs(
        mise_data_dir.as_deref(),
        xdg_data_home.as_deref(),
        local_app_data.as_deref(),
        home_dir,
    );
    let asdf_roots = collect_asdf_data_dirs(asdf_data_dir.as_deref(), home_dir);
    append_mise_asdf_runtime_dirs(dirs, home_dir, &mise_roots, &asdf_roots);
}

/// Append mise / asdf runtime dirs to the child process PATH.
///
/// mise/asdf shims are thin wrappers that `exec mise` / `exec asdf`, so they need the
/// manager binary itself on PATH. GUI-launched children lack `~/.local/bin` (where mise is
/// curl-installed) and Homebrew prefixes. Inject only when mise/asdf shims actually exist,
/// so non-mise/asdf environments are left untouched.
///
/// `mise_data_dirs` / `asdf_data_dirs` are injectable so unit tests do not depend on host
/// `MISE_DATA_DIR` / `ASDF_DATA_DIR`.
fn append_mise_asdf_runtime_dirs(
    dirs: &mut Vec<PathBuf>,
    home_dir: Option<&Path>,
    mise_data_dirs: &[PathBuf],
    asdf_data_dirs: &[PathBuf],
) {
    let Some(home) = home_dir else {
        return;
    };

    let mut need_manager_bins = false;
    for root in mise_data_dirs {
        let shims = root.join("shims");
        if shims.is_dir() {
            push_existing_dir(dirs, shims);
            need_manager_bins = true;
        }
    }
    for root in asdf_data_dirs {
        let shims = root.join("shims");
        if shims.is_dir() {
            push_existing_dir(dirs, shims);
            need_manager_bins = true;
        }
    }

    if !need_manager_bins {
        return;
    }

    push_existing_dir(dirs, home.join(".local").join("bin"));
    push_existing_dir(dirs, PathBuf::from("/opt/homebrew/bin"));
    push_existing_dir(dirs, PathBuf::from("/usr/local/bin"));
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

/// Collect mise data directories for candidate / runtime scanning.
///
/// Order: `$MISE_DATA_DIR` → `$XDG_DATA_HOME/mise` → `~/.local/share/mise` →
/// `%LOCALAPPDATA%\mise` (Windows). Duplicates are dropped.
fn collect_mise_data_dirs(
    mise_data_dir: Option<&Path>,
    xdg_data_home: Option<&Path>,
    local_app_data: Option<&Path>,
    home_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(mise_data_dir) = mise_data_dir {
        push_unique_path(&mut roots, mise_data_dir.to_path_buf());
    }
    if let Some(xdg_data_home) = xdg_data_home {
        push_unique_path(&mut roots, xdg_data_home.join("mise"));
    }
    if let Some(home_dir) = home_dir {
        push_unique_path(
            &mut roots,
            home_dir.join(".local").join("share").join("mise"),
        );
    }
    if let Some(local_app_data) = local_app_data {
        push_unique_path(&mut roots, local_app_data.join("mise"));
    }
    roots
}

/// Collect asdf data directories. Order: `$ASDF_DATA_DIR` → `~/.asdf`.
fn collect_asdf_data_dirs(asdf_data_dir: Option<&Path>, home_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(asdf_data_dir) = asdf_data_dir {
        push_unique_path(&mut roots, asdf_data_dir.to_path_buf());
    }
    if let Some(home_dir) = home_dir {
        push_unique_path(&mut roots, home_dir.join(".asdf"));
    }
    roots
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

/// Production entry for mise/asdf CLI candidates (reads env for data dirs).
fn append_mise_asdf_candidates(
    candidates: &mut Vec<PathBuf>,
    command_name: &str,
    home_dir: Option<&Path>,
) {
    let mise_data_dir = env_path("MISE_DATA_DIR");
    let asdf_data_dir = env_path("ASDF_DATA_DIR");
    let xdg_data_home = env_path("XDG_DATA_HOME");
    let local_app_data = env_path("LOCALAPPDATA");
    let mise_roots = collect_mise_data_dirs(
        mise_data_dir.as_deref(),
        xdg_data_home.as_deref(),
        local_app_data.as_deref(),
        home_dir,
    );
    let asdf_roots = collect_asdf_data_dirs(asdf_data_dir.as_deref(), home_dir);
    append_mise_asdf_candidates_with_roots(candidates, command_name, &mise_roots, &asdf_roots);
}

/// Append mise / asdf managed CLI candidates from explicit data roots.
///
/// Covers both the version-pinned node install bin (for `mise use node` + `npm -g` installs)
/// and the shim directory — the stable entry point for mise/asdf backend tools, including
/// `npm:` backend packages whose real bin path embeds the package name and cannot be
/// generalized.
fn append_mise_asdf_candidates_with_roots(
    candidates: &mut Vec<PathBuf>,
    command_name: &str,
    mise_roots: &[PathBuf],
    asdf_roots: &[PathBuf],
) {
    for root in mise_roots {
        append_node_version_bins(
            candidates,
            &root.join("installs").join("node"),
            command_name,
        );
        push_command_candidate(candidates, root.join("shims"), command_name);
    }

    for root in asdf_roots {
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
        normalize_node_version_dir, probe_cli_version_with_timeout,
    };

    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

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

    #[tokio::test]
    async fn cli_version_probe_times_out_for_hanging_process() {
        let test_dir = TestDir::new("probe-timeout");
        #[cfg(target_os = "windows")]
        let script_path = test_dir.path().join("hanging.ps1");
        #[cfg(target_os = "windows")]
        fs::write(
            &script_path,
            "Start-Sleep -Seconds 5\nWrite-Output 'too late'\n",
        )
        .expect("write hanging PowerShell script");

        #[cfg(not(target_os = "windows"))]
        let script_path = test_dir.path().join("hanging.sh");
        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(&script_path, "#!/bin/sh\nsleep 5\necho 'too late'\n")
                .expect("write hanging shell script");
            let mut permissions = fs::metadata(&script_path)
                .expect("read script metadata")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&script_path, permissions).expect("make script executable");
        }

        let started = std::time::Instant::now();
        let error = probe_cli_version_with_timeout(
            &script_path.to_string_lossy(),
            Duration::from_millis(50),
        )
        .await
        .expect_err("hanging CLI should time out");

        assert!(error.contains("版本探测超时"));
        assert!(started.elapsed() < Duration::from_secs(2));
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
        let default_alias = fnm_dir.join("aliases").join("default");
        let default_bin = default_alias.join("bin");
        let version_installation = fnm_dir
            .join("node-versions")
            .join("v22.18.0")
            .join("installation");
        let version_bin = version_installation.join("bin");

        fs::create_dir_all(&default_bin).expect("failed to create default alias bin dir");
        fs::create_dir_all(&version_bin).expect("failed to create fnm version bin dir");

        let candidates = collect_fnm_candidates(&fnm_dir, "opencode");

        #[cfg(target_os = "windows")]
        {
            assert_eq!(candidates.first(), Some(&default_alias.join("opencode")));
            assert!(candidates.contains(&default_alias.join("opencode.cmd")));
            assert!(candidates.contains(&version_installation.join("opencode")));
        }

        #[cfg(not(target_os = "windows"))]
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
        let mise_root = home_dir.join(".local").join("share").join("mise");
        let shims_dir = mise_root.join("shims");
        let node_bin = mise_root
            .join("installs")
            .join("node")
            .join("22.18.0")
            .join("bin");
        fs::create_dir_all(&shims_dir).expect("failed to create mise shims dir");
        fs::create_dir_all(&node_bin).expect("failed to create mise node bin dir");

        let mut candidates = Vec::new();
        super::append_mise_asdf_candidates_with_roots(&mut candidates, "pi", &[mise_root], &[]);

        assert!(candidates.contains(&shims_dir.join("pi")));
        assert!(candidates.contains(&node_bin.join("pi")));
    }

    #[test]
    fn asdf_shims_and_node_install_bins_are_candidates() {
        let test_dir = TestDir::new("asdf");
        let home_dir = test_dir.path().join("home");
        let asdf_root = home_dir.join(".asdf");
        let shims_dir = asdf_root.join("shims");
        let node_bin = asdf_root
            .join("installs")
            .join("nodejs")
            .join("22.18.0")
            .join("bin");
        fs::create_dir_all(&shims_dir).expect("failed to create asdf shims dir");
        fs::create_dir_all(&node_bin).expect("failed to create asdf node bin dir");

        let mut candidates = Vec::new();
        super::append_mise_asdf_candidates_with_roots(&mut candidates, "pi", &[], &[asdf_root]);

        assert!(candidates.contains(&shims_dir.join("pi")));
        assert!(candidates.contains(&node_bin.join("pi")));
    }

    #[test]
    fn mise_data_dir_env_precedes_default_home_and_xdg() {
        let test_dir = TestDir::new("mise-roots");
        let home_dir = test_dir.path().join("home");
        let custom = test_dir.path().join("custom-mise");
        let xdg = test_dir.path().join("xdg");
        let local_app_data = test_dir.path().join("localappdata");

        let roots = super::collect_mise_data_dirs(
            Some(&custom),
            Some(&xdg),
            Some(&local_app_data),
            Some(&home_dir),
        );

        assert_eq!(roots.first(), Some(&custom));
        assert!(roots.contains(&xdg.join("mise")));
        assert!(roots.contains(&home_dir.join(".local").join("share").join("mise")));
        assert!(roots.contains(&local_app_data.join("mise")));
    }

    #[test]
    fn asdf_data_dir_env_precedes_default_home() {
        let test_dir = TestDir::new("asdf-roots");
        let home_dir = test_dir.path().join("home");
        let custom = test_dir.path().join("custom-asdf");

        let roots = super::collect_asdf_data_dirs(Some(&custom), Some(&home_dir));

        assert_eq!(roots.first(), Some(&custom));
        assert!(roots.contains(&home_dir.join(".asdf")));
    }

    #[test]
    fn mise_runtime_dirs_added_only_when_shims_present() {
        let test_dir = TestDir::new("mise-runtime");
        let home_dir = test_dir.path().join("home");
        let mise_root = home_dir.join(".local").join("share").join("mise");
        let mise_shims = mise_root.join("shims");
        let local_bin = home_dir.join(".local").join("bin");

        // No mise shims yet -> nothing injected (even with a data root listed).
        let mut dirs = Vec::new();
        super::append_mise_asdf_runtime_dirs(&mut dirs, Some(&home_dir), &[mise_root.clone()], &[]);
        assert!(dirs.is_empty());

        // With shims present -> shims + manager bin dirs injected.
        fs::create_dir_all(&mise_shims).expect("failed to create mise shims dir");
        fs::create_dir_all(&local_bin).expect("failed to create local bin dir");
        super::append_mise_asdf_runtime_dirs(&mut dirs, Some(&home_dir), &[mise_root], &[]);

        assert!(dirs.contains(&mise_shims));
        assert!(dirs.contains(&local_bin));
    }

    #[test]
    fn asdf_runtime_dirs_added_only_when_shims_present() {
        let test_dir = TestDir::new("asdf-runtime");
        let home_dir = test_dir.path().join("home");
        let asdf_root = home_dir.join(".asdf");
        let asdf_shims = asdf_root.join("shims");
        let local_bin = home_dir.join(".local").join("bin");

        let mut dirs = Vec::new();
        super::append_mise_asdf_runtime_dirs(&mut dirs, Some(&home_dir), &[], &[asdf_root.clone()]);
        assert!(dirs.is_empty());

        fs::create_dir_all(&asdf_shims).expect("failed to create asdf shims dir");
        fs::create_dir_all(&local_bin).expect("failed to create local bin dir");
        super::append_mise_asdf_runtime_dirs(&mut dirs, Some(&home_dir), &[], &[asdf_root]);

        assert!(dirs.contains(&asdf_shims));
        assert!(dirs.contains(&local_bin));
    }

    #[test]
    fn custom_mise_data_dir_candidates_and_runtime_use_injected_root() {
        let test_dir = TestDir::new("custom-mise-data");
        let home_dir = test_dir.path().join("home");
        let custom_root = test_dir.path().join("custom-mise");
        let shims = custom_root.join("shims");
        let node_bin = custom_root
            .join("installs")
            .join("node")
            .join("22.18.0")
            .join("bin");
        let local_bin = home_dir.join(".local").join("bin");
        fs::create_dir_all(&shims).expect("failed to create custom mise shims");
        fs::create_dir_all(&node_bin).expect("failed to create custom mise node bin");
        fs::create_dir_all(&local_bin).expect("failed to create local bin");

        let mut candidates = Vec::new();
        super::append_mise_asdf_candidates_with_roots(
            &mut candidates,
            "pi",
            &[custom_root.clone()],
            &[],
        );
        assert!(candidates.contains(&shims.join("pi")));
        assert!(candidates.contains(&node_bin.join("pi")));

        let mut dirs = Vec::new();
        super::append_mise_asdf_runtime_dirs(&mut dirs, Some(&home_dir), &[custom_root], &[]);
        assert!(dirs.contains(&shims));
        assert!(dirs.contains(&local_bin));
    }

    #[test]
    fn manual_cli_override_prefers_existing_path_over_auto_resolution() {
        // This test mutates and reads the process-wide manual CLI override registry
        // (see `manual_cli_override_registry`). `settings::store` round-trip tests call
        // load/save, which re-sync the registry with (empty) default settings and can
        // wipe our override mid-test under parallel executors. Grab the shared
        // `test_env` lock so every reader/writer of the registry is serialized.
        let _guard = crate::coding::test_env::lock();
        let test_dir = TestDir::new("manual-cli-pi");
        #[cfg(target_os = "windows")]
        let fake_pi = test_dir.path().join("pi.cmd");
        #[cfg(not(target_os = "windows"))]
        let fake_pi = test_dir.path().join("pi");
        fs::write(&fake_pi, "").expect("write fake pi");
        let mut map = std::collections::HashMap::new();
        map.insert("pi".to_string(), fake_pi.to_string_lossy().to_string());
        super::set_manual_cli_overrides(map);

        let resolved = super::resolve_local_pi_program();
        assert_eq!(resolved.path, fake_pi);

        super::set_manual_cli_overrides(std::collections::HashMap::new());
    }

    #[test]
    fn manual_cli_override_falls_back_when_file_missing_and_hint_reported() {
        // Same registry-serialization rationale as the override-priority test above.
        let _guard = crate::coding::test_env::lock();
        let test_dir = TestDir::new("manual-cli-missing");
        let missing = test_dir.path().join("pi-does-not-exist");
        let mut map = std::collections::HashMap::new();
        map.insert("pi".to_string(), missing.to_string_lossy().to_string());
        super::set_manual_cli_overrides(map);

        assert!(super::manual_cli_path_configured("pi"));
        assert!(!super::has_manual_cli_override("pi"));

        let hint = super::local_cli_missing_hint("pi");
        assert!(hint.contains("更多选项"));

        super::set_manual_cli_overrides(std::collections::HashMap::new());
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
