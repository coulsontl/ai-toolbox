//! The Bun runtime the plugin host runs on.
//!
//! Bun is not shipped inside the installer; the first time a provider plugin
//! needs it, the matching release archive is downloaded once into
//! `<app_data>/plugins/runtimes/bun/<version>/<asset>/`. A user-configured
//! executable or a Bun already on PATH wins over the managed copy, so an
//! installation that already has Bun never pays for the download.
//!
//! Versions and asset checksums are pinned: the host's protocol and the
//! plugin SDKs are tested against one Bun release, and an unreviewed download
//! must never become executable code.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::coding::cli_resolver::{apply_create_no_window_tokio, resolve_local_cli_by_name};
use crate::db::SqliteDbState;

use super::paths::PluginPaths;
use super::store;

/// Every download and every probe expects exactly this release.
pub const BUN_VERSION: &str = "1.4.2";

/// Stable prefix the UI turns into an actionable "install Bun" message.
pub const BUN_UNAVAILABLE_PREFIX: &str = "PLUGIN_BUN_UNAVAILABLE";

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// A generous ceiling for one archive; the real archives are well under it.
const MAX_ARCHIVE_BYTES: u64 = 300 * 1024 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 300 * 1024 * 1024;
/// The archive is large and served from GitHub; a 30s request timeout would
/// abort a slow but healthy download.
const DOWNLOAD_TIMEOUT_SECS: u64 = 600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BunAsset {
    /// Release asset name, also the directory inside the zip.
    pub name: &'static str,
    pub sha256: &'static str,
}

/// The pinned asset for the platform this binary was built for.
pub fn current_bun_asset() -> Result<BunAsset, String> {
    let asset = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => BunAsset {
            name: "bun-windows-x64-baseline",
            sha256: "78c221c2376f79731ccf4e4af0b3bb46d81fefa3296c5abee09ad8a1b21e68c6",
        },
        ("windows", "aarch64") => BunAsset {
            name: "bun-windows-aarch64",
            sha256: "a7a16b876a305fd1029c66dbd27007b4f6112ae896532f675878731a21e50cfd",
        },
        ("macos", "x86_64") => BunAsset {
            name: "bun-darwin-x64",
            sha256: "80520d7e17526308c9185d261679ac6d27798d3803a0e9f7ff9121ab8affb012",
        },
        ("macos", "aarch64") => BunAsset {
            name: "bun-darwin-aarch64",
            sha256: "90987a3a16d7db556d886ac3d551e7b6d3edf0a1cf43acaed622e8676be1d12f",
        },
        ("linux", "x86_64") if cfg!(target_env = "musl") => BunAsset {
            name: "bun-linux-x64-musl-baseline",
            sha256: "76e1db84e98f22f78de0a87e309bfbbf297732847f9720db36750646c85c8c18",
        },
        ("linux", "aarch64") if cfg!(target_env = "musl") => BunAsset {
            name: "bun-linux-aarch64-musl",
            sha256: "71760b6c8ea30623b81a4907cb815d48e2ea266f2e73e751534a44a0607950df",
        },
        ("linux", "x86_64") => BunAsset {
            name: "bun-linux-x64-baseline",
            sha256: "c678040f14fe0440eb839d37cbd0ce4c051a32da72806ac97de6a6aab6bf728f",
        },
        ("linux", "aarch64") => BunAsset {
            name: "bun-linux-aarch64",
            sha256: "54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7",
        },
        (os, arch) => {
            return Err(format!(
                "{BUN_UNAVAILABLE_PREFIX}: no pinned Bun {BUN_VERSION} build for {os}/{arch}"
            ))
        }
    };
    Ok(asset)
}

pub fn bun_executable_name() -> &'static str {
    if cfg!(windows) {
        "bun.exe"
    } else {
        "bun"
    }
}

/// One executable that answered `--version`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunProbe {
    pub path: String,
    pub version: String,
    /// `manual` | `managed` | `system`.
    pub source: String,
}

/// Everything the settings pane shows about Bun without starting the host.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunRuntimeStatus {
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub expected_version: String,
    pub asset: String,
    pub managed_path: String,
    pub downloading: bool,
    pub last_error: Option<String>,
}

/// Result of probing a user-supplied executable.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunPathProbe {
    pub ok: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Default)]
struct RuntimeState {
    downloading: bool,
    last_probe: Option<BunProbe>,
    last_error: Option<String>,
}

/// Shared Bun locator and downloader.
pub struct BunRuntime {
    db: SqliteDbState,
    paths: PluginPaths,
    /// One download at a time; a second caller waits and reuses the result.
    ensure_lock: tokio::sync::Mutex<()>,
    state: std::sync::Mutex<RuntimeState>,
}

impl BunRuntime {
    pub fn new(db: SqliteDbState, paths: PluginPaths) -> Self {
        Self {
            db,
            paths,
            ensure_lock: tokio::sync::Mutex::new(()),
            state: std::sync::Mutex::new(RuntimeState::default()),
        }
    }

    /// `<app_data>/plugins/runtimes/bun/<version>/<asset>/bun(.exe)`.
    pub fn managed_executable_path(&self) -> Result<PathBuf, String> {
        let asset = current_bun_asset()?;
        Ok(self
            .paths
            .bun_runtimes_dir()
            .join(BUN_VERSION)
            .join(asset.name)
            .join(bun_executable_name()))
    }

    /// Find a usable Bun without downloading one.
    pub async fn probe(&self) -> Option<BunProbe> {
        let settings = match store::get_plugin_settings(&self.db) {
            Ok(settings) => settings,
            Err(error) => {
                self.set_last_error(Some(error));
                return None;
            }
        };

        let mut first_error: Option<String> = None;
        let manual = settings.bun_manual_path.trim();
        if !manual.is_empty() {
            let path = PathBuf::from(manual);
            if path.is_file() {
                match probe_executable(&path, false).await {
                    Ok(version) => return Some(record_probe(path, version, "manual", self)),
                    Err(error) => first_error = Some(error),
                }
            } else {
                first_error = Some(format!(
                    "{BUN_UNAVAILABLE_PREFIX}: configured Bun path '{}' does not exist",
                    path.display()
                ));
            }
        }

        if let Ok(path) = self.managed_executable_path() {
            if path.is_file() {
                match probe_executable(&path, true).await {
                    Ok(version) => return Some(record_probe(path, version, "managed", self)),
                    Err(error) => first_error = Some(error),
                }
            }
        }

        if let Some(program) = resolve_local_cli_by_name("bun") {
            match probe_executable(&program.path, false).await {
                Ok(version) => return Some(record_probe(program.path, version, "system", self)),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        self.set_last_error(first_error);
        None
    }

    /// Forget the last probe. Called when settings that change where Bun comes
    /// from are saved, so `status` does not keep showing the old executable.
    pub fn invalidate(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.last_probe = None;
        state.last_error = None;
    }

    /// Return a Bun executable, downloading the pinned release on first use.
    pub async fn ensure(&self) -> Result<BunProbe, String> {
        let _guard = self.ensure_lock.lock().await;

        // Whoever held the lock before us may have finished the download.
        if let Some(probe) = self.probe().await {
            return Ok(probe);
        }

        self.set_downloading(true);
        let result = self.download_and_probe().await;
        self.set_downloading(false);
        match &result {
            Ok(probe) => {
                self.set_last_error(None);
                self.state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .last_probe = Some(probe.clone());
            }
            Err(error) => self.set_last_error(Some(error.clone())),
        }
        result
    }

    /// Status for the settings pane. Probes once, then serves the cached
    /// answer so a polling UI does not spawn a process every second.
    pub async fn status(&self) -> BunRuntimeStatus {
        let cached = self.cached_probe();
        let probe = match cached {
            Some(probe) => Some(probe),
            None => self.probe().await,
        };
        self.status_from(probe)
    }

    /// Probe an arbitrary path the user typed into settings.
    pub async fn probe_path(&self, path: &str) -> BunPathProbe {
        let path = PathBuf::from(path.trim());
        if path.as_os_str().is_empty() {
            return BunPathProbe {
                ok: false,
                version: None,
                error: Some("A Bun executable path is required".to_string()),
            };
        }
        if !path.is_file() {
            return BunPathProbe {
                ok: false,
                version: None,
                error: Some(format!("'{}' does not exist", path.display())),
            };
        }
        match probe_executable(&path, false).await {
            Ok(version) => BunPathProbe {
                ok: true,
                version: Some(version),
                error: None,
            },
            Err(error) => BunPathProbe {
                ok: false,
                version: None,
                error: Some(error),
            },
        }
    }

    fn cached_probe(&self) -> Option<BunProbe> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_probe
            .clone()
    }

    fn status_from(&self, probe: Option<BunProbe>) -> BunRuntimeStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        BunRuntimeStatus {
            available: probe.is_some(),
            path: probe.as_ref().map(|value| value.path.clone()),
            version: probe.as_ref().map(|value| value.version.clone()),
            source: probe.as_ref().map(|value| value.source.clone()),
            expected_version: BUN_VERSION.to_string(),
            asset: current_bun_asset()
                .map(|asset| asset.name.to_string())
                .unwrap_or_default(),
            managed_path: self
                .managed_executable_path()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            downloading: state.downloading,
            last_error: state.last_error.clone(),
        }
    }

    fn set_downloading(&self, downloading: bool) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .downloading = downloading;
    }

    fn set_last_error(&self, error: Option<String>) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_error = error;
    }

    async fn download_and_probe(&self) -> Result<BunProbe, String> {
        let asset = current_bun_asset()?;
        let target = self.managed_executable_path()?;
        let directory = target
            .parent()
            .ok_or_else(|| "Managed Bun path has no parent directory".to_string())?;
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|error| format!("Failed to create '{}': {error}", directory.display()))?;

        let client = super::http_client_for(&self.db, DOWNLOAD_TIMEOUT_SECS).await?;
        let url = format!(
            "https://github.com/oven-sh/bun/releases/download/bun-v{BUN_VERSION}/{}.zip",
            asset.name
        );
        log::info!("downloading Bun {BUN_VERSION} from {url}");

        let archive_path = directory.join("bun.zip");
        let partial_path = directory.join("bun.zip.part");
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|error| format!("{BUN_UNAVAILABLE_PREFIX}: Bun download failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("{BUN_UNAVAILABLE_PREFIX}: Bun download failed: {error}"))?;
        if let Some(length) = response.content_length() {
            if length > MAX_ARCHIVE_BYTES {
                return Err(format!(
                    "{BUN_UNAVAILABLE_PREFIX}: Bun archive is unexpectedly large ({length} bytes)"
                ));
            }
        }

        let mut file = tokio::fs::File::create(&partial_path)
            .await
            .map_err(|error| format!("Failed to create '{}': {error}", partial_path.display()))?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                format!("{BUN_UNAVAILABLE_PREFIX}: Bun download failed: {error}")
            })?;
            written += chunk.len() as u64;
            if written > MAX_ARCHIVE_BYTES {
                let _ = tokio::fs::remove_file(&partial_path).await;
                return Err(format!(
                    "{BUN_UNAVAILABLE_PREFIX}: Bun archive exceeded {MAX_ARCHIVE_BYTES} bytes"
                ));
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("Failed to write Bun archive: {error}"))?;
        }
        file.flush()
            .await
            .map_err(|error| format!("Failed to write Bun archive: {error}"))?;
        drop(file);

        let digest = hex::encode(hasher.finalize());
        if digest != asset.sha256 {
            let _ = tokio::fs::remove_file(&partial_path).await;
            return Err(format!(
                "{BUN_UNAVAILABLE_PREFIX}: Bun archive checksum mismatch (expected {}, got {digest})",
                asset.sha256
            ));
        }
        tokio::fs::rename(&partial_path, &archive_path)
            .await
            .map_err(|error| format!("Failed to store Bun archive: {error}"))?;

        let archive_for_extract = archive_path.clone();
        let target_for_extract = target.clone();
        let asset_name = asset.name.to_string();
        tokio::task::spawn_blocking(move || {
            extract_archive(&archive_for_extract, &target_for_extract, &asset_name)
        })
        .await
        .map_err(|error| format!("Bun extraction task failed: {error}"))??;
        let _ = tokio::fs::remove_file(&archive_path).await;

        let version = probe_executable(&target, true).await?;
        Ok(BunProbe {
            path: target.to_string_lossy().to_string(),
            version,
            source: "managed".to_string(),
        })
    }
}

fn record_probe(path: PathBuf, version: String, source: &str, runtime: &BunRuntime) -> BunProbe {
    let probe = BunProbe {
        path: path.to_string_lossy().to_string(),
        version,
        source: source.to_string(),
    };
    let mut state = runtime
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.last_probe = Some(probe.clone());
    state.last_error = None;
    probe
}

async fn probe_executable(path: &Path, require_exact_version: bool) -> Result<String, String> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_remove("BUN_OPTIONS")
        .env_remove("NODE_OPTIONS")
        .env("DO_NOT_TRACK", "1");
    apply_create_no_window_tokio(&mut command);

    let output = match tokio::time::timeout(PROBE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Err(format!(
                "{BUN_UNAVAILABLE_PREFIX}: failed to run '{} --version': {error}",
                path.display()
            ))
        }
        Err(_) => {
            return Err(format!(
                "{BUN_UNAVAILABLE_PREFIX}: '{} --version' timed out",
                path.display()
            ))
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{BUN_UNAVAILABLE_PREFIX}: '{} --version' exited with {}: {}",
            path.display(),
            output.status,
            stderr.trim()
        ));
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        return Err(format!(
            "{BUN_UNAVAILABLE_PREFIX}: '{} --version' printed nothing",
            path.display()
        ));
    }
    if require_exact_version && version != BUN_VERSION {
        return Err(format!(
            "{BUN_UNAVAILABLE_PREFIX}: expected Bun {BUN_VERSION}, '{}' reports {version}",
            path.display()
        ));
    }
    Ok(version)
}

fn extract_archive(archive_path: &Path, target: &Path, asset_name: &str) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|error| format!("Failed to open '{}': {error}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("Failed to read Bun archive: {error}"))?;
    let entry_name = format!("{asset_name}/{}", bun_executable_name());
    let mut entry = archive
        .by_name(&entry_name)
        .map_err(|error| format!("Bun archive has no '{entry_name}': {error}"))?;
    if entry.size() > MAX_EXECUTABLE_BYTES {
        return Err(format!(
            "{BUN_UNAVAILABLE_PREFIX}: Bun executable is unexpectedly large ({} bytes)",
            entry.size()
        ));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(MAX_EXECUTABLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Failed to read Bun executable: {error}"))?;
    if bytes.len() as u64 > MAX_EXECUTABLE_BYTES {
        return Err(format!(
            "{BUN_UNAVAILABLE_PREFIX}: Bun executable exceeded {MAX_EXECUTABLE_BYTES} bytes"
        ));
    }

    let temporary = target.with_extension("tmp");
    std::fs::write(&temporary, &bytes)
        .map_err(|error| format!("Failed to write '{}': {error}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Failed to make Bun executable: {error}"))?;
    }
    std::fs::rename(&temporary, target)
        .map_err(|error| format!("Failed to place Bun at '{}': {error}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_runtime() -> (tempfile::TempDir, BunRuntime) {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let paths = PluginPaths::new(root.path());
        (root, BunRuntime::new(db, paths))
    }

    #[test]
    fn every_supported_target_has_a_pinned_asset() {
        let asset = current_bun_asset().expect("the test host is a supported target");
        assert!(asset.name.contains("bun-"));
        assert_eq!(asset.sha256.len(), 64);
        assert!(asset.sha256.chars().all(|value| value.is_ascii_hexdigit()));
        assert_eq!(BUN_VERSION, "1.4.2");
    }

    #[test]
    fn managed_bun_lives_under_the_pinned_version_and_asset() {
        let (_root, runtime) = test_runtime();
        let path = runtime.managed_executable_path().unwrap();
        let asset = current_bun_asset().unwrap();
        assert!(path.ends_with(
            PathBuf::from(BUN_VERSION)
                .join(asset.name)
                .join(bun_executable_name())
        ));
    }

    #[tokio::test]
    async fn status_reports_the_expected_version_without_a_download() {
        let (_root, runtime) = test_runtime();
        let status = runtime.status().await;
        assert_eq!(status.expected_version, BUN_VERSION);
        assert!(!status.downloading);
        assert!(!status.asset.is_empty());
    }

    #[tokio::test]
    async fn probing_a_missing_path_returns_a_readable_error() {
        let (_root, runtime) = test_runtime();
        let probe = runtime.probe_path("C:/definitely/missing/bun.exe").await;
        assert!(!probe.ok);
        assert!(probe.error.unwrap().contains("does not exist"));
    }

    #[test]
    fn extract_rejects_an_archive_without_the_expected_entry() {
        let root = tempfile::tempdir().unwrap();
        let archive_path = root.path().join("bun.zip");
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "something-else/bun.exe",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        std::io::Write::write_all(&mut writer, b"not bun").unwrap();
        writer.finish().unwrap();

        let error = extract_archive(
            &archive_path,
            &root.path().join("bun.exe"),
            "bun-windows-x64",
        )
        .unwrap_err();
        assert!(error.contains("has no"), "{error}");
    }
}
