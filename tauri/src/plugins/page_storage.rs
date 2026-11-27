//! Private persistent storage for plugin pages.
//!
//! Two capabilities, both gated by permissions the Rust side checks against
//! the installed record on every call — the frontend is never trusted:
//!
//! - `storage:kv` keeps one JSON object per plugin in
//!   `<plugin_data_dir>/kv.json`, written atomically.
//! - `storage:files` keeps a private file tree under
//!   `<plugin_data_dir>/files`, with relative paths only, symlink refusal and
//!   per-file / total quotas.
//!
//! Nothing here creates a SQLite table: plugin storage is deliberately a
//! file-side concern so a plugin can be updated or rolled back without a
//! schema migration.

use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Map, Value};

use crate::db::SqliteDbState;

use super::paths::{is_reserved_windows_name, PluginPaths};
use super::store;

pub const KV_PERMISSION: &str = "storage:kv";
pub const FILES_PERMISSION: &str = "storage:files";

/// One key's serialized value.
const KV_VALUE_LIMIT: usize = 1024 * 1024;
/// The whole `kv.json`.
const KV_TOTAL_LIMIT: usize = 20 * 1024 * 1024;
/// One file in `files/`.
const FILE_LIMIT: u64 = 20 * 1024 * 1024;
/// The whole `files/` tree.
const FILES_TOTAL_LIMIT: u64 = 200 * 1024 * 1024;
const MAX_PATH_DEPTH: usize = 16;

/// Serializes runs of key-value operations across every plugin. The lock is
/// held only for the read-modify-write of one file.
static KV_LOCK: Mutex<()> = Mutex::new(());
/// Serializes file operations so the directory-size quota cannot be raced.
static FILES_LOCK: Mutex<()> = Mutex::new(());

fn lock(mutex: &'static Mutex<()>) -> MutexGuard<'static, ()> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn require_storage_permission(
    db: &SqliteDbState,
    plugin_id: &str,
    permission: &str,
) -> Result<(), String> {
    let record = store::get_installed_plugin(db, plugin_id)?
        .ok_or_else(|| format!("Plugin '{plugin_id}' is not installed"))?;
    if !record.enabled {
        return Err(format!("Plugin '{plugin_id}' is disabled"));
    }
    if !record
        .permissions
        .iter()
        .any(|declared| declared == permission)
    {
        return Err(format!(
            "Plugin '{}' does not declare the '{permission}' permission",
            record.display_name
        ));
    }
    Ok(())
}

// ---- key-value store --------------------------------------------------------

/// One `storage.*` operation.
pub fn kv_op(
    db: &SqliteDbState,
    paths: &PluginPaths,
    plugin_id: &str,
    op: &str,
    key: Option<String>,
    value: Option<Value>,
    prefix: Option<String>,
) -> Result<Value, String> {
    require_storage_permission(db, plugin_id, KV_PERMISSION)?;
    let _guard = lock(&KV_LOCK);
    let path = paths.plugin_kv_path(plugin_id);
    let mut store = load_kv(&path)?;

    match op {
        "get" => {
            let key = require_key(key)?;
            Ok(store.get(&key).cloned().unwrap_or(Value::Null))
        }
        "set" => {
            let key = require_key(key)?;
            let value = value.ok_or_else(|| "A value is required".to_string())?;
            let serialized = serde_json::to_vec(&value)
                .map_err(|error| format!("Failed to serialize the value: {error}"))?;
            if serialized.len() > KV_VALUE_LIMIT {
                return Err(format!("One value is {} KB at most", KV_VALUE_LIMIT >> 10));
            }
            store.insert(key, value);
            save_kv(&path, &store)?;
            Ok(json!({ "ok": true }))
        }
        "delete" => {
            let key = require_key(key)?;
            let deleted = store.remove(&key).is_some();
            if deleted {
                save_kv(&path, &store)?;
            }
            Ok(json!({ "deleted": deleted }))
        }
        "list" => {
            let prefix = prefix.unwrap_or_default();
            let mut keys: Vec<String> = store
                .keys()
                .filter(|key| key.starts_with(&prefix))
                .cloned()
                .collect();
            keys.sort();
            Ok(json!(keys))
        }
        "clear" => {
            store.clear();
            save_kv(&path, &store)?;
            Ok(json!({ "ok": true }))
        }
        other => Err(format!("Unknown key-value operation '{other}'")),
    }
}

fn require_key(key: Option<String>) -> Result<String, String> {
    let key = key.ok_or_else(|| "A key is required".to_string())?;
    if key.is_empty() {
        return Err("A key is required".to_string());
    }
    if key.chars().count() > 256 {
        return Err("A key is at most 256 characters".to_string());
    }
    Ok(key)
}

fn load_kv(path: &Path) -> Result<Map<String, Value>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("Failed to read '{}': {error}", path.display())),
    };
    if bytes.len() > KV_TOTAL_LIMIT {
        return Err("The plugin's key-value store exceeds its 20 MB quota".to_string());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "The plugin's key-value store is not valid JSON ({}): {error}",
            path.display()
        )
    })?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err("The plugin's key-value store is not a JSON object".to_string()),
    }
}

fn save_kv(path: &Path, store: &Map<String, Value>) -> Result<(), String> {
    let bytes = serde_json::to_vec(store)
        .map_err(|error| format!("Failed to serialize the key-value store: {error}"))?;
    check_kv_total(bytes.len())?;
    write_atomic(path, &bytes)
}

fn check_kv_total(bytes: usize) -> Result<(), String> {
    if bytes > KV_TOTAL_LIMIT {
        return Err("The plugin's key-value store exceeds its 20 MB quota".to_string());
    }
    Ok(())
}

// ---- private files ----------------------------------------------------------

/// One `files.*` operation.
pub fn files_op(
    db: &SqliteDbState,
    paths: &PluginPaths,
    plugin_id: &str,
    op: &str,
    path: Option<String>,
    content: Option<String>,
) -> Result<Value, String> {
    require_storage_permission(db, plugin_id, FILES_PERMISSION)?;
    let _guard = lock(&FILES_LOCK);

    let root = paths.plugin_files_dir(plugin_id);
    fs::create_dir_all(&root)
        .map_err(|error| format!("Failed to create '{}': {error}", root.display()))?;
    let root = fs::canonicalize(&root)
        .map_err(|error| format!("Failed to resolve '{}': {error}", root.display()))?;

    match op {
        "readText" => {
            let relative = validate_relative_path(&require_path(path)?)?;
            let file = resolve_existing(&root, &relative)?
                .ok_or_else(|| "The file does not exist".to_string())?;
            if !fs::metadata(&file)
                .map_err(|error| format!("Failed to read '{}': {error}", file.display()))?
                .is_file()
            {
                return Err("The path is not a file".to_string());
            }
            let bytes = fs::read(&file)
                .map_err(|error| format!("Failed to read '{}': {error}", file.display()))?;
            let content =
                String::from_utf8(bytes).map_err(|_| "The file is not valid UTF-8".to_string())?;
            Ok(json!({ "content": content }))
        }
        "writeText" => {
            let relative = validate_relative_path(&require_path(path)?)?;
            let content = content.ok_or_else(|| "Content is required".to_string())?;
            let bytes = content.into_bytes();
            let size = bytes.len();
            write_plugin_file(&root, &relative, &bytes)?;
            Ok(json!({ "size": size }))
        }
        "readBytes" => {
            let relative = validate_relative_path(&require_path(path)?)?;
            let file = resolve_existing(&root, &relative)?
                .ok_or_else(|| "The file does not exist".to_string())?;
            if !fs::metadata(&file)
                .map_err(|error| format!("Failed to read '{}': {error}", file.display()))?
                .is_file()
            {
                return Err("The path is not a file".to_string());
            }
            let bytes = fs::read(&file)
                .map_err(|error| format!("Failed to read '{}': {error}", file.display()))?;
            Ok(json!({
                "base64": BASE64.encode(&bytes),
                "size": bytes.len(),
            }))
        }
        "writeBytes" => {
            let relative = validate_relative_path(&require_path(path)?)?;
            let content = content.ok_or_else(|| "Base64 content is required".to_string())?;
            if content.len() as u64 > FILE_LIMIT * 4 / 3 + 1024 {
                return Err("One file is 20 MB at most".to_string());
            }
            let bytes = BASE64
                .decode(content.trim())
                .map_err(|error| format!("The content is not valid base64: {error}"))?;
            let size = bytes.len();
            write_plugin_file(&root, &relative, &bytes)?;
            Ok(json!({ "size": size }))
        }
        "list" => {
            let relative = validate_relative_path_allow_empty(path.as_deref().unwrap_or_default())?;
            list_files(&root, &relative)
        }
        "stat" => {
            let relative = validate_relative_path(&require_path(path)?)?;
            stat_file(&root, &relative)
        }
        "delete" => {
            let relative = validate_relative_path(&require_path(path)?)?;
            delete_file(&root, &relative)
        }
        other => Err(format!("Unknown file operation '{other}'")),
    }
}

fn require_path(path: Option<String>) -> Result<String, String> {
    path.ok_or_else(|| "A path is required".to_string())
        .and_then(|path| {
            if path.trim().is_empty() {
                Err("A path is required".to_string())
            } else {
                Ok(path)
            }
        })
}

fn validate_relative_path(value: &str) -> Result<PathBuf, String> {
    let relative = validate_relative_path_allow_empty(value)?;
    if relative.as_os_str().is_empty() {
        return Err("A path is required".to_string());
    }
    Ok(relative)
}

/// Split a plugin-controlled relative path into safe components. Empty paths
/// mean the `files/` root and are only valid where a directory is expected.
fn validate_relative_path_allow_empty(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(PathBuf::new());
    }
    if value.starts_with('/') || value.starts_with('\\') || is_drive_prefix(value) {
        return Err("Absolute paths are not allowed".to_string());
    }
    if value.contains('\\') {
        return Err("Backslashes are not allowed".to_string());
    }
    if value.contains('\0') {
        return Err("NUL bytes are not allowed".to_string());
    }
    // Windows reads `file:stream` as an NTFS alternate data stream: it is
    // invisible in the directory listing and skipped by the quota walk.
    if value.contains(':') {
        return Err("Colons are not allowed in plugin file paths".to_string());
    }

    let segments: Vec<&str> = value.split('/').collect();
    if segments.len() > MAX_PATH_DEPTH {
        return Err(format!("A path has at most {MAX_PATH_DEPTH} segments"));
    }
    for segment in &segments {
        if segment.is_empty() {
            return Err("Empty path segments are not allowed".to_string());
        }
        if *segment == "." || *segment == ".." {
            return Err("The path must stay inside the plugin folder".to_string());
        }
        if is_reserved_windows_name(segment) {
            return Err(format!("'{segment}' is a reserved file name"));
        }
    }

    let mut relative = PathBuf::new();
    for segment in segments {
        relative.push(segment);
    }
    Ok(relative)
}

fn is_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Resolve an existing path under `root`, refusing symlinks in any component.
fn resolve_existing(root: &Path, relative: &Path) -> Result<Option<PathBuf>, String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("Symbolic links are not allowed in plugin files".to_string());
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "Failed to inspect '{}': {error}",
                    current.display()
                ))
            }
        }
    }
    let canonical = fs::canonicalize(&current)
        .map_err(|error| format!("Failed to resolve '{}': {error}", current.display()))?;
    if !canonical.starts_with(root) {
        return Err("The path resolves outside the plugin file folder".to_string());
    }
    Ok(Some(canonical))
}

/// Create missing parent folders one safe component at a time, then return the
/// target path and the size it currently occupies (0 when new).
fn prepare_write_target(root: &Path, relative: &Path) -> Result<(PathBuf, u64), String> {
    let components: Vec<_> = relative.components().collect();
    let Some((last, parents)) = components.split_last() else {
        return Err("A path is required".to_string());
    };

    let mut current = root.to_path_buf();
    for component in parents {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("Symbolic links are not allowed in plugin files".to_string());
                }
                if !metadata.is_dir() {
                    return Err(format!("'{}' is not a folder", current.display()));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => fs::create_dir(&current)
                .map_err(|error| format!("Failed to create '{}': {error}", current.display()))?,
            Err(error) => {
                return Err(format!(
                    "Failed to inspect '{}': {error}",
                    current.display()
                ))
            }
        }
    }

    let canonical_parent = fs::canonicalize(&current)
        .map_err(|error| format!("Failed to resolve '{}': {error}", current.display()))?;
    if !canonical_parent.starts_with(root) {
        return Err("The path resolves outside the plugin file folder".to_string());
    }

    let target = canonical_parent.join(last.as_os_str());
    let old_size = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err("Symbolic links are not allowed in plugin files".to_string());
            }
            if metadata.is_dir() {
                return Err("The path is a folder".to_string());
            }
            metadata.len()
        }
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => return Err(format!("Failed to inspect '{}': {error}", target.display())),
    };
    Ok((target, old_size))
}

fn write_plugin_file(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), String> {
    let (target, old_size) = prepare_write_target(root, relative)?;
    let total = directory_size(root)?;
    check_file_quota(total, old_size, bytes.len() as u64)?;
    write_atomic(&target, bytes)
}

fn check_file_quota(total: u64, old_size: u64, new_size: u64) -> Result<(), String> {
    if new_size > FILE_LIMIT {
        return Err("One file is 20 MB at most".to_string());
    }
    let new_total = total.saturating_sub(old_size).saturating_add(new_size);
    if new_total > FILES_TOTAL_LIMIT {
        return Err("The plugin's files exceed their 200 MB quota".to_string());
    }
    Ok(())
}

fn directory_size(root: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Failed to read '{}': {error}", directory.display())),
        };
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("Failed to read '{}': {error}", directory.display()))?;
            let file_type = entry.file_type().map_err(|error| {
                format!("Failed to inspect '{}': {error}", entry.path().display())
            })?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                let size = entry
                    .metadata()
                    .map_err(|error| {
                        format!("Failed to inspect '{}': {error}", entry.path().display())
                    })?
                    .len();
                total = total.saturating_add(size);
            }
        }
    }
    Ok(total)
}

fn list_files(root: &Path, relative: &Path) -> Result<Value, String> {
    let Some(directory) = resolve_existing(root, relative)? else {
        return Ok(json!([]));
    };
    let metadata = fs::metadata(&directory)
        .map_err(|error| format!("Failed to read '{}': {error}", directory.display()))?;
    if !metadata.is_dir() {
        return Err("The path is not a folder".to_string());
    }
    let base = relative_string(relative);

    let mut entries: Vec<(String, Value)> = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("Failed to read '{}': {error}", directory.display()))?
    {
        let entry =
            entry.map_err(|error| format!("Failed to read '{}': {error}", directory.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect '{}': {error}", entry.path().display()))?;
        // A symlink could point anywhere; it is neither listed nor reachable.
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry
            .metadata()
            .map_err(|error| format!("Failed to inspect '{}': {error}", entry.path().display()))?;
        let entry_path = if base.is_empty() {
            name.clone()
        } else {
            format!("{base}/{name}")
        };
        let is_dir = file_type.is_dir();
        entries.push((
            name.clone(),
            json!({
                "name": name,
                "path": entry_path,
                "isDir": is_dir,
                "size": if is_dir { 0 } else { metadata.len() },
                "modifiedAt": modified_rfc3339(&metadata),
            }),
        ));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(Value::Array(
        entries.into_iter().map(|(_, value)| value).collect(),
    ))
}

fn stat_file(root: &Path, relative: &Path) -> Result<Value, String> {
    let Some(target) = resolve_existing(root, relative)? else {
        return Ok(json!({
            "exists": false,
            "isDir": false,
            "size": 0u64,
            "modifiedAt": Value::Null,
        }));
    };
    let metadata = fs::metadata(&target)
        .map_err(|error| format!("Failed to read '{}': {error}", target.display()))?;
    let is_dir = metadata.is_dir();
    Ok(json!({
        "exists": true,
        "isDir": is_dir,
        "size": if is_dir { 0 } else { metadata.len() },
        "modifiedAt": modified_rfc3339(&metadata),
    }))
}

fn delete_file(root: &Path, relative: &Path) -> Result<Value, String> {
    let Some(target) = resolve_existing(root, relative)? else {
        return Ok(json!({ "deleted": false }));
    };
    let metadata = fs::symlink_metadata(&target)
        .map_err(|error| format!("Failed to inspect '{}': {error}", target.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(&target)
            .map_err(|error| format!("Failed to delete '{}': {error}", target.display()))?;
    } else {
        fs::remove_file(&target)
            .map_err(|error| format!("Failed to delete '{}': {error}", target.display()))?;
    }
    Ok(json!({ "deleted": true }))
}

fn relative_string(relative: &Path) -> String {
    relative
        .iter()
        .map(|component| component.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn modified_rfc3339(metadata: &fs::Metadata) -> Value {
    metadata
        .modified()
        .ok()
        .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339())
        .map(Value::String)
        .unwrap_or(Value::Null)
}

/// Write through a same-folder temporary file so a crash never leaves a
/// half-written store or file behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("'{}' has no parent folder", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create '{}': {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(".{file_name}.{}.{nonce}.tmp", std::process::id()));

    write_synced(&temporary, bytes)
        .map_err(|error| format!("Failed to write '{}': {error}", temporary.display()))?;
    if let Err(error) = rename_retrying_sharing_violation(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Failed to replace '{}': {error}", path.display()));
    }
    Ok(())
}

/// Write the whole file and flush it to disk, so a rename can never expose a
/// half-written store after a crash or a power loss.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Windows refuses to rename over a file another process briefly holds open
/// (an antivirus scanner, Explorer's preview). Three short retries ride that
/// out; every other error is returned immediately.
fn rename_retrying_sharing_violation(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last_error = None;
    for attempt in 0..3 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let sharing_violation = cfg!(windows) && error.raw_os_error() == Some(32);
                if !sharing_violation {
                    return Err(error);
                }
                last_error = Some(error);
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
    Err(last_error.expect("a failed rename always records its error"))
}

/// Delete a plugin's whole private data directory. Missing data is not an
/// error, so uninstalling twice or a plugin that never stored anything is
/// fine.
pub fn purge_plugin_data(paths: &PluginPaths, plugin_id: &str) -> Result<(), String> {
    let directory = paths.plugin_data_dir(plugin_id);
    match fs::remove_dir_all(&directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to delete '{}': {error}",
            directory.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::types::InstalledPluginRecord;

    struct Fixture {
        _root: tempfile::TempDir,
        paths: PluginPaths,
        db: SqliteDbState,
    }

    fn fixture(permissions: &[&str], enabled: bool) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        paths.ensure_dirs().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        store::upsert_installed_plugin(
            &db,
            &InstalledPluginRecord {
                id: "page-demo".to_string(),
                package_name: "page-demo".to_string(),
                display_name: "Page Demo".to_string(),
                kind: crate::plugins::types::PluginKind::Page,
                enabled,
                source_type: "local".to_string(),
                permissions: permissions.iter().map(|value| value.to_string()).collect(),
                ..InstalledPluginRecord::default()
            },
        )
        .unwrap();
        Fixture {
            _root: root,
            paths,
            db,
        }
    }

    fn kv(
        fixture: &Fixture,
        op: &str,
        key: Option<&str>,
        value: Option<Value>,
        prefix: Option<&str>,
    ) -> Result<Value, String> {
        kv_op(
            &fixture.db,
            &fixture.paths,
            "page-demo",
            op,
            key.map(str::to_string),
            value,
            prefix.map(str::to_string),
        )
    }

    fn files(
        fixture: &Fixture,
        op: &str,
        path: Option<&str>,
        content: Option<&str>,
    ) -> Result<Value, String> {
        files_op(
            &fixture.db,
            &fixture.paths,
            "page-demo",
            op,
            path.map(str::to_string),
            content.map(str::to_string),
        )
    }

    #[test]
    fn storage_requires_the_declared_permission_and_an_enabled_plugin() {
        let none = fixture(&[], true);
        let error = kv(&none, "get", Some("a"), None, None).unwrap_err();
        assert!(error.contains("does not declare"), "{error}");
        let error = files(&none, "list", None, None).unwrap_err();
        assert!(error.contains("does not declare"), "{error}");

        let kv_only = fixture(&[KV_PERMISSION], true);
        assert!(kv(&kv_only, "get", Some("a"), None, None).is_ok());
        let error = files(&kv_only, "list", None, None).unwrap_err();
        assert!(error.contains("does not declare"), "{error}");

        let disabled = fixture(&[KV_PERMISSION, FILES_PERMISSION], false);
        let error = kv(&disabled, "get", Some("a"), None, None).unwrap_err();
        assert!(error.contains("disabled"), "{error}");
        let error = files(&disabled, "list", None, None).unwrap_err();
        assert!(error.contains("disabled"), "{error}");

        let error = kv_op(
            &kv_only.db,
            &kv_only.paths,
            "missing",
            "get",
            Some("a".into()),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("not installed"), "{error}");
    }

    #[test]
    fn kv_roundtrip_listing_and_clear() {
        let fixture = fixture(&[KV_PERMISSION], true);
        assert_eq!(
            kv(&fixture, "set", Some("beta"), Some(json!(1)), None).unwrap(),
            json!({"ok": true})
        );
        kv(
            &fixture,
            "set",
            Some("alpha"),
            Some(json!({"nested": [1, 2]})),
            None,
        )
        .unwrap();
        kv(&fixture, "set", Some("alphabet"), Some(json!("text")), None).unwrap();

        assert_eq!(
            kv(&fixture, "get", Some("alpha"), None, None).unwrap(),
            json!({"nested": [1, 2]})
        );
        assert_eq!(
            kv(&fixture, "get", Some("missing"), None, None).unwrap(),
            Value::Null
        );
        assert_eq!(
            kv(&fixture, "list", None, None, Some("alph")).unwrap(),
            json!(["alpha", "alphabet"])
        );
        assert_eq!(
            kv(&fixture, "list", None, None, None).unwrap(),
            json!(["alpha", "alphabet", "beta"])
        );

        assert_eq!(
            kv(&fixture, "delete", Some("alpha"), None, None).unwrap(),
            json!({"deleted": true})
        );
        assert_eq!(
            kv(&fixture, "delete", Some("alpha"), None, None).unwrap(),
            json!({"deleted": false})
        );
        assert_eq!(
            kv(&fixture, "get", Some("alpha"), None, None).unwrap(),
            Value::Null
        );

        // The store persists to kv.json between calls.
        let saved = fs::read_to_string(fixture.paths.plugin_kv_path("page-demo")).unwrap();
        assert!(saved.contains("alphabet"));

        assert_eq!(
            kv(&fixture, "clear", None, None, None).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(kv(&fixture, "list", None, None, None).unwrap(), json!([]));
    }

    #[test]
    fn kv_rejects_bad_keys_and_oversized_values() {
        let fixture = fixture(&[KV_PERMISSION], true);
        assert!(kv(&fixture, "get", None, None, None).is_err());
        assert!(kv(&fixture, "get", Some(""), None, None).is_err());
        let long_key = "k".repeat(257);
        assert!(kv(&fixture, "get", Some(&long_key), None, None).is_err());
        assert!(kv(&fixture, "set", Some("a"), None, None).is_err());
        assert!(kv(&fixture, "unknown", Some("a"), None, None).is_err());

        let oversized = "x".repeat(KV_VALUE_LIMIT);
        let error = kv(&fixture, "set", Some("big"), Some(json!(oversized)), None).unwrap_err();
        assert!(error.contains("at most"), "{error}");

        assert!(check_kv_total(KV_TOTAL_LIMIT).is_ok());
        assert!(check_kv_total(KV_TOTAL_LIMIT + 1).is_err());
    }

    #[test]
    fn kv_refuses_a_store_over_the_total_quota() {
        let fixture = fixture(&[KV_PERMISSION], true);
        let path = fixture.paths.plugin_kv_path("page-demo");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b' '; KV_TOTAL_LIMIT + 1]).unwrap();

        let error = kv(&fixture, "get", Some("a"), None, None).unwrap_err();
        assert!(error.contains("20 MB"), "{error}");
    }

    #[test]
    fn files_roundtrip_list_stat_and_delete() {
        let fixture = fixture(&[FILES_PERMISSION], true);
        assert_eq!(
            files(&fixture, "writeText", Some("notes/a.txt"), Some("hello")).unwrap(),
            json!({"size": 5})
        );
        assert_eq!(
            files(&fixture, "readText", Some("notes/a.txt"), None).unwrap(),
            json!({"content": "hello"})
        );
        assert_eq!(
            files(&fixture, "readText", Some("notes"), None)
                .unwrap_err()
                .contains("not a file"),
            true
        );

        let payload = BASE64.encode([0u8, 1, 2, 255]);
        assert_eq!(
            files(&fixture, "writeBytes", Some("bin/data.bin"), Some(&payload)).unwrap(),
            json!({"size": 4})
        );
        let bytes = files(&fixture, "readBytes", Some("bin/data.bin"), None).unwrap();
        assert_eq!(bytes["base64"], payload);
        assert_eq!(bytes["size"], 4);

        let root_list = files(&fixture, "list", None, None).unwrap();
        assert_eq!(root_list.as_array().unwrap().len(), 2);
        assert_eq!(root_list[0]["name"], "bin");
        assert_eq!(root_list[0]["isDir"], true);
        assert_eq!(root_list[1]["name"], "notes");

        let notes = files(&fixture, "list", Some("notes"), None).unwrap();
        assert_eq!(notes[0]["path"], "notes/a.txt");
        assert_eq!(notes[0]["size"], 5);
        assert_eq!(notes[0]["isDir"], false);
        assert!(notes[0]["modifiedAt"].as_str().unwrap().contains('T'));

        let missing = files(&fixture, "stat", Some("none.txt"), None).unwrap();
        assert_eq!(missing["exists"], false);
        assert_eq!(missing["isDir"], false);
        assert_eq!(missing["size"], 0);
        assert_eq!(missing["modifiedAt"], Value::Null);

        let stat = files(&fixture, "stat", Some("notes/a.txt"), None).unwrap();
        assert_eq!(stat["exists"], true);
        assert_eq!(stat["size"], 5);

        assert_eq!(
            files(&fixture, "delete", Some("notes/a.txt"), None).unwrap(),
            json!({"deleted": true})
        );
        assert_eq!(
            files(&fixture, "delete", Some("notes/a.txt"), None).unwrap(),
            json!({"deleted": false})
        );
        assert_eq!(
            files(&fixture, "delete", Some("notes"), None).unwrap(),
            json!({"deleted": true})
        );
        assert_eq!(
            files(&fixture, "list", Some("notes"), None).unwrap(),
            json!([])
        );
        assert_eq!(
            files(&fixture, "list", None, None)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn files_reject_traversal_absolute_and_reserved_paths() {
        let fixture = fixture(&[FILES_PERMISSION], true);
        for path in [
            "../escape.txt",
            "a/../../escape.txt",
            "/etc/passwd",
            "\\windows\\system32",
            "C:/Windows/win.ini",
            "a//b.txt",
            "a/./b.txt",
            "CON",
            "dir/NUL.txt",
            "com1",
            // NTFS alternate data streams.
            "notes.txt:hidden",
            "dir/file:stream",
        ] {
            let error = files(&fixture, "writeText", Some(path), Some("x")).unwrap_err();
            assert!(
                error.contains("not allowed")
                    || error.contains("inside")
                    || error.contains("segments")
                    || error.contains("reserved"),
                "{path}: {error}"
            );
        }

        let deep = (0..17)
            .map(|index| format!("d{index}"))
            .collect::<Vec<_>>()
            .join("/");
        assert!(files(&fixture, "writeText", Some(&deep), Some("x")).is_err());
        assert!(files(&fixture, "delete", Some(""), None).is_err());

        // A rejected write never creates a file.
        let files_dir = fixture.paths.plugin_files_dir("page-demo");
        let entries = fs::read_dir(&files_dir).unwrap().count();
        assert_eq!(entries, 0);
    }

    #[test]
    fn files_enforce_single_file_and_total_quotas() {
        let fixture = fixture(&[FILES_PERMISSION], true);
        assert!(check_file_quota(0, 0, FILE_LIMIT).is_ok());
        assert!(check_file_quota(0, 0, FILE_LIMIT + 1).is_err());
        assert!(check_file_quota(FILES_TOTAL_LIMIT - 1, 0, 1).is_ok());
        assert!(check_file_quota(FILES_TOTAL_LIMIT, 0, 1).is_err());
        // Replacing a file counts only the new bytes toward the total.
        assert!(check_file_quota(FILES_TOTAL_LIMIT, FILE_LIMIT, 1).is_ok());

        let oversized = "x".repeat(FILE_LIMIT as usize + 1);
        let error = files(&fixture, "writeText", Some("big.txt"), Some(&oversized)).unwrap_err();
        assert!(error.contains("20 MB"), "{error}");
        assert!(!fixture
            .paths
            .plugin_files_dir("page-demo")
            .join("big.txt")
            .exists());
    }

    #[test]
    fn purge_plugin_data_removes_the_whole_tree() {
        let fixture = fixture(&[KV_PERMISSION, FILES_PERMISSION], true);
        kv(&fixture, "set", Some("a"), Some(json!(1)), None).unwrap();
        files(&fixture, "writeText", Some("a/b.txt"), Some("x")).unwrap();

        let data = fixture.paths.plugin_data_dir("page-demo");
        assert!(data.is_dir());
        purge_plugin_data(&fixture.paths, "page-demo").unwrap();
        assert!(!data.exists());
        // Missing data is fine.
        purge_plugin_data(&fixture.paths, "page-demo").unwrap();
    }
}
