use std::path::Path;
use std::process::Command;
use super::key_file;
use super::session::SshSession;
use super::types::{SSHConnection, SSHConnectionResult, SSHFileMapping, SyncResult};

// ============================================================================
// Connection Testing
// ============================================================================

/// 测试 SSH 连接（独立短连接，不复用主连接）
/// 用于测试未保存的连接配置
pub fn test_connection(conn: &SSHConnection, app_data_dir: &Path) -> SSHConnectionResult {
    let target = format!("{}@{}", conn.username, conn.host);

    let mut cmd = if conn.auth_method == "password" && !conn.password.is_empty() {
        let mut c = Command::new("sshpass");
        c.args(["-e", "ssh"]);           // 修复：-e 替代 -p
        c.env("SSHPASS", &conn.password); // 修复：环境变量传递密码
        c
    } else {
        Command::new("ssh")
    };

    cmd.args(["-p", &conn.port.to_string()]);
    cmd.args(["-o", "StrictHostKeyChecking=accept-new"]);
    cmd.args(["-o", "ConnectTimeout=10"]);
    if conn.auth_method == "key" {
        let key_path = key_file::resolve_key_path(
            app_data_dir,
            &conn.private_key_path,
            &conn.private_key_content,
        ).unwrap_or_default();
        if !key_path.is_empty() {
            cmd.args(["-i", &key_path]);
            if conn.passphrase.is_empty() {
                cmd.args(["-o", "BatchMode=yes"]);
            }
        }
    }
    cmd.arg(&target);
    cmd.args(["echo __connected__ && uname -a"]);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if output.status.success() && stdout.contains("__connected__") {
                let server_info = stdout
                    .lines()
                    .find(|line| !line.contains("__connected__"))
                    .map(|s| s.trim().to_string());
                SSHConnectionResult {
                    connected: true,
                    error: None,
                    server_info,
                }
            } else {
                SSHConnectionResult {
                    connected: false,
                    error: Some(if stderr.is_empty() {
                        "Connection failed".to_string()
                    } else {
                        stderr.trim().to_string()
                    }),
                    server_info: None,
                }
            }
        }
        Err(e) => SSHConnectionResult {
            connected: false,
            error: Some(format!("Failed to execute ssh command: {}", e)),
            server_info: None,
        },
    }
}

// ============================================================================
// Path Expansion
// ============================================================================

/// Expand local path: ~, $HOME, %USERPROFILE%
pub fn expand_local_path(path: &str) -> Result<String, String> {
    let mut result = path.to_string();

    // Expand ~ to home directory
    if result.starts_with("~/") || result == "~" {
        if let Some(home) = dirs::home_dir() {
            result = result.replacen("~", &home.to_string_lossy(), 1);
        }
    }

    // Common environment variables
    let vars = [
        ("USERPROFILE", std::env::var("USERPROFILE")),
        ("APPDATA", std::env::var("APPDATA")),
        ("LOCALAPPDATA", std::env::var("LOCALAPPDATA")),
        (
            "HOME",
            std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")),
        ),
    ];

    for (var, value) in vars {
        if let Ok(val) = value {
            result = result.replace(&format!("%{}%", var), &val);
            result = result.replace(&format!("${}", var), &val);
        }
    }

    Ok(result)
}

// ============================================================================
// File Sync Operations (复用长连接)
// ============================================================================

/// 同步单个文件到远程（通过 SCP）
pub fn sync_single_file(
    local_path: &str,
    remote_path: &str,
    session: &SshSession,
) -> Result<Vec<String>, String> {
    let expanded = expand_local_path(local_path)?;

    if !Path::new(&expanded).exists() {
        return Ok(vec![]);
    }

    let remote_target = remote_path.replace("~", "$HOME");
    let target = session.target_str()?;

    // 创建远程目录
    let mkdir_cmd = format!("mkdir -p \"$(dirname \"{}\")\"", remote_target);
    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&mkdir_cmd);
    let output = ssh
        .output()
        .map_err(|e| format!("创建远程目录失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("创建远程目录失败: {}", stderr.trim()));
    }

    // SCP 传输文件
    let remote_dest = format!("{}:{}", target, remote_path);
    let mut scp = session.create_scp_command()?;
    scp.args([&expanded, &remote_dest]);

    let output = scp
        .output()
        .map_err(|e| format!("SCP 执行失败: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", local_path, remote_path)])
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("SCP 失败: {}", stderr.trim()))
    }
}

/// 同步整个目录到远程（通过 SCP -r）
pub fn sync_directory(
    local_path: &str,
    remote_path: &str,
    session: &SshSession,
) -> Result<Vec<String>, String> {
    let expanded = expand_local_path(local_path)?;

    if !Path::new(&expanded).exists() {
        return Ok(vec![]);
    }

    let remote_target = remote_path.replace("~", "$HOME");

    // 安全检查：禁止对根路径或家目录执行 rm -rf
    let trimmed = remote_path.trim();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "~" || trimmed == "$HOME" {
        return Err(format!("拒绝同步到危险路径: '{}'", remote_path));
    }

    let target = session.target_str()?;

    // 创建远程父目录并删除已存在的目录
    let mkdir_cmd = format!(
        "mkdir -p \"$(dirname \"{}\")\" && rm -rf \"{}\"",
        remote_target, remote_target
    );
    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&mkdir_cmd);
    let output = ssh
        .output()
        .map_err(|e| format!("准备远程目录失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("准备远程目录失败: {}", stderr.trim()));
    }

    // SCP -r 递归传输目录
    let remote_dest = format!("{}:{}", target, remote_path);
    let mut scp = session.create_scp_command()?;
    scp.args(["-r", &expanded, &remote_dest]);

    let output = scp
        .output()
        .map_err(|e| format!("SCP 执行失败: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", local_path, remote_path)])
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("SCP 目录同步失败: {}", stderr.trim()))
    }
}

/// 同步符合 glob 模式的文件到远程
pub fn sync_pattern_files(
    local_pattern: &str,
    remote_dir: &str,
    session: &SshSession,
) -> Result<Vec<String>, String> {
    let expanded = expand_local_path(local_pattern)?;

    // 使用 glob 查找匹配的文件
    let matches: Vec<_> = glob::glob(&expanded)
        .map_err(|e| format!("无效的 glob 模式: {}", e))?
        .filter_map(|entry| entry.ok())
        .collect();

    if matches.is_empty() {
        return Ok(vec![]);
    }

    let remote_target = remote_dir.replace("~", "$HOME");
    let target = session.target_str()?;

    // 创建远程目录
    let mkdir_cmd = format!("mkdir -p \"{}\"", remote_target);
    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&mkdir_cmd);
    let _ = ssh.output();

    let mut synced = vec![];
    for file_path in &matches {
        let file_str = file_path.to_string_lossy().to_string();
        let file_name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let remote_dest = format!("{}:{}/{}", target, remote_dir, file_name);
        let mut scp = session.create_scp_command()?;
        scp.args([&file_str, &remote_dest]);

        match scp.output() {
            Ok(output) if output.status.success() => {
                synced.push(format!("{} -> {}/{}", file_str, remote_dir, file_name));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::warn!("SCP 模式文件失败 {}: {}", file_str, stderr.trim());
            }
            Err(e) => {
                log::warn!("SCP 模式文件失败 {}: {}", file_str, e);
            }
        }
    }

    Ok(synced)
}

/// 同步单个文件映射
pub fn sync_file_mapping(
    mapping: &SSHFileMapping,
    session: &SshSession,
) -> Result<Vec<String>, String> {
    if mapping.is_directory {
        sync_directory(&mapping.local_path, &mapping.remote_path, session)
    } else if mapping.is_pattern {
        sync_pattern_files(&mapping.local_path, &mapping.remote_path, session)
    } else {
        sync_single_file(&mapping.local_path, &mapping.remote_path, session)
    }
}

/// 同步所有启用的文件映射
pub fn sync_mappings(
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

    for mapping in filtered_mappings {
        match sync_file_mapping(mapping, session) {
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

/// 从远程服务器读取文件内容
pub fn read_remote_file(session: &SshSession, path: &str) -> Result<String, String> {
    let remote_path = path.replace("~", "$HOME");

    let command = format!(
        "if [ -f \"{}\" ]; then cat \"{}\"; else echo ''; fi",
        remote_path, remote_path
    );

    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&command);

    let output = ssh
        .output()
        .map_err(|e| format!("读取远程文件失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("SSH 命令失败: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 将内容写入远程文件
pub fn write_remote_file(session: &SshSession, path: &str, content: &str) -> Result<(), String> {
    let remote_path = path.replace("~", "$HOME");

    let command = format!(
        "mkdir -p \"$(dirname \"{}\")\" && cat > \"{}\"",
        remote_path, remote_path
    );

    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&command);
    ssh.stdin(std::process::Stdio::piped());

    let mut child = ssh
        .spawn()
        .map_err(|e| format!("启动 SSH 命令失败: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| format!("写入 stdin 失败: {}", e))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("等待 SSH 命令失败: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err("SSH 写入命令失败".to_string())
    }
}

/// 在远程创建符号链接
pub fn create_remote_symlink(
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

    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&command);

    let output = ssh
        .output()
        .map_err(|e| format!("创建远程符号链接失败: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("远程符号链接失败: {}", stderr.trim()))
    }
}

/// 删除远程文件或目录
pub fn remove_remote_path(session: &SshSession, path: &str) -> Result<(), String> {
    // 安全检查：禁止删除空路径或根路径
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "~" || trimmed == "$HOME" {
        return Err(format!("拒绝删除危险路径: '{}'", path));
    }

    let remote_path = path.replace("~", "$HOME");
    let command = format!("rm -rf \"{}\"", remote_path);

    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&command);

    let output = ssh
        .output()
        .map_err(|e| format!("删除远程路径失败: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("远程删除失败: {}", stderr.trim()))
    }
}

/// 列出远程目录中的子目录
pub fn list_remote_dir(session: &SshSession, path: &str) -> Result<Vec<String>, String> {
    let remote_path = path.replace("~", "$HOME");
    let command = format!(
        "if [ -d \"{}\" ]; then ls -1 \"{}\"; fi",
        remote_path, remote_path
    );

    let mut ssh = session.create_ssh_command()?;
    ssh.arg(&command);

    let output = ssh
        .output()
        .map_err(|e| format!("列出远程目录失败: {}", e))?;

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// 检查远程符号链接是否存在并指向预期的目标
pub fn check_remote_symlink_exists(
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

    let mut ssh = match session.create_ssh_command() {
        Ok(cmd) => cmd,
        Err(_) => return false,
    };
    ssh.arg(&command);

    if let Ok(output) = ssh.output() {
        String::from_utf8_lossy(&output.stdout).trim() == "yes"
    } else {
        false
    }
}
