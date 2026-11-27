//! The JavaScript host bundle, embedded at compile time.
//!
//! The files come from Magpie's plugin host and are written into
//! `<app_data>/plugins/runtime-host/` before the host starts. Each write is
//! content-addressed: an existing file whose SHA-256 matches the embedded
//! copy is reused, anything else is replaced, so an edited or truncated
//! runtime never runs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const HOST_JS: &str = include_str!("assets/host.js");
pub const OPENS_JS: &str = include_str!("assets/opens.js");
pub const PI_JS: &str = include_str!("assets/pi.js");

pub const HOST_FILE: &str = "host.js";
pub const OPENS_FILE: &str = "opens.js";
pub const PI_FILE: &str = "pi.js";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHostFile {
    pub name: String,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHostFiles {
    pub directory: String,
    pub files: Vec<RuntimeHostFile>,
}

#[derive(Clone, Copy)]
struct EmbeddedFile {
    name: &'static str,
    contents: &'static str,
}

fn embedded_files() -> [EmbeddedFile; 3] {
    [
        EmbeddedFile {
            name: HOST_FILE,
            contents: HOST_JS,
        },
        EmbeddedFile {
            name: OPENS_FILE,
            contents: OPENS_JS,
        },
        EmbeddedFile {
            name: PI_FILE,
            contents: PI_JS,
        },
    ]
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Write the bundle into `directory` and return every file's digest.
pub fn materialize(directory: &Path) -> Result<RuntimeHostFiles, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create '{}': {error}", directory.display()))?;

    let mut files = Vec::new();
    for embedded in embedded_files() {
        let bytes = embedded.contents.as_bytes();
        let digest = sha256_hex(bytes);
        let path = directory.join(embedded.name);
        let needs_write = match fs::read(&path) {
            Ok(existing) => sha256_hex(&existing) != digest,
            Err(_) => true,
        };
        if needs_write {
            write_atomic(&path, bytes)?;
        }
        files.push(RuntimeHostFile {
            name: embedded.name.to_string(),
            sha256: digest,
            bytes: bytes.len(),
        });
    }

    Ok(RuntimeHostFiles {
        directory: directory.to_string_lossy().to_string(),
        files,
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Failed to write '{}': {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("Failed to replace '{}': {error}", path.display()))?;
    Ok(())
}

/// Paths the host needs once the bundle is materialized.
pub struct RuntimeHostPaths {
    pub host: PathBuf,
    pub opens: PathBuf,
    pub pi: PathBuf,
}

impl RuntimeHostPaths {
    pub fn new(directory: &Path) -> Self {
        Self {
            host: directory.join(HOST_FILE),
            opens: directory.join(OPENS_FILE),
            pi: directory.join(PI_FILE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_and_repairs_tampered_files() {
        let root = tempfile::tempdir().unwrap();
        let first = materialize(root.path()).unwrap();
        assert_eq!(first.files.len(), 3);
        assert!(first.files.iter().all(|file| file.sha256.len() == 64));

        let host_path = root.path().join(HOST_FILE);
        fs::write(&host_path, b"tampered").unwrap();
        let second = materialize(root.path()).unwrap();
        assert_eq!(fs::read_to_string(&host_path).unwrap(), HOST_JS);
        assert_eq!(second.files[0].sha256, sha256_hex(HOST_JS.as_bytes()));
        assert_eq!(second.files[0].bytes, HOST_JS.len());
    }

    #[test]
    fn embedded_sources_are_the_expected_host_scripts() {
        assert!(HOST_JS.contains("\"credit\""));
        assert!(HOST_JS.contains("gatewayRequest"));
        assert!(OPENS_JS.contains("magpie.opens"));
        assert!(PI_JS.contains("isPi"));
    }
}
