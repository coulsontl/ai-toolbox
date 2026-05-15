use super::session::{self, upload_file_via_sftp, SshSession};
use super::types::{
    normalize_directory_excludes, SSHConnection, SSHConnectionResult, SSHFileMapping, SyncResult,
};
use std::path::{Path, PathBuf};

type CurrentFileReporter<'a> = &'a (dyn Fn(String) + Send + Sync);

fn mapping_kind(mapping: &SSHFileMapping) -> &'static str {
    if mapping.is_directory {
        "directory"
    } else if mapping.is_pattern {
        "pattern"
    } else {
        "file"
    }
}

fn normalize_display_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn file_name_or_path(path: &Path, fallback: &str) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(normalize_display_path)
        .unwrap_or_else(|| normalize_display_path(fallback))
}

fn glob_static_base(pattern: &str) -> Option<PathBuf> {
    let meta_index = pattern
        .char_indices()
        .find(|(_, ch)| matches!(ch, '*' | '?' | '['))
        .map(|(index, _)| index)?;
    let prefix = &pattern[..meta_index];
    if prefix.is_empty() {
        return None;
    }

    let prefix_path = Path::new(prefix);
    if prefix.ends_with('/') || prefix.ends_with('\\') {
        Some(prefix_path.to_path_buf())
    } else {
        prefix_path.parent().map(Path::to_path_buf)
    }
}

fn pattern_file_display_path(expanded_pattern: &str, file_path: &Path) -> String {
    if let Some(base_dir) = glob_static_base(expanded_pattern) {
        if let Ok(relative_path) = file_path.strip_prefix(&base_dir) {
            let display = normalize_display_path(&relative_path.to_string_lossy());
            if !display.is_empty() {
                return display;
            }
        }
    }

    file_name_or_path(file_path, &file_path.to_string_lossy())
}

// ============================================================================
// Connection Testing
// ============================================================================

/// 测试 SSH 连接（独立短连接，不复用主连接）
/// 用于测试未保存的连接配置
pub async fn test_connection(conn: &SSHConnection) -> SSHConnectionResult {
    match session::test_connection_with_command(conn, "uname -a").await {
        Ok(output) => {
            let server_info = output.trim().to_string();
            SSHConnectionResult {
                connected: true,
                error: None,
                server_info: if server_info.is_empty() {
                    None
                } else {
                    Some(server_info)
                },
            }
        }
        Err(e) => SSHConnectionResult {
            connected: false,
            error: Some(e),
            server_info: None,
        },
    }
}

// ============================================================================
// Path Expansion
// ============================================================================

/// Expand local path: ~, $HOME, %USERPROFILE%
pub fn expand_local_path(path: &str) -> Result<String, String> {
    super::super::expand_local_path(path)
}

// ============================================================================
// File Sync Operations (复用长连接)
// ============================================================================

/// 同步单个文件到远程（通过 SFTP）
pub async fn sync_single_file_with_progress(
    local_path: &str,
    remote_path: &str,
    session: &SshSession,
    current_file_reporter: Option<CurrentFileReporter<'_>>,
) -> Result<Vec<String>, String> {
    let expanded = expand_local_path(local_path)?;
    log::trace!(
        "SSH single file sync start: local_path={}, expanded_local_path={}, remote_path={}",
        local_path,
        expanded,
        remote_path
    );

    if !Path::new(&expanded).exists() {
        log::warn!(
            "SSH single file sync skipped because local file does not exist: local_path={}, expanded_local_path={}, remote_path={}",
            local_path,
            expanded,
            remote_path
        );
        return Ok(vec![]);
    }

    if let Some(reporter) = current_file_reporter {
        reporter(file_name_or_path(Path::new(&expanded), local_path));
    }

    let remote_target = remote_path.replace("~", "$HOME");

    // 创建远程目录
    let mkdir_cmd = format!("mkdir -p \"$(dirname \"{}\")\"", remote_target);
    session.exec_command(&mkdir_cmd).await?;

    // SFTP 上传文件
    session.upload_file(&expanded, remote_path).await?;
    log::trace!(
        "SSH single file sync uploaded successfully: expanded_local_path={}, remote_path={}",
        expanded,
        remote_path
    );

    Ok(vec![format!("{} -> {}", local_path, remote_path)])
}

/// 同步整个目录到远程（通过 SFTP）
/// 使用临时目录 + mv 实现原子替换，防止上传中断导致数据丢失
pub async fn sync_directory_with_progress(
    local_path: &str,
    remote_path: &str,
    session: &SshSession,
    directory_excludes: &[String],
    current_file_reporter: Option<CurrentFileReporter<'_>>,
) -> Result<Vec<String>, String> {
    let expanded = expand_local_path(local_path)?;
    let directory_excludes = normalize_directory_excludes(directory_excludes);
    log::trace!(
        "SSH directory sync start: local_path={}, expanded_local_path={}, remote_path={}, directory_excludes={:?}",
        local_path,
        expanded,
        remote_path,
        directory_excludes
    );

    if !Path::new(&expanded).exists() {
        log::warn!(
            "SSH directory sync skipped because local path does not exist: local_path={}, expanded_local_path={}, remote_path={}",
            local_path,
            expanded,
            remote_path
        );
        return Ok(vec![]);
    }

    let remote_target = remote_path.replace("~", "$HOME");

    // 安全检查：禁止对根路径或家目录执行操作
    let trimmed = remote_path.trim();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "~" || trimmed == "$HOME" {
        return Err(format!("拒绝同步到危险路径: '{}'", remote_path));
    }

    // 使用临时目录上传，完成后原子替换
    let tmp_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let tmp_remote_path = format!("{}.tmp_{}", remote_path, tmp_suffix);
    let tmp_remote_target = format!("{}.tmp_{}", remote_target, tmp_suffix);

    // 创建远程父目录
    let mkdir_cmd = format!("mkdir -p \"$(dirname \"{}\")\"", remote_target);
    session.exec_command(&mkdir_cmd).await?;

    // SFTP 递归上传到临时目录（upload_dir 内部会展开 ~ 和 $HOME）
    session
        .upload_dir_with_excludes_and_progress(
            &expanded,
            &tmp_remote_path,
            &directory_excludes,
            current_file_reporter,
        )
        .await?;

    // 原子替换：rm 旧目录 + mv 临时目录到目标
    let swap_cmd = format!(
        "rm -rf \"{}\" && mv \"{}\" \"{}\"",
        remote_target, tmp_remote_target, remote_target
    );
    if let Err(e) = session.exec_command(&swap_cmd).await {
        // 替换失败，清理临时目录
        let _ = session
            .exec_command(&format!("rm -rf \"{}\"", tmp_remote_target))
            .await;
        return Err(format!("目录替换失败: {}", e));
    }
    log::trace!(
        "SSH directory sync uploaded successfully: expanded_local_path={}, remote_path={}, tmp_remote_path={}, directory_excludes={:?}",
        expanded,
        remote_path,
        tmp_remote_path,
        directory_excludes
    );

    Ok(vec![format!("{} -> {}", local_path, remote_path)])
}

/// 同步符合 glob 模式的文件到远程
pub async fn sync_pattern_files_with_progress(
    local_pattern: &str,
    remote_dir: &str,
    session: &SshSession,
    current_file_reporter: Option<CurrentFileReporter<'_>>,
) -> Result<Vec<String>, String> {
    let expanded = expand_local_path(local_pattern)?;
    log::trace!(
        "SSH pattern sync start: local_pattern={}, expanded_pattern={}, remote_dir={}",
        local_pattern,
        expanded,
        remote_dir
    );

    // 使用 glob 查找匹配的文件
    let matches: Vec<_> = glob::glob(&expanded)
        .map_err(|e| format!("无效的 glob 模式: {}", e))?
        .filter_map(|entry| entry.ok())
        .collect();

    if matches.is_empty() {
        log::warn!(
            "SSH pattern sync skipped because no local files matched: local_pattern={}, expanded_pattern={}, remote_dir={}",
            local_pattern,
            expanded,
            remote_dir
        );
        return Ok(vec![]);
    }

    let remote_target = remote_dir.replace("~", "$HOME");

    // 创建远程目录
    let mkdir_cmd = format!("mkdir -p \"{}\"", remote_target);
    session.exec_command(&mkdir_cmd).await?;

    // 复用同一个 SFTP session 上传所有文件
    let sftp = session.create_sftp_session().await?;

    let mut synced = vec![];
    let mut failed_upload_count = 0usize;
    for file_path in &matches {
        let file_str = file_path.to_string_lossy().to_string();
        let file_name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let remote_dest = format!("{}/{}", remote_dir.trim_end_matches('/'), file_name);

        if let Some(reporter) = current_file_reporter {
            reporter(pattern_file_display_path(&expanded, file_path));
        }

        match upload_file_via_sftp(&sftp, &file_str, &remote_dest).await {
            Ok(()) => {
                synced.push(format!(
                    "{} -> {}/{}",
                    file_str,
                    remote_dir.trim_end_matches('/'),
                    file_name
                ));
            }
            Err(e) => {
                failed_upload_count += 1;
                log::warn!("SFTP 模式文件失败 {}: {}", file_str, e);
            }
        }
    }

    log::trace!(
        "SSH pattern sync finished: local_pattern={}, matched_files={}, uploaded_files={}, failed_uploads={}, remote_dir={}",
        local_pattern,
        matches.len(),
        synced.len(),
        failed_upload_count,
        remote_dir
    );
    if synced.is_empty() {
        log::warn!(
            "SSH pattern sync produced zero uploaded files after matching local files: local_pattern={}, matched_files={}, remote_dir={}",
            local_pattern,
            matches.len(),
            remote_dir
        );
    }

    Ok(synced)
}

/// 同步单个文件映射
pub async fn sync_file_mapping(
    mapping: &SSHFileMapping,
    session: &SshSession,
) -> Result<Vec<String>, String> {
    sync_file_mapping_with_progress(mapping, session, None).await
}

pub async fn sync_file_mapping_with_progress(
    mapping: &SSHFileMapping,
    session: &SshSession,
    current_file_reporter: Option<CurrentFileReporter<'_>>,
) -> Result<Vec<String>, String> {
    let kind = mapping_kind(mapping);
    log::trace!(
        "SSH sync mapping start: id={}, name={}, module={}, kind={}, local_path={}, remote_path={}",
        mapping.id,
        mapping.name,
        mapping.module,
        kind,
        mapping.local_path,
        mapping.remote_path
    );

    let result = if mapping.is_directory {
        sync_directory_with_progress(
            &mapping.local_path,
            &mapping.remote_path,
            session,
            &mapping.directory_excludes,
            current_file_reporter,
        )
        .await
    } else if mapping.is_pattern {
        sync_pattern_files_with_progress(
            &mapping.local_path,
            &mapping.remote_path,
            session,
            current_file_reporter,
        )
        .await
    } else {
        sync_single_file_with_progress(
            &mapping.local_path,
            &mapping.remote_path,
            session,
            current_file_reporter,
        )
        .await
    };

    match &result {
        Ok(files) if files.is_empty() => {
            log::warn!(
                "SSH sync mapping finished without uploaded files: id={}, name={}, module={}, kind={}, local_path={}, remote_path={}",
                mapping.id,
                mapping.name,
                mapping.module,
                kind,
                mapping.local_path,
                mapping.remote_path
            );
        }
        Ok(files) => {
            log::trace!(
                "SSH sync mapping finished successfully: id={}, name={}, module={}, kind={}, uploaded_files={}, remote_path={}",
                mapping.id,
                mapping.name,
                mapping.module,
                kind,
                files.len(),
                mapping.remote_path
            );
        }
        Err(error) => {
            log::warn!(
                "SSH sync mapping execution failed: id={}, name={}, module={}, kind={}, local_path={}, remote_path={}, error={}",
                mapping.id,
                mapping.name,
                mapping.module,
                kind,
                mapping.local_path,
                mapping.remote_path,
                error
            );
        }
    }

    result
}

/// 同步所有启用的文件映射
pub async fn sync_mappings(
    mappings: &[SSHFileMapping],
    session: &SshSession,
    module_filter: Option<&str>,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];

    let filtered_mappings: Vec<_> = mappings
        .iter()
        .filter(|m| m.enabled)
        .filter(|m| module_filter.is_none() || Some(m.module.as_str()) == module_filter)
        .collect();
    log::info!(
        "SSH sync_mappings start: total_mappings={}, selected_mappings={}, module_filter={:?}",
        mappings.len(),
        filtered_mappings.len(),
        module_filter
    );

    for mapping in filtered_mappings {
        match sync_file_mapping(mapping, session).await {
            Ok(files) if files.is_empty() => {
                skipped_files.push(mapping.name.clone());
            }
            Ok(files) => {
                synced_files.extend(files);
            }
            Err(e) => {
                errors.push(format!("{}: {}", mapping.name, e));
            }
        }
    }

    log::info!(
        "SSH sync_mappings completed: success={}, synced_files={}, skipped_files={}, errors={}, module_filter={:?}",
        errors.is_empty(),
        synced_files.len(),
        skipped_files.len(),
        errors.len(),
        module_filter
    );
    SyncResult {
        success: errors.is_empty(),
        synced_files,
        skipped_files,
        errors,
    }
}

// ============================================================================
// Remote File Operations (复用长连接)
// ============================================================================

/// Check if content looks like valid UTF-8 text config (not binary/corrupted/wrong encoding)
///
/// `exec_command` uses `String::from_utf8_lossy`, which replaces invalid UTF-8 bytes
/// with U+FFFD (�). If the content contains replacement characters, it means the file
/// is not valid UTF-8 (likely GBK/GB2312).
pub fn check_file_encoding(content: &str, file_path: &str) -> Result<(), String> {
    if content.contains('\u{FFFD}') {
        return Err(format!(
            "文件 {} 编码不是 UTF-8（可能是 GBK/GB2312），请手动转换后重试。\n\
             修复方法: iconv -f GBK -t UTF-8 \"{}\" -o \"{}.tmp\" && mv \"{}.tmp\" \"{}\"",
            file_path, file_path, file_path, file_path, file_path
        ));
    }

    // Check for binary/corrupted content
    let non_printable_count = content
        .chars()
        .take(256)
        .filter(|c| !c.is_ascii_graphic() && !c.is_ascii_whitespace())
        .count();
    let sample_len = content.chars().take(256).count().max(1);
    if non_printable_count * 10 >= sample_len {
        return Err(format!(
            "文件 {} 内容疑似二进制或已损坏，请检查文件内容是否正确",
            file_path
        ));
    }

    Ok(())
}

/// 从远程服务器读取文件内容（原始版本，不做编码检测）
///
/// 适用于我们自己控制的文件（hash 文件等），不需要编码检测。
/// 对于用户配置文件（claude.json, opencode.json 等），应使用 `read_remote_file`。
pub async fn read_remote_file_raw(session: &SshSession, path: &str) -> Result<String, String> {
    let remote_path = path.replace("~", "$HOME");

    let command = format!(
        "if [ -f \"{}\" ]; then cat \"{}\"; else echo ''; fi",
        remote_path, remote_path
    );

    session.exec_command(&command).await
}

/// 查询远端登录用户的真实 $HOME。
///
/// 用于把 `~` 嵌入到文件**内容**(例如 Claude `known_marketplaces.json`
/// 的 `installLocation`)前先解析成绝对路径。读写文件路径仍然走 shell 的
/// `$HOME` 展开,这里只服务于"作为字段值落盘"的场景 —— Claude CLI 2.1.126+
/// 不会展开字段值里的 `~`,直接判定 corrupted。
pub async fn get_remote_user_home(session: &SshSession) -> Result<String, String> {
    let raw = session.exec_command("echo $HOME").await?;
    let home = raw.trim().to_string();
    if home.is_empty() {
        return Err("远端 $HOME 为空".to_string());
    }
    Ok(home)
}

/// 从远程服务器读取文件内容，带编码检测和自动 GBK→UTF-8 转换
///
/// Flow:
/// 1. Read raw content
/// 2. Validate encoding — if valid UTF-8, return directly
/// 3. If non-UTF-8, try iconv GBK→UTF-8 on remote server
/// 4. If iconv succeeds, return converted content
/// 5. If iconv fails, return error with instructions
pub async fn read_remote_file(session: &SshSession, path: &str) -> Result<String, String> {
    let content = read_remote_file_raw(session, path).await?;

    // Already valid UTF-8 — return directly
    if check_file_encoding(&content, path).is_ok() {
        return Ok(content);
    }

    // Non-UTF-8 detected, try iconv GBK→UTF-8 on remote
    log::warn!(
        "File {} is non-UTF-8, attempting remote iconv GBK→UTF-8...",
        path
    );

    let remote_path = path.replace("~", "$HOME");
    let convert_cmd = format!("iconv -f GBK -t UTF-8 \"{}\" 2>/dev/null", remote_path);

    match session.exec_command(&convert_cmd).await {
        Ok(converted) if check_file_encoding(&converted, path).is_ok() => {
            log::info!("Auto-converted {} from GBK to UTF-8", path);
            Ok(converted)
        }
        _ => Err(format!(
            "文件 {} 编码不是 UTF-8，自动转换失败。请手动转换后重试。\n\
             修复方法: iconv -f GBK -t UTF-8 \"{}\" -o \"{}.tmp\" && mv \"{}.tmp\" \"{}\"",
            path, path, path, path, path
        )),
    }
}

/// 将内容写入远程文件
pub async fn write_remote_file(
    session: &SshSession,
    path: &str,
    content: &str,
) -> Result<(), String> {
    let remote_path = path.replace("~", "$HOME");

    let command = format!(
        "mkdir -p \"$(dirname \"{}\")\" && cat > \"{}\"",
        remote_path, remote_path
    );

    session
        .exec_command_with_stdin(&command, content.as_bytes())
        .await
}

/// 在远程创建符号链接
pub async fn create_remote_symlink(
    session: &SshSession,
    target: &str,
    link_path: &str,
) -> Result<(), String> {
    let target_expanded = target.replace("~", "$HOME");
    let link_expanded = link_path.replace("~", "$HOME");

    let command = format!(
        "mkdir -p \"$(dirname \"{}\")\" && rm -rf \"{}\" && ln -s \"{}\" \"{}\"",
        link_expanded, link_expanded, target_expanded, link_expanded
    );

    session.exec_command(&command).await?;
    Ok(())
}

/// 删除远程文件或目录
pub async fn remove_remote_path(session: &SshSession, path: &str) -> Result<(), String> {
    // 安全检查：禁止删除空路径或根路径
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "~" || trimmed == "$HOME" {
        return Err(format!("拒绝删除危险路径: '{}'", path));
    }

    let remote_path = path.replace("~", "$HOME");
    let command = format!("rm -rf \"{}\"", remote_path);

    session.exec_command(&command).await?;
    Ok(())
}

/// 列出远程目录中的子目录
pub async fn list_remote_dir(session: &SshSession, path: &str) -> Result<Vec<String>, String> {
    let remote_path = path.replace("~", "$HOME");
    let command = format!(
        "if [ -d \"{}\" ]; then ls -1 \"{}\"; fi",
        remote_path, remote_path
    );

    let output = session.exec_command(&command).await?;

    Ok(output
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// 检查远程符号链接是否存在并指向预期的目标
pub async fn check_remote_symlink_exists(
    session: &SshSession,
    link_path: &str,
    expected_target: &str,
) -> bool {
    let link_expanded = link_path.replace("~", "$HOME");
    let target_expanded = expected_target.replace("~", "$HOME");
    let command = format!(
        "[ -L \"{}\" ] && [ \"$(readlink \"{}\")\" = \"{}\" ] && echo yes || echo no",
        link_expanded, link_expanded, target_expanded
    );

    match session.exec_command(&command).await {
        Ok(output) => output.trim() == "yes",
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::pattern_file_display_path;
    use std::path::Path;

    #[test]
    fn pattern_file_display_path_uses_static_glob_root() {
        let pattern = "/home/me/.config/opencode/*.mjs";
        let file_path = Path::new("/home/me/.config/opencode/plugin.mjs");

        assert_eq!(pattern_file_display_path(pattern, file_path), "plugin.mjs");
    }

    #[test]
    fn pattern_file_display_path_preserves_nested_relative_path() {
        let pattern = "/home/me/.config/opencode/**/*.mjs";
        let file_path = Path::new("/home/me/.config/opencode/plugins/plugin.mjs");

        assert_eq!(
            pattern_file_display_path(pattern, file_path),
            "plugins/plugin.mjs"
        );
    }
}
