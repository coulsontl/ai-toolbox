use chrono::Local;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::Manager;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::utils::{
    RestoreResult, add_custom_backup_entries_to_zip, add_directory_contents_to_zip,
    add_image_assets_to_zip, add_text_to_zip, get_backup_custom_entries_from_db,
    get_backup_image_assets_enabled_from_db, get_claude_mcp_path_from_db,
    get_claude_mcp_restore_path, get_claude_prompt_path_from_db, get_claude_restore_dir,
    get_claude_settings_path_from_db, get_codex_auth_path_from_db, get_codex_config_path_from_db,
    get_codex_prompt_backup_zip_path, get_codex_prompt_paths_from_db, get_codex_restore_dir,
    get_custom_root_dir_path_info, get_db_path, get_gemini_cli_env_path_from_db,
    get_gemini_cli_oauth_creds_path_from_db, get_gemini_cli_prompt_backup_zip_path,
    get_gemini_cli_prompt_path_from_db, get_gemini_cli_restore_dir,
    get_gemini_cli_settings_path_from_db, get_gemini_cli_tmp_dir_from_db, get_image_assets_dir,
    get_models_cache_file, get_openclaw_config_path_from_db, get_opencode_auth_path_from_db,
    get_opencode_auth_restore_path, get_opencode_config_path_from_db,
    get_opencode_prompt_path_from_db, get_opencode_restore_dir, get_preset_models_cache_file,
    get_skills_dir, push_restore_warning, read_root_dir_override,
    resolve_external_config_restore_output_path, resolve_restore_dir_override,
    resolve_skills_restore_output_path, restore_custom_backup_entries,
};

fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

/// Add a file to the zip archive with a specific path prefix
fn add_file_to_zip(
    zip: &mut ZipWriter<File>,
    file_path: &Path,
    zip_path: &str,
    options: SimpleFileOptions,
) -> Result<(), String> {
    let mut file = File::open(file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    zip.start_file(zip_path, options)
        .map_err(|e| format!("Failed to start file in zip: {}", e))?;
    zip.write_all(&buffer)
        .map_err(|e| format!("Failed to write to zip: {}", e))?;

    Ok(())
}

/// Backup database to a zip file
#[tauri::command]
pub async fn backup_database(
    app_handle: tauri::AppHandle,
    backup_path: String,
) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;
    let db_state = app_handle.state::<crate::DbState>();
    let db = db_state.db();
    let backup_image_assets_enabled = get_backup_image_assets_enabled_from_db(&db).await?;
    let backup_custom_entries = get_backup_custom_entries_from_db(&db).await?;

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
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Walk through the database directory and add files to zip under "db/" prefix
    let mut has_files = false;
    for entry in WalkDir::new(&db_path) {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();
        let relative_path = path
            .strip_prefix(&db_path)
            .map_err(|e| format!("Failed to get relative path: {}", e))?;

        if path.is_file() {
            // Skip system files like .DS_Store
            if let Some(file_name) = path.file_name() {
                let name_str = file_name.to_string_lossy();
                if name_str == ".DS_Store" || name_str.starts_with("._") {
                    continue;
                }
            }

            has_files = true;
            // Use forward slashes for cross-platform compatibility in zip files
            let relative_str = relative_path.to_string_lossy().replace('\\', "/");
            let name = format!("db/{}", relative_str);
            zip.start_file(name, options)
                .map_err(|e| format!("Failed to start file in zip: {}", e))?;

            let mut file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)
                .map_err(|e| format!("Failed to read file: {}", e))?;
            zip.write_all(&buffer)
                .map_err(|e| format!("Failed to write to zip: {}", e))?;
        } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
            // Use forward slashes for cross-platform compatibility in zip files
            let relative_str = relative_path.to_string_lossy().replace('\\', "/");
            let name = format!("db/{}/", relative_str);
            zip.add_directory(name, options)
                .map_err(|e| format!("Failed to add directory to zip: {}", e))?;
        }
    }

    // If no database files, add a placeholder to ensure valid zip
    if !has_files {
        zip.start_file("db/.backup_marker", options)
            .map_err(|e| format!("Failed to create marker file: {}", e))?;
        zip.write_all(b"AI Toolbox Backup")
            .map_err(|e| format!("Failed to write marker: {}", e))?;
    }

    // Add external-configs directory
    zip.add_directory("external-configs/", options)
        .map_err(|e| format!("Failed to add external-configs directory: {}", e))?;

    if let Some(custom_dir) = get_custom_root_dir_path_info(&db, "opencode").await {
        zip.add_directory("external-configs/opencode/", options)
            .map_err(|e| format!("Failed to add opencode directory: {}", e))?;
        add_text_to_zip(
            &mut zip,
            "external-configs/opencode/root-dir.txt",
            &custom_dir,
            options,
        )?;
    }

    // Backup OpenCode config if exists
    if let Some(opencode_path) = get_opencode_config_path_from_db(&db).await? {
        let file_name = opencode_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "opencode.json".to_string());
        let zip_path = format!("external-configs/opencode/{}", file_name);

        zip.add_directory("external-configs/opencode/", options)
            .map_err(|e| format!("Failed to add opencode directory: {}", e))?;

        add_file_to_zip(&mut zip, &opencode_path, &zip_path, options)?;
    }

    // Backup OpenCode auth.json if exists
    if let Some(opencode_auth_path) = get_opencode_auth_path_from_db(&db).await? {
        let zip_path = "external-configs/opencode/auth.json";

        // Directory may already exist from opencode config backup
        let _ = zip.add_directory("external-configs/opencode/", options);

        add_file_to_zip(&mut zip, &opencode_auth_path, zip_path, options)?;
    }

    // Backup OpenCode AGENTS.md if exists
    if let Some(opencode_prompt_path) = get_opencode_prompt_path_from_db(&db).await? {
        let zip_path = "external-configs/opencode/AGENTS.md";

        let _ = zip.add_directory("external-configs/opencode/", options);

        add_file_to_zip(&mut zip, &opencode_prompt_path, zip_path, options)?;
    }

    if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "claude").await {
        zip.add_directory("external-configs/claude/", options)
            .map_err(|e| format!("Failed to add claude directory: {}", e))?;
        add_text_to_zip(
            &mut zip,
            "external-configs/claude/root-dir.txt",
            &custom_root_dir,
            options,
        )?;
    }

    // Backup Claude settings.json if exists
    if let Some(claude_path) = get_claude_settings_path_from_db(&db).await? {
        let zip_path = "external-configs/claude/settings.json";

        zip.add_directory("external-configs/claude/", options)
            .map_err(|e| format!("Failed to add claude directory: {}", e))?;

        add_file_to_zip(&mut zip, &claude_path, zip_path, options)?;
    }

    // Backup Claude CLAUDE.md if exists
    if let Some(claude_prompt_path) = get_claude_prompt_path_from_db(&db).await? {
        let zip_path = "external-configs/claude/CLAUDE.md";

        let _ = zip.add_directory("external-configs/claude/", options);

        add_file_to_zip(&mut zip, &claude_prompt_path, zip_path, options)?;
    }

    if let Some(claude_mcp_path) = get_claude_mcp_path_from_db(&db).await? {
        let zip_path = "external-configs/claude/.claude.json";
        let _ = zip.add_directory("external-configs/claude/", options);
        add_file_to_zip(&mut zip, &claude_mcp_path, zip_path, options)?;
    }

    if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "codex").await {
        zip.add_directory("external-configs/codex/", options)
            .map_err(|e| format!("Failed to add codex directory: {}", e))?;
        add_text_to_zip(
            &mut zip,
            "external-configs/codex/root-dir.txt",
            &custom_root_dir,
            options,
        )?;
    }

    // Backup Codex auth.json if exists
    if let Some(codex_auth_path) = get_codex_auth_path_from_db(&db).await? {
        let zip_path = "external-configs/codex/auth.json";

        zip.add_directory("external-configs/codex/", options)
            .map_err(|e| format!("Failed to add codex directory: {}", e))?;

        add_file_to_zip(&mut zip, &codex_auth_path, zip_path, options)?;
    }

    // Backup Codex config.toml if exists
    if let Some(codex_config_path) = get_codex_config_path_from_db(&db).await? {
        let zip_path = "external-configs/codex/config.toml";

        // Directory may already exist from auth.json backup
        let _ = zip.add_directory("external-configs/codex/", options);

        add_file_to_zip(&mut zip, &codex_config_path, zip_path, options)?;
    }

    // Backup all existing Codex global prompt files.
    for codex_prompt_path in get_codex_prompt_paths_from_db(&db).await? {
        let zip_path = get_codex_prompt_backup_zip_path(&codex_prompt_path);

        let _ = zip.add_directory("external-configs/codex/", options);

        add_file_to_zip(&mut zip, &codex_prompt_path, &zip_path, options)?;
    }

    if let Some(custom_dir) = get_custom_root_dir_path_info(&db, "openclaw").await {
        zip.add_directory("external-configs/openclaw/", options)
            .map_err(|e| format!("Failed to add openclaw directory: {}", e))?;
        add_text_to_zip(
            &mut zip,
            "external-configs/openclaw/root-dir.txt",
            &custom_dir,
            options,
        )?;
    }

    if let Some(openclaw_config_path) = get_openclaw_config_path_from_db(&db).await? {
        let zip_path = "external-configs/openclaw/openclaw.json";
        let _ = zip.add_directory("external-configs/openclaw/", options);
        add_file_to_zip(&mut zip, &openclaw_config_path, zip_path, options)?;
    }

    if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "geminicli").await {
        zip.add_directory("external-configs/geminicli/", options)
            .map_err(|e| format!("Failed to add Gemini CLI directory: {}", e))?;
        add_text_to_zip(
            &mut zip,
            "external-configs/geminicli/root-dir.txt",
            &custom_root_dir,
            options,
        )?;
    }

    if let Some(gemini_env_path) = get_gemini_cli_env_path_from_db(&db).await? {
        let zip_path = "external-configs/geminicli/.env";
        let _ = zip.add_directory("external-configs/geminicli/", options);
        add_file_to_zip(&mut zip, &gemini_env_path, zip_path, options)?;
    }

    if let Some(gemini_settings_path) = get_gemini_cli_settings_path_from_db(&db).await? {
        let zip_path = "external-configs/geminicli/settings.json";
        let _ = zip.add_directory("external-configs/geminicli/", options);
        add_file_to_zip(&mut zip, &gemini_settings_path, zip_path, options)?;
    }

    if let Some(gemini_prompt_path) = get_gemini_cli_prompt_path_from_db(&db).await? {
        let zip_path = get_gemini_cli_prompt_backup_zip_path(&gemini_prompt_path);
        let _ = zip.add_directory("external-configs/geminicli/", options);
        add_file_to_zip(&mut zip, &gemini_prompt_path, &zip_path, options)?;
    }

    if let Some(gemini_oauth_path) = get_gemini_cli_oauth_creds_path_from_db(&db).await? {
        let zip_path = "external-configs/geminicli/oauth_creds.json";
        let _ = zip.add_directory("external-configs/geminicli/", options);
        add_file_to_zip(&mut zip, &gemini_oauth_path, zip_path, options)?;
    }

    if let Some(gemini_tmp_dir) = get_gemini_cli_tmp_dir_from_db(&db).await? {
        add_directory_contents_to_zip(
            &mut zip,
            &gemini_tmp_dir,
            "external-configs/geminicli/tmp",
            options,
        )?;
    }

    // Backup models.dev.json cache if exists
    if let Some(models_cache_path) = get_models_cache_file() {
        add_file_to_zip(&mut zip, &models_cache_path, "models.dev.json", options)?;
    }

    // Backup preset_models.json cache if exists
    if let Some(preset_models_cache_path) = get_preset_models_cache_file() {
        add_file_to_zip(
            &mut zip,
            &preset_models_cache_path,
            "preset_models.json",
            options,
        )?;
    }

    // Backup skills directory if exists
    let skills_dir = get_skills_dir(&app_handle)?;
    if skills_dir.exists() {
        zip.add_directory("skills/", options)
            .map_err(|e| format!("Failed to add skills directory: {}", e))?;

        for entry in WalkDir::new(&skills_dir) {
            let entry = entry.map_err(|e| format!("Failed to read skills entry: {}", e))?;
            let path = entry.path();
            let relative_path = path
                .strip_prefix(&skills_dir)
                .map_err(|e| format!("Failed to get relative path: {}", e))?;

            if path.is_file() {
                // Skip system files
                if let Some(file_name) = path.file_name() {
                    let name_str = file_name.to_string_lossy();
                    if name_str == ".DS_Store" || name_str.starts_with("._") {
                        continue;
                    }
                }

                let relative_str = relative_path.to_string_lossy().replace('\\', "/");
                let name = format!("skills/{}", relative_str);
                add_file_to_zip(&mut zip, path, &name, options)?;
            } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
                let relative_str = relative_path.to_string_lossy().replace('\\', "/");
                let name = format!("skills/{}/", relative_str);
                zip.add_directory(name, options)
                    .map_err(|e| format!("Failed to add skills subdirectory: {}", e))?;
            }
        }
    }

    add_custom_backup_entries_to_zip(&mut zip, &backup_custom_entries, options)?;

    if backup_image_assets_enabled {
        add_image_assets_to_zip(&app_handle, &mut zip, options)?;
    }

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
    let openclaw_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/openclaw/root-dir.txt");
    let gemini_cli_restore_dir_override =
        read_root_dir_override(&mut archive, "external-configs/geminicli/root-dir.txt");
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
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            } else if file_name.starts_with("external-configs/openclaw/") {
                let relative_path = &file_name[26..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
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
            } else if file_name.starts_with("external-configs/codex/") {
                // Restore Codex settings
                let relative_path = &file_name[23..]; // Remove "external-configs/codex/" prefix
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
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
            } else if file_name.starts_with("external-configs/geminicli/") {
                let relative_path = &file_name["external-configs/geminicli/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
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
