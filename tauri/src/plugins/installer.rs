//! Plugin source parsing, package inspection and local/Bun installation.
//!
//! A plugin is an npm package (installed with `bun add` into
//! `plugins/packages`), a `github:owner/repo` repository, or a local directory
//! / single `*.middleware.js` file. Local paths are referenced in place; the
//! user's files are never copied or deleted.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::coding::cli_resolver::apply_create_no_window_tokio;
use crate::db::SqliteDbState;

use super::bun_runtime::BunRuntime;
use super::market;
use super::paths::PluginPaths;
use super::store;
use super::types::{empty_json_object, InstalledPluginRecord, PluginKind, PluginPageManifest};

/// Single-file middleware entries Magpie accepts, in match order.
const MIDDLEWARE_FILE_SUFFIXES: [&str; 4] = [
    ".middleware.js",
    ".middleware.mjs",
    ".gateway.js",
    ".gateway.mjs",
];
const BUN_ADD_TIMEOUT: Duration = Duration::from_secs(300);

fn middleware_suffix(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_ascii_lowercase();
    MIDDLEWARE_FILE_SUFFIXES
        .iter()
        .copied()
        .find(|suffix| lower.ends_with(suffix))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginSource {
    Npm {
        spec: String,
        package_name: String,
    },
    Github {
        spec: String,
        owner: String,
        repo: String,
    },
    Local {
        spec: String,
        path: PathBuf,
    },
}

impl PluginSource {
    pub fn spec(&self) -> &str {
        match self {
            Self::Npm { spec, .. } | Self::Github { spec, .. } | Self::Local { spec, .. } => spec,
        }
    }

    pub fn source_type(&self) -> &'static str {
        match self {
            Self::Npm { .. } => "npm",
            Self::Github { .. } => "github",
            Self::Local { .. } => "local",
        }
    }

    pub fn package_name(&self) -> Option<&str> {
        match self {
            Self::Npm { package_name, .. } => Some(package_name),
            _ => None,
        }
    }
}

/// Resolve what the user typed into a source. Local paths win over npm names so
/// `./plugin.middleware.js` is never mistaken for a package.
pub fn parse_source(spec: &str) -> Result<PluginSource, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("A plugin source is required".to_string());
    }

    if let Some(rest) = spec.strip_prefix("github:") {
        let (owner, repo) = parse_github_spec(rest)?;
        return Ok(PluginSource::Github {
            spec: spec.to_string(),
            owner,
            repo,
        });
    }

    if looks_like_local_path(spec) {
        let path = resolve_local_path(spec)?;
        return Ok(PluginSource::Local {
            spec: spec.to_string(),
            path,
        });
    }

    let (package_name, _version) = split_npm_spec(spec);
    if market::is_valid_package_name(package_name) {
        return Ok(PluginSource::Npm {
            spec: spec.to_string(),
            package_name: package_name.to_string(),
        });
    }

    Err(format!(
        "'{spec}' is not an npm package name, github:owner/repo, or a local plugin path"
    ))
}

fn parse_github_spec(rest: &str) -> Result<(String, String), String> {
    let rest = rest.split('#').next().unwrap_or("").trim_end_matches('/');
    let (owner, repo) = rest
        .split_once('/')
        .ok_or_else(|| "github: sources must be github:owner/repo".to_string())?;
    let valid = |value: &str| {
        !value.is_empty()
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    };
    if !valid(owner) || !valid(repo) {
        return Err("github: sources must be github:owner/repo".to_string());
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn looks_like_local_path(spec: &str) -> bool {
    spec.starts_with("file://")
        || spec.starts_with('.')
        || spec.starts_with('/')
        || spec.starts_with('\\')
        || middleware_suffix(spec).is_some()
        || is_drive_path(spec)
        || Path::new(spec).is_absolute()
}

fn is_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn resolve_local_path(spec: &str) -> Result<PathBuf, String> {
    let raw = spec.strip_prefix("file://").unwrap_or(spec);
    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(format!(
            "Local plugin path '{}' does not exist",
            path.display()
        ));
    }
    fs::canonicalize(&path).map_err(|error| {
        format!(
            "Failed to resolve local plugin path '{}': {error}",
            path.display()
        )
    })
}

/// Split `@scope/name@1.2.3` into the package name and optional version. The
/// leading `@` of a scope is never a version separator.
fn split_npm_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.rfind('@') {
        Some(index) if index > 0 => (&spec[..index], Some(&spec[index + 1..])),
        _ => (spec, None),
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PackageJson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    main: String,
    #[serde(default)]
    exports: Value,
    #[serde(default)]
    magpie: Value,
}

/// Everything installation and refresh need to know about one package on disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackageInspection {
    pub package_name: String,
    pub version: String,
    pub description: String,
    pub display_name: String,
    pub kind: PluginKind,
    pub middleware_entry: String,
    pub has_provider_entry: bool,
    pub providers: Vec<String>,
    pub pages: Vec<PluginPageManifest>,
    pub permissions: Vec<String>,
    pub options: Value,
    pub summary: BTreeMap<String, String>,
    pub icon: String,
}

impl PackageInspection {
    /// A package that only contributes pages: no provider entry and no page
    /// declaring a local service, so installing it must not start Bun.
    pub fn is_page_only(&self) -> bool {
        !self.has_provider_entry
            && !self.pages.is_empty()
            && self.pages.iter().all(|page| page.service_port.is_none())
    }
}

/// Read a package directory (or a single `*.middleware.js` file) and decide
/// what it contributes. A directory without a readable `package.json` is an
/// error; a package that names neither a provider nor a middleware entry is
/// reported as [`PluginKind::Unknown`] so the caller can refuse it.
pub fn inspect_installed_package(path: &Path) -> Result<PackageInspection, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Failed to read plugin path '{}': {error}", path.display()))?;

    if metadata.is_file() {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("Plugin file '{}' has no usable name", path.display()))?;
        let Some(suffix) = middleware_suffix(file_name) else {
            return Err(format!(
                "Single-file plugins must be one of {}: {}",
                MIDDLEWARE_FILE_SUFFIXES.join(", "),
                path.display()
            ));
        };
        let package_name = file_name[..file_name.len() - suffix.len()].to_string();
        return Ok(PackageInspection {
            package_name: package_name.clone(),
            display_name: package_name,
            kind: PluginKind::Middleware,
            middleware_entry: file_name.to_string(),
            options: empty_json_object(),
            ..PackageInspection::default()
        });
    }

    let package_json_path = path.join("package.json");
    let bytes = fs::read(&package_json_path)
        .map_err(|error| format!("Failed to read '{}': {error}", package_json_path.display()))?;
    let package: PackageJson = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Invalid package.json at '{}': {error}",
            package_json_path.display()
        )
    })?;

    let magpie = package.magpie;
    let middleware_marker = magpie_entry(&magpie, "middleware");
    let gateway_marker = magpie_entry(&magpie, "gateway");
    let middleware_file = find_middleware_file(path);
    let has_middleware =
        middleware_marker.is_some() || gateway_marker.is_some() || middleware_file.is_some();
    let has_provider_entry = provider_entry_present(&package.main, &package.exports);
    // Magpie only attaches a page service when the package has a host part:
    // a page-only package must never expose an arbitrary local port.
    let pages = parse_pages(path, &magpie, has_provider_entry)?;
    let permissions = dedupe_strings(magpie_string_array(&magpie, "permissions"));

    let kind = match (has_provider_entry, has_middleware) {
        (true, true) => PluginKind::Both,
        (true, false) => PluginKind::Provider,
        (false, true) => PluginKind::Middleware,
        (false, false) if !pages.is_empty() => PluginKind::Page,
        (false, false) => PluginKind::Unknown,
    };

    let middleware_entry = [middleware_marker, gateway_marker, middleware_file]
        .into_iter()
        .flatten()
        .find(|entry| !entry.is_empty())
        .unwrap_or_default();

    let package_name = if package.name.trim().is_empty() {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        package.name.trim().to_string()
    };
    let display_name = magpie_string(&magpie, "name")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| package_name.clone());

    Ok(PackageInspection {
        package_name,
        version: package.version.trim().to_string(),
        description: package.description.trim().to_string(),
        display_name,
        kind,
        middleware_entry,
        has_provider_entry,
        providers: magpie_string_array(&magpie, "providers"),
        pages,
        permissions,
        options: magpie
            .get("options")
            .filter(|options| options.is_object())
            .cloned()
            .unwrap_or_else(empty_json_object),
        summary: magpie_summary(&magpie),
        icon: magpie_string(&magpie, "icon").unwrap_or_default(),
    })
}

fn provider_entry_present(main: &str, exports: &Value) -> bool {
    if !main.trim().is_empty() {
        return true;
    }
    match exports {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty() && value != "null",
        _ => true,
    }
}

/// A `magpie.<key>` marker. A string is the entry file, an object may name one
/// in `entry` / `path`, and `true` only marks the package (the file scan finds
/// the entry).
fn magpie_entry(magpie: &Value, key: &str) -> Option<String> {
    match magpie.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Some(Value::Object(object)) => object
            .get("entry")
            .or_else(|| object.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| Some(String::new())),
        Some(Value::Bool(true)) => Some(String::new()),
        _ => None,
    }
}

fn find_middleware_file(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| middleware_suffix(name).is_some())
        .collect();
    names.sort();
    names.into_iter().next()
}

fn magpie_string(magpie: &Value, key: &str) -> Option<String> {
    magpie
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn magpie_string_array(magpie: &Value, key: &str) -> Vec<String> {
    magpie
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn magpie_summary(magpie: &Value) -> BTreeMap<String, String> {
    magpie
        .get("summary")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(language, value)| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| (language.clone(), value.to_string()))
        })
        .collect()
}

/// Every page a package declares.
///
/// `magpie.pages` wins over `magpie.page` when both are present, matching
/// Magpie. A declaration that names an unsafe or missing entry is an error:
/// a package must never make the page server serve files it does not own.
fn parse_pages(
    root: &Path,
    magpie: &Value,
    has_provider_entry: bool,
) -> Result<Vec<PluginPageManifest>, String> {
    let mut pages = Vec::new();
    let mut used_ids = HashSet::new();

    if let Some(Value::Array(entries)) = magpie.get("pages") {
        if !entries.is_empty() {
            for (index, entry) in entries.iter().enumerate() {
                let Some(mut page) = parse_page(root, entry, index, has_provider_entry)? else {
                    continue;
                };
                if page.id.trim().is_empty() {
                    page.id = slug_for_page(&page.title, &page.entry_file);
                }
                page.id = unique_page_id(&page.id, &mut used_ids);
                pages.push(page);
            }
            return Ok(pages);
        }
    }

    if let Some(page @ Value::Object(_)) = magpie.get("page") {
        let Some(mut page) = parse_page(root, page, 0, has_provider_entry)? else {
            return Ok(pages);
        };
        if page.id.trim().is_empty() {
            page.id = "default".to_string();
        }
        page.id = unique_page_id(&page.id, &mut used_ids);
        pages.push(page);
    }

    Ok(pages)
}

/// One `magpie.page` object or one entry of `magpie.pages`.
fn parse_page(
    root: &Path,
    raw: &Value,
    index: usize,
    has_provider_entry: bool,
) -> Result<Option<PluginPageManifest>, String> {
    let Some(object) = raw.as_object() else {
        // A non-object among `magpie.pages` is not a page, not a broken package.
        return Ok(None);
    };

    let entry_file = object
        .get("file")
        .or_else(|| object.get("entry"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("plugin page #{} does not name its HTML file", index + 1))?;
    let entry_file = normalize_relative_path(entry_file);
    resolve_page_entry(root, &entry_file)
        .map_err(|error| format!("plugin page '{}': {error}", entry_file))?;

    let icon_file = object
        .get("icon")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_relative_path)
        .map(|icon| {
            resolve_page_icon(root, &icon)
                .map_err(|error| format!("plugin page icon '{icon}': {error}"))?;
            Ok::<String, String>(icon)
        })
        .transpose()?;

    let title = object
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    // An entry of `magpie.pages` without an id gets a slug of its title; the
    // caller turns an empty id into `default` for a single `magpie.page`.
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .map(sanitize_page_id)
        .unwrap_or_default();

    let (service_port, service_option) = if has_provider_entry {
        parse_page_service(object.get("service"))
    } else {
        (None, None)
    };

    Ok(Some(PluginPageManifest {
        id,
        title,
        titles: page_titles(object.get("titles")),
        entry_file,
        icon_file,
        keep_alive: object
            .get("keepAlive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        service_port,
        service_option,
    }))
}

fn page_titles(raw: Option<&Value>) -> BTreeMap<String, String> {
    let mut titles = BTreeMap::new();
    let Some(object) = raw.and_then(Value::as_object) else {
        return titles;
    };
    for (language, value) in object {
        let language = language.trim();
        let value = value.as_str().map(str::trim).unwrap_or_default();
        if language.is_empty() || value.is_empty() || language.chars().count() > 16 {
            continue;
        }
        if titles.len() == 16 {
            break;
        }
        titles.insert(language.to_string(), value.to_string());
    }
    titles
}

/// `magpie.page.service` is `{port, option}`. A service without a valid port
/// is no service, matching Magpie.
fn parse_page_service(raw: Option<&Value>) -> (Option<u16>, Option<String>) {
    let Some(object) = raw.and_then(Value::as_object) else {
        return (None, None);
    };
    let Some(port) = object.get("port").and_then(parse_tcp_port) else {
        return (None, None);
    };
    let option = object
        .get("option")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= 64)
        .map(str::to_string);
    (Some(port), option)
}

fn parse_tcp_port(value: &Value) -> Option<u16> {
    let port = match value {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => text.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    u16::try_from(port).ok().filter(|port| *port > 0)
}

fn normalize_relative_path(value: &str) -> String {
    let trimmed = value.trim().replace('\\', "/");
    let mut normalized = trimmed.as_str();
    while let Some(rest) = normalized.strip_prefix("./") {
        normalized = rest;
    }
    normalized.to_string()
}

/// Resolve a page entry to a canonical file inside `plugin_root`, rejecting
/// absolute paths, `..`, backslashes and links that leave the package.
pub fn resolve_page_entry(plugin_root: &Path, entry_file: &str) -> Result<PathBuf, String> {
    let entry_file = normalize_relative_path(entry_file);
    let lower = entry_file.to_ascii_lowercase();
    if !(lower.ends_with(".html") || lower.ends_with(".htm")) {
        return Err("its entry must be an .html or .htm file".to_string());
    }
    let resolved = resolve_inside_plugin(plugin_root, &entry_file)?;
    let metadata = fs::metadata(&resolved)
        .map_err(|error| format!("Failed to read '{}': {error}", resolved.display()))?;
    if !metadata.is_file() {
        return Err(format!("'{}' is not a file", resolved.display()));
    }
    Ok(resolved)
}

fn resolve_page_icon(plugin_root: &Path, icon_file: &str) -> Result<(), String> {
    let resolved = resolve_inside_plugin(plugin_root, icon_file)?;
    let metadata = fs::metadata(&resolved)
        .map_err(|error| format!("Failed to read '{}': {error}", resolved.display()))?;
    if !metadata.is_file() {
        return Err(format!("'{}' is not a file", resolved.display()));
    }
    Ok(())
}

/// `rel` joined to `root`, only when the canonical result stays inside the
/// canonical root. Both sides are canonicalized so a symlink cannot escape.
fn resolve_inside_plugin(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = normalize_relative_path(rel);
    if rel.is_empty() {
        return Err("its path is empty".to_string());
    }
    if rel.starts_with('/') || is_drive_path(&rel) {
        return Err("absolute paths are not allowed".to_string());
    }
    if rel.contains('\\') || rel.contains('\0') {
        return Err("backslashes and NUL bytes are not allowed".to_string());
    }
    let relative = Path::new(&rel);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("the path must stay inside the plugin folder".to_string());
    }

    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("Failed to resolve '{}': {error}", root.display()))?;
    let candidate = canonical_root.join(relative);
    let resolved = fs::canonicalize(&candidate)
        .map_err(|error| format!("Failed to resolve '{}': {error}", candidate.display()))?;
    if !resolved.starts_with(&canonical_root) {
        return Err("the path resolves outside the plugin folder".to_string());
    }
    Ok(resolved)
}

fn sanitize_page_id(value: &str) -> String {
    let mut id = String::new();
    for character in value.trim().chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() || character == '-' {
            id.push(character);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    id.trim_matches('-').chars().take(64).collect()
}

fn slug_for_page(title: &str, entry_file: &str) -> String {
    let from_title = sanitize_page_id(title);
    if !from_title.is_empty() {
        return from_title;
    }
    let stem = Path::new(entry_file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("page");
    let slug = sanitize_page_id(stem);
    if slug.is_empty() {
        "page".to_string()
    } else {
        slug
    }
}

fn unique_page_id(base: &str, used: &mut HashSet<String>) -> String {
    let base = if base.trim().is_empty() {
        "default"
    } else {
        base
    };
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

/// Cut whatever follows the `package.json` object off.
///
/// Bun on Windows can write a shorter `package.json` over a longer one without
/// truncating it, leaving old bytes after the closing brace; every following
/// `bun add` then fails to parse the file. The object is kept as written and the
/// original file is preserved as `package.json.broken`. Returns whether a
/// repair happened.
pub fn mend_package_json(packages_dir: &Path) -> Result<bool, String> {
    let package_json_path = packages_dir.join("package.json");
    let bytes = match fs::read(&package_json_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Failed to read '{}': {error}",
                package_json_path.display()
            ))
        }
    };
    if serde_json::from_slice::<Value>(&bytes).is_ok() {
        return Ok(false);
    }

    let mut stream = serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>();
    let Some(Ok(value)) = stream.next() else {
        return Ok(false);
    };
    if !value.is_object() {
        return Ok(false);
    }

    let mut end = stream.byte_offset();
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    let mut head = bytes[..end].to_vec();
    head.push(b'\n');
    if serde_json::from_slice::<Value>(&head).is_err() {
        return Ok(false);
    }

    fs::write(packages_dir.join("package.json.broken"), &bytes)
        .map_err(|error| format!("Failed to back up the broken package.json: {error}"))?;
    write_atomic(&package_json_path, &head)?;
    Ok(true)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Failed to write '{}': {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("Failed to replace '{}': {error}", path.display()))
}

/// Create the Bun workspace's `package.json` if it does not exist yet.
pub fn ensure_packages_dir(paths: &PluginPaths) -> Result<(), String> {
    let packages_dir = paths.packages_dir();
    fs::create_dir_all(&packages_dir)
        .map_err(|error| format!("Failed to create '{}': {error}", packages_dir.display()))?;
    let package_json = packages_dir.join("package.json");
    if !package_json.exists() {
        write_atomic(
            &package_json,
            b"{\n  \"name\": \"ai-toolbox-plugins\",\n  \"private\": true\n}\n",
        )?;
    }
    Ok(())
}

/// Install an npm or GitHub package into the plugin workspace with `bun add`.
/// The runtime is downloaded on first use, so a fresh installation does not
/// need Bun on PATH.
pub async fn install_package_with_bun(
    runtime: &BunRuntime,
    paths: &PluginPaths,
    spec: &str,
) -> Result<(), String> {
    let bun = runtime.ensure().await?;
    ensure_packages_dir(paths)?;
    let packages_dir = paths.packages_dir();
    mend_package_json(&packages_dir)?;

    let mut command = tokio::process::Command::new(&bun.path);
    command
        .current_dir(&packages_dir)
        .args(["add", "--ignore-scripts", spec])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_remove("BUN_OPTIONS")
        .env_remove("NODE_OPTIONS")
        .env("DO_NOT_TRACK", "1");
    apply_create_no_window_tokio(&mut command);

    let output = match tokio::time::timeout(BUN_ADD_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("Failed to run '{} add {spec}': {error}", bun.path)),
        Err(_) => {
            return Err(format!(
                "bun add {spec} timed out after {} seconds",
                BUN_ADD_TIMEOUT.as_secs()
            ))
        }
    };
    mend_package_json(&packages_dir)?;
    if !output.status.success() {
        return Err(format!(
            "bun add {spec} failed: {}",
            summarize_command_output(&output.stdout, &output.stderr)
        ));
    }
    Ok(())
}

fn summarize_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut combined = String::from_utf8_lossy(stdout).trim().to_string();
    let errors = String::from_utf8_lossy(stderr).trim().to_string();
    if !errors.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&errors);
    }
    let mut summary: String = combined
        .chars()
        .rev()
        .take(2000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if summary.is_empty() {
        summary = "bun exited without output".to_string();
    }
    summary
}

fn installed_package_name_for_spec(packages_dir: &Path, spec: &str) -> Result<String, String> {
    let package_json_path = packages_dir.join("package.json");
    let bytes = fs::read(&package_json_path).map_err(|error| {
        format!(
            "Failed to read '{}' after bun add: {error}",
            package_json_path.display()
        )
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Invalid package.json at '{}' after bun add: {error}",
            package_json_path.display()
        )
    })?;
    if let Some(dependencies) = value.get("dependencies").and_then(Value::as_object) {
        for (name, dependency_spec) in dependencies {
            if dependency_spec.as_str() == Some(spec) {
                return Ok(name.clone());
            }
        }
    }
    Err(format!(
        "bun installed {spec} but plugins/packages/package.json does not list it"
    ))
}

/// Where an installed record's package lives on disk: the user's own path for
/// a local plugin, the Bun workspace for npm/GitHub packages.
pub fn plugin_target_path(paths: &PluginPaths, record: &InstalledPluginRecord) -> PathBuf {
    if record.source_type == "local" {
        PathBuf::from(&record.local_path)
    } else {
        paths
            .packages_dir()
            .join("node_modules")
            .join(&record.package_name)
    }
}

fn target_path(paths: &PluginPaths, record: &InstalledPluginRecord) -> PathBuf {
    plugin_target_path(paths, record)
}

fn record_from_inspection(
    id: String,
    spec: String,
    source_type: &str,
    market_id: Option<String>,
    local_path: String,
    inspection: PackageInspection,
) -> InstalledPluginRecord {
    InstalledPluginRecord {
        id,
        spec,
        package_name: inspection.package_name,
        display_name: inspection.display_name,
        version: inspection.version,
        kind: inspection.kind,
        enabled: true,
        source_type: source_type.to_string(),
        market_id,
        local_path,
        middleware_entry: inspection.middleware_entry,
        has_provider_entry: inspection.has_provider_entry,
        providers: inspection.providers,
        pages: inspection.pages,
        permissions: inspection.permissions,
        options: inspection.options,
        summary: inspection.summary,
        icon: inspection.icon,
        ..InstalledPluginRecord::default()
    }
}

/// Install one plugin and persist its record. Reinstalling the same package
/// replaces the record in place, preserving `enabled` and user options.
pub async fn install_plugin(
    db: &SqliteDbState,
    paths: &PluginPaths,
    runtime: &BunRuntime,
    spec: &str,
    market_id: Option<String>,
) -> Result<InstalledPluginRecord, String> {
    let source = parse_source(spec)?;
    let market_id = market_id.filter(|value| !value.trim().is_empty());

    let (id, inspection, source_type, local_path) = match &source {
        PluginSource::Local { path, .. } => {
            let inspection = inspect_installed_package(path)?;
            if inspection.kind == PluginKind::Unknown {
                return Err(format!(
                    "{} is not a plugin: its package.json names no main/exports and it has no middleware file",
                    path.display()
                ));
            }
            (
                path.to_string_lossy().to_string(),
                inspection,
                "local",
                path.to_string_lossy().to_string(),
            )
        }
        PluginSource::Npm { .. } | PluginSource::Github { .. } => {
            install_package_with_bun(runtime, paths, spec).await?;
            let package_name = match &source {
                PluginSource::Npm { package_name, .. } => package_name.clone(),
                _ => installed_package_name_for_spec(&paths.packages_dir(), spec)?,
            };
            let target = paths
                .packages_dir()
                .join("node_modules")
                .join(&package_name);
            let inspection = inspect_installed_package(&target)?;
            if inspection.kind == PluginKind::Unknown {
                return Err(format!(
                    "{package_name} is not a plugin: its package.json names no main/exports and it has no middleware file"
                ));
            }
            (
                package_name,
                inspection,
                source.source_type(),
                String::new(),
            )
        }
    };

    let mut record = record_from_inspection(
        id,
        source.spec().to_string(),
        source_type,
        market_id,
        local_path,
        inspection,
    );

    // Keep user state across reinstalls.
    if let Some(existing) = store::get_installed_plugin(db, &record.id)? {
        record.enabled = existing.enabled;
        if record.options == empty_json_object() {
            record.options = existing.options;
        }
        record.installed_at = existing.installed_at;
    }

    store::upsert_installed_plugin(db, &record)
}

/// Re-read every installed package and persist metadata changes. A package that
/// is missing or broken keeps its last-known fields and gains `lastError`.
pub fn refresh_installed_plugins(
    db: &SqliteDbState,
    paths: &PluginPaths,
) -> Result<Vec<InstalledPluginRecord>, String> {
    let records = store::list_installed_plugins(db)?;
    let mut refreshed = Vec::with_capacity(records.len());
    for record in records {
        let target = target_path(paths, &record);
        let updated = match inspect_installed_package(&target) {
            Ok(inspection) => InstalledPluginRecord {
                package_name: inspection.package_name,
                display_name: inspection.display_name,
                version: inspection.version,
                kind: inspection.kind,
                middleware_entry: inspection.middleware_entry,
                has_provider_entry: inspection.has_provider_entry,
                providers: inspection.providers,
                pages: inspection.pages,
                permissions: inspection.permissions,
                summary: inspection.summary,
                icon: inspection.icon,
                last_error: None,
                ..record.clone()
            },
            Err(error) => InstalledPluginRecord {
                last_error: Some(error),
                ..record.clone()
            },
        };
        if updated == record {
            refreshed.push(record);
        } else {
            refreshed.push(store::upsert_installed_plugin(db, &updated)?);
        }
    }
    Ok(refreshed)
}

/// Remove a plugin. Local records are only unlinked; npm/GitHub packages are
/// removed from the Bun workspace when Bun is available, with the installed
/// directory cleaned up regardless. Uninstalling never downloads Bun.
pub async fn uninstall_plugin(
    db: &SqliteDbState,
    paths: &PluginPaths,
    runtime: &BunRuntime,
    id: &str,
) -> Result<(), String> {
    let Some(record) = store::get_installed_plugin(db, id)? else {
        return Err(format!("Plugin '{id}' is not installed"));
    };

    if record.source_type != "local" && !record.package_name.is_empty() {
        if let Some(bun) = runtime.probe().await {
            let packages_dir = paths.packages_dir();
            let mut command = tokio::process::Command::new(&bun.path);
            command
                .current_dir(&packages_dir)
                .args(["remove", &record.package_name])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .env_remove("BUN_OPTIONS")
                .env_remove("NODE_OPTIONS")
                .env("DO_NOT_TRACK", "1");
            apply_create_no_window_tokio(&mut command);
            let _ = tokio::time::timeout(Duration::from_secs(60), command.output()).await;
        }
        let target = paths
            .packages_dir()
            .join("node_modules")
            .join(&record.package_name);
        if target.starts_with(paths.packages_dir()) && target.is_dir() {
            let _ = fs::remove_dir_all(target);
        }
    }

    store::delete_installed_plugin(db, id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_package(dir: &Path, package_json: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("package.json"), package_json).unwrap();
    }

    #[test]
    fn recognizes_provider_middleware_and_both_packages() {
        let root = tempfile::tempdir().unwrap();

        let provider = root.path().join("provider");
        write_package(
            &provider,
            r#"{"name":"provider-demo","version":"1.2.3","description":"A provider","main":"index.js","magpie":{"icon":"demo","providers":["demo"]}}"#,
        );
        let inspection = inspect_installed_package(&provider).unwrap();
        assert_eq!(inspection.kind, PluginKind::Provider);
        assert_eq!(inspection.package_name, "provider-demo");
        assert_eq!(inspection.version, "1.2.3");
        assert!(inspection.has_provider_entry);
        assert_eq!(inspection.providers, vec!["demo"]);

        let middleware = root.path().join("middleware");
        write_package(
            &middleware,
            r#"{"name":"middleware-demo","version":"1.0.0","magpie":{"middleware":"entry.middleware.js","options":{"mode":"on"}}}"#,
        );
        fs::write(middleware.join("entry.middleware.js"), "export default {}").unwrap();
        let inspection = inspect_installed_package(&middleware).unwrap();
        assert_eq!(inspection.kind, PluginKind::Middleware);
        assert_eq!(inspection.middleware_entry, "entry.middleware.js");
        assert_eq!(inspection.options["mode"], "on");

        let both = root.path().join("both");
        write_package(
            &both,
            r#"{"name":"both-demo","main":"index.js","magpie":{"gateway":true}}"#,
        );
        fs::write(both.join("both.middleware.js"), "export default {}").unwrap();
        let inspection = inspect_installed_package(&both).unwrap();
        assert_eq!(inspection.kind, PluginKind::Both);
        assert_eq!(inspection.middleware_entry, "both.middleware.js");

        let unknown = root.path().join("unknown");
        write_package(&unknown, r#"{"name":"utility","bin":{"tool":"cli.js"}}"#);
        assert_eq!(
            inspect_installed_package(&unknown).unwrap().kind,
            PluginKind::Unknown
        );
    }

    #[test]
    fn inspects_a_single_middleware_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("word-guard.middleware.js");
        fs::write(&path, "export default {}").unwrap();

        let inspection = inspect_installed_package(&path).unwrap();
        assert_eq!(inspection.kind, PluginKind::Middleware);
        assert_eq!(inspection.package_name, "word-guard");
        assert_eq!(inspection.display_name, "word-guard");
        assert_eq!(inspection.middleware_entry, "word-guard.middleware.js");
        assert!(!inspection.has_provider_entry);
    }

    #[test]
    fn rejects_single_files_that_are_not_middleware() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("plugin.js");
        fs::write(&path, "export default {}").unwrap();
        assert!(inspect_installed_package(&path).is_err());
    }

    #[test]
    fn mend_package_json_cuts_trailing_dirty_bytes_and_keeps_a_backup() {
        let root = tempfile::tempdir().unwrap();
        let packages = root.path();
        let dirty =
            b"{\n  \"name\": \"ai-toolbox-plugins\",\n  \"private\": true\n}\n  old-tail-garbage";
        fs::write(packages.join("package.json"), dirty).unwrap();

        assert!(mend_package_json(packages).unwrap());
        let repaired = fs::read_to_string(packages.join("package.json")).unwrap();
        serde_json::from_str::<Value>(&repaired).expect("repaired package.json must parse");
        assert!(repaired.ends_with('\n'));

        let backup = fs::read(packages.join("package.json.broken")).unwrap();
        assert_eq!(backup, dirty);

        // Valid files are untouched and already-clean files are a no-op.
        assert!(!mend_package_json(packages).unwrap());
    }

    #[test]
    fn mend_package_json_leaves_non_object_content_alone() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("package.json"), b"not json at all").unwrap();
        assert!(!mend_package_json(root.path()).unwrap());
        assert_eq!(
            fs::read_to_string(root.path().join("package.json")).unwrap(),
            "not json at all"
        );
        assert!(!root.path().join("package.json.broken").exists());
    }

    #[test]
    fn parses_npm_github_and_local_sources() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("demo.middleware.js");
        fs::write(&file, "export default {}").unwrap();

        assert_eq!(
            parse_source("@scope/demo@1.2.3").unwrap(),
            PluginSource::Npm {
                spec: "@scope/demo@1.2.3".to_string(),
                package_name: "@scope/demo".to_string(),
            }
        );
        assert_eq!(
            parse_source("plain-package").unwrap(),
            PluginSource::Npm {
                spec: "plain-package".to_string(),
                package_name: "plain-package".to_string(),
            }
        );
        assert_eq!(
            parse_source("github:owner/repo#main").unwrap(),
            PluginSource::Github {
                spec: "github:owner/repo#main".to_string(),
                owner: "owner".to_string(),
                repo: "repo".to_string(),
            }
        );
        let local = parse_source(file.to_str().unwrap()).unwrap();
        assert_eq!(local.source_type(), "local");
        assert_eq!(local.spec(), file.to_str().unwrap());

        assert!(parse_source("").is_err());
        assert!(parse_source("Not A Package").is_err());
        assert!(parse_source("github:only-owner").is_err());
    }

    fn write_page(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn recognizes_a_single_magpie_page() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("page-demo");
        write_package(
            &plugin,
            r#"{
                "name": "page-demo",
                "version": "2.0.0",
                "main": "index.js",
                "magpie": {
                    "name": "Page Demo",
                    "page": {
                        "title": "Daily",
                        "titles": { "zh": "每日", "ja": "" },
                        "file": "page/index.html",
                        "icon": "page/icon.svg",
                        "keepAlive": true,
                        "service": { "port": "3477", "option": "port" }
                    },
                    "permissions": ["usage:summary", "usage:day", "usage:summary"]
                }
            }"#,
        );
        write_page(&plugin, "page/index.html", "<html></html>");
        write_page(&plugin, "page/icon.svg", "<svg></svg>");

        let inspection = inspect_installed_package(&plugin).unwrap();
        // A provider entry plus a page: the package still runs in the host,
        // so its page service is honoured.
        assert_eq!(inspection.kind, PluginKind::Provider);
        assert!(inspection.has_provider_entry);
        assert_eq!(inspection.pages.len(), 1);
        let page = &inspection.pages[0];
        assert_eq!(page.id, "default");
        assert_eq!(page.title, "Daily");
        assert_eq!(page.titles.get("zh").map(String::as_str), Some("每日"));
        assert!(!page.titles.contains_key("ja"));
        assert_eq!(page.entry_file, "page/index.html");
        assert_eq!(page.icon_file.as_deref(), Some("page/icon.svg"));
        assert!(page.keep_alive);
        assert_eq!(page.service_port, Some(3477));
        assert_eq!(page.service_option.as_deref(), Some("port"));
        assert_eq!(inspection.permissions, vec!["usage:summary", "usage:day"]);
        // A declared service means the package has a host part to run.
        assert!(!inspection.is_page_only());
    }

    #[test]
    fn page_only_packages_need_no_bun() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("page-only");
        write_package(
            &plugin,
            r#"{"name":"page-only","version":"1.0.0","magpie":{"page":{"file":"index.html","id":"Hello World"}}}"#,
        );
        write_page(&plugin, "index.html", "<html></html>");

        let inspection = inspect_installed_package(&plugin).unwrap();
        assert_eq!(inspection.kind, PluginKind::Page);
        assert!(inspection.is_page_only());
        assert_eq!(inspection.pages[0].id, "hello-world");
    }

    #[test]
    fn page_only_packages_drop_a_declared_service() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("page-only-service");
        write_package(
            &plugin,
            r#"{"name":"page-only-service","version":"1.0.0","magpie":{"page":{"file":"index.html","service":{"port":3477,"option":"port"}}}}"#,
        );
        write_page(&plugin, "index.html", "<html></html>");

        let inspection = inspect_installed_package(&plugin).unwrap();
        assert_eq!(inspection.kind, PluginKind::Page);
        assert!(inspection.is_page_only());
        assert_eq!(inspection.pages[0].service_port, None);
        assert_eq!(inspection.pages[0].service_option, None);
    }

    #[test]
    fn single_file_gateway_suffixes_are_recognized() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "guard.gateway.js",
            "guard.gateway.mjs",
            "guard.middleware.mjs",
        ] {
            let path = root.path().join(name);
            fs::write(&path, "export default {}").unwrap();
            let inspection = inspect_installed_package(&path).unwrap();
            assert_eq!(inspection.kind, PluginKind::Middleware, "{name}");
            assert_eq!(inspection.middleware_entry, name);
            assert_eq!(inspection.package_name, "guard");
        }
    }

    #[test]
    fn magpie_pages_wins_and_ids_are_made_unique() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("multi");
        write_package(
            &plugin,
            r#"{
                "name": "multi",
                "magpie": {
                    "page": { "file": "single.html" },
                    "pages": [
                        { "id": "Stats", "title": "Stats", "file": "pages/stats.html" },
                        { "title": "Stats", "file": "pages/other.html" }
                    ]
                }
            }"#,
        );
        write_page(&plugin, "single.html", "<html></html>");
        write_page(&plugin, "pages/stats.html", "<html></html>");
        write_page(&plugin, "pages/other.html", "<html></html>");

        let inspection = inspect_installed_package(&plugin).unwrap();
        assert_eq!(inspection.kind, PluginKind::Page);
        assert_eq!(inspection.pages.len(), 2);
        assert_eq!(inspection.pages[0].id, "stats");
        assert_eq!(inspection.pages[1].id, "stats-2");
        assert_eq!(inspection.pages[1].entry_file, "pages/other.html");
    }

    #[test]
    fn rejects_page_entries_and_icons_outside_the_plugin_folder() {
        let root = tempfile::tempdir().unwrap();
        write_page(root.path(), "secret.html", "<html></html>");

        for entry in ["../secret.html", "page/../../secret.html", "C:/secret.html"] {
            let plugin = root.path().join("bad");
            write_package(
                &plugin,
                &format!(r#"{{"name":"bad","magpie":{{"page":{{"file":"{entry}"}}}}}}"#),
            );
            let error = inspect_installed_package(&plugin).unwrap_err();
            assert!(
                error.contains("outside") || error.contains("absolute") || error.contains("inside"),
                "{error}"
            );
        }

        let plugin = root.path().join("bad-icon");
        write_package(
            &plugin,
            r#"{"name":"bad-icon","magpie":{"page":{"file":"index.html","icon":"../secret.html"}}}"#,
        );
        write_page(&plugin, "index.html", "<html></html>");
        assert!(inspect_installed_package(&plugin).is_err());
    }

    #[test]
    fn rejects_page_entries_that_are_not_html_or_missing() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("bad-entry");
        write_package(
            &plugin,
            r#"{"name":"bad-entry","magpie":{"page":{"file":"page/index.js"}}}"#,
        );
        write_page(&plugin, "page/index.js", "console.log(1)");
        assert!(inspect_installed_package(&plugin).is_err());

        let plugin = root.path().join("missing-entry");
        write_package(
            &plugin,
            r#"{"name":"missing-entry","magpie":{"page":{"file":"page/index.html"}}}"#,
        );
        assert!(inspect_installed_package(&plugin).is_err());
    }

    #[test]
    fn refresh_rediscovers_pages_and_permissions() {
        use serde_json::json;

        let db = SqliteDbState::in_memory_for_test().unwrap();
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        let plugin = paths.root().join("local-page");
        write_package(
            &plugin,
            r#"{"name":"local-page","magpie":{"page":{"file":"index.html"},"permissions":["usage:summary"]}}"#,
        );
        write_page(&plugin, "index.html", "<html></html>");

        store::upsert_installed_plugin(
            &db,
            &InstalledPluginRecord {
                id: "local-page".to_string(),
                spec: plugin.to_string_lossy().to_string(),
                package_name: "local-page".to_string(),
                display_name: "Local Page".to_string(),
                source_type: "local".to_string(),
                local_path: plugin.to_string_lossy().to_string(),
                enabled: true,
                ..InstalledPluginRecord::default()
            },
        )
        .unwrap();

        let refreshed = refresh_installed_plugins(&db, &paths).unwrap();
        assert_eq!(refreshed[0].kind, PluginKind::Page);
        assert_eq!(refreshed[0].pages.len(), 1);
        assert_eq!(refreshed[0].permissions, vec!["usage:summary"]);

        write_package(
            &plugin,
            r#"{"name":"local-page","magpie":{"pages":[{"file":"index.html"},{"file":"other.html"}],"permissions":["usage:day"]}}"#,
        );
        write_page(&plugin, "other.html", "<html></html>");
        let refreshed = refresh_installed_plugins(&db, &paths).unwrap();
        assert_eq!(refreshed[0].pages.len(), 2);
        assert_eq!(refreshed[0].permissions, vec!["usage:day"]);

        // A broken package keeps its last known pages and gains lastError.
        write_package(
            &plugin,
            &json!({"name": "local-page", "magpie": {"page": {"file": "../escape.html"}}})
                .to_string(),
        );
        let refreshed = refresh_installed_plugins(&db, &paths).unwrap();
        assert_eq!(refreshed[0].pages.len(), 2);
        assert!(refreshed[0].last_error.is_some());
    }

    #[test]
    fn package_targets_are_the_bun_workspace_or_the_local_path() {
        let paths = PluginPaths::new(tempfile::tempdir().unwrap().path());
        let npm = InstalledPluginRecord {
            source_type: "npm".to_string(),
            package_name: "@scope/demo".to_string(),
            ..InstalledPluginRecord::default()
        };
        assert_eq!(
            plugin_target_path(&paths, &npm),
            paths
                .packages_dir()
                .join("node_modules")
                .join("@scope/demo")
        );

        let local = InstalledPluginRecord {
            source_type: "local".to_string(),
            local_path: "/tmp/demo".to_string(),
            ..InstalledPluginRecord::default()
        };
        assert_eq!(
            plugin_target_path(&paths, &local),
            PathBuf::from("/tmp/demo")
        );
    }
}
