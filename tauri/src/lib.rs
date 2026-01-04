#[allow(unused_imports)]
use tauri::Manager;

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use surrealdb::engine::local::{Db, SurrealKv};
use surrealdb::sql::Thing;
use surrealdb::Surreal;
use tokio::sync::Mutex;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

// Database state wrapper
pub struct DbState(pub Arc<Mutex<Surreal<Db>>>);

// Provider - Database record (with Thing id from SurrealDB)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub id: Thing,
    pub provider_id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<String>,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

// Provider - API response (with string id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<String>,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ProviderRecord> for Provider {
    fn from(record: ProviderRecord) -> Self {
        Provider {
            id: record.provider_id,
            name: record.name,
            provider_type: record.provider_type,
            base_url: record.base_url,
            api_key: record.api_key,
            headers: record.headers,
            sort_order: record.sort_order,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

// Provider - Content for create/update (without Thing id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderContent {
    pub provider_id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<String>,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInput {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<String>,
    pub sort_order: i32,
}

// Model - Database record (with Thing id from SurrealDB)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub id: Thing,
    pub model_id: String,
    pub provider_id: String,
    pub name: String,
    pub context_limit: i64,
    pub output_limit: i64,
    pub options: String,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

// Model - API response (with string id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub context_limit: i64,
    pub output_limit: i64,
    pub options: String,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ModelRecord> for Model {
    fn from(record: ModelRecord) -> Self {
        Model {
            id: record.model_id,
            provider_id: record.provider_id,
            name: record.name,
            context_limit: record.context_limit,
            output_limit: record.output_limit,
            options: record.options,
            sort_order: record.sort_order,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

// Model - Content for create/update (without Thing id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelContent {
    pub model_id: String,
    pub provider_id: String,
    pub name: String,
    pub context_limit: i64,
    pub output_limit: i64,
    pub options: String,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInput {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub context_limit: i64,
    pub output_limit: i64,
    pub options: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderWithModels {
    pub provider: Provider,
    pub models: Vec<Model>,
}

// Settings data structures
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebDAVConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct S3Config {
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub region: String,
    pub prefix: String,
    pub endpoint_url: String,
    pub force_path_style: bool,
    pub public_domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub language: String,
    pub current_module: String,
    pub current_sub_tab: String,
    pub backup_type: String,
    pub local_backup_path: String,
    pub webdav: WebDAVConfig,
    pub s3: S3Config,
    pub last_backup_time: Option<String>,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Get the database directory path
fn get_db_path(app_handle: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(app_data_dir.join("database"))
}

/// Get settings from database
#[tauri::command]
async fn get_settings(state: tauri::State<'_, DbState>) -> Result<AppSettings, String> {
    let db = state.0.lock().await;

    let result: Option<AppSettings> = db
        .select(("settings", "app"))
        .await
        .map_err(|e| format!("Failed to get settings: {}", e))?;

    Ok(result.unwrap_or_else(|| AppSettings {
        language: "zh-CN".to_string(),
        current_module: "coding".to_string(),
        current_sub_tab: "opencode".to_string(),
        backup_type: "local".to_string(),
        local_backup_path: String::new(),
        webdav: WebDAVConfig::default(),
        s3: S3Config::default(),
        last_backup_time: None,
    }))
}

/// Save settings to database
#[tauri::command]
async fn save_settings(
    state: tauri::State<'_, DbState>,
    settings: AppSettings,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let _: Option<AppSettings> = db
        .upsert(("settings", "app"))
        .content(settings)
        .await
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    Ok(())
}

/// Backup database to a zip file
#[tauri::command]
async fn backup_database(
    app_handle: tauri::AppHandle,
    backup_path: String,
) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;

    // Ensure database directory exists
    if !db_path.exists() {
        fs::create_dir_all(&db_path)
            .map_err(|e| format!("Failed to create database dir: {}", e))?;
    }

    // Ensure backup directory exists
    let backup_dir = Path::new(&backup_path);
    if !backup_dir.exists() {
        fs::create_dir_all(backup_dir)
            .map_err(|e| format!("Failed to create backup dir: {}", e))?;
    }

    // Generate backup filename with timestamp
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let backup_filename = format!("ai-toolbox-backup-{}.zip", timestamp);
    let backup_file_path = backup_dir.join(&backup_filename);

    // Create zip file
    let file = File::create(&backup_file_path)
        .map_err(|e| format!("Failed to create backup file: {}", e))?;
    let mut zip = ZipWriter::new(file);
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Walk through the database directory and add files to zip
    let mut has_files = false;
    for entry in WalkDir::new(&db_path) {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();
        let relative_path = path
            .strip_prefix(&db_path)
            .map_err(|e| format!("Failed to get relative path: {}", e))?;

        if path.is_file() {
            has_files = true;
            let name = relative_path.to_string_lossy();
            zip.start_file(name.to_string(), options)
                .map_err(|e| format!("Failed to start file in zip: {}", e))?;

            let mut file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)
                .map_err(|e| format!("Failed to read file: {}", e))?;
            zip.write_all(&buffer)
                .map_err(|e| format!("Failed to write to zip: {}", e))?;
        } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
            let name = format!("{}/", relative_path.to_string_lossy());
            zip.add_directory(name, options)
                .map_err(|e| format!("Failed to add directory to zip: {}", e))?;
        }
    }

    // If no files, add a placeholder to ensure valid zip
    if !has_files {
        zip.start_file(".backup_marker", options)
            .map_err(|e| format!("Failed to create marker file: {}", e))?;
        zip.write_all(b"AI Toolbox Backup")
            .map_err(|e| format!("Failed to write marker: {}", e))?;
    }

    zip.finish()
        .map_err(|e| format!("Failed to finish zip: {}", e))?;

    Ok(backup_file_path.to_string_lossy().to_string())
}

/// Restore database from a zip file
#[tauri::command]
async fn restore_database(
    app_handle: tauri::AppHandle,
    zip_file_path: String,
) -> Result<(), String> {
    let db_path = get_db_path(&app_handle)?;
    let zip_path = Path::new(&zip_file_path);

    if !zip_path.exists() {
        return Err("Backup file does not exist".to_string());
    }

    // Open zip file
    let file = File::open(zip_path).map_err(|e| format!("Failed to open backup file: {}", e))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| format!("Failed to read zip archive: {}", e))?;

    // Remove existing database directory
    if db_path.exists() {
        fs::remove_dir_all(&db_path)
            .map_err(|e| format!("Failed to remove existing database: {}", e))?;
    }

    // Create database directory
    fs::create_dir_all(&db_path)
        .map_err(|e| format!("Failed to create database directory: {}", e))?;

    // Extract zip contents
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {}", e))?;

        // Skip the backup marker file
        if file.name() == ".backup_marker" {
            continue;
        }

        let outpath = db_path.join(file.name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                }
            }
            let mut outfile =
                File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }
    }

    Ok(())
}

/// Get database directory path for frontend
#[tauri::command]
fn get_database_path(app_handle: tauri::AppHandle) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;
    Ok(db_path.to_string_lossy().to_string())
}

/// Create a temporary backup zip file and return its contents as bytes
fn create_backup_zip(db_path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    let mut buffer = Cursor::new(Vec::new());

    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let mut has_files = false;
        for entry in WalkDir::new(db_path) {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let path = entry.path();
            let relative_path = path
                .strip_prefix(db_path)
                .map_err(|e| format!("Failed to get relative path: {}", e))?;

            if path.is_file() {
                has_files = true;
                let name = relative_path.to_string_lossy();
                zip.start_file(name.to_string(), options)
                    .map_err(|e| format!("Failed to start file in zip: {}", e))?;

                let mut file =
                    File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
                let mut file_buffer = Vec::new();
                file.read_to_end(&mut file_buffer)
                    .map_err(|e| format!("Failed to read file: {}", e))?;
                zip.write_all(&file_buffer)
                    .map_err(|e| format!("Failed to write to zip: {}", e))?;
            } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
                let name = format!("{}/", relative_path.to_string_lossy());
                zip.add_directory(name, options)
                    .map_err(|e| format!("Failed to add directory to zip: {}", e))?;
            }
        }

        if !has_files {
            zip.start_file(".backup_marker", options)
                .map_err(|e| format!("Failed to create marker file: {}", e))?;
            zip.write_all(b"AI Toolbox Backup")
                .map_err(|e| format!("Failed to write marker: {}", e))?;
        }

        zip.finish()
            .map_err(|e| format!("Failed to finish zip: {}", e))?;
    }

    Ok(buffer.into_inner())
}

/// Backup database to WebDAV server
#[tauri::command]
async fn backup_to_webdav(
    app_handle: tauri::AppHandle,
    url: String,
    username: String,
    password: String,
    remote_path: String,
) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;

    // Ensure database directory exists
    if !db_path.exists() {
        fs::create_dir_all(&db_path)
            .map_err(|e| format!("Failed to create database dir: {}", e))?;
    }

    // Create backup zip in memory
    let zip_data = create_backup_zip(&db_path)?;

    // Generate backup filename with timestamp
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let backup_filename = format!("ai-toolbox-backup-{}.zip", timestamp);

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, backup_filename)
    } else {
        format!("{}/{}/{}", base_url, remote, backup_filename)
    };

    // Upload to WebDAV using PUT request
    let client = reqwest::Client::new();
    let response = client
        .put(&full_url)
        .basic_auth(&username, Some(&password))
        .body(zip_data)
        .send()
        .await
        .map_err(|e| format!("Failed to upload to WebDAV: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "WebDAV upload failed with status: {}",
            response.status()
        ));
    }

    Ok(full_url)
}

/// List backup files from WebDAV server
#[tauri::command]
async fn list_webdav_backups(
    url: String,
    username: String,
    password: String,
    remote_path: String,
) -> Result<Vec<String>, String> {
    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let folder_url = if remote.is_empty() {
        format!("{}/", base_url)
    } else {
        format!("{}/{}/", base_url, remote)
    };

    // Send PROPFIND request to list files
    let client = reqwest::Client::new();
    let response = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &folder_url)
        .basic_auth(&username, Some(&password))
        .header("Depth", "1")
        .send()
        .await
        .map_err(|e| format!("Failed to list WebDAV files: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "WebDAV list failed with status: {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Parse XML response to extract backup files
    // WebDAV returns XML like: <D:href>/path/to/ai-toolbox-backup-20250101-120000.zip</D:href>
    // Use regex to extract filenames from href tags
    use regex::Regex;
    let re = Regex::new(r"ai-toolbox-backup-\d{8}-\d{6}\.zip").unwrap();
    
    let mut backups = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for cap in re.find_iter(&body) {
        let filename = cap.as_str();
        if seen.insert(filename.to_string()) {
            backups.push(filename.to_string());
        }
    }

    backups.sort();
    backups.reverse(); // Most recent first
    Ok(backups)
}

/// Restore database from WebDAV server
#[tauri::command]
async fn restore_from_webdav(
    app_handle: tauri::AppHandle,
    url: String,
    username: String,
    password: String,
    remote_path: String,
    filename: String,
) -> Result<(), String> {
    let db_path = get_db_path(&app_handle)?;

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, filename)
    } else {
        format!("{}/{}/{}", base_url, remote, filename)
    };

    // Download from WebDAV
    let client = reqwest::Client::new();
    let response = client
        .get(&full_url)
        .basic_auth(&username, Some(&password))
        .send()
        .await
        .map_err(|e| format!("Failed to download from WebDAV: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "WebDAV download failed with status: {}",
            response.status()
        ));
    }

    let zip_data = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Remove existing database directory
    if db_path.exists() {
        fs::remove_dir_all(&db_path)
            .map_err(|e| format!("Failed to remove existing database: {}", e))?;
    }

    // Create database directory
    fs::create_dir_all(&db_path)
        .map_err(|e| format!("Failed to create database directory: {}", e))?;

    // Extract zip contents
    let cursor = std::io::Cursor::new(zip_data);
    let mut archive =
        ZipArchive::new(cursor).map_err(|e| format!("Failed to read zip archive: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {}", e))?;

        if file.name() == ".backup_marker" {
            continue;
        }

        let outpath = db_path.join(file.name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                }
            }
            let mut outfile =
                File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }
    }

    Ok(())
}

// ============================================================================
// Provider Management Commands
// ============================================================================

/// List all providers ordered by sort_order
#[tauri::command]
async fn list_providers(state: tauri::State<'_, DbState>) -> Result<Vec<Provider>, String> {
    let db = state.0.lock().await;
    
    let records: Vec<ProviderRecord> = db
        .select("provider")
        .await
        .map_err(|e| format!("Failed to list providers: {}", e))?;
    
    let mut result: Vec<Provider> = records.into_iter().map(Provider::from).collect();
    result.sort_by_key(|p| p.sort_order);
    Ok(result)
}

/// Create a new provider
#[tauri::command]
async fn create_provider(
    state: tauri::State<'_, DbState>,
    provider: ProviderInput,
) -> Result<Provider, String> {
    let db = state.0.lock().await;
    
    // Check if ID already exists
    let existing: Option<ProviderRecord> = db
        .select(("provider", &provider.id))
        .await
        .map_err(|e| format!("Failed to check provider existence: {}", e))?;
    
    if existing.is_some() {
        return Err(format!("Provider with ID '{}' already exists", provider.id));
    }
    
    // Set timestamps
    let now = Local::now().to_rfc3339();
    let content = ProviderContent {
        provider_id: provider.id.clone(),
        name: provider.name,
        provider_type: provider.provider_type,
        base_url: provider.base_url,
        api_key: provider.api_key,
        headers: provider.headers,
        sort_order: provider.sort_order,
        created_at: now.clone(),
        updated_at: now,
    };
    
    // Create provider
    let created: Option<ProviderRecord> = db
        .create(("provider", &provider.id))
        .content(content)
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;
    
    created
        .map(Provider::from)
        .ok_or_else(|| "Failed to create provider".to_string())
}

/// Update an existing provider
#[tauri::command]
async fn update_provider(
    state: tauri::State<'_, DbState>,
    provider: Provider,
) -> Result<Provider, String> {
    let db = state.0.lock().await;
    
    // Update timestamp
    let now = Local::now().to_rfc3339();
    let content = ProviderContent {
        provider_id: provider.id.clone(),
        name: provider.name,
        provider_type: provider.provider_type,
        base_url: provider.base_url,
        api_key: provider.api_key,
        headers: provider.headers,
        sort_order: provider.sort_order,
        created_at: provider.created_at,
        updated_at: now,
    };
    
    // Update provider
    let updated: Option<ProviderRecord> = db
        .update(("provider", &provider.id))
        .content(content)
        .await
        .map_err(|e| format!("Failed to update provider: {}", e))?;
    
    updated
        .map(Provider::from)
        .ok_or_else(|| "Provider not found".to_string())
}

/// Delete a provider and its associated models
#[tauri::command]
async fn delete_provider(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    // Delete all models associated with this provider
    let models: Vec<ModelRecord> = db
        .select("model")
        .await
        .map_err(|e| format!("Failed to query models: {}", e))?;
    
    for model in models {
        if model.provider_id == id {
            let _: Option<ModelRecord> = db
                .delete(("model", &format!("{}:{}", model.provider_id, model.model_id)))
                .await
                .map_err(|e| format!("Failed to delete model: {}", e))?;
        }
    }
    
    // Delete provider
    let _: Option<ProviderRecord> = db
        .delete(("provider", &id))
        .await
        .map_err(|e| format!("Failed to delete provider: {}", e))?;
    
    Ok(())
}

/// Reorder providers
#[tauri::command]
async fn reorder_providers(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    for (index, id) in ids.iter().enumerate() {
        let record: Option<ProviderRecord> = db
            .select(("provider", id))
            .await
            .map_err(|e| format!("Failed to get provider: {}", e))?;
        
        if let Some(r) = record {
            let content = ProviderContent {
                provider_id: r.provider_id,
                name: r.name,
                provider_type: r.provider_type,
                base_url: r.base_url,
                api_key: r.api_key,
                headers: r.headers,
                sort_order: index as i32,
                created_at: r.created_at,
                updated_at: Local::now().to_rfc3339(),
            };
            
            let _: Option<ProviderRecord> = db
                .update(("provider", id))
                .content(content)
                .await
                .map_err(|e| format!("Failed to update provider order: {}", e))?;
        }
    }
    
    Ok(())
}

// ============================================================================
// Model Management Commands
// ============================================================================

/// List models for a specific provider ordered by sort_order
#[tauri::command(rename_all = "snake_case")]
async fn list_models(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<Vec<Model>, String> {
    let db = state.0.lock().await;
    
    let all_records: Vec<ModelRecord> = db
        .select("model")
        .await
        .map_err(|e| format!("Failed to list models: {}", e))?;
    
    let mut filtered: Vec<Model> = all_records
        .into_iter()
        .filter(|m| m.provider_id == provider_id)
        .map(Model::from)
        .collect();
    
    filtered.sort_by_key(|m| m.sort_order);
    Ok(filtered)
}

/// Create a new model
#[tauri::command]
async fn create_model(
    state: tauri::State<'_, DbState>,
    model: ModelInput,
) -> Result<Model, String> {
    let db = state.0.lock().await;
    
    // Check if model ID already exists under this provider
    let record_id = format!("{}:{}", model.provider_id, model.id);
    let existing: Option<ModelRecord> = db
        .select(("model", record_id.as_str()))
        .await
        .map_err(|e| format!("Failed to check model existence: {}", e))?;
    
    if existing.is_some() {
        return Err(format!(
            "Model with ID '{}' already exists under provider '{}'",
            model.id, model.provider_id
        ));
    }
    
    // Set timestamps
    let now = Local::now().to_rfc3339();
    let content = ModelContent {
        model_id: model.id.clone(),
        provider_id: model.provider_id,
        name: model.name,
        context_limit: model.context_limit,
        output_limit: model.output_limit,
        options: model.options,
        sort_order: model.sort_order,
        created_at: now.clone(),
        updated_at: now,
    };
    
    // Create model
    let created: Option<ModelRecord> = db
        .create(("model", record_id.as_str()))
        .content(content)
        .await
        .map_err(|e| format!("Failed to create model: {}", e))?;
    
    created
        .map(Model::from)
        .ok_or_else(|| "Failed to create model".to_string())
}

/// Update an existing model
#[tauri::command]
async fn update_model(
    state: tauri::State<'_, DbState>,
    model: Model,
) -> Result<Model, String> {
    let db = state.0.lock().await;
    
    let record_id = format!("{}:{}", model.provider_id, model.id);
    
    // Update timestamp
    let now = Local::now().to_rfc3339();
    let content = ModelContent {
        model_id: model.id,
        provider_id: model.provider_id,
        name: model.name,
        context_limit: model.context_limit,
        output_limit: model.output_limit,
        options: model.options,
        sort_order: model.sort_order,
        created_at: model.created_at,
        updated_at: now,
    };
    
    // Update model
    let updated: Option<ModelRecord> = db
        .update(("model", record_id.as_str()))
        .content(content)
        .await
        .map_err(|e| format!("Failed to update model: {}", e))?;
    
    updated
        .map(Model::from)
        .ok_or_else(|| "Model not found".to_string())
}

/// Delete a model
#[tauri::command(rename_all = "snake_case")]
async fn delete_model(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    let record_id = format!("{}:{}", provider_id, id);
    
    let _: Option<ModelRecord> = db
        .delete(("model", record_id.as_str()))
        .await
        .map_err(|e| format!("Failed to delete model: {}", e))?;
    
    Ok(())
}

/// Reorder models for a specific provider
#[tauri::command(rename_all = "snake_case")]
async fn reorder_models(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    for (index, id) in ids.iter().enumerate() {
        let record_id = format!("{}:{}", provider_id, id);
        let record: Option<ModelRecord> = db
            .select(("model", record_id.as_str()))
            .await
            .map_err(|e| format!("Failed to get model: {}", e))?;
        
        if let Some(r) = record {
            let content = ModelContent {
                model_id: r.model_id,
                provider_id: r.provider_id,
                name: r.name,
                context_limit: r.context_limit,
                output_limit: r.output_limit,
                options: r.options,
                sort_order: index as i32,
                created_at: r.created_at,
                updated_at: Local::now().to_rfc3339(),
            };
            
            let _: Option<ModelRecord> = db
                .update(("model", record_id.as_str()))
                .content(content)
                .await
                .map_err(|e| format!("Failed to update model order: {}", e))?;
        }
    }
    
    Ok(())
}

/// Get all providers with their models
#[tauri::command]
async fn get_all_providers_with_models(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<ProviderWithModels>, String> {
    let db = state.0.lock().await;
    
    // Get all providers
    let provider_records: Vec<ProviderRecord> = db
        .select("provider")
        .await
        .map_err(|e| format!("Failed to list providers: {}", e))?;
    
    let mut providers: Vec<Provider> = provider_records.into_iter().map(Provider::from).collect();
    providers.sort_by_key(|p| p.sort_order);
    
    // Get all models
    let model_records: Vec<ModelRecord> = db
        .select("model")
        .await
        .map_err(|e| format!("Failed to list models: {}", e))?;
    
    let all_models: Vec<Model> = model_records.into_iter().map(Model::from).collect();
    
    // Build result
    let mut result = Vec::new();
    for provider in providers {
        let mut models: Vec<Model> = all_models
            .iter()
            .filter(|m| m.provider_id == provider.id)
            .cloned()
            .collect();
        
        models.sort_by_key(|m| m.sort_order);
        
        result.push(ProviderWithModels { provider, models });
    }
    
    Ok(result)
}

// ============================================================================
// ClaudeCode Configuration Management
// ============================================================================

// ClaudeCodeProvider - Database record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProviderRecord {
    pub id: Thing,
    #[serde(alias = "providerId", alias = "provider_id", rename = "providerId")]
    pub provider_id: String,
    pub name: String,
    pub category: String,
    #[serde(alias = "settingsConfig", alias = "settings_config", rename = "settingsConfig")]
    pub settings_config: String,
    #[serde(alias = "sourceProviderId", alias = "source_provider_id", rename = "sourceProviderId", skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(alias = "websiteUrl", alias = "website_url", rename = "websiteUrl", skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(alias = "iconColor", alias = "icon_color", rename = "iconColor", skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(alias = "sortIndex", alias = "sort_index", rename = "sortIndex", skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(alias = "isCurrent", alias = "is_current", rename = "isCurrent")]
    pub is_current: bool,
    #[serde(alias = "isApplied", alias = "is_applied", rename = "isApplied")]
    pub is_applied: bool,
    #[serde(alias = "createdAt", alias = "created_at", rename = "createdAt")]
    pub created_at: String,
    #[serde(alias = "updatedAt", alias = "updated_at", rename = "updatedAt")]
    pub updated_at: String,
}

// ClaudeCodeProvider - API response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProvider {
    pub id: String,
    pub name: String,
    pub category: String,
    #[serde(alias = "settings_config", alias = "settingsConfig", rename = "settingsConfig")]
    pub settings_config: String,
    #[serde(alias = "source_provider_id", alias = "sourceProviderId", skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(alias = "website_url", alias = "websiteUrl", skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(alias = "icon_color", alias = "iconColor", skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(alias = "sort_index", alias = "sortIndex", rename = "sortIndex", skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(alias = "is_current", alias = "isCurrent", rename = "isCurrent")]
    pub is_current: bool,
    #[serde(alias = "is_applied", alias = "isApplied", rename = "isApplied")]
    pub is_applied: bool,
    #[serde(alias = "created_at", alias = "createdAt", rename = "createdAt")]
    pub created_at: String,
    #[serde(alias = "updated_at", alias = "updatedAt", rename = "updatedAt")]
    pub updated_at: String,
}

impl From<ClaudeCodeProviderRecord> for ClaudeCodeProvider {
    fn from(record: ClaudeCodeProviderRecord) -> Self {
        ClaudeCodeProvider {
            id: record.provider_id,
            name: record.name,
            category: record.category,
            settings_config: record.settings_config,
            source_provider_id: record.source_provider_id,
            website_url: record.website_url,
            notes: record.notes,
            icon: record.icon,
            icon_color: record.icon_color,
            sort_index: record.sort_index,
            is_current: record.is_current,
            is_applied: record.is_applied,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

// ClaudeCodeProvider - Content for create/update (Database storage)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProviderContent {
    pub provider_id: String,
    pub name: String,
    pub category: String,
    pub settings_config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(alias = "isCurrent", rename = "isCurrent")]
    pub is_current: bool,
    #[serde(alias = "isApplied", rename = "isApplied")]
    pub is_applied: bool,
    #[serde(alias = "createdAt", rename = "createdAt")]
    pub created_at: String,
    #[serde(alias = "updatedAt", rename = "updatedAt")]
    pub updated_at: String,
}

// ClaudeCodeProvider - Input from frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProviderInput {
    pub id: String,
    pub name: String,
    pub category: String,
    #[serde(alias = "settings_config", rename = "settingsConfig")]
    pub settings_config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(alias = "isCurrent", rename = "isCurrent")]
    pub is_current: bool,
    #[serde(alias = "isApplied", rename = "isApplied")]
    pub is_applied: bool,
    #[serde(alias = "createdAt", rename = "createdAt")]
    pub created_at: String,
    #[serde(alias = "updatedAt", rename = "updatedAt")]
    pub updated_at: String,
}

// ClaudeCommonConfig - Database record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCommonConfigRecord {
    pub id: Thing,
    pub config: String,
    pub updated_at: String,
}

// ClaudeCommonConfig - API response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCommonConfig {
    pub config: String,
    pub updated_at: String,
}

impl From<ClaudeCommonConfigRecord> for ClaudeCommonConfig {
    fn from(record: ClaudeCommonConfigRecord) -> Self {
        ClaudeCommonConfig {
            config: record.config,
            updated_at: record.updated_at,
        }
    }
}

// Claude settings.json structure (for reading/writing config file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<serde_json::Value>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

/// List all Claude Code providers ordered by sort_index
#[tauri::command]
async fn list_claude_providers(state: tauri::State<'_, DbState>) -> Result<Vec<ClaudeCodeProvider>, String> {
    let db = state.0.lock().await;
    
    let records: Vec<ClaudeCodeProviderRecord> = db
        .select("claude_provider")
        .await
        .map_err(|e| format!("Failed to list claude providers: {}", e))?;
    
    let mut result: Vec<ClaudeCodeProvider> = records.into_iter().map(ClaudeCodeProvider::from).collect();
    result.sort_by_key(|p| p.sort_index.unwrap_or(0));
    Ok(result)
}

/// Create a new Claude Code provider
#[tauri::command]
async fn create_claude_provider(
    state: tauri::State<'_, DbState>,
    provider: ClaudeCodeProviderInput,
) -> Result<ClaudeCodeProvider, String> {
    let db = state.0.lock().await;
    
    // Check if ID already exists
    let existing: Option<ClaudeCodeProviderRecord> = db
        .select(("claude_provider", &provider.id))
        .await
        .map_err(|e| format!("Failed to check provider existence: {}", e))?;
    
    if existing.is_some() {
        return Err(format!("Claude provider with ID '{}' already exists", provider.id));
    }
    
    let now = Local::now().to_rfc3339();
    let content = ClaudeCodeProviderContent {
        provider_id: provider.id.clone(),
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        is_current: false,
        is_applied: false,
        created_at: now.clone(),
        updated_at: now,
    };
    
    let created: Option<ClaudeCodeProviderRecord> = db
        .create(("claude_provider", &provider.id))
        .content(content)
        .await
        .map_err(|e| format!("Failed to create claude provider: {}", e))?;
    
    created
        .map(ClaudeCodeProvider::from)
        .ok_or_else(|| "Failed to create claude provider".to_string())
}

/// Update an existing Claude Code provider
#[tauri::command]
async fn update_claude_provider(
    state: tauri::State<'_, DbState>,
    provider: ClaudeCodeProvider,
) -> Result<ClaudeCodeProvider, String> {
    let db = state.0.lock().await;
    
    let now = Local::now().to_rfc3339();
    let content = ClaudeCodeProviderContent {
        provider_id: provider.id.clone(),
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        is_current: provider.is_current,
        is_applied: provider.is_applied,
        created_at: provider.created_at,
        updated_at: now,
    };
    
    let updated: Option<ClaudeCodeProviderRecord> = db
        .update(("claude_provider", &provider.id))
        .content(content)
        .await
        .map_err(|e| format!("Failed to update claude provider: {}", e))?;
    
    updated
        .map(ClaudeCodeProvider::from)
        .ok_or_else(|| "Claude provider not found".to_string())
}

/// Delete a Claude Code provider
#[tauri::command]
async fn delete_claude_provider(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    let _: Option<ClaudeCodeProviderRecord> = db
        .delete(("claude_provider", &id))
        .await
        .map_err(|e| format!("Failed to delete claude provider: {}", e))?;
    
    Ok(())
}

/// Select a Claude Code provider as current (deselect others)
#[tauri::command]
async fn select_claude_provider(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let records: Vec<ClaudeCodeProviderRecord> = db
        .select("claude_provider")
        .await
        .map_err(|e| format!("Failed to list providers: {}", e))?;

    for record in records {
        let is_selected = record.provider_id == id;

        let content = ClaudeCodeProviderContent {
            provider_id: record.provider_id.clone(),
            name: record.name,
            category: record.category,
            settings_config: record.settings_config,
            source_provider_id: record.source_provider_id,
            website_url: record.website_url,
            notes: record.notes,
            icon: record.icon,
            icon_color: record.icon_color,
            sort_index: record.sort_index,
            is_current: is_selected,
            is_applied: record.is_applied,
            created_at: record.created_at,
            updated_at: Local::now().to_rfc3339(),
        };

        let thing_id = record.provider_id.clone();
        let _: Option<ClaudeCodeProviderRecord> = db
            .update(("claude_provider", thing_id))
            .content(content)
            .await
            .map_err(|e| format!("Failed to update provider: {}", e))?;
    }

    Ok(())
}

/// Reorder Claude Code providers
#[tauri::command]
async fn reorder_claude_providers(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    for (index, id) in ids.iter().enumerate() {
        let record: Option<ClaudeCodeProviderRecord> = db
            .select(("claude_provider", id))
            .await
            .map_err(|e| format!("Failed to get provider: {}", e))?;
        
        if let Some(r) = record {
            let content = ClaudeCodeProviderContent {
                provider_id: r.provider_id,
                name: r.name,
                category: r.category,
                settings_config: r.settings_config,
                source_provider_id: r.source_provider_id,
                website_url: r.website_url,
                notes: r.notes,
                icon: r.icon,
                icon_color: r.icon_color,
                sort_index: Some(index as i32),
                is_current: r.is_current,
                is_applied: r.is_applied,
                created_at: r.created_at,
                updated_at: Local::now().to_rfc3339(),
            };
            
            let _: Option<ClaudeCodeProviderRecord> = db
                .update(("claude_provider", id))
                .content(content)
                .await
                .map_err(|e| format!("Failed to update provider order: {}", e))?;
        }
    }
    
    Ok(())
}

/// Get Claude config file path (~/.claude/settings.json)
#[tauri::command]
fn get_claude_config_path() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;
    
    let config_path = Path::new(&home_dir).join(".claude").join("settings.json");
    Ok(config_path.to_string_lossy().to_string())
}

/// Reveal Claude config folder in file explorer
#[tauri::command]
fn reveal_claude_config_folder() -> Result<(), String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;
    
    let config_dir = Path::new(&home_dir).join(".claude");
    
    // Ensure directory exists
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create .claude directory: {}", e))?;
    }
    
    // Open in file explorer (platform-specific)
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    
    Ok(())
}

/// Read Claude settings.json file
#[tauri::command]
async fn read_claude_settings() -> Result<ClaudeSettings, String> {
    let config_path_str = get_claude_config_path()?;
    let config_path = Path::new(&config_path_str);
    
    if !config_path.exists() {
        // Return empty settings if file doesn't exist
        return Ok(ClaudeSettings {
            env: None,
            other: serde_json::Map::new(),
        });
    }
    
    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;
    
    let settings: ClaudeSettings = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings file: {}", e))?;
    
    Ok(settings)
}

/// Apply Claude Code provider configuration to settings.json
#[tauri::command]
async fn apply_claude_config(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    // Get the provider
    let provider: Option<ClaudeCodeProviderRecord> = db
        .select(("claude_provider", &provider_id))
        .await
        .map_err(|e| format!("Failed to get provider: {}", e))?;
    
    let provider = provider.ok_or_else(|| "Provider not found".to_string())?;
    
    // Parse provider settings_config
    let provider_config: serde_json::Value = serde_json::from_str(&provider.settings_config)
        .map_err(|e| format!("Failed to parse provider config: {}", e))?;
    
    // Get common config
    let common_config_record: Option<ClaudeCommonConfigRecord> = db
        .select(("claude_common_config", "common"))
        .await
        .map_err(|e| format!("Failed to get common config: {}", e))?;
    
    let common_config: serde_json::Value = if let Some(record) = common_config_record {
        serde_json::from_str(&record.config)
            .map_err(|e| format!("Failed to parse common config: {}", e))?
    } else {
        serde_json::json!({})
    };
    
    // Build env section from provider config
    let mut env = serde_json::Map::new();
    
    // Get env section from provider config
    if let Some(env_config) = provider_config.get("env").and_then(|v| v.as_object()) {
        if let Some(api_key) = env_config.get("ANTHROPIC_API_KEY").and_then(|v| v.as_str()) {
            env.insert("ANTHROPIC_API_KEY".to_string(), serde_json::json!(api_key));
        }
        
        if let Some(base_url) = env_config.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()) {
            env.insert("ANTHROPIC_BASE_URL".to_string(), serde_json::json!(base_url));
        }
        
        if let Some(auth_token) = env_config.get("ANTHROPIC_AUTH_TOKEN").and_then(|v| v.as_str()) {
            env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), serde_json::json!(auth_token));
        }
    }
    
    if let Some(model) = provider_config.get("model").and_then(|v| v.as_str()) {
        env.insert("ANTHROPIC_MODEL".to_string(), serde_json::json!(model));
    }
    
    if let Some(haiku) = provider_config.get("haikuModel").and_then(|v| v.as_str()) {
        env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(), serde_json::json!(haiku));
    }
    
    if let Some(sonnet) = provider_config.get("sonnetModel").and_then(|v| v.as_str()) {
        env.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(), serde_json::json!(sonnet));
    }
    
    if let Some(opus) = provider_config.get("opusModel").and_then(|v| v.as_str()) {
        env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), serde_json::json!(opus));
    }
    
    // Merge common config into final settings
    let mut final_settings = if let serde_json::Value::Object(map) = common_config {
        map
    } else {
        serde_json::Map::new()
    };
    
    final_settings.insert("env".to_string(), serde_json::json!(env));
    
    // Write to settings.json
    let config_path_str = get_claude_config_path()?;
    let config_path = Path::new(&config_path_str);
    
    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create .claude directory: {}", e))?;
        }
    }
    
    let json_content = serde_json::to_string_pretty(&final_settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    
    fs::write(config_path, json_content)
        .map_err(|e| format!("Failed to write settings file: {}", e))?;
    
    // Update provider's is_applied status
    let all_providers: Vec<ClaudeCodeProviderRecord> = db
        .select("claude_provider")
        .await
        .map_err(|e| format!("Failed to list providers: {}", e))?;
    
    for p in all_providers.iter() {
        let content = ClaudeCodeProviderContent {
            provider_id: p.provider_id.clone(),
            name: p.name.clone(),
            category: p.category.clone(),
            settings_config: p.settings_config.clone(),
            source_provider_id: p.source_provider_id.clone(),
            website_url: p.website_url.clone(),
            notes: p.notes.clone(),
            icon: p.icon.clone(),
            icon_color: p.icon_color.clone(),
            sort_index: p.sort_index,
            is_current: p.is_current,
            is_applied: p.provider_id == provider_id,
            created_at: p.created_at.clone(),
            updated_at: Local::now().to_rfc3339(),
        };
        
        let _: Option<ClaudeCodeProviderRecord> = db
            .update(("claude_provider", &p.provider_id))
            .content(content)
            .await
            .map_err(|e| format!("Failed to update provider: {}", e))?;
    }
    
    Ok(())
}

/// Get Claude common config
#[tauri::command]
async fn get_claude_common_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<ClaudeCommonConfig>, String> {
    let db = state.0.lock().await;
    
    let record: Option<ClaudeCommonConfigRecord> = db
        .select(("claude_common_config", "common"))
        .await
        .map_err(|e| format!("Failed to get common config: {}", e))?;
    
    Ok(record.map(ClaudeCommonConfig::from))
}

/// Save Claude common config
#[tauri::command]
async fn save_claude_common_config(
    state: tauri::State<'_, DbState>,
    config: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    
    // Validate JSON
    let _: serde_json::Value = serde_json::from_str(&config)
        .map_err(|e| format!("Invalid JSON: {}", e))?;
    
    let now = Local::now().to_rfc3339();
    
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct CommonConfigContent {
        config: String,
        updated_at: String,
    }
    
    let content = CommonConfigContent {
        config,
        updated_at: now,
    };
    
    let _: Option<ClaudeCommonConfigRecord> = db
        .upsert(("claude_common_config", "common"))
        .content(content)
        .await
        .map_err(|e| format!("Failed to save common config: {}", e))?;
    
    Ok(())
}

// ============================================================================
// OpenCode Configuration Management
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeModelLimit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<OpenCodeModelLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeProviderOptions {
    #[serde(rename = "baseURL")]
    pub base_url: String,
    #[serde(rename = "apiKey", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeProvider {
    pub npm: String,
    pub name: String,
    pub options: OpenCodeProviderOptions,
    pub models: std::collections::HashMap<String, OpenCodeModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeConfig {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub provider: std::collections::HashMap<String, OpenCodeProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "small_model", skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
}

/// Get OpenCode config file path
/// Priority: ~/.config/opencode/opencode.json(c)
#[tauri::command]
fn get_opencode_config_path() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;
    
    let config_dir = Path::new(&home_dir).join(".config").join("opencode");
    
    // Check for .json first, then .jsonc
    let json_path = config_dir.join("opencode.json");
    let jsonc_path = config_dir.join("opencode.jsonc");
    
    if json_path.exists() {
        Ok(json_path.to_string_lossy().to_string())
    } else if jsonc_path.exists() {
        Ok(jsonc_path.to_string_lossy().to_string())
    } else {
        // Return default path for new file
        Ok(json_path.to_string_lossy().to_string())
    }
}

/// Read OpenCode configuration file
#[tauri::command]
async fn read_opencode_config() -> Result<Option<OpenCodeConfig>, String> {
    let config_path_str = get_opencode_config_path()?;
    let config_path = Path::new(&config_path_str);
    
    if !config_path.exists() {
        return Ok(None);
    }
    
    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    
    // Strip JSONC comments for parsing
    let json_content = strip_jsonc_comments(&content);
    
    let config: OpenCodeConfig = serde_json::from_str(&json_content)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;
    
    Ok(Some(config))
}

/// Save OpenCode configuration file
#[tauri::command]
async fn save_opencode_config(config: OpenCodeConfig) -> Result<(), String> {
    let config_path_str = get_opencode_config_path()?;
    let config_path = Path::new(&config_path_str);
    
    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
    }
    
    // Serialize with pretty printing
    let json_content = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    
    fs::write(config_path, json_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;
    
    Ok(())
}

/// Strip JSONC comments (simple implementation)
fn strip_jsonc_comments(content: &str) -> String {
    let mut result = String::new();
    let mut in_string = false;
    let mut escape_next = false;
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    
    while i < chars.len() {
        let ch = chars[i];
        
        if escape_next {
            result.push(ch);
            escape_next = false;
            i += 1;
            continue;
        }
        
        if ch == '\\' && in_string {
            result.push(ch);
            escape_next = true;
            i += 1;
            continue;
        }
        
        if ch == '"' {
            in_string = !in_string;
            result.push(ch);
            i += 1;
            continue;
        }
        
        if !in_string {
            // Check for // comments
            if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                // Skip until end of line
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            
            // Check for /* */ comments
            if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                i += 2;
                while i + 1 < chars.len() {
                    if chars[i] == '*' && chars[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        
        result.push(ch);
        i += 1;
    }
    
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Create app data directory
            let app_data_dir = app_handle
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");

            if !app_data_dir.exists() {
                fs::create_dir_all(&app_data_dir).expect("Failed to create app data dir");
            }

            let db_path = app_data_dir.join("database");

            // Initialize SurrealDB
            tauri::async_runtime::block_on(async {
                let db = Surreal::new::<SurrealKv>(db_path)
                    .await
                    .expect("Failed to initialize SurrealDB");

                db.use_ns("ai_toolbox")
                    .use_db("main")
                    .await
                    .expect("Failed to select namespace and database");

                app.manage(DbState(Arc::new(Mutex::new(db))));
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_settings,
            save_settings,
            backup_database,
            restore_database,
            get_database_path,
            backup_to_webdav,
            list_webdav_backups,
            restore_from_webdav,
            list_providers,
            create_provider,
            update_provider,
            delete_provider,
            reorder_providers,
            list_models,
            create_model,
            update_model,
            delete_model,
            reorder_models,
            get_all_providers_with_models,
            list_claude_providers,
            create_claude_provider,
            update_claude_provider,
            delete_claude_provider,
            select_claude_provider,
            reorder_claude_providers,
            get_claude_config_path,
            reveal_claude_config_folder,
            read_claude_settings,
            apply_claude_config,
            get_claude_common_config,
            save_claude_common_config,
            get_opencode_config_path,
            read_opencode_config,
            save_opencode_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
