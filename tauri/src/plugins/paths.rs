//! Every path the plugin system owns, derived from one app data directory.
//!
//! Keeping these in one place means a future per-user or portable data-root
//! change only has to move `app_data_dir`; no caller builds a plugin path by
//! hand.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginPaths {
    app_data_dir: PathBuf,
}

impl PluginPaths {
    pub fn new(app_data_dir: impl Into<PathBuf>) -> Self {
        Self {
            app_data_dir: app_data_dir.into(),
        }
    }

    /// `<app_data>/plugins`.
    pub fn root(&self) -> PathBuf {
        self.app_data_dir.join("plugins")
    }

    /// `<app_data>/plugins/packages` — the Bun workspace (`package.json`,
    /// `node_modules`) npm and GitHub packages are installed into.
    pub fn packages_dir(&self) -> PathBuf {
        self.root().join("packages")
    }

    /// `<app_data>/plugins/plugin-auth.json` — OpenCode-shaped sign-in state.
    pub fn auth_path(&self) -> PathBuf {
        self.root().join("plugin-auth.json")
    }

    /// `<app_data>/plugins/plugin-auth.json.lock` — cross-process writer lock
    /// shared by Rust and the Bun host.
    pub fn auth_lock_path(&self) -> PathBuf {
        self.root().join("plugin-auth.json.lock")
    }

    /// `<app_data>/plugins/cache/markets`.
    pub fn market_cache_dir(&self) -> PathBuf {
        self.root().join("cache").join("markets")
    }

    /// `<app_data>/plugins/cache/markets/<market_id>.json`.
    pub fn market_cache_path(&self, market_id: &str) -> PathBuf {
        self.market_cache_dir().join(format!("{market_id}.json"))
    }

    /// `<app_data>/plugins/cache/npm-meta.json`.
    pub fn npm_cache_path(&self) -> PathBuf {
        self.root().join("cache").join("npm-meta.json")
    }

    /// `<app_data>/plugins/runtime-host` — the Bun host bundle.
    pub fn runtime_host_dir(&self) -> PathBuf {
        self.root().join("runtime-host")
    }

    /// `<app_data>/plugins/runtimes/bun`.
    pub fn bun_runtimes_dir(&self) -> PathBuf {
        self.root().join("runtimes").join("bun")
    }

    /// `<app_data>/plugins/data` — private per-plugin persistent storage.
    pub fn data_dir(&self) -> PathBuf {
        self.root().join("data")
    }

    /// `<app_data>/plugins/data/<slug>-<hash>`, a collision-free directory for
    /// one plugin. The slug is cosmetic; the SHA-256 prefix is what keeps two
    /// plugin ids with the same slug apart, and the joined name can never
    /// escape `data/`.
    pub fn plugin_data_dir(&self, plugin_id: &str) -> PathBuf {
        self.data_dir().join(plugin_data_dir_name(plugin_id))
    }

    /// `<plugin_data_dir>/kv.json`.
    pub fn plugin_kv_path(&self, plugin_id: &str) -> PathBuf {
        self.plugin_data_dir(plugin_id).join("kv.json")
    }

    /// `<plugin_data_dir>/files` — the plugin's private file tree.
    pub fn plugin_files_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugin_data_dir(plugin_id).join("files")
    }

    /// Create the directories the plugin system writes into.
    pub fn ensure_dirs(&self) -> Result<(), String> {
        for dir in [
            self.root(),
            self.packages_dir(),
            self.market_cache_dir(),
            self.runtime_host_dir(),
            self.bun_runtimes_dir(),
            self.data_dir(),
        ] {
            fs::create_dir_all(&dir)
                .map_err(|error| format!("Failed to create {}: {error}", dir.display()))?;
        }
        Ok(())
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }
}

fn plugin_data_dir_name(plugin_id: &str) -> String {
    let digest = Sha256::digest(plugin_id.as_bytes());
    format!("{}-{}", plugin_slug(plugin_id), hex::encode(&digest[..4]))
}

fn plugin_slug(plugin_id: &str) -> String {
    let mut slug = String::new();
    for character in plugin_id.chars() {
        let lowered = character.to_ascii_lowercase();
        if lowered.is_ascii_lowercase() || lowered.is_ascii_digit() || matches!(lowered, '-' | '_')
        {
            slug.push(lowered);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() == 32 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() || is_reserved_windows_name(slug) {
        "plugin".to_string()
    } else {
        slug.to_string()
    }
}

/// Whether `name` is one of Windows' reserved device names (`CON`, `NUL`,
/// `COM1`…), with or without an extension.
pub(crate) fn is_reserved_windows_name(name: &str) -> bool {
    let upper = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper[3..]
                .chars()
                .all(|character| character.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_plugin_paths_live_under_the_app_data_plugins_dir() {
        let paths = PluginPaths::new(PathBuf::from("/data/app"));
        let root = paths.root();

        assert_eq!(paths.packages_dir(), root.join("packages"));
        assert_eq!(paths.auth_path(), root.join("plugin-auth.json"));
        assert_eq!(paths.auth_lock_path(), root.join("plugin-auth.json.lock"));
        assert_eq!(paths.market_cache_dir(), root.join("cache").join("markets"));
        assert_eq!(
            paths.market_cache_path("magpie-community"),
            root.join("cache")
                .join("markets")
                .join("magpie-community.json")
        );
        assert_eq!(
            paths.npm_cache_path(),
            root.join("cache").join("npm-meta.json")
        );
        assert_eq!(paths.runtime_host_dir(), root.join("runtime-host"));
        assert_eq!(paths.bun_runtimes_dir(), root.join("runtimes").join("bun"));
        assert_eq!(paths.data_dir(), root.join("data"));

        let data = paths.plugin_data_dir("@scope/demo");
        assert_eq!(data.parent(), Some(paths.data_dir().as_path()));
        assert_eq!(data.file_name().unwrap(), "scope-demo-6b73fbdc");
        assert_eq!(paths.plugin_kv_path("@scope/demo"), data.join("kv.json"));
        assert_eq!(paths.plugin_files_dir("@scope/demo"), data.join("files"));
    }

    #[test]
    fn ensure_dirs_creates_every_directory() {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());

        paths.ensure_dirs().unwrap();

        for dir in [
            paths.root(),
            paths.packages_dir(),
            paths.market_cache_dir(),
            paths.runtime_host_dir(),
            paths.bun_runtimes_dir(),
            paths.data_dir(),
        ] {
            assert!(dir.is_dir(), "{} was not created", dir.display());
        }
    }

    #[test]
    fn plugin_data_dirs_are_deterministic_and_collision_free() {
        let paths = PluginPaths::new(PathBuf::from("/data/app"));
        assert_eq!(
            paths.plugin_data_dir("@scope/demo"),
            paths.plugin_data_dir("@scope/demo")
        );
        assert_ne!(
            paths.plugin_data_dir("@scope/demo"),
            paths.plugin_data_dir("scope-demo")
        );

        // The slug is cosmetic: ids that sanitize to the same text still get
        // different folders.
        assert_ne!(
            paths.plugin_data_dir("../../evil"),
            paths.plugin_data_dir("..\\..\\evil")
        );
        for plugin_id in ["../../evil", "..\\..\\evil", "C:/Windows", "CON", "", "   "] {
            let name = paths
                .plugin_data_dir(plugin_id)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            assert!(!name.contains(".."), "{name}");
            assert!(!name.contains('/'), "{name}");
            assert!(!name.contains('\\'), "{name}");
            assert!(!name.contains(':'), "{name}");
            assert!(!is_reserved_windows_name(&name), "{name}");
            assert!(name.len() <= 32 + 1 + 8, "{name}");
        }
        assert!(paths
            .plugin_data_dir("CON")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("plugin-"));
        assert!(paths
            .plugin_data_dir("")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("plugin-"));

        let long = "A".repeat(80);
        let name = paths
            .plugin_data_dir(&long)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(name.starts_with("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-"));
        assert_eq!(name.len(), 32 + 1 + 8);
    }
}
