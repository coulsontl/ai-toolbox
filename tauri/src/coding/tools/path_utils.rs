//! Path normalization utilities for custom tools
//!
//! Provides functions to normalize user-input paths to relative paths.

use std::path::PathBuf;

/// Path type indicator for normalized paths
#[derive(Debug, Clone, PartialEq)]
pub enum PathType {
    /// Relative to home directory (~/...)
    HomeRelative,
    /// Relative to APPDATA/config directory (%APPDATA%/...)
    AppDataRelative,
    /// Absolute path (no conversion possible)
    Absolute,
}

/// Result of path normalization
#[derive(Debug, Clone)]
pub struct NormalizedPath {
    /// The normalized path string (without ~/  or %APPDATA%/ prefix)
    pub path: String,
    /// The type of path
    pub path_type: PathType,
}

/// Normalize a user-input path to a relative path for storage.
///
/// Rules:
/// 1. If path starts with `~` or `~/` -> strip prefix, HomeRelative
/// 2. If path starts with `%APPDATA%` -> strip prefix, AppDataRelative
/// 3. If path is absolute and starts with home_dir -> convert to relative, HomeRelative
/// 4. If path is absolute and starts with config_dir -> convert to relative, AppDataRelative
/// 5. Otherwise, save as-is (Absolute)
///
/// Path separators are normalized to `/` for cross-platform storage.
pub fn normalize_path(input: &str) -> NormalizedPath {
    let input = input.trim();

    // Normalize path separators to forward slashes for comparison
    let normalized_input = input.replace('\\', "/");

    // Check for ~ prefix
    if normalized_input.starts_with("~/") {
        return NormalizedPath {
            path: normalized_input[2..].to_string(),
            path_type: PathType::HomeRelative,
        };
    }
    if normalized_input == "~" {
        return NormalizedPath {
            path: String::new(),
            path_type: PathType::HomeRelative,
        };
    }

    // Check for %APPDATA% prefix (case insensitive)
    let upper_input = normalized_input.to_uppercase();
    if upper_input.starts_with("%APPDATA%/") {
        return NormalizedPath {
            path: normalized_input[10..].to_string(),
            path_type: PathType::AppDataRelative,
        };
    }
    if upper_input.starts_with("%APPDATA%\\") {
        return NormalizedPath {
            path: input[10..].replace('\\', "/"),
            path_type: PathType::AppDataRelative,
        };
    }
    if upper_input == "%APPDATA%" {
        return NormalizedPath {
            path: String::new(),
            path_type: PathType::AppDataRelative,
        };
    }

    // Try to detect if it's an absolute path matching home_dir
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy().replace('\\', "/");
        if normalized_input.starts_with(&home_str) {
            let relative = if normalized_input.len() > home_str.len() {
                // Skip the separator after home path
                let rest = &normalized_input[home_str.len()..];
                rest.trim_start_matches('/')
            } else {
                ""
            };
            return NormalizedPath {
                path: relative.to_string(),
                path_type: PathType::HomeRelative,
            };
        }
    }

    // Try to detect if it's an absolute path matching config_dir (APPDATA on Windows)
    if let Some(config) = dirs::config_dir() {
        let config_str = config.to_string_lossy().replace('\\', "/");
        if normalized_input.starts_with(&config_str) {
            let relative = if normalized_input.len() > config_str.len() {
                let rest = &normalized_input[config_str.len()..];
                rest.trim_start_matches('/')
            } else {
                ""
            };
            return NormalizedPath {
                path: relative.to_string(),
                path_type: PathType::AppDataRelative,
            };
        }
    }

    // If it doesn't match any known base, save as absolute
    NormalizedPath {
        path: normalized_input,
        path_type: PathType::Absolute,
    }
}

/// Convert a normalized path back to its storage format.
/// - HomeRelative: stored with ~/ prefix
/// - AppDataRelative: stored with %APPDATA%/ prefix
/// - Absolute: stored as-is
pub fn to_storage_path(normalized: &NormalizedPath) -> String {
    match normalized.path_type {
        PathType::HomeRelative => {
            if normalized.path.is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", normalized.path)
            }
        }
        PathType::AppDataRelative => {
            if normalized.path.is_empty() {
                "%APPDATA%".to_string()
            } else {
                format!("%APPDATA%/{}", normalized.path)
            }
        }
        PathType::Absolute => normalized.path.clone(),
    }
}

/// Resolve a storage path to an absolute path.
/// - If path starts with ~/, resolve using home_dir
/// - If path starts with %APPDATA%/, resolve using config_dir
/// - If path is absolute (starts with / or contains :), use as-is
/// - Otherwise, treat as home-relative for backward compatibility
pub fn resolve_storage_path(storage_path: &str) -> Option<PathBuf> {
    let path = storage_path.trim();
    let normalized = path.replace('\\', "/");

    // Check for ~/ prefix (home directory)
    if normalized.starts_with("~/") {
        let relative = &normalized[2..];
        return dirs::home_dir().map(|h| h.join(relative));
    }
    if normalized == "~" {
        return dirs::home_dir();
    }

    // Check for %APPDATA% prefix (config directory)
    let upper_path = normalized.to_uppercase();
    if upper_path.starts_with("%APPDATA%/") {
        let relative = &normalized[10..];
        return dirs::config_dir().map(|c| c.join(relative));
    }
    if upper_path == "%APPDATA%" {
        return dirs::config_dir();
    }

    // Check if it's an absolute path
    if normalized.starts_with('/') || normalized.contains(':') {
        return Some(PathBuf::from(path));
    }

    // Backward compatibility: treat plain relative paths as home-relative
    dirs::home_dir().map(|h| h.join(path))
}

/// Check if a storage path is a root directory (home or appdata) that would be dangerous to scan.
/// Returns true if the path resolves to a root directory that shouldn't be scanned.
pub fn is_root_directory(storage_path: &str) -> bool {
    let path = storage_path.trim();
    if path.is_empty() {
        return true;
    }

    let normalized = path.replace('\\', "/");
    let upper = normalized.to_uppercase();

    // Check for bare home or appdata references
    if normalized == "~" || upper == "%APPDATA%" {
        return true;
    }

    // Check if resolved path equals home_dir or config_dir
    if let Some(resolved) = resolve_storage_path(storage_path) {
        if let Some(home) = dirs::home_dir() {
            if resolved == home {
                return true;
            }
        }
        if let Some(config) = dirs::config_dir() {
            if resolved == config {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tilde_prefix() {
        let result = normalize_path("~/.config/myapp");
        assert_eq!(result.path, ".config/myapp");
        assert_eq!(result.path_type, PathType::HomeRelative);
    }

    #[test]
    fn test_appdata_prefix() {
        let result = normalize_path("%APPDATA%/Code/User");
        assert_eq!(result.path, "Code/User");
        assert_eq!(result.path_type, PathType::AppDataRelative);
    }

    #[test]
    fn test_windows_backslash() {
        let result = normalize_path("%APPDATA%\\Code\\User");
        assert_eq!(result.path, "Code/User");
        assert_eq!(result.path_type, PathType::AppDataRelative);
    }

    #[test]
    fn test_storage_path_home() {
        let normalized = NormalizedPath {
            path: ".config/myapp".to_string(),
            path_type: PathType::HomeRelative,
        };
        assert_eq!(to_storage_path(&normalized), "~/.config/myapp");
    }

    #[test]
    fn test_storage_path_appdata() {
        let normalized = NormalizedPath {
            path: "Code/User".to_string(),
            path_type: PathType::AppDataRelative,
        };
        assert_eq!(to_storage_path(&normalized), "%APPDATA%/Code/User");
    }

    #[test]
    fn test_storage_path_empty_home() {
        let normalized = NormalizedPath {
            path: String::new(),
            path_type: PathType::HomeRelative,
        };
        assert_eq!(to_storage_path(&normalized), "~");
    }
}
