//! 插件宿主子进程的单例监管。
//!
//! ── 生命周期 ──────────────────────────────────────────────────
//! 首次需要插件时才起（懒启动）：没人装插件时，2API 的普通渠道不该为一个用
//! 不到的进程付内存。起来之后常驻，空闲不退出——插件的 `config` 钩子与登录
//! 态都在进程内，重启一次就要全部重建，而用户的请求随时可能到。
//!
//! ── 崩溃退避 ──────────────────────────────────────────────────
//! 宿主崩了就地重启，退避 2s→4s→8s…上限 60s。连续失败 5 次判定「坏包」并
//! **停止自动重启**：一个每次启动都崩的插件包会变成一个无限重启循环，而用户
//! 唯一的出路是手动重试。计数在一次成功启动后清零。
//!
//! ── 为什么要 `kill_on_drop` ────────────────────────────────────
//! 应用退出时若宿主还活着，它会因为 stdin 关闭而自行退出——但那是「会自理的
//! 宿主」才有的行为。插件可能装了 `process.on("exit")` 钩子或卡在同步 I/O 里，
//! 所以还得有一条强杀路径。`kill_on_drop` 保证 Child 被丢弃时进程被回收。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Child;
use tokio::sync::Mutex;

use super::bun;
use super::protocol::{self, HostLink};

/// host.js 编译进二进制，运行时写到数据目录再交给 Bun 执行。
///
/// 写文件而不是把源码塞进命令行：源码几千行，命令行长会在 Windows 上撞上限。
const HOST_SCRIPT: &str = include_str!("../../../../resources/relay_host.js");

/// 连续崩溃多少次后停止自动重启。
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// 退避上限。
const MAX_BACKOFF_SECS: u64 = 60;

/// 传给 host.js 的 `listMillis`：模型列表钩子的超时。
const LIST_TIMEOUT_MILLIS: u64 = 12_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub running: bool,
    /// Consecutive failed starts; reset by a successful one.
    pub failures: u32,
    /// `true` once the failure limit was hit: auto-restart is off until the
    /// user asks for a retry.
    pub disabled: bool,
    pub bun_path: String,
}

/// What `init` is told about one installed plugin.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPluginSpec {
    pub spec: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInitResult {
    #[serde(default)]
    pub plugins: Vec<HostLoadedPlugin>,
    #[serde(default)]
    pub exports: Vec<HostExport>,
    #[serde(default)]
    pub providers: Vec<HostProvider>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostLoadedPlugin {
    #[serde(default)]
    pub spec: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub exports: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostExport {
    #[serde(default)]
    pub spec: String,
    /// The module export that produced the plugin function.
    #[serde(default)]
    pub export_name: String,
    #[serde(default)]
    pub file: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostProvider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub npm: String,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub list_error: String,
    #[serde(default)]
    pub methods: Vec<serde_json::Value>,
    #[serde(default)]
    pub options: Vec<serde_json::Value>,
    #[serde(default)]
    pub usage: bool,
    #[serde(default)]
    pub checkin: bool,
    #[serde(default)]
    pub single: bool,
    #[serde(default)]
    pub signed_in: bool,
    #[serde(default)]
    pub account: String,
    /// The package that declared this provider. A single package can export
    /// several plugin functions, so the provider is attributed by export, not
    /// by package.
    #[serde(default)]
    pub spec: String,
    #[serde(default)]
    pub export_name: String,
    #[serde(default)]
    pub file: String,
}

struct RunningHost {
    child: Child,
    link: Arc<HostLink>,
    /// The `init` reply this host was built with.
    init: HostInitResult,
}

struct HostRegistry {
    host: Option<RunningHost>,
    failures: u32,
    disabled: bool,
}

fn registry() -> &'static Mutex<HostRegistry> {
    static REGISTRY: std::sync::OnceLock<Mutex<HostRegistry>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(HostRegistry {
            host: None,
            failures: 0,
            disabled: false,
        })
    })
}

/// Serialises startup. Without it two concurrent callers both see "no host"
/// and both spawn one, and the loser's child lingers with nothing reading it.
fn startup_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Set once the app is shutting down, so a host killed on purpose is not
/// treated as a crash and restarted.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub fn note_app_exit() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
}

fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::SeqCst)
}

pub async fn status() -> HostStatus {
    let registry = registry().lock().await;
    HostStatus {
        running: registry
            .host
            .as_ref()
            .is_some_and(|host| host.link.is_alive()),
        failures: registry.failures,
        disabled: registry.disabled,
        bun_path: bun::status().path,
    }
}

/// The host script's path, written beside the Bun runtime.
///
/// Rewritten whenever its contents change, so a new build's host.js replaces
/// the old one without a version bump.
pub fn host_script_path() -> Result<PathBuf, String> {
    let dir = bun::runtime_root();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create the 2API runtime directory: {error}"))?;
    let path = dir.join("relay_host.js");
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != Some(HOST_SCRIPT) {
        std::fs::write(&path, HOST_SCRIPT)
            .map_err(|error| format!("Failed to write the plugin host script: {error}"))?;
    }
    Ok(path)
}

/// Start (or hand back) the host, applying `init`.
///
/// `ensure` is the only way in: every operation that needs the host calls it,
/// and a dead host is restarted here rather than by a background watchdog. The
/// `init` reply is returned alongside the link — it carries the plugin list,
/// the exports each package produced, and the providers they declare, and the
/// caller persists those.
///
/// The `init` payload is only consulted when a host is actually started. A
/// live host keeps the plugins it was started with; call [`restart`] to change
/// that.
pub async fn ensure(
    plugins: Vec<HostPluginSpec>,
    auth: serde_json::Value,
    options: serde_json::Value,
    directory: &Path,
) -> Result<(Arc<HostLink>, HostInitResult), String> {
    {
        let registry = registry().lock().await;
        if registry.disabled {
            return Err(
                "The plugin host was stopped after repeated crashes. Retry it from the plugins page."
                    .to_string(),
            );
        }
        if let Some(host) = registry.host.as_ref() {
            if host.link.is_alive() {
                return Ok((host.link.clone(), host.init.clone()));
            }
        }
    }

    let _starting = startup_lock().lock().await;
    // Another caller may have started it while we waited for the lock.
    {
        let registry = registry().lock().await;
        if registry.disabled {
            return Err(
                "The plugin host was stopped after repeated crashes. Retry it from the plugins page."
                    .to_string(),
            );
        }
        if let Some(host) = registry.host.as_ref() {
            if host.link.is_alive() {
                return Ok((host.link.clone(), host.init.clone()));
            }
        }
    }

    let bun_path = bun::installed_bun_path(bun::BUN_VERSION);
    if !bun_path.is_file() {
        return Err(
            "The Bun runtime is not installed yet. Install it from the plugins page.".to_string(),
        );
    }
    let script = host_script_path()?;

    // Back off before retrying: the first attempt after a failure waits 2s,
    // then 4s, 8s … capped. A host that starts cleanly never sleeps here.
    let failures = registry().lock().await.failures;
    if failures > 0 && !is_shutting_down() {
        let secs = (1u64 << failures.min(6)).min(MAX_BACKOFF_SECS);
        tokio::time::sleep(Duration::from_secs(secs)).await;
    }

    let mut command = crate::coding::cli_resolver::build_local_tokio_command(&bun_path);
    command
        .arg("run")
        .arg(&script)
        .current_dir(directory)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        // Plugins that shell out to a package manager otherwise unpack into a
        // temp dir and re-download every restart.
        .env("BUN_INSTALL_CACHE_DIR", bun::runtime_root().join("cache"));
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start the plugin host: {error}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "The plugin host has no stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "The plugin host has no stdout".to_string())?;
    let stderr = child.stderr.take();

    if let Some(stderr) = stderr {
        // Plugin logs go to stderr; anything else there is a crash trace.
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log::debug!("2API plugin host: {line}");
            }
        });
    }

    let link = HostLink::new(stdin);
    tokio::spawn(protocol::read_loop(stdout, link.clone()));

    let init_params = serde_json::json!({
        "plugins": plugins,
        "auth": auth,
        "options": options,
        "directory": directory.display().to_string(),
        "listMillis": LIST_TIMEOUT_MILLIS,
    });
    let init = match link.request("init", init_params).await {
        Ok(value) => serde_json::from_value::<HostInitResult>(value)
            .map_err(|error| format!("Failed to read the plugin host's init reply: {error}")),
        Err(error) => Err(error),
    };

    let mut registry = registry().lock().await;
    match init {
        Ok(init) => {
            registry.failures = 0;
            registry.host = Some(RunningHost {
                child,
                link: link.clone(),
                init: init.clone(),
            });
            Ok((link, init))
        }
        Err(error) => {
            // A host that cannot even init is a broken install: count it and
            // stop auto-restarting once the limit is reached. Dropping `child`
            // here reaps the process (kill_on_drop).
            registry.failures += 1;
            if registry.failures >= MAX_CONSECUTIVE_FAILURES {
                registry.disabled = true;
            }
            log::warn!("2API plugin host failed to initialise: {error}");
            Err(error)
        }
    }
}

/// Stop the host on purpose. A subsequent [`ensure`] starts a new one.
pub async fn shutdown() {
    let mut registry = registry().lock().await;
    if let Some(mut host) = registry.host.take() {
        host.link.close().await;
        // Give the host a moment to exit on its own so plugins can flush, then
        // make sure it is gone. Dropping a still-running Child reaps it.
        if tokio::time::timeout(Duration::from_secs(2), host.child.wait())
            .await
            .is_err()
        {
            let _ = host.child.start_kill();
        }
    }
}

/// Restart with a new plugin set.
///
/// Bun caches module imports for the life of the process, so a newly installed
/// plugin is only visible after a restart — re-sending `init` to a live host
/// would report the old set.
pub async fn restart(
    plugins: Vec<HostPluginSpec>,
    auth: serde_json::Value,
    options: serde_json::Value,
    directory: &Path,
) -> Result<HostInitResult, String> {
    shutdown().await;
    let (_link, init) = ensure(plugins, auth, options, directory).await?;
    Ok(init)
}

/// Forget the crash counter so the next [`ensure`] tries again.
///
/// The user's "retry" on the plugins page, and also what installing a plugin
/// does — a new package deserves a fresh chance.
pub async fn reset_failures() {
    let mut registry = registry().lock().await;
    registry.failures = 0;
    registry.disabled = false;
}

/// Stop the host because the app is exiting.
pub async fn shutdown_for_app_exit() {
    note_app_exit();
    shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_script_is_compiled_in() {
        // A missing resource file is a build error, but an empty one would slip
        // through and fail only at runtime.
        assert!(HOST_SCRIPT.contains("readline.createInterface"));
        assert!(HOST_SCRIPT.contains("async function fetchAs"));
    }

    #[test]
    fn the_backoff_doubles_and_caps() {
        let secs = |failures: u32| (1u64 << failures.min(6)).min(MAX_BACKOFF_SECS);
        assert_eq!(secs(1), 2);
        assert_eq!(secs(3), 8);
        assert_eq!(secs(6), 60);
        assert_eq!(secs(9), 60);
    }

    #[tokio::test]
    async fn status_reports_no_host_before_anything_starts() {
        let status = status().await;
        assert!(!status.running);
    }
}
