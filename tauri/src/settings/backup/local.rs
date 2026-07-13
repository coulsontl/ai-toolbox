use chrono::Local;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tauri::Manager;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::utils::{
    get_claude_mcp_restore_path, get_claude_restore_dir, get_codex_restore_dir, get_db_path,
    get_gemini_cli_restore_dir, get_grok_restore_dir, get_image_assets_dir,
    get_opencode_auth_restore_path, get_opencode_restore_dir, get_skills_dir,
    harden_restored_sensitive_file, push_restore_warning, read_root_dir_override,
    resolve_external_config_restore_output_path, resolve_restore_dir_override,
    resolve_skills_restore_output_path, restore_claude_external_config_file,
    restore_custom_backup_entries, restore_sqlite_database_snapshot_from_zip,
    sanitize_restored_claude_database_for_current_os, should_filter_external_config_entry,
    write_backup_zip_contents, RestoreResult,
};
use crate::db::SqliteDbState;
use crate::settings::store;
use crate::settings::types::default_backup_file_filter_rules;

fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

#[cfg(unix)]
fn set_pi_auth_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn set_pi_auth_file_permissions(_path: &Path) {}

/// Backup database to a zip file
#[tauri::command]
pub async fn backup_database(
    app_handle: tauri::AppHandle,
    backup_path: String,
) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;
    let sqlite_state = app_handle.state::<SqliteDbState>();
    let settings = store::load_settings_from_sqlite_state(&sqlite_state)?;
    let backup_image_assets_enabled = settings.backup_image_assets_enabled;
    let filter_rules = settings.backup_file_filter_rules.clone();

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

    let file = File::create(&backup_file_path)
        .map_err(|e| format!("Failed to create backup file: {}", e))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    write_backup_zip_contents(
        &mut zip,
        &app_handle,
        &db_path,
        backup_image_assets_enabled,
        &filter_rules,
        options,
    )
    .await?;
    zip.finish()
        .map_err(|e| format!("Failed to finish zip: {}", e))?;

    Ok(backup_file_path.to_string_lossy().to_string())
}

/// Restore database from a zip file
#[tauri::command]
pub async fn restore_database(
    app_handle: tauri::AppHandle,
    zip_file_path: String,
) -> Result<RestoreResult, String> {
    let db_path = get_db_path(&app_handle)?;
    let zip_path = Path::new(&zip_file_path);

    if !zip_path.exists() {
        return Err("Backup file does not exist".to_string());
    }

    // Open zip file
    let file = File::open(zip_path).map_err(|e| format!("Failed to open backup file: {}", e))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| format!("Failed to read zip archive: {}", e))?;

    // Check if this is a new format backup (with db/ prefix) or old format
    let is_new_format = (0..archive.len()).any(|i| {
        archive
            .by_index(i)
            .map(|f| f.name().starts_with("db/"))
            .unwrap_or(false)
    });

    // Use the currently configured rules for this restore operation so excluded
    // local tool paths are not overwritten by older backup settings.
    let filter_rules = {
        let sqlite_state = app_handle.state::<SqliteDbState>();
        store::load_settings_from_sqlite_state(&sqlite_state)
            .map(|settings| settings.backup_file_filter_rules)
            .unwrap_or_else(|_| default_backup_file_filter_rules())
    };

    let restored_sqlite = restore_sqlite_database_snapshot_from_zip(&mut archive, &app_handle)?;
    if restored_sqlite {
        sanitize_restored_claude_database_for_current_os(&app_handle)?;
    }

    // Remove existing database directory
    if db_path.exists() {
        fs::remove_dir_all(&db_path)
            .map_err(|e| format!("Failed to remove existing database: {}", e))?;
    }

    // Create database directory
    fs::create_dir_all(&db_path)
        .map_err(|e| format!("Failed to create database directory: {}", e))?;

    let home_dir = get_home_dir()?;
    let opencode_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/opencode/root-dir.txt");
    let claude_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/claude/root-dir.txt");
    let codex_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/codex/root-dir.txt");
    let grok_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/grok/root-dir.txt");
    let openclaw_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/openclaw/root-dir.txt");
    let gemini_cli_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/geminicli/root-dir.txt");
    let pi_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/pi/root-dir.txt");
    let mut restore_result = RestoreResult::default();

    let (opencode_restore_dir, opencode_warning) = resolve_restore_dir_override(
        "opencode",
        opencode_restore_dir_override,
        get_opencode_restore_dir()?,
    );
    if let Some(warning) = opencode_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (claude_restore_dir, claude_warning) = resolve_restore_dir_override(
        "claude",
        claude_restore_dir_override,
        get_claude_restore_dir()?,
    );
    if let Some(warning) = claude_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (codex_restore_dir, codex_warning) = resolve_restore_dir_override(
        "codex",
        codex_restore_dir_override,
        get_codex_restore_dir()?,
    );
    if let Some(warning) = codex_warning {
        push_restore_warning(&mut restore_result, warning);
    }
    let (grok_restore_dir, grok_warning) =
        resolve_restore_dir_override("grok", grok_restore_dir_override, get_grok_restore_dir()?);
    if let Some(warning) = grok_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (openclaw_restore_dir, openclaw_warning) = resolve_restore_dir_override(
        "openclaw",
        openclaw_restore_dir_override,
        home_dir.join(".openclaw"),
    );
    if let Some(warning) = openclaw_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (gemini_cli_restore_dir, gemini_cli_warning) = resolve_restore_dir_override(
        "geminicli",
        gemini_cli_restore_dir_override,
        get_gemini_cli_restore_dir()?,
    );
    if let Some(warning) = gemini_cli_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (pi_restore_dir, pi_warning) = resolve_restore_dir_override(
        "pi",
        pi_restore_dir_override,
        home_dir.join(".pi").join("agent"),
    );
    if let Some(warning) = pi_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    // Extract zip contents
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {}", e))?;

        // Normalize path separators for cross-platform compatibility
        // Windows backups may contain backslashes which need to be converted
        let file_name = file.name().to_string().replace('\\', "/");

        // Skip the backup marker file
        if file_name == ".backup_marker" || file_name == "db/.backup_marker" {
            continue;
        }

        // Handle database files
        if is_new_format {
            if file_name.starts_with("db/") {
                let relative_path = &file_name[3..]; // Remove "db/" prefix
                if relative_path.is_empty() {
                    continue;
                }

                let outpath = db_path.join(relative_path);

                if file_name.ends_with('/') {
                    fs::create_dir_all(&outpath)
                        .map_err(|e| format!("Failed to create directory: {}", e))?;
                } else {
                    if let Some(parent) = outpath.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                        }
                    }
                    let mut outfile = File::create(&outpath)
                        .map_err(|e| format!("Failed to create file: {}", e))?;
                    std::io::copy(&mut file, &mut outfile)
                        .map_err(|e| format!("Failed to extract file: {}", e))?;
                }
            } else if file_name.starts_with("external-configs/opencode/") {
                // Restore OpenCode config to the appropriate directory based on env/shell/default
                let relative_path = &file_name[26..]; // Remove "external-configs/opencode/" prefix
                if relative_path.is_empty() || file_name.ends_with('/') {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "opencode", relative_path) {
                    continue;
                }

                if relative_path == "auth.json" {
                    let outpath = get_opencode_auth_restore_path(Some(&opencode_restore_dir))?;
                    let auth_dir = outpath.parent().ok_or_else(|| {
                        "Failed to determine OpenCode auth parent directory".to_string()
                    })?;
                    if !auth_dir.exists() {
                        fs::create_dir_all(&auth_dir).map_err(|e| {
                            format!("Failed to create opencode auth directory: {}", e)
                        })?;
                    }
                    let mut outfile = File::create(&outpath)
                        .map_err(|e| format!("Failed to create file: {}", e))?;
                    std::io::copy(&mut file, &mut outfile)
                        .map_err(|e| format!("Failed to extract file: {}", e))?;
                } else {
                    if relative_path == "root-dir.txt" {
                        continue;
                    }
                    if !opencode_restore_dir.exists() {
                        fs::create_dir_all(&opencode_restore_dir).map_err(|e| {
                            format!("Failed to create opencode config directory: {}", e)
                        })?;
                    }

                    let outpath = opencode_restore_dir.join(relative_path);

                    // Just copy the file - MCP cmd /c normalization will be handled
                    // by mcp_sync_all during startup resync (triggered by .resync_required flag)
                    let mut outfile = File::create(&outpath)
                        .map_err(|e| format!("Failed to create file: {}", e))?;
                    std::io::copy(&mut file, &mut outfile)
                        .map_err(|e| format!("Failed to extract file: {}", e))?;
                }
            } else if file_name.starts_with("external-configs/claude/") {
                // Restore Claude settings
                let relative_path = &file_name[24..]; // Remove "external-configs/claude/" prefix
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "claude", relative_path) {
                    continue;
                }

                let outpath = if relative_path == ".claude.json" {
                    get_claude_mcp_restore_path(Some(&claude_restore_dir))?
                } else {
                    claude_restore_dir.join(relative_path)
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create claude config directory: {}", e)
                        })?;
                    }
                }
                restore_claude_external_config_file(&mut file, &outpath, relative_path)?;
            } else if file_name.starts_with("external-configs/openclaw/") {
                let relative_path = &file_name[26..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "openclaw", relative_path) {
                    continue;
                }

                if !openclaw_restore_dir.exists() {
                    fs::create_dir_all(&openclaw_restore_dir).map_err(|e| {
                        format!("Failed to create openclaw config directory: {}", e)
                    })?;
                }

                let outpath = openclaw_restore_dir.join(relative_path);
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
                if relative_path == "auth.json" {
                    harden_restored_sensitive_file(&outpath)?;
                }
            } else if file_name.starts_with("external-configs/codex/") {
                // Restore Codex settings
                let relative_path = &file_name[23..]; // Remove "external-configs/codex/" prefix
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "codex", relative_path) {
                    continue;
                }

                if !codex_restore_dir.exists() {
                    fs::create_dir_all(&codex_restore_dir)
                        .map_err(|e| format!("Failed to create codex config directory: {}", e))?;
                }

                let outpath = codex_restore_dir.join(relative_path);

                // Just copy the file - MCP cmd /c normalization will be handled
                // by mcp_sync_all during startup resync (triggered by .resync_required flag)
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
                if relative_path == "auth.json" {
                    harden_restored_sensitive_file(&outpath)?;
                }
            } else if file_name.starts_with("external-configs/grok/") {
                let relative_path = &file_name["external-configs/grok/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }
                if should_filter_external_config_entry(&filter_rules, "grok", relative_path) {
                    continue;
                }
                let Some(outpath) =
                    resolve_external_config_restore_output_path(&grok_restore_dir, relative_path)?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create Grok restore directory: {}", e))?;
                }
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
                if matches!(relative_path, "auth.json" | "config.toml") {
                    harden_restored_sensitive_file(&outpath)?;
                }
            } else if file_name.starts_with("external-configs/geminicli/") {
                let relative_path = &file_name["external-configs/geminicli/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "geminicli", relative_path) {
                    continue;
                }

                if !gemini_cli_restore_dir.exists() {
                    fs::create_dir_all(&gemini_cli_restore_dir).map_err(|e| {
                        format!("Failed to create Gemini CLI config directory: {}", e)
                    })?;
                }

                let Some(outpath) = resolve_external_config_restore_output_path(
                    &gemini_cli_restore_dir,
                    relative_path,
                )?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create Gemini CLI parent directory: {}", e)
                        })?;
                    }
                }
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            } else if file_name.starts_with("external-configs/pi/") {
                let relative_path = &file_name["external-configs/pi/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "pi", relative_path) {
                    continue;
                }

                if !pi_restore_dir.exists() {
                    fs::create_dir_all(&pi_restore_dir)
                        .map_err(|e| format!("Failed to create Pi config directory: {}", e))?;
                }

                let Some(outpath) =
                    resolve_external_config_restore_output_path(&pi_restore_dir, relative_path)?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create Pi config parent directory: {}", e)
                        })?;
                    }
                }
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
                if relative_path == "auth.json" {
                    set_pi_auth_file_permissions(&outpath);
                }
            } else if file_name == "models.dev.json" {
                // Restore models.dev.json to app data directory
                if let Some(cache_path) =
                    crate::coding::open_code::free_models::get_models_cache_path()
                {
                    if let Some(parent) = cache_path.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
                        }
                    }
                    let mut outfile = File::create(&cache_path)
                        .map_err(|e| format!("Failed to create models cache file: {}", e))?;
                    std::io::copy(&mut file, &mut outfile)
                        .map_err(|e| format!("Failed to extract models cache file: {}", e))?;
                }
            } else if file_name == "preset_models.json" {
                // Restore preset_models.json to app data directory
                if let Some(cache_path) =
                    crate::coding::preset_models::get_preset_models_cache_path()
                {
                    if let Some(parent) = cache_path.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
                        }
                    }
                    let mut outfile = File::create(&cache_path)
                        .map_err(|e| format!("Failed to create preset models cache file: {}", e))?;
                    std::io::copy(&mut file, &mut outfile).map_err(|e| {
                        format!("Failed to extract preset models cache file: {}", e)
                    })?;
                }
            } else if file_name == "model_pricing.json" {
                // Restore model_pricing.json to app data directory
                if let Some(cache_path) =
                    crate::db::model_pricing_seed::get_model_pricing_cache_path()
                {
                    if let Some(parent) = cache_path.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
                        }
                    }
                    let mut outfile = File::create(&cache_path)
                        .map_err(|e| format!("Failed to create model pricing cache file: {}", e))?;
                    std::io::copy(&mut file, &mut outfile).map_err(|e| {
                        format!("Failed to extract model pricing cache file: {}", e)
                    })?;
                }
            } else if file_name == "gateway_provider_profiles.json" {
                // Restore gateway_provider_profiles.json to app data directory
                if let Some(cache_path) =
                    crate::coding::proxy_gateway::provider_profiles::get_gateway_provider_profiles_cache_path()
                {
                    if let Some(parent) = cache_path.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
                        }
                    }
                    let mut outfile = File::create(&cache_path).map_err(|e| {
                        format!("Failed to create gateway provider profiles cache file: {}", e)
                    })?;
                    std::io::copy(&mut file, &mut outfile).map_err(|e| {
                        format!("Failed to extract gateway provider profiles cache file: {}", e)
                    })?;
                }
            } else if file_name.starts_with("skills/") {
                // Restore skills directory
                let skills_dir = get_skills_dir(&app_handle)?;
                if !skills_dir.exists() {
                    fs::create_dir_all(&skills_dir)
                        .map_err(|e| format!("Failed to create skills directory: {}", e))?;
                }

                let Some((outpath, warning)) =
                    resolve_skills_restore_output_path(&skills_dir, &file_name)?
                else {
                    continue;
                };
                if let Some(warning) = warning {
                    push_restore_warning(&mut restore_result, warning);
                }

                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create skills parent directory: {}", e)
                        })?;
                    }
                }
                let mut outfile = File::create(&outpath)
                    .map_err(|e| format!("Failed to create skills file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract skills file: {}", e))?;
            } else if file_name.starts_with("image-studio/assets/") {
                let relative_path = &file_name["image-studio/assets/".len()..];
                if relative_path.is_empty() || file_name.ends_with('/') {
                    continue;
                }

                let image_assets_dir = get_image_assets_dir(&app_handle)?;
                if !image_assets_dir.exists() {
                    fs::create_dir_all(&image_assets_dir)
                        .map_err(|e| format!("Failed to create image assets directory: {}", e))?;
                }

                let outpath = image_assets_dir.join(relative_path);
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create image asset parent directory: {}", e)
                        })?;
                    }
                }
                let mut outfile = File::create(&outpath)
                    .map_err(|e| format!("Failed to create image asset file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract image asset file: {}", e))?;
            }
        } else {
            // Old format: all files are database files
            let outpath = db_path.join(&file_name);

            if file_name.ends_with('/') {
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
    }

    restore_custom_backup_entries(&mut archive)?;

    // Create resync flag file to trigger skills and MCP resync on next startup
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let resync_flag = app_data_dir.join(".resync_required");
    let _ = fs::write(&resync_flag, "1");

    Ok(restore_result)
}

/// Get database directory path for frontend
#[tauri::command]
pub fn get_database_path(app_handle: tauri::AppHandle) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;
    Ok(db_path.to_string_lossy().to_string())
}

/// Open the app data directory in the file explorer
#[tauri::command]
pub fn open_app_data_dir(app_handle: tauri::AppHandle) -> Result<(), String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    // Ensure directory exists
    if !app_data_dir.exists() {
        fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("Failed to create app data directory: {}", e))?;
    }

    // Open in file explorer (platform-specific)
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&app_data_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&app_data_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&app_data_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}
