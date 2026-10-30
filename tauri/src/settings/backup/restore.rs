//! Single shared restore implementation for every backup source.
//!
//! Local files, WebDAV downloads, and repository downloads all resolve to plaintext
//! backup bytes first (`prepare_backup_bytes` handles header-based encryption
//! detection and optional decryption), then feed the same `restore_from_archive`
//! pipeline. The pipeline keeps the historical semantics: pre-restore settings read,
//! SQLite snapshot swap with rollback, external-config restore with filters,
//! post-restore flags, and warnings.

use std::fs::{self, File};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use tauri::Manager;
use zip::ZipArchive;

use super::credentials::{self, backup_error};
use super::encryption::{self, CryptoError};
use super::utils::{
    clear_restored_cli_custom_roots, get_antigravity_restore_dir, get_zcode_restore_dir,
    get_claude_desktop_settings_paths, get_claude_mcp_restore_path, get_claude_restore_dir,
    get_codex_restore_dir, get_db_path, get_dsh_restore_dir, get_gemini_cli_restore_dir,
    get_grok_restore_dir, get_hermes_restore_dir, get_image_assets_dir, get_kimi_restore_dir,
    get_opencode_auth_restore_path, get_opencode_restore_dir, get_skills_dir,
    harden_restored_sensitive_file, normalize_restore_entry_name, push_restore_warning,
    read_backup_meta_from_archive, read_root_dir_override,
    record_restored_external_config_wsl_module, resolve_external_config_restore_output_path,
    resolve_restore_dir_override, resolve_skills_restore_output_path,
    restore_claude_external_config_file, restore_custom_backup_entries,
    restore_sqlite_database_snapshot_from_zip, sanitize_restored_claude_database_for_current_os,
    should_filter_external_config_entry, should_reapply_applied_runtime,
    should_skip_external_config_on_restore, should_use_root_override_for_tool,
    write_post_restore_flags, RestoreResult,
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

fn crypto_error_string(error: CryptoError) -> String {
    match error {
        CryptoError::PasswordRequired => backup_error(
            "passwordRequired",
            "settings.backupSettings.encryption.errors.passwordRequired",
            "backup is encrypted and no password was provided",
        ),
        CryptoError::AuthFailed => backup_error(
            "passwordWrong",
            "settings.backupSettings.encryption.errors.passwordWrong",
            "backup decryption failed: wrong password or corrupted file",
        ),
        CryptoError::InvalidFormat => backup_error(
            "invalidBackup",
            "settings.backupSettings.encryption.errors.invalidBackup",
            "backup file is truncated or not an encrypted backup",
        ),
        CryptoError::RandomFailed => backup_error(
            "encryptionFailed",
            "settings.backupSettings.encryption.errors.encryption",
            "secure random generation failed",
        ),
    }
}

/// Resolve the password for an encrypted backup: explicit user input wins, otherwise
/// fall back to the local credential store (auto restore / restore without retyping).
/// A credential-store read failure is reported as `passwordRequired` rather than a
/// store error: the user who knows the backup password must still get the manual
/// one-shot input prompt, and nothing has been written at this point either way.
fn resolve_password(
    explicit: Option<&str>,
    read_password: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<String, String> {
    if let Some(password) = explicit.filter(|password| !password.is_empty()) {
        return Ok(password.to_string());
    }
    match read_password() {
        Ok(Some(password)) => Ok(password),
        Ok(None) => Err(crypto_error_string(CryptoError::PasswordRequired)),
        Err(store_error) => {
            log::warn!(
                "Backup credential store unavailable, asking for manual password: {store_error}"
            );
            Err(crypto_error_string(CryptoError::PasswordRequired))
        }
    }
}

/// Detect (by header, never by extension) and decrypt encrypted backup bytes.
/// Plaintext input passes through untouched. On any failure nothing has been written.
pub fn prepare_backup_bytes(bytes: Vec<u8>, password: Option<&str>) -> Result<Vec<u8>, String> {
    prepare_backup_bytes_with_password_reader(bytes, password, credentials::read_password)
}

fn prepare_backup_bytes_with_password_reader(
    bytes: Vec<u8>,
    password: Option<&str>,
    read_password: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Vec<u8>, String> {
    if !encryption::is_encrypted(&bytes) {
        return Ok(bytes);
    }
    let resolved = zeroize::Zeroizing::new(resolve_password(password, read_password)?);
    encryption::decrypt(&bytes, &resolved).map_err(crypto_error_string)
}

#[cfg(test)]
mod password_tests {
    use super::*;

    fn encrypted_archive() -> (Vec<u8>, Vec<u8>) {
        use std::io::{Cursor, Write};
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .start_file("fixture.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"backup fixture").unwrap();
        let plain = archive.finish().unwrap().into_inner();
        let encrypted = encryption::encrypt(&plain, "fixture-password").unwrap();
        (plain, encrypted)
    }

    #[test]
    fn plaintext_restore_never_requires_a_credential_store() {
        let plain = b"plaintext ZIP bytes".to_vec();
        assert_eq!(
            prepare_backup_bytes_with_password_reader(plain.clone(), None, || {
                panic!("plaintext restore must not read credentials");
            })
            .unwrap(),
            plain,
        );
    }

    #[test]
    fn encrypted_restore_can_use_an_explicit_password_without_a_credential_store() {
        let (plain, encrypted) = encrypted_archive();
        let prepared =
            prepare_backup_bytes_with_password_reader(encrypted, Some("fixture-password"), || {
                panic!("explicit password must not read the credential store")
            })
            .unwrap();
        assert_eq!(prepared, plain);
        assert!(zip::ZipArchive::new(std::io::Cursor::new(prepared)).is_ok());
    }

    #[test]
    fn missing_and_unavailable_credentials_allow_the_same_manual_password_retry() {
        let (_, encrypted) = encrypted_archive();
        for stored in [
            Ok(None),
            Err("fixture credential store unavailable".to_string()),
        ] {
            let error =
                prepare_backup_bytes_with_password_reader(encrypted.clone(), None, || stored)
                    .unwrap_err();
            let error: serde_json::Value = serde_json::from_str(&error).unwrap();
            assert_eq!(error["type"], "passwordRequired");
        }
    }

    #[test]
    fn wrong_password_or_damaged_archive_never_produces_restore_bytes() {
        let (_, encrypted) = encrypted_archive();
        let mut tampered = encrypted.clone();
        *tampered.last_mut().unwrap() ^= 1;
        let truncated = encrypted[..encryption::ENCRYPTION_MAGIC.len()].to_vec();
        for (bytes, password, expected_error) in [
            (encrypted, "wrong-password", "passwordWrong"),
            (tampered, "fixture-password", "passwordWrong"),
            (truncated, "fixture-password", "invalidBackup"),
        ] {
            let error = prepare_backup_bytes_with_password_reader(bytes, Some(password), || {
                panic!("explicit password must win")
            })
            .unwrap_err();
            let error: serde_json::Value = serde_json::from_str(&error).unwrap();
            assert_eq!(error["type"], expected_error);
        }
    }
}

#[cfg(test)]
mod coverage_tests {
    /// 备份侧写出的每个 `external-configs/<tool>/` 前缀，恢复侧都必须有对应分支。
    ///
    /// 这两份清单在代码里是**两处独立的字符串**：备份侧在 `utils.rs` 的
    /// `add_*_to_zip` 调用里，恢复侧在 `restore_from_archive` 的
    /// `file_name.starts_with(...)` 链里。新增 CLI 时只改一处，另一处不会报错——
    /// 归档里带着文件，恢复时被 `continue` 静默丢弃（2026-10-07：OmO Native
    /// 就是这样漏了整整一轮，5 个文件全部恢复不出来）。
    ///
    /// 断言方式：把本文件里所有 `external-configs/<tool>/` 字面量收集起来，
    /// 确认每一个都出现在一个 `file_name.starts_with` 判断里。
    #[test]
    fn every_backed_up_external_config_tool_has_a_restore_branch() {
        const SOURCE: &str = include_str!("restore.rs");

        // 恢复侧的判断形态：`file_name.starts_with("external-configs/<tool>/")`。
        // 取 `<tool>` 部分：marker 之后到下一个 `/` 为止。
        let handled: std::collections::HashSet<&str> = SOURCE
            .match_indices("file_name.starts_with(\"external-configs/")
            .filter_map(|(index, marker)| {
                let rest = &SOURCE[index + marker.len()..];
                rest.split('/').next()
            })
            .collect();

        assert!(
            !handled.is_empty(),
            "没解析出任何恢复分支——本测试的匹配串已经和实现脱节了"
        );

        // 这些工具在备份侧会写出 external-configs/<tool>/ 条目，恢复侧必须认识。
        // 新增 CLI 时**两个清单一起加**（见 SOP §8.2）。
        for tool in [
            "antigravity",
            "claude",
            "claude_desktop",
            "codex",
            "dsh",
            "geminicli",
            "grok",
            "hermes",
            "kimi",
            "oh_my_pi",
            "omo_native",
            "openclaw",
            "opencode",
            "pi",
            "zcode",
            "workbuddy",
            "codebuddy",
        ] {
            assert!(
                handled.contains(tool),
                "备份会写出 external-configs/{tool}/，但恢复侧没有分支处理它——\
                 恢复时这些文件会被静默丢弃"
            );
        }
    }
}

/// Writes one `external-configs/<workbuddy|codebuddy>/` entry to disk.
///
/// Both products share the same `models.json` layout, so the extraction itself
/// is identical; only the destination directory differs.
fn restore_workbuddy_family_entry<R: Read>(
    restore_dir: &Path,
    relative_path: &str,
    file: &mut R,
    restored_wsl_modules: &mut Vec<String>,
    tool: &str,
) -> Result<(), String> {
    if !restore_dir.exists() {
        fs::create_dir_all(restore_dir)
            .map_err(|e| format!("Failed to create {tool} config directory: {e}"))?;
    }
    // CodeBuddy's user-scope MCP file is a sibling of the config directory
    // (`~/.codebuddy.json`). It cannot go through the generic resolver — that
    // helper rejects `..` to block zip path traversal — so the entry carries a
    // marker name and is redirected to the parent here.
    let sibling_target = if tool == "codebuddy"
        && relative_path == crate::coding::codebuddy::constants::CODEBUDDY_SIBLING_MCP_ENTRY
    {
        restore_dir
            .parent()
            .map(|parent| parent.join(crate::coding::codebuddy::constants::CODEBUDDY_USER_MCP_FILE))
    } else {
        None
    };
    let resolved = match sibling_target {
        Some(path) => Some(path),
        None => resolve_external_config_restore_output_path(restore_dir, relative_path)?,
    };
    let Some(outpath) = resolved else {
        return Ok(());
    };
    if let Some(parent) = outpath.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {tool} parent directory: {e}"))?;
        }
    }
    record_restored_external_config_wsl_module(restored_wsl_modules, tool);
    let mut outfile = File::create(&outpath).map_err(|e| format!("Failed to create file: {e}"))?;
    std::io::copy(file, &mut outfile).map_err(|e| format!("Failed to extract file: {e}"))?;
    Ok(())
}

/// Shared restore pipeline over an already-open archive.
pub(crate) fn restore_from_archive<R: Read + Seek>(
    app_handle: &tauri::AppHandle,
    archive: &mut ZipArchive<R>,
    skip_cli_custom_roots: bool,
) -> Result<RestoreResult, String> {
    let db_path = get_db_path(app_handle)?;

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
    let backup_meta = read_backup_meta_from_archive(archive);

    let restored_sqlite = restore_sqlite_database_snapshot_from_zip(archive, app_handle)?;
    if restored_sqlite {
        sanitize_restored_claude_database_for_current_os(app_handle)?;
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
    .then(|| read_root_dir_override(archive, "external-configs/opencode/root-dir.txt"))
    .flatten();
    let claude_restore_dir_override = should_use_root_override_for_tool(
        "claude",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/claude/root-dir.txt"))
    .flatten();
    let codex_restore_dir_override =
        should_use_root_override_for_tool("codex", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(archive, "external-configs/codex/root-dir.txt"))
            .flatten();
    let grok_restore_dir_override =
        should_use_root_override_for_tool("grok", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(archive, "external-configs/grok/root-dir.txt"))
            .flatten();
    let kimi_restore_dir_override =
        should_use_root_override_for_tool("kimi", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(archive, "external-configs/kimi/root-dir.txt"))
            .flatten();
    let openclaw_restore_dir_override = should_use_root_override_for_tool(
        "openclaw",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/openclaw/root-dir.txt"))
    .flatten();
    let gemini_cli_restore_dir_override = should_use_root_override_for_tool(
        "geminicli",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/geminicli/root-dir.txt"))
    .flatten();
    let antigravity_restore_dir_override = should_use_root_override_for_tool(
        "antigravity",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/antigravity/root-dir.txt"))
    .flatten();
    let zcode_restore_dir_override =
        should_use_root_override_for_tool("zcode", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(archive, "external-configs/zcode/root-dir.txt"))
            .flatten();
    let pi_restore_dir_override =
        should_use_root_override_for_tool("pi", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(archive, "external-configs/pi/root-dir.txt"))
            .flatten();
    let oh_my_pi_restore_dir_override = should_use_root_override_for_tool(
        "oh_my_pi",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/oh_my_pi/root-dir.txt"))
    .flatten();
    let hermes_restore_dir_override = should_use_root_override_for_tool(
        "hermes",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/hermes/root-dir.txt"))
    .flatten();
    let dsh_restore_dir_override =
        should_use_root_override_for_tool("dsh", include_cli_config_files, skip_cli_custom_roots)
            .then(|| read_root_dir_override(archive, "external-configs/dsh/root-dir.txt"))
            .flatten();
    // OmO Native：备份侧一直会打包它的 5 个引擎文件与 root-dir.txt，但恢复侧
    // 此前**完全没有对应分支**——文件会被静默丢弃（2026-10-07 复查发现）。
    let omo_native_restore_dir_override = should_use_root_override_for_tool(
        "omo_native",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/omo_native/root-dir.txt"))
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
    let (antigravity_restore_dir, antigravity_warning) = resolve_restore_dir_override(
        "antigravity",
        antigravity_restore_dir_override,
        get_antigravity_restore_dir()?,
    );
    if let Some(warning) = antigravity_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (zcode_restore_dir, zcode_warning) = resolve_restore_dir_override(
        "zcode",
        zcode_restore_dir_override,
        get_zcode_restore_dir()?,
    );
    if let Some(warning) = zcode_warning {
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

    // 回退目录与 `resolve_omo_native_path_without_db` 的默认值一致（`~/.omo/agent`）。
    let (omo_native_restore_dir, omo_native_warning) = resolve_restore_dir_override(
        "omo_native",
        omo_native_restore_dir_override,
        home_dir.join(".omo").join("agent"),
    );
    if let Some(warning) = omo_native_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    // WorkBuddy desktop / CodeBuddy Code are runtime-file-owned (always backed up),
    // so their restore target follows the normal restore-dir rules: a packaged
    // `root-dir.txt` wins when the caller allows root overrides, otherwise the
    // product's own default directory is used.
    let workbuddy_restore_dir_override = should_use_root_override_for_tool(
        "workbuddy",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/workbuddy/root-dir.txt"))
    .flatten();
    let codebuddy_restore_dir_override = should_use_root_override_for_tool(
        "codebuddy",
        include_cli_config_files,
        skip_cli_custom_roots,
    )
    .then(|| read_root_dir_override(archive, "external-configs/codebuddy/root-dir.txt"))
    .flatten();

    let (workbuddy_restore_dir, workbuddy_warning) = resolve_restore_dir_override(
        "workbuddy",
        workbuddy_restore_dir_override,
        crate::coding::workbuddy::paths::default_config_dir()?,
    );
    if let Some(warning) = workbuddy_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    let (codebuddy_restore_dir, codebuddy_warning) = resolve_restore_dir_override(
        "codebuddy",
        codebuddy_restore_dir_override,
        crate::coding::codebuddy::paths::default_config_dir()?,
    );
    if let Some(warning) = codebuddy_warning {
        push_restore_warning(&mut restore_result, warning);
    }

    // Extract zip contents
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {}", e))?;

        // Normalize path separators for cross-platform compatibility
        // Windows backups may contain backslashes which need to be converted
        let file_name = normalize_restore_entry_name(file.name());

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

                    let Some(outpath) = resolve_external_config_restore_output_path(
                        &opencode_restore_dir,
                        relative_path,
                    )?
                    else {
                        continue;
                    };

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
            } else if file_name.starts_with("external-configs/antigravity/") {
                let relative_path = &file_name["external-configs/antigravity/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "antigravity", relative_path)
                {
                    continue;
                }

                // The global rules file lives under `~/.gemini/config/GEMINI.md` on
                // every platform, while `.env` / `settings.json` / `tmp` belong to the
                // runtime root. Restore each into its real location.
                let base_dir = if relative_path
                    == crate::coding::antigravity::DEFAULT_ANTIGRAVITY_PROMPT_FILE
                {
                    crate::coding::antigravity::get_antigravity_global_rules_dir()?
                } else {
                    antigravity_restore_dir.clone()
                };
                if !base_dir.exists() {
                    fs::create_dir_all(&base_dir).map_err(|e| {
                        format!("Failed to create Antigravity config directory: {}", e)
                    })?;
                }

                let Some(outpath) =
                    resolve_external_config_restore_output_path(&base_dir, relative_path)?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create Antigravity parent directory: {}", e)
                        })?;
                    }
                }
                record_restored_external_config_wsl_module(
                    &mut restored_wsl_modules,
                    "antigravity",
                );
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            } else if file_name.starts_with("external-configs/zcode/") {
                let relative_path = &file_name["external-configs/zcode/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "zcode", relative_path) {
                    continue;
                }

                // Every ZCode entry keeps its path relative to the data root,
                // so it lands back where ZCode reads it.
                let restore_relative_path = relative_path;
                if !zcode_restore_dir.exists() {
                    fs::create_dir_all(&zcode_restore_dir).map_err(|e| {
                        format!("Failed to create ZCode config directory: {}", e)
                    })?;
                }

                let Some(outpath) = resolve_external_config_restore_output_path(
                    &zcode_restore_dir,
                    restore_relative_path,
                )?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create ZCode parent directory: {}", e)
                        })?;
                    }
                }
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "zcode");
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
            } else if file_name.starts_with("external-configs/workbuddy/") {
                let relative_path = &file_name["external-configs/workbuddy/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }
                if should_filter_external_config_entry(&filter_rules, "workbuddy", relative_path) {
                    continue;
                }
                restore_workbuddy_family_entry(
                    &workbuddy_restore_dir,
                    relative_path,
                    &mut file,
                    &mut restored_wsl_modules,
                    "workbuddy",
                )?;
            } else if file_name.starts_with("external-configs/codebuddy/") {
                let relative_path = &file_name["external-configs/codebuddy/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }
                if should_filter_external_config_entry(&filter_rules, "codebuddy", relative_path) {
                    continue;
                }
                restore_workbuddy_family_entry(
                    &codebuddy_restore_dir,
                    relative_path,
                    &mut file,
                    &mut restored_wsl_modules,
                    "codebuddy",
                )?;
            } else if file_name.starts_with("external-configs/omo_native/") {
                let relative_path = &file_name["external-configs/omo_native/".len()..];
                if relative_path.is_empty()
                    || file_name.ends_with('/')
                    || relative_path == "root-dir.txt"
                {
                    continue;
                }

                if should_filter_external_config_entry(&filter_rules, "omo_native", relative_path) {
                    continue;
                }

                if !omo_native_restore_dir.exists() {
                    fs::create_dir_all(&omo_native_restore_dir).map_err(|e| {
                        format!("Failed to create OmO Native config directory: {}", e)
                    })?;
                }

                let Some(outpath) = resolve_external_config_restore_output_path(
                    &omo_native_restore_dir,
                    relative_path,
                )?
                else {
                    continue;
                };
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create OmO Native parent directory: {}", e)
                        })?;
                    }
                }
                record_restored_external_config_wsl_module(&mut restored_wsl_modules, "omo_native");
                let mut outfile =
                    File::create(&outpath).map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
                // `auth.json` 是明文凭据，与 Pi 的 `auth.json`、dsh 的
                // `.credentials.yaml` 同等对待。
                if relative_path == "auth.json" {
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
                let skills_dir = get_skills_dir(app_handle)?;
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

                let image_assets_dir = get_image_assets_dir(app_handle)?;
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

    restore_custom_backup_entries(archive)?;

    let need_reapply =
        should_reapply_applied_runtime(skipped_optional_cli_runtime, backup_meta.as_ref());
    restore_result.will_reapply_applied = need_reapply;
    write_post_restore_flags(app_handle, need_reapply, &restored_wsl_modules)?;

    Ok(restore_result)
}
