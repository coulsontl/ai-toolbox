use chrono::Local;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::adapter;
use super::builtin_models::{builtin_model_ids, builtin_models_for, has_builtin_models};
use super::constants::{
    builtin_provider_name, is_builtin_provider, DSH_CREDENTIALS_FILE, DSH_CREDENTIALS_RECORDS_KEY,
    DSH_CREDENTIALS_REFS_KEY, DSH_CREDENTIALS_VERSION, DSH_CREDENTIALS_VERSION_KEY,
    DSH_CREDENTIAL_RECORD_SCOPE, DSH_DEFAULT_MODEL_SECTION, DSH_ENV_KEY, DSH_LLM_PI_AI_SECTION,
    DSH_PROMPT_FILE, DSH_PROVIDERS_KEY, DSH_SETTINGS_FILE,
};
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::open_code::shell_env;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::runtime_location;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::{Emitter, Runtime};

/// Top-level YAML keys managed by this module (or by dsh itself) and thus
/// hidden from / preserved across the "Other settings" editor. `llm-pi-ai`
/// providers and the `agent-default-model` section are runtime-owned.
const DSH_OTHER_SETTINGS_PROTECTED_KEYS: [&str; 2] =
    [DSH_LLM_PI_AI_SECTION, DSH_DEFAULT_MODEL_SECTION];

fn get_home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Failed to get home directory".to_string())
}

/// Platform-default dsh config directory: `~/.dsh` (Windows `%USERPROFILE%\.dsh`).
fn default_dsh_config_dir() -> Result<PathBuf, String> {
    Ok(get_home_dir()?.join(".dsh"))
}

fn get_dsh_config_dir_from_shell() -> Option<PathBuf> {
    shell_env::get_env_from_shell_config(DSH_ENV_KEY)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

/// Resolve the config dir without consulting the DB: env -> shell -> default.
fn resolve_dsh_config_dir_without_db() -> Result<PathBuf, String> {
    if let Ok(env_path) = std::env::var(DSH_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return Ok(PathBuf::from(env_path));
        }
    }
    if let Some(shell_path) = get_dsh_config_dir_from_shell() {
        return Ok(shell_path);
    }
    default_dsh_config_dir()
}

/// `(path, source)` resolution without DB. Source is one of
/// `env` / `shell` / `default`, mirroring `runtime_location`.
pub(crate) fn resolve_dsh_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var(DSH_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }
    if let Some(shell_path) = get_dsh_config_dir_from_shell() {
        return (shell_path, "shell".to_string());
    }
    (
        default_dsh_config_dir().unwrap_or_default(),
        "default".to_string(),
    )
}

/// Custom config dir stored in the DB (id fixed to "common").
///
/// The shared runtime-location cache calls this resolver so WSL Direct status
/// follows the same directory precedence as dsh's file operations.
pub async fn get_dsh_custom_config_dir_async(db: &SqliteDbState) -> Option<PathBuf> {
    db.with_conn(|conn| db_get(conn, DbTable::DshSettingsConfig, "common"))
        .ok()
        .flatten()
        .and_then(|value| adapter::settings_from_db_value(value).config_dir)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub async fn get_dsh_config_dir_from_db_async(
    db: &SqliteDbState,
) -> Result<(PathBuf, String), String> {
    if let Some(custom) = get_dsh_custom_config_dir_async(db).await {
        return Ok((custom, "custom".to_string()));
    }
    Ok(resolve_dsh_path_without_db())
}

fn get_dsh_config_dir_from_db_sync(db: &SqliteDbState) -> Result<(PathBuf, String), String> {
    let custom = db
        .with_conn(|conn| db_get(conn, DbTable::DshSettingsConfig, "common"))
        .ok()
        .flatten()
        .and_then(|value| adapter::settings_from_db_value(value).config_dir)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    if let Some(custom) = custom {
        return Ok((custom, "custom".to_string()));
    }
    Ok(resolve_dsh_path_without_db())
}

pub fn get_dsh_root_path_info_from_db(db: &SqliteDbState) -> Result<DshPathInfo, String> {
    let (path, source) = get_dsh_config_dir_from_db_sync(db)?;
    Ok(DshPathInfo {
        path: path.to_string_lossy().to_string(),
        source,
    })
}

pub async fn get_dsh_root_path_info_from_db_async(
    db: &SqliteDbState,
) -> Result<DshPathInfo, String> {
    let (path, source) = get_dsh_config_dir_from_db_async(db).await?;
    Ok(DshPathInfo {
        path: path.to_string_lossy().to_string(),
        source,
    })
}

pub async fn get_dsh_root_dir_from_db_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_dsh_config_dir_from_db_async(db).await?.0)
}

pub fn get_dsh_config_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(DSH_SETTINGS_FILE)
}

pub fn get_dsh_credentials_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(DSH_CREDENTIALS_FILE)
}

pub fn get_dsh_prompt_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(DSH_PROMPT_FILE)
}

pub async fn get_dsh_config_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_dsh_config_path_from_root(
        &get_dsh_root_dir_from_db_async(db).await?,
    ))
}

pub async fn get_dsh_credentials_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_dsh_credentials_path_from_root(
        &get_dsh_root_dir_from_db_async(db).await?,
    ))
}

pub async fn get_dsh_prompt_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_dsh_prompt_path_from_root(
        &get_dsh_root_dir_from_db_async(db).await?,
    ))
}

// ---------------------------------------------------------------------------
// YAML I/O
// ---------------------------------------------------------------------------

fn read_yaml_object_or_empty(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let parsed: Value = serde_json::to_value(yaml)
        .map_err(|error| format!("Failed to convert {}: {error}", path.display()))?;
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(format!("{} must contain a YAML mapping", path.display()))
    }
}

fn write_yaml_object(path: &Path, value: &Value) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "{} must be written as a YAML mapping",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let content = serde_yaml::to_string(value)
        .map_err(|error| format!("Failed to serialize {}: {error}", path.display()))?;
    fs::write(path, content)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    Ok(())
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, String> {
    value
        .as_object_mut()
        .ok_or_else(|| "Expected a YAML mapping object".to_string())
}

// ---------------------------------------------------------------------------
// Credentials I/O
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn set_credentials_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn set_credentials_file_permissions(_path: &Path) {}

/// Read `.credentials.yaml` as a `ref_name -> value` map. Missing/empty files
/// yield an empty map.
pub fn read_credentials_map(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let yaml: serde_yaml::Value = if content.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(&content)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?
    };
    let parsed: Value = serde_json::to_value(yaml)
        .map_err(|error| format!("Failed to convert {}: {error}", path.display()))?;
    Ok(parsed.as_object().cloned().unwrap_or_default())
}

fn write_credentials_map(path: &Path, map: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let content = serde_yaml::to_string(&Value::Object(map.clone()))
        .map_err(|error| format!("Failed to serialize {}: {error}", path.display()))?;
    fs::write(path, content)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    set_credentials_file_permissions(path);
    Ok(())
}

/// Parsed `.credentials.yaml`.
///
/// dsh stores the document in a versioned layout — a `version: 1` marker with
/// the refs nested under `refs:` and sign-in credentials under `records:` —
/// and migrates older documents at boot (dsh >= 0.1.1-rc.1). This app always
/// persists the versioned layout too; only ref entries are touched, while
/// `records:` belongs to dsh's login flow and is carried through untouched.
struct CredentialsDocument {
    /// Ref entries: POSIX env-var-style names over secret values.
    refs: Map<String, Value>,
    /// Stored sign-in records keyed `<scope>/<provider_id>`.
    records: Map<String, Value>,
    /// The full document as parsed; rewritten verbatim on save/delete.
    document: Map<String, Value>,
}

impl CredentialsDocument {
    /// Read the document at `path`; missing files yield an empty store.
    fn read(path: &Path) -> Result<Self, String> {
        Ok(Self::from_document(read_credentials_map(path)?))
    }

    fn from_document(document: Map<String, Value>) -> Self {
        let versioned = document
            .get(DSH_CREDENTIALS_VERSION_KEY)
            .and_then(Value::as_i64)
            .is_some_and(|version| version == DSH_CREDENTIALS_VERSION);
        let section = |key: &str| {
            document
                .get(key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
        };
        Self {
            records: if versioned {
                section(DSH_CREDENTIALS_RECORDS_KEY)
            } else {
                Map::new()
            },
            // Versioned layout reads the nested refs; a not-yet-migrated flat
            // document keeps its top-level entries visible until the next
            // write adopts the versioned layout.
            refs: if versioned {
                section(DSH_CREDENTIALS_REFS_KEY)
            } else {
                document.clone()
            },
            document,
        }
    }

    /// Move a pre-release flat document into the versioned layout before the
    /// first write, mirroring dsh's boot migration: every existing top-level
    /// entry nests verbatim under `refs:` so no stored secret is dropped.
    fn adopt_versioned_layout(&mut self) {
        if self.document.contains_key(DSH_CREDENTIALS_VERSION_KEY) {
            return;
        }
        let legacy = std::mem::take(&mut self.document);
        let mut refs = Map::new();
        for (key, value) in legacy {
            if !matches!(
                key.as_str(),
                DSH_CREDENTIALS_VERSION_KEY
                    | DSH_CREDENTIALS_REFS_KEY
                    | DSH_CREDENTIALS_RECORDS_KEY
            ) {
                refs.insert(key, value);
            }
        }
        self.document.insert(
            DSH_CREDENTIALS_VERSION_KEY.to_string(),
            json!(DSH_CREDENTIALS_VERSION),
        );
        self.document
            .insert(DSH_CREDENTIALS_REFS_KEY.to_string(), Value::Object(refs));
    }

    /// Set or delete one ref entry; a None or blank value deletes it.
    fn set_ref(&mut self, ref_name: &str, value: Option<&str>) {
        let has_value = value.is_some_and(|value| !value.trim().is_empty());
        self.adopt_versioned_layout();
        let refs = self
            .document
            .entry(DSH_CREDENTIALS_REFS_KEY.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !refs.is_object() {
            *refs = Value::Object(Map::new());
        }
        if let Some(refs) = refs.as_object_mut() {
            apply_ref_entry(refs, ref_name, has_value, value);
        }
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        write_credentials_map(path, &self.document)
    }
}

/// Insert or remove one entry inside a refs mapping.
fn apply_ref_entry(
    section: &mut Map<String, Value>,
    ref_name: &str,
    has_value: bool,
    value: Option<&str>,
) {
    if has_value {
        section.insert(ref_name.to_string(), json!(value.unwrap().trim()));
    } else {
        section.remove(ref_name);
    }
}

fn credential_has_value(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(value)) => !value.trim().is_empty(),
        Some(Value::Null) => false,
        Some(_) => true,
        None => false,
    }
}

fn read_credentials_views(path: &Path) -> Vec<DshCredentialView> {
    CredentialsDocument::read(path)
        .map(|document| {
            let mut views: Vec<DshCredentialView> = document
                .refs
                .iter()
                .map(|(ref_name, value)| DshCredentialView {
                    ref_name: ref_name.clone(),
                    has_value: credential_has_value(Some(value)),
                })
                .collect();
            views.sort_by(|a, b| a.ref_name.cmp(&b.ref_name));
            views
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Model / other settings helpers
// ---------------------------------------------------------------------------

fn apply_string_field(object: &mut Map<String, Value>, key: &str, value: Option<String>) {
    match value {
        Some(value) if !value.trim().is_empty() => {
            object.insert(key.to_string(), json!(value.trim()));
        }
        Some(_) => {
            object.remove(key);
        }
        None => {}
    }
}

/// `settings["llm-pi-ai"]["providers"]` dict as `(route, provider)` pairs.
fn get_providers_dict(config: &Value) -> Vec<(String, Value)> {
    config
        .get(DSH_LLM_PI_AI_SECTION)
        .and_then(|section| section.get(DSH_PROVIDERS_KEY))
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(route, provider)| (route.clone(), provider.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn model_settings_from_config(config: &Value) -> DshModelSettingsInput {
    let model = config.get(DSH_DEFAULT_MODEL_SECTION);
    DshModelSettingsInput {
        provider: model
            .and_then(|m| m.get("provider"))
            .and_then(Value::as_str)
            .map(str::to_string),
        model: model
            .and_then(|m| m.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string),
        reasoning_effort: model
            .and_then(|m| m.get("reasoningEffort"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn is_dsh_protected_key(key: &str) -> bool {
    DSH_OTHER_SETTINGS_PROTECTED_KEYS.contains(&key)
}

fn build_other_settings(config: &Value) -> Value {
    let mut other = config.as_object().cloned().unwrap_or_default();
    for key in other.keys().cloned().collect::<Vec<_>>() {
        if is_dsh_protected_key(&key) {
            other.remove(&key);
        }
    }
    Value::Object(other)
}

fn apply_dsh_other_settings(object: &mut Map<String, Value>, other_settings: &Map<String, Value>) {
    for key in object.keys().cloned().collect::<Vec<_>>() {
        if !is_dsh_protected_key(&key) {
            object.remove(&key);
        }
    }
    for (key, value) in other_settings {
        if !is_dsh_protected_key(key) {
            object.insert(key.clone(), value.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Provider views
// ---------------------------------------------------------------------------

/// Model ids from a provider's `models` array (`[{ id, contextWindow, maxTokens }]`).
fn model_ids_from_provider(provider: &Value) -> Vec<String> {
    provider
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve `(credential_exists, api_key)` for one provider route.
///
/// Mirrors pi-ai's resolution order: a sign-in record wins over the `apiKeyEnv`
/// reference when both exist, because that is the credential the runtime
/// actually uses. A record without a displayable secret (an OAuth grant, or an
/// env-only api-key) still counts as configured; its value stays hidden.
fn resolve_provider_credential(
    credentials: &CredentialsDocument,
    provider_key: &str,
    api_key_env: Option<&str>,
) -> (bool, String) {
    let record_key = format!("{DSH_CREDENTIAL_RECORD_SCOPE}/{provider_key}");
    match credentials.records.get(&record_key) {
        Some(record) => match record.get("kind").and_then(Value::as_str) {
            Some("api-key") => {
                let key = record
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let has_env = record
                    .get("env")
                    .and_then(Value::as_object)
                    .is_some_and(|env| !env.is_empty());
                (!key.trim().is_empty() || has_env, key)
            }
            Some("grant") => (true, String::new()),
            _ => ref_credential(credentials, api_key_env),
        },
        None => ref_credential(credentials, api_key_env),
    }
}

/// `(exists, value)` from the provider's `apiKeyEnv` reference entry.
fn ref_credential(credentials: &CredentialsDocument, api_key_env: Option<&str>) -> (bool, String) {
    api_key_env
        .and_then(|ref_name| credentials.refs.get(ref_name))
        .map(|value| {
            (
                credential_has_value(Some(value)),
                value.as_str().unwrap_or_default().to_string(),
            )
        })
        .unwrap_or((false, String::new()))
}

fn build_provider_views(
    config: &Value,
    credentials: &CredentialsDocument,
) -> Vec<DshRuntimeProviderView> {
    if !config.is_object() {
        return Vec::new();
    }
    let default_section = config.get(DSH_DEFAULT_MODEL_SECTION);
    let default_provider = default_section
        .and_then(|m| m.get("provider"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let default_model = default_section
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let providers = get_providers_dict(config);

    let mut keys = BTreeSet::new();
    for (route, _) in &providers {
        keys.insert(route.clone());
    }
    if let Some(default_provider) = &default_provider {
        if !default_provider.trim().is_empty() {
            keys.insert(default_provider.clone());
        }
    }

    let mut views = Vec::new();
    for provider_key in keys {
        let raw = providers.iter().find(|(route, _)| *route == provider_key);
        let is_default = default_provider.as_deref() == Some(provider_key.as_str());
        let is_builtin = is_builtin_provider(&provider_key);

        let display_name = raw
            .and_then(|(_, value)| value.get("displayName").and_then(Value::as_str))
            .map(str::to_string)
            .or_else(|| builtin_provider_name(&provider_key).map(str::to_string))
            .unwrap_or_else(|| provider_key.clone());

        let api_key_env = raw
            .and_then(|(_, value)| value.get("apiKeyEnv").and_then(Value::as_str))
            .map(str::to_string);
        let (credential_exists, api_key) =
            resolve_provider_credential(credentials, &provider_key, api_key_env.as_deref());
        let api = raw
            .and_then(|(_, value)| value.get("api").and_then(Value::as_str))
            .map(str::to_string);

        // Models come from the route's explicit `models` list, or the bundled
        // adapter catalog when none is declared (matching the official
        // llm-pi-ai behavior of serving the installed catalog; an empty list
        // counts as "none declared").
        let has_explicit_models = raw
            .and_then(|(_, value)| value.get("models"))
            .and_then(Value::as_array)
            .map(|models| !models.is_empty())
            .unwrap_or(false);
        let inherited_models = if has_explicit_models {
            None
        } else {
            builtin_models_for(&provider_key)
        };
        let model_ids = if has_explicit_models {
            raw.map(|(_, value)| model_ids_from_provider(value))
                .unwrap_or_default()
        } else if let Some(models) = inherited_models {
            builtin_model_ids(models)
        } else {
            Vec::new()
        };
        let model_source = if has_explicit_models {
            DshModelSource::Explicit
        } else if inherited_models.is_some() {
            DshModelSource::Builtin
        } else {
            DshModelSource::Explicit
        };
        let builtin_models = inherited_models.map(|models| models.to_vec());

        let mut warnings = Vec::new();
        if is_default {
            if raw.is_none() {
                // A route the catalog serves (built-in or bundled) may stay
                // configuration-free without being flagged missing.
                if !is_builtin && !has_builtin_models(&provider_key) {
                    warnings.push(DshProviderWarning::MissingProvider);
                }
            } else if let Some(default_model) = default_model.as_deref() {
                if !default_model.trim().is_empty()
                    && !model_ids.is_empty()
                    && !model_ids.iter().any(|id| id == default_model)
                {
                    warnings.push(DshProviderWarning::MissingModel);
                }
            }
        }

        views.push(DshRuntimeProviderView {
            provider_key,
            display_name,
            api_key_env,
            credential_exists,
            api_key,
            api,
            provider: raw.map(|(_, value)| value.clone()),
            model_ids,
            model_source,
            builtin_models,
            is_builtin,
            is_default,
            warnings,
        });
    }

    views
}

fn builtin_providers() -> Vec<DshBuiltinProvider> {
    super::constants::DSH_BUILTIN_PROVIDERS
        .iter()
        .map(|(key, name)| DshBuiltinProvider {
            key: (*key).to_string(),
            name: (*name).to_string(),
        })
        .collect()
}

fn emit_config_changed<R: Runtime>(app: &tauri::AppHandle<R>, payload: &str) {
    let _ = app.emit("config-changed", payload);
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-dsh", ());
}

// ---------------------------------------------------------------------------
// Root / path commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_dsh_default_config_dir() -> Result<String, String> {
    default_dsh_config_dir().map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_dsh_config_dir_without_db() -> Result<String, String> {
    resolve_dsh_config_dir_without_db().map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_dsh_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<DshPathInfo, String> {
    get_dsh_root_path_info_from_db_async(state.db()).await
}

// ---------------------------------------------------------------------------
// Settings commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_dsh_settings_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<DshSettingsConfig>, String> {
    Ok(state
        .db()
        .with_conn(|conn| db_get(conn, DbTable::DshSettingsConfig, "common"))?
        .map(adapter::settings_from_db_value))
}

#[tauri::command]
pub async fn save_dsh_settings_config<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    input: DshSettingsConfigInput,
) -> Result<(), String> {
    let db = state.db();
    let existing = get_dsh_settings_config(state.clone()).await?;
    let config_dir = if input.clear_root_dir {
        None
    } else {
        input
            .root_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| existing.and_then(|value| value.config_dir))
    };
    let data = adapter::settings_to_db_value(config_dir.as_deref());
    db.with_conn(|conn| db_put(conn, DbTable::DshSettingsConfig, "common", &data))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(db, "dsh").await?;
    let _ = app.emit("wsl-config-changed", ());
    emit_config_changed(&app, "window");
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime config
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn read_dsh_runtime_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<DshRuntimeConfig, String> {
    let db = state.db();
    let root_path_info = get_dsh_root_path_info_from_db_async(&db).await?;
    let root_dir = PathBuf::from(&root_path_info.path);
    let config_path = get_dsh_config_path_from_root(&root_dir);
    let credentials_path = get_dsh_credentials_path_from_root(&root_dir);
    let prompt_path = get_dsh_prompt_path_from_root(&root_dir);

    let config = read_yaml_object_or_empty(&config_path)?;
    let credentials = CredentialsDocument::read(&credentials_path)?;
    let cordis_patch_path = get_dsh_cordis_patch_path(&db).await?;

    Ok(DshRuntimeConfig {
        root_path_info,
        config_path: config_path.to_string_lossy().to_string(),
        credentials_path: credentials_path.to_string_lossy().to_string(),
        prompt_path: prompt_path.to_string_lossy().to_string(),
        model_settings: model_settings_from_config(&config),
        other_settings: build_other_settings(&config),
        providers: build_provider_views(&config, &credentials),
        builtin_providers: builtin_providers(),
        credentials: read_credentials_views(&credentials_path),
        config_content: fs::read_to_string(&config_path).ok(),
        credentials_content: fs::read_to_string(&credentials_path).ok(),
        prompt_content: fs::read_to_string(&prompt_path).ok(),
        cordis_patch_path: cordis_patch_path.to_string_lossy().to_string(),
        cordis_patch_content: fs::read_to_string(&cordis_patch_path).ok(),
        config,
    })
}

// ---------------------------------------------------------------------------
// Provider commands
// ---------------------------------------------------------------------------

/// Read the writable `llm-pi-ai.providers.<route>` dict from `config`, inserting
/// the namespace/providers containers when missing.
fn providers_dict_mut(config: &mut Value) -> Result<&mut Map<String, Value>, String> {
    let llm_section = config
        .as_object_mut()
        .ok_or_else(|| "Expected a YAML mapping object".to_string())?
        .entry(DSH_LLM_PI_AI_SECTION.to_string())
        .or_insert_with(|| json!({}));
    if !llm_section.is_object() {
        return Err(format!(
            "'{DSH_LLM_PI_AI_SECTION}' must be a YAML mapping section"
        ));
    }
    let providers = llm_section
        .as_object_mut()
        .unwrap()
        .entry(DSH_PROVIDERS_KEY.to_string())
        .or_insert_with(|| json!({}));
    if !providers.is_object() {
        return Err(format!(
            "'{DSH_LLM_PI_AI_SECTION}.{DSH_PROVIDERS_KEY}' must be a YAML mapping"
        ));
    }
    Ok(providers.as_object_mut().unwrap())
}

fn write_provider_with_credential(
    config_path: &Path,
    config: &Value,
    credentials_path: &Path,
    credential: &DshCredentialInput,
) -> Result<(), String> {
    fn snapshot(path: &Path) -> Result<Option<Vec<u8>>, String> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("Failed to read {}: {error}", path.display())),
        }
    }
    let previous_config = snapshot(config_path)?;
    let previous_credentials = snapshot(credentials_path)?;
    let mut credentials = CredentialsDocument::read(credentials_path)?;
    if credential.ref_name.trim().is_empty() {
        return Err("Credential ref is required".to_string());
    }
    credentials.set_ref(credential.ref_name.trim(), Some(credential.value.trim()));
    let result = credentials.write(credentials_path)
        .and_then(|()| write_yaml_object(config_path, config));
    if let Err(error) = result {
        let mut failures = Vec::new();
        for (path, previous) in [(config_path, previous_config), (credentials_path, previous_credentials)] {
            let restore = match previous {
                Some(bytes) => fs::write(path, bytes),
                None => match fs::remove_file(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    result => result,
                },
            };
            if let Err(restore_error) = restore { failures.push(format!("{}: {restore_error}", path.display())); }
        }
        set_credentials_file_permissions(credentials_path);
        return Err(if failures.is_empty() { error } else { format!("{error}; rollback failed: {}", failures.join("; ")) });
    }
    Ok(())
}

#[tauri::command]
pub async fn save_dsh_models_provider<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    input: DshModelsProviderInput,
) -> Result<DshRuntimeConfig, String> {
    let provider_key = input.provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider name is required".to_string());
    }
    if !input.provider.is_object() {
        return Err("dsh provider config must be an object".to_string());
    }

    let db = state.db();
    let config_path = get_dsh_config_path_async(&db).await?;
    let mut config = read_yaml_object_or_empty(&config_path)?;

    let mut payload = input.provider;
    payload.as_object_mut().map(|obj| {
        // Drop UI-only markers so they never reach YAML.
        obj.remove("provider_key");
    });
    let providers = providers_dict_mut(&mut config)?;
    providers.insert(provider_key.to_string(), payload);

    if let Some(credential) = &input.credential {
        let credentials_path = get_dsh_credentials_path_async(&db).await?;
        write_provider_with_credential(&config_path, &config, &credentials_path, credential)?;
    } else {
        write_yaml_object(&config_path, &config)?;
    }
    emit_config_changed(&app, "window");
    read_dsh_runtime_config(state).await
}

#[tauri::command]
pub async fn delete_dsh_runtime_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_key: String,
) -> Result<DshRuntimeConfig, String> {
    let provider_key = provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider name is required".to_string());
    }
    let db = state.db();
    let config_path = get_dsh_config_path_async(&db).await?;
    let mut config = read_yaml_object_or_empty(&config_path)?;

    let removed = {
        if let Some(llm_section) = config
            .get_mut(DSH_LLM_PI_AI_SECTION)
            .and_then(Value::as_object_mut)
        {
            let removed = llm_section
                .get_mut(DSH_PROVIDERS_KEY)
                .and_then(Value::as_object_mut)
                .map(|providers| providers.remove(provider_key).is_some())
                .unwrap_or(false);
            // Drop now-empty namespace containers so the file stays tidy.
            let providers_empty = llm_section
                .get(DSH_PROVIDERS_KEY)
                .and_then(Value::as_object)
                .map(|providers| providers.is_empty())
                .unwrap_or(false);
            if providers_empty {
                llm_section.remove(DSH_PROVIDERS_KEY);
            }
            if llm_section.is_empty() {
                config
                    .as_object_mut()
                    .map(|object| object.remove(DSH_LLM_PI_AI_SECTION));
            }
            removed
        } else {
            false
        }
    };
    if !removed {
        return Err(format!("Provider '{provider_key}' not found"));
    }

    write_yaml_object(&config_path, &config)?;
    emit_config_changed(&app, "window");
    read_dsh_runtime_config(state).await
}

// ---------------------------------------------------------------------------
// Model settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_dsh_model_settings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: DshModelSettingsInput,
) -> Result<DshRuntimeConfig, String> {
    let db = state.db();
    let config_path = get_dsh_config_path_async(&db).await?;
    let mut config = read_yaml_object_or_empty(&config_path)?;
    let object = object_mut(&mut config)?;

    let mut model_section = object
        .get(DSH_DEFAULT_MODEL_SECTION)
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let model_object = object_mut(&mut model_section)?;
    apply_string_field(model_object, "provider", input.provider);
    apply_string_field(model_object, "model", input.model);
    apply_string_field(model_object, "reasoningEffort", input.reasoning_effort);
    if model_section
        .as_object()
        .map(|m| m.is_empty())
        .unwrap_or(false)
    {
        object.remove(DSH_DEFAULT_MODEL_SECTION);
    } else {
        object.insert(DSH_DEFAULT_MODEL_SECTION.to_string(), model_section);
    }

    write_yaml_object(&config_path, &config)?;
    emit_config_changed(&app, "window");
    read_dsh_runtime_config(state).await
}

pub async fn apply_dsh_model_internal<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_key: &str,
    model_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    let provider_key = provider_key.trim();
    let model_id = model_id.trim();
    if provider_key.is_empty() {
        return Err("Provider name is required".to_string());
    }
    if model_id.is_empty() {
        return Err("Model id is required".to_string());
    }

    let config_path = get_dsh_config_path_async(db).await?;
    let mut config = read_yaml_object_or_empty(&config_path)?;
    let object = object_mut(&mut config)?;

    let mut model_section = object
        .get(DSH_DEFAULT_MODEL_SECTION)
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let model_object = object_mut(&mut model_section)?;
    model_object.insert("provider".to_string(), json!(provider_key));
    model_object.insert("model".to_string(), json!(model_id));
    object.insert(DSH_DEFAULT_MODEL_SECTION.to_string(), model_section);

    write_yaml_object(&config_path, &config)?;
    emit_config_changed(app, if from_tray { "tray" } else { "window" });
    if let Err(error) =
        crate::settings::provider_list_state::record_provider_last_used_in_sqlite_state(
            db,
            "dsh",
            provider_key,
        )
    {
        log::warn!("Failed to record provider last-used for dsh:{provider_key}: {error}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_dsh_credential(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: DshCredentialInput,
) -> Result<DshRuntimeConfig, String> {
    let ref_name = input.ref_name.trim();
    if ref_name.is_empty() {
        return Err("Credential ref is required".to_string());
    }
    let db = state.db();
    let credentials_path = get_dsh_credentials_path_async(&db).await?;
    let mut credentials = CredentialsDocument::read(&credentials_path)?;
    // A blank value deletes the ref (provider card clearing the key).
    credentials.set_ref(ref_name, Some(input.value.trim()));
    credentials.write(&credentials_path)?;
    emit_config_changed(&app, "window");
    read_dsh_runtime_config(state).await
}

#[tauri::command]
pub async fn delete_dsh_credential(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ref_name: String,
) -> Result<DshRuntimeConfig, String> {
    let ref_name = ref_name.trim();
    if ref_name.is_empty() {
        return Err("Credential ref is required".to_string());
    }
    let db = state.db();
    let credentials_path = get_dsh_credentials_path_async(&db).await?;
    let mut credentials = CredentialsDocument::read(&credentials_path)?;
    // Tolerated no-op when the ref is already gone — e.g. the effective
    // credential lives in a dsh-managed sign-in record this app never touches.
    credentials.set_ref(ref_name, None);
    credentials.write(&credentials_path)?;
    emit_config_changed(&app, "window");
    read_dsh_runtime_config(state).await
}

/// Return the raw value stored under a credential ref, so the upstream model
/// fetch / connectivity flows can authenticate with the real secret rather
/// than the ref name. The value is never surfaced in the general runtime
/// config; it is resolved on demand for the provider being acted on.
#[tauri::command]
pub async fn get_dsh_credential_value(
    state: tauri::State<'_, SqliteDbState>,
    ref_name: String,
) -> Result<Option<String>, String> {
    let ref_name = ref_name.trim();
    if ref_name.is_empty() {
        return Ok(None);
    }
    let db = state.db();
    let credentials_path = get_dsh_credentials_path_async(&db).await?;
    Ok(CredentialsDocument::read(&credentials_path)?
        .refs
        .get(ref_name)
        .and_then(Value::as_str)
        .map(str::to_string))
}

// ---------------------------------------------------------------------------
// Other settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_dsh_other_settings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    other_settings: Value,
) -> Result<DshRuntimeConfig, String> {
    if !other_settings.is_object() {
        return Err("dsh other settings must be an object".to_string());
    }
    let db = state.db();
    let config_path = get_dsh_config_path_async(&db).await?;
    let mut config = read_yaml_object_or_empty(&config_path)?;
    let object = object_mut(&mut config)?;
    apply_dsh_other_settings(object, other_settings.as_object().unwrap());

    write_yaml_object(&config_path, &config)?;
    emit_config_changed(&app, "window");
    read_dsh_runtime_config(state).await
}

// ---------------------------------------------------------------------------
// Prompt CRUD
// ---------------------------------------------------------------------------

fn prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?]))
}

fn put_dsh_prompt_to_sqlite(
    db: &SqliteDbState,
    id: &str,
    content: &DshPromptConfigContent,
) -> Result<(), String> {
    let value = adapter::prompt_to_db_value(content);
    db.with_conn(|conn| db_put(conn, DbTable::DshPromptConfig, id, &value))
}

fn get_dsh_prompt_from_sqlite(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<DshPromptConfig>, String> {
    Ok(db
        .with_conn(|conn| db_get(conn, DbTable::DshPromptConfig, id))?
        .map(adapter::prompt_from_db_value))
}

async fn get_local_prompt_config(db: &SqliteDbState) -> Result<Option<DshPromptConfig>, String> {
    let prompt_path = get_dsh_prompt_path_async(db).await?;
    if !prompt_path.exists() {
        return Ok(None);
    }
    let Some(content) = read_prompt_content_file(&prompt_path, "dsh")? else {
        return Ok(None);
    };
    Ok(Some(DshPromptConfig {
        id: "__local__".to_string(),
        name: "Local AGENTS.md".to_string(),
        content,
        is_applied: false,
        sort_index: Some(-1),
        created_at: None,
        updated_at: None,
    }))
}

async fn write_prompt_content_to_file(
    db: &SqliteDbState,
    content: Option<&str>,
) -> Result<(), String> {
    let path = get_dsh_prompt_path_async(db).await?;
    write_prompt_content_file(&path, content, "dsh")
}

#[tauri::command]
pub async fn list_dsh_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<DshPromptConfig>, String> {
    let db = state.db();
    let mut prompts = db.with_conn(|conn| {
        Ok(
            db_list(conn, DbTable::DshPromptConfig, Some(&prompt_order()?))?
                .into_iter()
                .map(adapter::prompt_from_db_value)
                .collect::<Vec<_>>(),
        )
    })?;
    if !prompts.iter().any(|prompt| prompt.is_applied) {
        if let Some(local_prompt) = get_local_prompt_config(&db).await? {
            prompts.insert(0, local_prompt);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_dsh_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: DshPromptConfigInput,
) -> Result<DshPromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let next_sort_index = db.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::DshPromptConfig,
            &JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;
    let content = DshPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };
    let prompt_id = db_new_id();
    put_dsh_prompt_to_sqlite(&db, &prompt_id, &content)?;
    let _ = app.emit("config-changed", "window");
    Ok(adapter::prompt_from_db_value(json!({
        "id": prompt_id,
        "name": content.name,
        "content": content.content,
        "is_applied": content.is_applied,
        "sort_index": content.sort_index,
        "created_at": content.created_at,
        "updated_at": content.updated_at
    })))
}

#[tauri::command]
pub async fn update_dsh_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: DshPromptConfigInput,
) -> Result<DshPromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let existing = get_dsh_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found"))?;
    let content = DshPromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied: existing.is_applied,
        sort_index: existing.sort_index,
        created_at: existing.created_at.unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    put_dsh_prompt_to_sqlite(&db, &config_id, &content)?;
    if existing.is_applied {
        write_prompt_content_to_file(&db, Some(input.content.as_str())).await?;
        emit_config_changed(&app, "window");
    } else {
        let _ = app.emit("config-changed", "window");
    }
    get_dsh_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found after update"))
}

#[tauri::command]
pub async fn delete_dsh_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    // Only delete the DB prompt record. The live AGENTS.md on disk is kept so
    // deleting a saved prompt never wipes the local runtime prompt.
    let db = state.db();
    db.with_conn(|conn| db_delete(conn, DbTable::DshPromptConfig, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub async fn apply_dsh_prompt_config_internal<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_dsh_prompt_config_internal_with_events(state, app, config_id, from_tray, true).await
}

pub async fn apply_dsh_prompt_config_internal_without_events<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_dsh_prompt_config_internal_with_events(state, app, config_id, false, false).await
}

async fn apply_dsh_prompt_config_internal_with_events<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
    emit_events: bool,
) -> Result<(), String> {
    let db = state.db();
    if config_id == "__local__" {
        let local_prompt = get_local_prompt_config(&db)
            .await?
            .ok_or_else(|| "Local dsh prompt not found".to_string())?;
        write_prompt_content_to_file(&db, Some(local_prompt.content.as_str())).await?;
        if emit_events {
            emit_config_changed(app, if from_tray { "tray" } else { "window" });
        }
        return Ok(());
    }

    let prompt = get_dsh_prompt_from_sqlite(&db, config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found"))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::DshPromptConfig, Some(config_id), &now)
    })?;
    write_prompt_content_to_file(&db, Some(prompt.content.as_str())).await?;
    if emit_events {
        emit_config_changed(app, if from_tray { "tray" } else { "window" });
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_dsh_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_dsh_prompt_config_internal(state, &app, &config_id, false).await
}

/// Disable the applied dsh prompt: clear every applied flag and empty the
/// live prompt file, while keeping the DB record so it can be re-applied later.
#[tauri::command]
pub async fn disable_dsh_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    get_dsh_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found"))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| db_update_applied_status(conn, DbTable::DshPromptConfig, None, &now))?;
    write_prompt_content_to_file(&db, Some("")).await?;
    emit_config_changed(&app, "window");
    Ok(())
}

#[tauri::command]
pub async fn reorder_dsh_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::DshPromptConfig,
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn save_dsh_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: DshPromptConfigInput,
) -> Result<DshPromptConfig, String> {
    let db = state.db();
    let content = if input.content.trim().is_empty() {
        get_local_prompt_config(&db)
            .await?
            .map(|prompt| prompt.content)
            .unwrap_or_default()
    } else {
        input.content
    };
    let created = create_dsh_prompt_config(
        state.clone(),
        app.clone(),
        DshPromptConfigInput {
            id: None,
            name: input.name,
            content,
        },
    )
    .await?;
    apply_dsh_prompt_config_internal(state.clone(), &app, &created.id, false).await?;
    Ok(get_dsh_prompt_from_sqlite(state.db(), &created.id)?.unwrap_or(created))
}

// ============================================================================
// DSh Web UI
// ============================================================================

/// 打开 DSh Web UI:探测本地服务在线后,用系统浏览器打开。
/// 服务离线返回 `Err`,前端据此引导用户启动 `dsh web`。
#[tauri::command]
pub async fn open_dsh_web_ui(path: Option<String>) -> Result<(), String> {
    use super::web_ui;

    let port = web_ui::resolve_web_port();
    if !web_ui::probe_web_up(port).await {
        return Err("DSh Web UI 未运行,请先启动 dsh web".to_string());
    }
    web_ui::open_web_ui_browser(port, path.as_deref())
}

/// 在用户终端里非阻塞启动 `dsh web`(或经 `use_npx` 回退 `npx @deepseek-ai/dsh web`)。
#[tauri::command]
pub async fn launch_dsh_dashboard(use_npx: Option<bool>) -> Result<(), String> {
    use super::web_ui;

    web_ui::launch_dsh_web_in_terminal(use_npx.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A parsed credentials document from a JSON object shape.
    fn credentials_document(value: Value) -> CredentialsDocument {
        CredentialsDocument::from_document(value.as_object().cloned().expect("object"))
    }

    #[test]
    fn model_settings_from_config_reads_section() {
        let config = json!({
            "agent-default-model": {
                "provider": "deepseek",
                "model": "deepseek-chat",
                "reasoningEffort": "high"
            }
        });
        let ms = model_settings_from_config(&config);
        assert_eq!(ms.provider.as_deref(), Some("deepseek"));
        assert_eq!(ms.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(ms.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn build_other_settings_hides_managed_keys() {
        let config = json!({
            "llm-pi-ai": { "providers": { "deepseek": {} } },
            "agent-default-model": { "provider": "deepseek" },
            "agent": { "auto_accept": true }
        });
        assert_eq!(
            build_other_settings(&config),
            json!({ "agent": { "auto_accept": true } })
        );
    }

    #[test]
    fn apply_dsh_other_settings_preserves_managed_keys() {
        let mut config = json!({
            "llm-pi-ai": { "providers": { "deepseek": {} } },
            "agent-default-model": { "provider": "deepseek" },
            "agent": { "auto_accept": false }
        });
        let other_settings = json!({
            "agent": { "auto_accept": true },
            "llm-pi-ai": { "should-not-overwrite": {} }
        });
        apply_dsh_other_settings(
            config.as_object_mut().unwrap(),
            other_settings.as_object().unwrap(),
        );
        assert_eq!(config["agent"]["auto_accept"], true);
        assert!(config["llm-pi-ai"]["providers"].get("deepseek").is_some());
        assert!(config["llm-pi-ai"].get("should-not-overwrite").is_none());
    }

    #[test]
    fn build_provider_views_merges_default_and_credentials() {
        let config = json!({
            "llm-pi-ai": {
                "providers": {
                    "deepseek": {
                        "apiKeyEnv": "DEEPSEEK_API_KEY",
                        "displayName": "DeepSeek",
                        "models": [ { "id": "deepseek-chat" }, { "id": "deepseek-reasoner" } ]
                    }
                }
            },
            "agent-default-model": { "provider": "deepseek", "model": "deepseek-chat" }
        });
        let views = build_provider_views(
            &config,
            &credentials_document(json!({
                "DEEPSEEK_API_KEY": "sk-123"
            })),
        );
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(view.provider_key, "deepseek");
        assert_eq!(view.display_name, "DeepSeek");
        assert_eq!(view.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert!(view.credential_exists);
        assert!(view.is_builtin);
        assert!(view.is_default);
        assert_eq!(view.model_ids, vec!["deepseek-chat", "deepseek-reasoner"]);
        assert!(view.warnings.is_empty());
    }

    #[test]
    fn build_provider_views_serves_builtin_catalog_when_models_absent() {
        let config = json!({
            "llm-pi-ai": {
                "providers": {
                    "deepseek": { "apiKeyEnv": "DEEPSEEK_API_KEY" }
                }
            },
            "agent-default-model": { "provider": "deepseek", "model": "deepseek-v4-flash" }
        });
        let views = build_provider_views(&config, &credentials_document(json!({})));
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(view.model_source, DshModelSource::Builtin);
        assert!(view
            .builtin_models
            .as_ref()
            .map(|models| !models.is_empty())
            .unwrap_or(false));
        assert!(view.model_ids.iter().any(|id| id == "deepseek-v4-flash"));
        assert!(view.model_ids.iter().any(|id| id == "deepseek-v4-pro"));
        assert!(view.warnings.is_empty());
    }

    #[test]
    fn build_provider_views_official_suffix_inherits_catalog() {
        let config = json!({
            "llm-pi-ai": {
                "providers": {
                    "deepseek-official": { "apiKeyEnv": "DEEPSEEK_API_KEY" }
                }
            },
            "agent-default-model": { "provider": "deepseek-official", "model": "deepseek-v4-flash" }
        });
        let views = build_provider_views(&config, &credentials_document(json!({})));
        let view = &views[0];
        assert_eq!(view.model_source, DshModelSource::Builtin);
        assert!(view.model_ids.iter().any(|id| id == "deepseek-v4-pro"));
    }

    #[test]
    fn build_provider_views_keeps_explicit_models_as_explicit_source() {
        let config = json!({
            "llm-pi-ai": {
                "providers": {
                    "deepseek": { "models": [ { "id": "deepseek-chat" } ] }
                }
            },
            "agent-default-model": { "provider": "deepseek", "model": "deepseek-chat" }
        });
        let views = build_provider_views(&config, &credentials_document(json!({})));
        let view = &views[0];
        assert_eq!(view.model_source, DshModelSource::Explicit);
        assert_eq!(view.model_ids, vec!["deepseek-chat"]);
        assert!(view.builtin_models.is_none());
    }

    #[test]
    fn build_provider_views_flags_missing_credential_and_model() {
        let config = json!({
            "llm-pi-ai": {
                "providers": {
                    "custom-x": {
                        "apiKeyEnv": "CUSTOM_X_API_KEY",
                        "models": [ { "id": "m1" } ]
                    }
                }
            },
            "agent-default-model": { "provider": "custom-x", "model": "missing-model" }
        });
        let views = build_provider_views(&config, &credentials_document(json!({})));
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert!(!view.is_builtin);
        assert!(!view.credential_exists);
        assert!(view
            .warnings
            .iter()
            .any(|w| matches!(w, DshProviderWarning::MissingModel)));
    }

    #[test]
    fn credential_has_value_handles_null_and_blank() {
        assert!(!credential_has_value(None));
        assert!(!credential_has_value(Some(&Value::Null)));
        assert!(!credential_has_value(Some(&json!("  "))));
        assert!(credential_has_value(Some(&json!("sk-123"))));
        assert!(credential_has_value(Some(&json!(42))));
    }

    #[test]
    fn credential_yaml_roundtrip_preserves_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".credentials.yaml");

        let mut document = CredentialsDocument::from_document(Map::new());
        document.set_ref("DEEPSEEK_API_KEY", Some("sk-proj-12345"));
        document.write(&path).expect("write");

        let read = CredentialsDocument::read(&path).expect("read");
        assert_eq!(
            read.refs.get("DEEPSEEK_API_KEY"),
            Some(&json!("sk-proj-12345"))
        );

        let view = read_credentials_views(&path);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].ref_name, "DEEPSEEK_API_KEY");
        assert!(view[0].has_value);

        // Deleting the last ref leaves an empty (version-stamped) store.
        let mut document = CredentialsDocument::read(&path).expect("re-read");
        document.set_ref("DEEPSEEK_API_KEY", None);
        document.write(&path).expect("write empty");
        assert!(CredentialsDocument::read(&path)
            .expect("read empty")
            .refs
            .is_empty());
    }

    #[test]
    fn versioned_document_write_preserves_version_and_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".credentials.yaml");

        let versioned = json!({
            "version": 1,
            "refs": { "OLD_KEY": "sk-old" },
            "records": {
                "llm-pi-ai/deepseek": { "kind": "api-key", "key": "sk-signin" }
            }
        });
        let mut document = credentials_document(versioned);
        document.set_ref("NEW_KEY", Some("  sk-new  "));
        document.set_ref("OLD_KEY", None);
        document.write(&path).expect("write");

        let text = fs::read_to_string(&path).expect("text");
        assert!(text.contains("version: 1"), "version stamp kept: {text}");
        assert!(text.contains("llm-pi-ai/deepseek"), "records kept: {text}");
        assert!(!text.contains("OLD_KEY"), "deleted ref removed: {text}");

        let read = CredentialsDocument::read(&path).expect("read");
        assert_eq!(read.refs.get("NEW_KEY"), Some(&json!("sk-new")));
        assert!(read.refs.get("OLD_KEY").is_none());
        assert_eq!(
            read.records
                .get("llm-pi-ai/deepseek")
                .and_then(|record| record.get("key"))
                .and_then(Value::as_str),
            Some("sk-signin")
        );
    }

    #[test]
    fn flat_document_adopted_to_versioned_on_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".credentials.yaml");

        // A pre-release flat document, as an older ai-toolbox build wrote it.
        write_credentials_map(
            &path,
            &json!({ "LEGACY_KEY": "sk-legacy" })
                .as_object()
                .cloned()
                .expect("object"),
        )
        .expect("seed flat file");

        let mut document = CredentialsDocument::read(&path).expect("read flat");
        assert_eq!(document.refs.get("LEGACY_KEY"), Some(&json!("sk-legacy")));
        document.set_ref("NEW_KEY", Some("sk-new"));
        document.write(&path).expect("write");

        let read = CredentialsDocument::read(&path).expect("read migrated");
        assert_eq!(
            read.refs.get("LEGACY_KEY"),
            Some(&json!("sk-legacy")),
            "existing entries nest into refs instead of being dropped"
        );
        assert_eq!(read.refs.get("NEW_KEY"), Some(&json!("sk-new")));
        let text = fs::read_to_string(&path).expect("text");
        assert!(text.contains("version: 1"), "stamped: {text}");
    }

    #[test]
    fn provider_and_credential_write_rolls_back_both_files_when_config_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("settings.yaml");
        let credentials_path = dir.path().join(".credentials.yaml");
        let original_config = b"# Keep the original bytes\nagent: { enabled: true }\n";
        let original_credentials = b"version: 1\nrefs: { OLD_KEY: keep-key }\nrecords: { login: { key: keep-login } }\n";
        fs::write(&config_path, original_config).unwrap();
        fs::write(&credentials_path, original_credentials).unwrap();
        let input = DshCredentialInput { ref_name: "NEW_KEY".to_string(), value: "new-key".to_string() };
        // The second writer rejects a non-mapping after the credential write.
        let result = write_provider_with_credential(&config_path, &Value::Null, &credentials_path, &input);
        assert!(result.is_err());
        assert_eq!(fs::read(&config_path).unwrap(), original_config);
        assert_eq!(fs::read(&credentials_path).unwrap(), original_credentials);

        let new_config = dir.path().join("new/settings.yaml");
        let new_credentials = dir.path().join("new/.credentials.yaml");
        assert!(write_provider_with_credential(&new_config, &Value::Null, &new_credentials, &input).is_err());
        assert!(!new_config.exists());
        assert!(!new_credentials.exists());
    }

    #[test]
    fn delete_missing_ref_is_tolerated_noop() {
        let mut document = credentials_document(json!({
            "version": 1,
            "refs": { "OTHER": "sk-x" },
            "records": {}
        }));
        document.set_ref("NOT_PRESENT", None);
        assert!(document.refs.contains_key("OTHER"));
        assert!(!document.refs.contains_key("NOT_PRESENT"));
    }

    #[test]
    fn build_provider_views_backfills_from_sign_in_records() {
        let config = json!({
            "llm-pi-ai": {
                "providers": {
                    "deepseek": { "apiKeyEnv": "DEEPSEEK_API_KEY" },
                    "custom-oauth": {},
                    "custom-envonly": {}
                }
            }
        });
        let credentials = credentials_document(json!({
            "version": 1,
            "refs": { "DEEPSEEK_API_KEY": "sk-ref" },
            "records": {
                // A stored api-key record wins over the apiKeyEnv reference —
                // mirroring pi-ai's resolution order.
                "llm-pi-ai/deepseek": { "kind": "api-key", "key": "sk-record" },
                // An OAuth grant counts as configured without a displayable key.
                "llm-pi-ai/custom-oauth": {
                    "kind": "grant",
                    "payload": { "access_token": "tok" }
                },
                // An env-only api-key record counts as configured too.
                "llm-pi-ai/custom-envonly": {
                    "kind": "api-key",
                    "env": { "AWS_ACCESS_KEY_ID": "aws" }
                }
            }
        }));

        let views = build_provider_views(&config, &credentials);
        let by_key = |key: &str| {
            views
                .iter()
                .find(|view| view.provider_key == key)
                .unwrap_or_else(|| panic!("view for {key}"))
        };

        let deepseek = by_key("deepseek");
        assert_eq!(deepseek.api_key, "sk-record", "record beats ref");
        assert!(deepseek.credential_exists);

        let oauth = by_key("custom-oauth");
        assert!(oauth.credential_exists);
        assert_eq!(oauth.api_key, "", "grant payload stays hidden");

        let env_only = by_key("custom-envonly");
        assert!(env_only.credential_exists);
        assert_eq!(env_only.api_key, "");
    }

    #[test]
    fn build_provider_views_falls_back_to_ref_when_record_absent() {
        let config = json!({
            "llm-pi-ai": {
                "providers": {
                    "deepseek": { "apiKeyEnv": "DEEPSEEK_API_KEY" }
                }
            }
        });
        let credentials = credentials_document(json!({
            "version": 1,
            "refs": { "DEEPSEEK_API_KEY": "sk-ref" },
            "records": {}
        }));
        let views = build_provider_views(&config, &credentials);
        assert_eq!(views[0].api_key, "sk-ref");
        assert!(views[0].credential_exists);
    }
}

// ============================================================================
// All API Hub import (DSH)
// ============================================================================

#[tauri::command]
pub async fn list_dsh_all_api_hub_providers(
) -> Result<crate::coding::all_api_hub::AllApiHubProvidersResult, String> {
    let discovery = crate::coding::all_api_hub::list_provider_candidates()?;
    let providers = crate::coding::all_api_hub::build_all_api_hub_items(
        &discovery.providers,
        crate::coding::all_api_hub::candidate_to_dsh_provider,
    );
    Ok(crate::coding::all_api_hub::AllApiHubProvidersResult {
        found: discovery.found,
        profiles: discovery.profiles,
        providers,
        message: discovery.message,
    })
}

#[tauri::command]
pub async fn resolve_dsh_all_api_hub_providers(
    state: tauri::State<'_, crate::db::SqliteDbState>,
    request: crate::coding::all_api_hub::AllApiHubResolveRequest,
) -> Result<Vec<crate::coding::all_api_hub::AllApiHubProviderItem>, String> {
    let providers = crate::coding::all_api_hub::resolve_provider_candidates_with_keys(
        &state,
        &request.provider_ids,
    )
    .await?;
    Ok(crate::coding::all_api_hub::build_all_api_hub_items(
        &providers,
        crate::coding::all_api_hub::candidate_to_dsh_provider,
    ))
}

// ---------------------------------------------------------------------------
// Agent Instructions plugin check / enable
// ---------------------------------------------------------------------------

/// The plugin id for the dsh workspace-instruction loader.
const AGENT_INSTRUCTIONS_PLUGIN_ID: &str = "agent-instructions";

/// `config.maxBytes` written when enabling the plugin: 256 KiB, so the
/// workspace-instruction baseline budget can fit both `~/.dsh/AGENTS.md` and
/// a sizeable project `AGENTS.md`, instead of the bundle default 65536 bytes.
const AGENT_INSTRUCTIONS_MAX_BYTES: u64 = 256 * 1024;

/// Resolve the home-level `cordis.patch.yml` path via DB-priority config dir.
async fn get_dsh_cordis_patch_path(db: &SqliteDbState) -> Result<PathBuf, String> {
    let root = get_dsh_root_dir_from_db_async(db).await?;
    Ok(root.join("cordis.patch.yml"))
}

/// Check whether the `agent-instructions` plugin is enabled.
///
/// The dsh-web-app bundle disables it by default. Users enable it by adding
/// `- id: agent-instructions, disabled: false` to the home-level
/// `cordis.patch.yml`. When the home patch has no override for this plugin,
/// the bundle default (disabled) applies.
#[tauri::command]
pub async fn check_dsh_agent_instructions(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<AgentInstructionsStatus, String> {
    let db = state.db();
    let cordis_path = get_dsh_cordis_patch_path(&db).await?;

    // When the home patch explicitly sets disabled: false, the plugin is enabled.
    // When the home patch sets disabled: true or has no override (the web-app
    // bundle default is disabled), the plugin is not effectively enabled.
    let enabled = match crate::coding::mcp::cordis_patch::get_plugin_disabled_state(
        &cordis_path,
        AGENT_INSTRUCTIONS_PLUGIN_ID,
    )? {
        Some(false) => true,
        _ => false,
    };

    Ok(AgentInstructionsStatus { enabled })
}

/// Enable the `agent-instructions` plugin by writing `disabled: false` and
/// `config.maxBytes: 262144` (256 KiB) to the home-level `cordis.patch.yml`.
#[tauri::command]
pub async fn enable_dsh_agent_instructions(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<(), String> {
    let db = state.db();
    let cordis_path = get_dsh_cordis_patch_path(&db).await?;

    crate::coding::mcp::cordis_patch::set_plugin_disabled(
        &cordis_path,
        AGENT_INSTRUCTIONS_PLUGIN_ID,
        false,
    )?;
    crate::coding::mcp::cordis_patch::set_plugin_config_field(
        &cordis_path,
        AGENT_INSTRUCTIONS_PLUGIN_ID,
        "maxBytes",
        json!(AGENT_INSTRUCTIONS_MAX_BYTES),
    )?;

    Ok(())
}
