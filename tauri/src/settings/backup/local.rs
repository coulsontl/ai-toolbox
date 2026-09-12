use chrono::Local;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tauri::Manager;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::utils::{
    clear_restored_cli_custom_roots, get_claude_desktop_settings_paths,
    get_claude_mcp_restore_path, get_claude_restore_dir, get_codex_restore_dir, get_db_path,
    get_dsh_restore_dir, get_gemini_cli_restore_dir, get_grok_restore_dir, get_hermes_restore_dir,
    get_image_assets_dir, get_kimi_restore_dir, get_opencode_auth_restore_path,
    get_opencode_restore_dir, get_skills_dir, harden_restored_sensitive_file, push_restore_warning,
    read_backup_meta_from_archive, read_root_dir_override,
    record_restored_external_config_wsl_module, resolve_external_config_restore_output_path,
    resolve_restore_dir_override, resolve_skills_restore_output_path,
    restore_claude_external_config_file, restore_custom_backup_entries,
    restore_sqlite_database_snapshot_from_zip, sanitize_restored_claude_database_for_current_os,
    should_filter_external_config_entry, should_reapply_applied_runtime,
    should_skip_external_config_on_restore, should_use_root_override_for_tool,
    write_backup_zip_contents, write_post_restore_flags, RestoreResult,
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
    let backup_cli_config_files_enabled = settings.backup_cli_config_files_enabled;
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
        backup_cli_config_files_enabled,
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
    skip_cli_custom_roots: Option<bool>,
) -> Result<RestoreResult, String> {
    let db_path = get_db_path(&app_handle)?;
    let zip_path = Path::new(&zip_file_path);
    let skip_cli_custom_roots = skip_cli_custom_roots.unwrap_or(false);

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

    // Read pre-restore settings BEFORE overwriting SQLite so skip/filter decisions
    // are not taken from the restored machine's settings document.
    let (filter_rules, include_cli_config_files) = {
        let sqlite_state = app_handle.state::<SqliteDbState>();
        match store::load_settings_from_sqlite_state(&sqlite_state) {
            Ok(settings) => (
                settings.backup_file_filter_rules,
                settings.backup_cli_config_files_enabled,
            ),
            Err(_) => (default_backup_file_filter_rules(), true),
        }
    };
    // When false, only optional (DB-backed) CLI runtime files are skipped on restore.
    // OpenCode / OpenClaw / Pi remain always-restored when present in the zip.
    let skipped_optional_cli_runtime = !include_cli_config_files;
    let backup_meta = read_backup_meta_from_archive(&mut archive);

    let restored_sqlite = restore_sqlite_database_snapshot_from_zip(&mut archive, &app_handle)?;
    if restored_sqlite {
        sanitize_restored_claude_database_for_current_os(&app_handle)?;
        // Only clear roots on the restored snapshot — never mutate the live DB when
        // the backup did not include/replace sqlite/.
        if skip_cli_custom_roots {
            let sqlite_state = app_handle.state::<SqliteDbState>();
            clear_restored_cli_custom_roots(&sqlite_state)?;
        }
    }

    // Legacy SurrealDB-only backups (db/ entries, no sqlite/ snapshot) cannot be
    // restored: the app no longer ships the SurrealDB import path, so a restored
    // legacy dir would be silently ignored on next startup. Fail loudly instead of
    // pretending success.
    if !restored_sqlite && is_new_format {
        return Err(
            "该备份是旧版 SurrealDB 格式，当前应用已不再支持，无法恢复此备份。请使用新版应用创建的备份文件。"
                .to_string(),
        );
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
    // Always-include tools may still read root-dir.txt when optional CLI files are skipped.
    // Never read overrides when the user explicitly requested local/default roots.
    let opencode_restore_dir_override = should_use_root_override_for_tool(
        "opencode",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(&mut archive, "external-configs/opencode/root-dir.txt"))
    .flatten();
    let claude_restore_dir_override = should_use_root_override_for_tool(
        "claude",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(&mut archive, "external-configs/claude/root-dir.txt"))
    .flatten();
    let codex_restore_dir_override =
        should_use_root_override_for_tool("codex", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(&mut archive, "external-configs/codex/root-dir.txt"))
            .flatten();
    let grok_restore_dir_override =
        should_use_root_override_for_tool("grok", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(&mut archive, "external-configs/grok/root-dir.txt"))
            .flatten();
    let kimi_restore_dir_override =
        should_use_root_override_for_tool("kimi", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(&mut archive, "external-configs/kimi/root-dir.txt"))
            .flatten();
    let openclaw_restore_dir_override = should_use_root_override_for_tool(
        "openclaw",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(&mut archive, "external-configs/openclaw/root-dir.txt"))
    .flatten();
    let gemini_cli_restore_dir_override = should_use_root_override_for_tool(
        "geminicli",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(&mut archive, "external-configs/geminicli/root-dir.txt"))
    .flatten();
    let pi_restore_dir_override =
        should_use_root_override_for_tool("pi", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(&mut archive, "external-configs/pi/root-dir.txt"))
            .flatten();
    let oh_my_pi_restore_dir_override = should_use_root_override_for_tool(
        "oh_my_pi",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(&mut archive, "external-configs/oh_my_pi/root-dir.txt"))
    .flatten();
    let hermes_restore_dir_override = should_use_root_override_for_tool(
        "hermes",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(&mut archive, "external-configs/hermes/root-dir.txt"))
    .flatten();
    let dsh_restore_dir_override =
        should_use_root_override_for_tool("dsh", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(&mut archive, "external-configs/dsh/root-dir.txt"))
            .flatten();
    let mut restore_result = RestoreResult::default();
    let mut restored_wsl_modules = Vec::new();

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
    let (kimi_restore_dir, kimi_warning) =
        resolve_restore_dir_override("kimi", kimi_restore_dir_override, get_kimi_restore_dir()?);
    if let Some(warning) = kimi_warning {
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

    let (oh_my_pi_restore_dir, oh_my_pi_warning) = resolve_restore_dir_override(
        "oh_my_pi",
        oh_my_pi_restore_dir_override,
        home_dir.join(".omp").join("agent"),
    );
    if let Some(warning) = oh_my_pi_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (hermes_restore_dir, hermes_warning) = resolve_restore_dir_override(
        "hermes",
        hermes_restore_dir_override,
        get_hermes_restore_dir()?,
    );
    if let Some(warning) = hermes_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (dsh_restore_dir, dsh_warning) =
        resolve_restore_dir_override("dsh", dsh_restore_dir_override, get_dsh_restore_dir()?);
    if let Some(warning) = dsh_warning {
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
        if file_name == "backup_meta.json" {
            continue;
        }
        // Skip optional (DB-backed) CLI runtime configs when the pre-restore setting disables them.
        // OpenCode / OpenClaw / Pi always restore when present.
        if should_skip_external_config_on_restore(include_cli_config_files, &file_name) {
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
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "opencode", relative_path) {
                    continue;
                }
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "opencode");

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
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "claude");

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
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "openclaw");

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
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "codex");

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
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "grok");
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
            } else if file_name.starts_with("external-configs/kimi/") {
                let relative_path = &file_name["external-configs/kimi/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }
                if should_filter_external_config_entry(&filter_rules, "kimi", relative_path) {
                    continue;
                }
                let Some(outpath) =
                    resolve_external_config_restore_output_path(&kimi_restore_dir, relative_path)?
                else {
                    continue;
                };
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "kimi");
                if let Some(parent) = outpath.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create Kimi restore directory: {}", e))?;
                }
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
                if matches!(relative_path, "config.toml")
                    || relative_path.starts_with("credentials/")
                {
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
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "geminicli");
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
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "pi");
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
            } else if file_name.starts_with("external-configs/oh_my_pi/") {
                let relative_path = &file_name["external-configs/oh_my_pi/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "oh_my_pi", relative_path) {
                    continue;
                }

                if !oh_my_pi_restore_dir.exists() {
                    fs::create_dir_all(&oh_my_pi_restore_dir).map_err(|e| {
                        format!("Failed to create Oh My Pi config directory: {}", e)
                    })?;
                }

                let Some(outpath) = resolve_external_config_restore_output_path(
                    &oh_my_pi_restore_dir,
                    relative_path,
                )?
                else {
                    continue;
                };
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "oh_my_pi");
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create Oh My Pi config parent directory: {}", e)
                        })?;
                    }
                }
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            } else if file_name.starts_with("external-configs/hermes/") {
                let relative_path = &file_name["external-configs/hermes/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "hermes", relative_path) {
                    continue;
                }

                if !hermes_restore_dir.exists() {
                    fs::create_dir_all(&hermes_restore_dir)
                        .map_err(|e| format!("Failed to create Hermes config directory: {}", e))?;
                }

                let Some(outpath) = resolve_external_config_restore_output_path(
                    &hermes_restore_dir,
                    relative_path,
                )?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create Hermes parent directory: {}", e)
                        })?;
                    }
                }
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "hermes");
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            } else if file_name.starts_with("external-configs/dsh/") {
                let relative_path = &file_name["external-configs/dsh/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "dsh", relative_path) {
                    continue;
                }

                if !dsh_restore_dir.exists() {
                    fs::create_dir_all(&dsh_restore_dir)
                        .map_err(|e| format!("Failed to create dsh config directory: {}", e))?;
                }

                let Some(outpath) =
                    resolve_external_config_restore_output_path(&dsh_restore_dir, relative_path)?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("Failed to create dsh parent directory: {}", e))?;
                    }
                }
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "dsh");
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
                if relative_path == ".credentials.yaml" {
                    harden_restored_sensitive_file(&outpath)?;
                }
            } else if file_name.starts_with("external-configs/claude_desktop/") {
                let relative_path = &file_name["external-configs/claude_desktop/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(
                    &filter_rules,
                    "claude_desktop",
                    relative_path,
                ) {
                    continue;
                }

                let Some((normal_config_path, config_library_path)) =
                    get_claude_desktop_settings_paths()
                else {
                    continue;
                };
                let outpath = if relative_path == "claude_desktop_config.json" {
                    let parent = normal_config_path.parent().ok_or_else(|| {
                        "Failed to resolve Claude Desktop config directory".to_string()
                    })?;
                    let Some(outpath) =
                        resolve_external_config_restore_output_path(parent, relative_path)?
                    else {
                        continue;
                    };
                    outpath
                } else if let Some(rest) = relative_path.strip_prefix("configLibrary/") {
                    let Some(outpath) =
                        resolve_external_config_restore_output_path(&config_library_path, rest)?
                    else {
                        continue;
                    };
                    outpath
                } else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create Claude Desktop parent directory: {}", e)
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

    let need_reapply =
        should_reapply_applied_runtime(skipped_optional_cli_runtime, backup_meta.as_ref());
    restore_result.will_reapply_applied = need_reapply;
    write_post_restore_flags(&app_handle, need_reapply, &restored_wsl_modules)?;

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
    let _ = app_handle;
    let app_data_dir = crate::app_paths::resolved_data_dir();

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
