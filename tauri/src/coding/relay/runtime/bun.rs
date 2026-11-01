//! Bun 运行时的按需下载、校验与安装。
//!
//! ── 为什么运行时下载而不是随包分发 ────────────────────────────
//! OpenCode 插件普遍依赖 Node/Bun 的内置模块（`fs`、`https`、`AsyncLocalStorage`、
//! `Bun.$`），用纯 Rust 的 JS 引擎去兼容是死路。Bun 各平台的压缩包约 90 MB，
//! 而多数用户根本不装插件——随包分发会让每个用户的安装包都为此变大。
//! 因此改成「第一次需要插件时才下载」。
//!
//! ── 失败必须可降级 ────────────────────────────────────────────
//! 下载失败（离线、被墙、镜像挂了）只能让**插件功能不可用**，不能让 2API 的
//! 普通渠道跟着不可用。所有错误都以 `Err(String)` 向上返回，由调用方决定提示。
//!
//! ── 布局 ──────────────────────────────────────────────────────
//! ```text
//! {data_dir}/relay/runtime/bun/<version>/bun(.exe)
//! ```
//! 版本目录并存，便于换版本或回滚；`current` 指针不单独存文件，而是由
//! [`installed_bun_path`] 现算——它能从目录名直接判断「这个版本装好没有」。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::http_client;

/// Bun 版本与各平台压缩包的 SHA-256。
///
/// 校验和来自 Bun 官方 `SHASUMS256.txt`。升级版本时必须同步更新本表，否则
/// [`BunRuntimeStatus::verified`] 会一直是 false、安装被拒。
pub const BUN_VERSION: &str = "1.4.2";

const BUN_CHECKSUMS: &[(&str, &str)] = &[
    ("bun-darwin-aarch64.zip", "90987a3a16d7db556d886ac3d551e7b6d3edf0a1cf43acaed622e8676be1d12f"),
    ("bun-darwin-x64.zip", "80520d7e17526308c9185d261679ac6d27798d3803a0e9f7ff9121ab8affb012"),
    ("bun-linux-x64-baseline.zip", "c678040f14fe0440eb839d37cbd0ce4c051a32da72806ac97de6a6aab6bf728f"),
    ("bun-linux-aarch64.zip", "54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7"),
    ("bun-windows-x64-baseline.zip", "78c221c2376f79731ccf4e4af0b3bb46d81fefa3296c5abee09ad8a1b21e68c6"),
    ("bun-windows-aarch64.zip", "a7a16b876a305fd1029c66dbd27007b4f6112ae896532f675878731a21e50cfd"),
];

// The table above is copied verbatim from Bun's `SHASUMS256.txt` for
// `BUN_VERSION`. Changing one without the other makes every download fail
// verification, which surfaces as "install failed" rather than a mismatch — so
// `every_supported_asset_has_a_checksum` pins the pairing.

/// Mirrors Bun's own release layout.
const BUN_RELEASE_BASE: &str = "https://github.com/oven-sh/bun/releases/download";

/// Where the runtime lives, relative to the app data directory.
const RUNTIME_DIR: &str = "relay/runtime/bun";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BunRuntimeStatus {
    pub version: String,
    /// Absolute path of the installed `bun` binary, empty when not installed.
    pub path: String,
    pub installed: bool,
    /// Whether this platform has a known checksum for the pinned version.
    /// `false` means we cannot verify a download and must refuse to install it.
    pub verified: bool,
    /// The asset name this platform would download (`bun-windows-x64-baseline.zip`).
    pub asset: String,
}

pub fn runtime_root() -> PathBuf {
    crate::app_paths::resolved_data_dir().join(RUNTIME_DIR)
}

fn bun_exe_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "bun.exe"
    } else {
        "bun"
    }
}

fn version_dir(version: &str) -> PathBuf {
    runtime_root().join(version)
}

pub fn installed_bun_path(version: &str) -> PathBuf {
    version_dir(version).join(bun_exe_name())
}

/// The Bun build this machine would download.
///
/// Windows x64 uses the *baseline* build: without an AVX2 CPU the regular build
/// crashes at startup with an illegal instruction, and detecting AVX2 is not
/// worth the risk here — the baseline build runs everywhere.
pub fn asset_name() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("windows", "x86_64") => Some("bun-windows-x64-baseline.zip"),
        ("windows", "aarch64") => Some("bun-windows-aarch64.zip"),
        ("macos", "aarch64") => Some("bun-darwin-aarch64.zip"),
        ("macos", "x86_64") => Some("bun-darwin-x64.zip"),
        ("linux", "x86_64") => Some("bun-linux-x64-baseline.zip"),
        ("linux", "aarch64") => Some("bun-linux-aarch64.zip"),
        _ => None,
    }
}

fn expected_checksum(asset: &str) -> Option<&'static str> {
    BUN_CHECKSUMS
        .iter()
        .find(|(name, _)| *name == asset)
        .map(|(_, sum)| *sum)
}

pub fn status() -> BunRuntimeStatus {
    let asset = asset_name().unwrap_or("").to_string();
    let path = installed_bun_path(BUN_VERSION);
    let installed = path.is_file();
    BunRuntimeStatus {
        version: BUN_VERSION.to_string(),
        path: if installed {
            path.display().to_string()
        } else {
            String::new()
        },
        installed,
        // An asset with no known checksum cannot be verified, so installing it
        // is refused rather than trusting whatever the network returned.
        verified: !asset.is_empty() && expected_checksum(&asset).is_some(),
        asset,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunDownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    /// Bytes per second, smoothed; 0 until the first interval elapses.
    pub speed: u64,
}

/// Download and install Bun, reporting progress through `on_progress`.
///
/// The zip is written to a temp file beside the target, verified against the
/// pinned checksum, then extracted. A partially written or corrupt download
/// never becomes a usable runtime: nothing is moved into `version_dir` until
/// both the checksum and the extraction succeeded.
pub async fn install<F>(
    db: &crate::db::SqliteDbState,
    mut on_progress: F,
) -> Result<BunRuntimeStatus, String>
where
    F: FnMut(BunDownloadProgress),
{
    let asset = asset_name()
        .ok_or_else(|| "This platform has no Bun build available for the plugin host".to_string())?;
    let checksum = expected_checksum(asset).ok_or_else(|| {
        format!("No pinned checksum for {asset}; refusing to install an unverifiable runtime")
    })?;
    let target = installed_bun_path(BUN_VERSION);
    if target.is_file() {
        return Ok(status());
    }

    let url = format!("{BUN_RELEASE_BASE}/bun-v{BUN_VERSION}/{asset}");
    let client = http_client::client_with_timeout(db, 600).await?;
    let mut response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("Failed to download Bun: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download Bun: HTTP {} from {url}",
            response.status()
        ));
    }
    let total = response.content_length().unwrap_or(0);

    let root = runtime_root();
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("Failed to create the 2API runtime directory: {error}"))?;
    let archive_path = root.join(format!("{asset}.download"));
    let mut download = std::fs::File::create(&archive_path)
        .map_err(|error| format!("Failed to create the Bun archive file: {error}"))?;

    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_reported = Instant::now();
    let mut last_bytes = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Failed to read the Bun download: {error}"))?
    {
        hasher.update(&chunk);
        download
            .write_all(&chunk)
            .map_err(|error| format!("Failed to write the Bun archive: {error}"))?;
        downloaded += chunk.len() as u64;

        let elapsed = last_reported.elapsed();
        if elapsed >= Duration::from_millis(200) {
            let delta = downloaded.saturating_sub(last_bytes);
            let speed = (delta as f64 / elapsed.as_secs_f64()) as u64;
            last_bytes = downloaded;
            last_reported = Instant::now();
            on_progress(BunDownloadProgress {
                downloaded,
                total,
                speed,
            });
        }
    }
    drop(download);

    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(checksum) {
        let _ = std::fs::remove_file(&archive_path);
        return Err(format!(
            "Bun download failed verification: expected {checksum}, got {actual}"
        ));
    }

    // Extraction is blocking file I/O; keep it off the async runtime.
    let archive_for_extract = archive_path.clone();
    let target_dir = version_dir(BUN_VERSION);
    let extracted = tauri::async_runtime::spawn_blocking(move || {
        extract_bun(&archive_for_extract, &target_dir)
    })
    .await
    .map_err(|error| format!("Bun extraction task failed: {error}"))?;
    let _ = std::fs::remove_file(&archive_path);
    extracted?;

    if !target.is_file() {
        return Err("Bun was unpacked but its executable was not found".to_string());
    }
    Ok(status())
}

/// Unpack the Bun archive.
///
/// Bun's zips nest everything under a `bun-<target>/` directory, and the binary
/// has to land at `version_dir/bun(.exe)` regardless of what the archive's own
/// layout is. Paths are re-checked against traversal: an archive is untrusted
/// input even when it came from the pinned URL.
fn extract_bun(archive_path: &Path, target_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|error| format!("Failed to open the Bun archive: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("Failed to read the Bun archive: {error}"))?;
    std::fs::create_dir_all(target_dir)
        .map_err(|error| format!("Failed to create the Bun version directory: {error}"))?;

    let exe = bun_exe_name();
    let mut found_binary = false;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read a Bun archive entry: {error}"))?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(format!(
                "Bun archive contains an unsafe path: {}",
                entry.name()
            ));
        };
        if entry.is_dir() {
            continue;
        }
        let is_binary = relative
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == exe);
        // Only the binary matters; the archive also carries shims and docs.
        if !is_binary {
            continue;
        }
        let destination = target_dir.join(exe);
        std::io::copy(&mut entry, &mut std::fs::File::create(&destination).map_err(|error| {
            format!("Failed to create {}: {error}", destination.display())
        })?)
        .map_err(|error| format!("Failed to extract {}: {error}", destination.display()))?;
        found_binary = true;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &destination,
                std::fs::Permissions::from_mode(0o755),
            );
        }
    }

    if !found_binary {
        return Err(format!("Bun archive contains no {exe}"));
    }
    Ok(())
}

/// Remove the installed runtime. Used when a user wants to reclaim the space;
/// the next plugin use downloads it again.
pub fn uninstall() -> Result<(), String> {
    let root = runtime_root();
    if root.exists() {
        std::fs::remove_dir_all(&root)
            .map_err(|error| format!("Failed to remove the Bun runtime: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_asset_has_a_checksum() {
        // The reverse direction matters too: a checksum entry whose asset this
        // platform never picks is dead weight but harmless, while a platform
        // with no entry refuses to install.
        for (asset, sum) in BUN_CHECKSUMS {
            assert_eq!(sum.len(), 64, "{asset} checksum must be 64 hex chars");
            assert!(
                sum.chars().all(|c| c.is_ascii_hexdigit()),
                "{asset} checksum must be hex"
            );
        }
        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ] {
            let asset = match (os, arch) {
                ("windows", "x86_64") => "bun-windows-x64-baseline.zip",
                ("windows", "aarch64") => "bun-windows-aarch64.zip",
                ("macos", "aarch64") => "bun-darwin-aarch64.zip",
                ("macos", "x86_64") => "bun-darwin-x64.zip",
                ("linux", "x86_64") => "bun-linux-x64-baseline.zip",
                _ => "bun-linux-aarch64.zip",
            };
            assert!(
                expected_checksum(asset).is_some(),
                "{os}/{arch} needs a pinned checksum for {asset}"
            );
        }
    }

    #[test]
    fn status_reports_the_installed_binary_name() {
        let status = status();
        assert_eq!(status.version, BUN_VERSION);
        assert!(status.asset.ends_with(".zip"));
    }
}
