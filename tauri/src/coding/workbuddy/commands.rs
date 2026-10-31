use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::adapter;
use super::constants::WORKBUDDY_SETTINGS_CONFIG_ID;
use super::paths;
use super::types::*;
use crate::coding::workbuddy_family::models_json::{self, ModelEntryInput, ModelsDocument};
use crate::coding::workbuddy_family::runtime_config::{build_runtime_config, FamilyRuntimeConfig};
use crate::db::helpers::db_get;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;
use tauri::{Emitter, Runtime};

/// Serializes every read-modify-write of `models.json`.
///
/// The desktop app watches the same file, and two AI Toolbox saves dispatched
/// concurrently each hold their own snapshot; without this lock the second one
/// would write back the first one's pre-save view.
fn models_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_models_write() -> std::sync::MutexGuard<'static, ()> {
    models_write_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn custom_config_dir_from_db(db: &SqliteDbState) -> Option<PathBuf> {
    db.with_conn(|conn| db_get(conn, DbTable::WorkbuddySettingsConfig, WORKBUDDY_SETTINGS_CONFIG_ID))
        .ok()
        .flatten()
        .and_then(|value| adapter::settings_from_db_value(value).config_dir)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

async fn config_root_from_db(db: &SqliteDbState) -> (PathBuf, String) {
    match custom_config_dir_from_db(db) {
        Some(path) => (path, "custom".to_string()),
        None => paths::resolve_path_without_db(),
    }
}

fn emit_config_changed<R: Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("config-changed", "window");
}

// ============================================================================
// Path info / settings
// ============================================================================

#[tauri::command]
pub async fn get_workbuddy_default_config_dir() -> Result<String, String> {
    Ok(paths::default_config_dir()?.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn get_workbuddy_config_dir_without_db() -> String {
    paths::resolve_path_without_db().0.to_string_lossy().into_owned()
}

#[tauri::command]
pub async fn get_workbuddy_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<WorkbuddyPathInfo, String> {
    let (path, source) = config_root_from_db(&state).await;
    Ok(WorkbuddyPathInfo {
        path: path.to_string_lossy().into_owned(),
        source,
    })
}

#[tauri::command]
pub async fn get_workbuddy_settings_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<WorkbuddySettingsConfig>, String> {
    let record = state
        .with_conn(|conn| db_get(conn, DbTable::WorkbuddySettingsConfig, WORKBUDDY_SETTINGS_CONFIG_ID))
        .map_err(|error| format!("Failed to read WorkBuddy settings: {error}"))?;
    Ok(record.map(adapter::settings_from_db_value))
}

#[tauri::command]
pub async fn save_workbuddy_settings_config(
    state: tauri::State<'_, SqliteDbState>,
    input: WorkbuddySettingsConfigInput,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let normalized = if input.clear_config_dir {
        None
    } else {
        input
            .config_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let expanded = crate::coding::path_expand::expand_local_path(value)?;
                Ok::<String, String>(
                    paths::normalize_selected_root(std::path::Path::new(&expanded))
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .transpose()?
    };

    if let Some(dir) = normalized.as_deref() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("Failed to create WorkBuddy config directory: {error}"))?;
    }

    state
        .with_conn(|conn| {
            crate::db::helpers::db_put(
                conn,
                DbTable::WorkbuddySettingsConfig,
                WORKBUDDY_SETTINGS_CONFIG_ID,
                &adapter::settings_to_db_value(normalized.as_deref()),
            )
        })
        .map_err(|error| format!("Failed to save WorkBuddy settings: {error}"))?;

    crate::coding::runtime_location::refresh_runtime_location_cache_for_module_async(
        &state, "workbuddy",
    )
    .await
    .ok();

    emit_config_changed(&app);
    Ok(())
}

// ============================================================================
// models.json
// ============================================================================

fn read_document(root: &Path) -> Result<ModelsDocument, String> {
    ModelsDocument::load(root)
}

#[tauri::command]
pub async fn read_workbuddy_runtime_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<FamilyRuntimeConfig, String> {
    let (root, _) = config_root_from_db(&state).await;
    let document = read_document(&root)?;
    Ok(build_runtime_config(&root, &document))
}

/// Returns `models.json` verbatim for the preview dialog.
///
/// Never rebuild the text from `FamilyRuntimeConfig`: the channel view drops
/// entries without a URL, reorders fields and loses product-owned keys, so a
/// round-tripped preview would show a file the user does not have.
#[tauri::command]
pub async fn read_workbuddy_models_file(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<String>, String> {
    let (root, _) = config_root_from_db(&state).await;
    let path = root.join(models_json::MODELS_FILE_NAME);
    if !path.is_file() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))
}

#[tauri::command]
pub async fn save_workbuddy_model(
    state: tauri::State<'_, SqliteDbState>,
    input: WorkbuddyModelInput,
    app: tauri::AppHandle,
) -> Result<FamilyRuntimeConfig, String> {
    let model_id = input.id.trim().to_string();
    if model_id.is_empty() {
        return Err("Model id is required".to_string());
    }
    if input.base_url.trim().is_empty() {
        return Err("Model endpoint is required".to_string());
    }

    crate::coding::workbuddy_family::ensure_models_file_direct(
        crate::coding::proxy_gateway::types::GatewayCliKey::Workbuddy,
    )?;
    let (root, _) = config_root_from_db(&state).await;
    let _guard = lock_models_write();

    let mut document = read_document(&root)?;
    let entry = models_json::build_model_entry(&ModelEntryInput {
        id: &model_id,
        name: Some(&input.name),
        vendor: Some(&input.vendor),
        base_url: &input.base_url,
        api_key: Some(&input.api_key),
        max_input_tokens: input.max_input_tokens,
        max_output_tokens: input.max_output_tokens,
        temperature: input.temperature,
        supports_tool_call: input.supports_tool_call,
        supports_images: input.supports_images,
        supports_reasoning: input.supports_reasoning,
        only_reasoning: input.only_reasoning,
        use_custom_protocol: input.use_custom_protocol,
    });
    document.upsert_model_renaming(entry, input.previous_id.as_deref());
    document.save(&root)?;

    emit_config_changed(&app);
    Ok(build_runtime_config(&root, &document))
}

#[tauri::command]
pub async fn delete_workbuddy_model(
    state: tauri::State<'_, SqliteDbState>,
    model_id: String,
    app: tauri::AppHandle,
) -> Result<FamilyRuntimeConfig, String> {
    crate::coding::workbuddy_family::ensure_models_file_direct(
        crate::coding::proxy_gateway::types::GatewayCliKey::Workbuddy,
    )?;
    let (root, _) = config_root_from_db(&state).await;
    let _guard = lock_models_write();

    let mut document = read_document(&root)?;
    if !document.remove_model(model_id.trim()) {
        return Err(format!("Model '{model_id}' was not found"));
    }
    document.save(&root)?;

    emit_config_changed(&app);
    Ok(build_runtime_config(&root, &document))
}

/// Deletes every model entry of one channel.
#[tauri::command]
pub async fn delete_workbuddy_provider(
    state: tauri::State<'_, SqliteDbState>,
    base_url: String,
    api_key: String,
    app: tauri::AppHandle,
) -> Result<FamilyRuntimeConfig, String> {
    crate::coding::workbuddy_family::ensure_models_file_direct(
        crate::coding::proxy_gateway::types::GatewayCliKey::Workbuddy,
    )?;
    let (root, _) = config_root_from_db(&state).await;
    let _guard = lock_models_write();

    let mut document = read_document(&root)?;
    let members = document.models_for_channel(&base_url, &api_key);
    if members.is_empty() {
        return Err("This provider has no models to delete".to_string());
    }
    for entry in members {
        if let Some(id) = models_json::entry_id(&entry) {
            document.remove_model(&id);
        }
    }
    document.save(&root)?;

    emit_config_changed(&app);
    Ok(build_runtime_config(&root, &document))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_config_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path()).expect("root");
        dir
    }

    /// Save a model, read it back through the runtime view, and confirm the
    /// raw-file command returns the on-disk text unchanged.
    ///
    /// The second half is the point: the preview must not be a re-serialized
    /// view, so product-owned keys the entry carries must still be in the text
    /// the command returns.
    #[test]
    fn a_saved_model_survives_the_runtime_view_and_the_raw_file_read() {
        let dir = seed_config_root();
        let models_path = dir.path().join(models_json::MODELS_FILE_NAME);
        // A product-written entry the user never touched: no URL, plus a key
        // AI Toolbox knows nothing about. It must stay in the file.
        std::fs::write(
            &models_path,
            r#"{"models":[{"id":"product-builtin","credits":"included"}]}"#,
        )
        .expect("seed models.json");

        let mut document = ModelsDocument::load(dir.path()).expect("load");
        document.upsert_model(models_json::build_model_entry(&ModelEntryInput {
            id: "gpt-4.1",
            name: Some("GPT 4.1"),
            vendor: Some("OpenAI"),
            base_url: "https://api.example.com/v1",
            api_key: Some("sk-test"),
            max_input_tokens: Some(128_000),
            max_output_tokens: Some(16_384),
            temperature: None,
            supports_tool_call: true,
            supports_images: false,
            supports_reasoning: false,
            only_reasoning: false,
            use_custom_protocol: false,
        }));
        document.save(dir.path()).expect("save");

        // 1. The runtime view surfaces the new channel...
        let reloaded = ModelsDocument::load(dir.path()).expect("reload");
        let runtime = build_runtime_config(dir.path(), &reloaded);
        let channel = runtime
            .providers
            .iter()
            .find(|view| view.base_url == "https://api.example.com/v1")
            .expect("channel visible");
        assert_eq!(channel.models.len(), 1);
        assert_eq!(channel.models[0].id, "gpt-4.1");

        // 2. ...and the raw read is the file itself, not a rebuilt view.
        let raw = std::fs::read_to_string(&models_path).expect("raw read");
        assert!(
            raw.contains("product-builtin"),
            "an entry with no URL is invisible in the channel view but must stay in the file"
        );
        assert!(
            raw.contains("credits"),
            "product-owned keys must not be dropped by the save"
        );
        assert!(raw.contains("gpt-4.1"));
    }

    /// Renaming a model keeps both its position and its product-owned keys.
    ///
    /// Deleting the old entry and appending the new one would still "work" —
    /// the model would be renamed — but the extras the product wrote onto it
    /// (`credits`, `tags`) would be gone and the row would have moved to the
    /// end of the list. Both are silent.
    #[test]
    fn renaming_a_model_keeps_its_position_and_product_owned_keys() {
        let dir = seed_config_root();
        let models_path = dir.path().join(models_json::MODELS_FILE_NAME);
        std::fs::write(
            &models_path,
            r#"{"models":[
                {"id":"first","url":"https://a.example.com/v1/chat/completions","credits":"included","tags":["fast"]},
                {"id":"second","url":"https://b.example.com/v1/chat/completions"}
            ]}"#,
        )
        .expect("seed");

        let mut document = ModelsDocument::load(dir.path()).expect("load");
        let entry = models_json::build_model_entry(&ModelEntryInput {
            id: "first-renamed",
            name: None,
            vendor: None,
            base_url: "https://a.example.com/v1",
            api_key: None,
            max_input_tokens: None,
            max_output_tokens: None,
            temperature: None,
            supports_tool_call: true,
            supports_images: false,
            supports_reasoning: false,
            only_reasoning: false,
            use_custom_protocol: false,
        });
        document.upsert_model_renaming(entry, Some("first"));
        document.save(dir.path()).expect("save");

        let rows: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&models_path).expect("read"),
        )
        .expect("parse");
        let models = rows
            .get("models")
            .and_then(serde_json::Value::as_array)
            .expect("models");
        assert_eq!(models.len(), 2, "a rename must not add a row");
        assert_eq!(models[0]["id"], "first-renamed", "the edited row keeps its position");
        assert_eq!(models[0]["credits"], "included", "product-owned key survives");
        assert_eq!(models[0]["tags"][0], "fast");
        assert_eq!(models[1]["id"], "second");
    }

    /// The preview command returns the file verbatim, and `None` when it does
    /// not exist — never an empty string, which would read as an empty file.
    #[test]
    fn the_raw_read_reports_a_missing_file_as_none() {
        let dir = seed_config_root();
        let models_path = dir.path().join(models_json::MODELS_FILE_NAME);
        assert!(!models_path.is_file());
        // Mirrors `read_workbuddy_models_file`'s body without needing a Tauri
        // `State`, which the module cannot construct in a unit test.
        let read = |path: &Path| -> Option<String> {
            path.is_file()
                .then(|| std::fs::read_to_string(path).expect("read"))
        };
        assert!(read(&models_path).is_none());

        std::fs::write(&models_path, "[]").expect("seed");
        assert_eq!(read(&models_path).as_deref(), Some("[]"));
    }
}
