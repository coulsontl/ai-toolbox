//! SSH 持久连接会话管理（基于 russh 库）
//!
//! 维护一个进程内持久 SSH 连接，所有操作复用该连接。
//! 网络断开后自动重连。跨平台兼容（Windows/macOS/Linux）。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use log::{info, warn};
use russh::keys::ssh_key;
use russh::{client, ChannelMsg, Disconnect};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::key_file;
use super::types::SSHConnection;

/// 加载私钥：优先从内容直接解析，否则从文件路径加载
fn load_private_key(conn: &SSHConnection) -> Result<russh::keys::PrivateKey, String> {
    let passphrase = if conn.passphrase.is_empty() {
        None
    } else {
        Some(conn.passphrase.as_str())
    };

    let content = conn.private_key_content.trim();
    if !content.is_empty() && key_file::is_private_key_content(content) {
        // 直接从内存解析私钥，不写入文件
        russh::keys::decode_secret_key(content, passphrase)
            .map_err(|e| format!("解析私钥内容失败: {}", e))
    } else if !conn.private_key_path.is_empty() {
        let expanded = crate::coding::expand_local_path(&conn.private_key_path)?;
        russh::keys::load_secret_key(&expanded, passphrase)
            .map_err(|e| format!("加载私钥文件失败: {}", e))
    } else {
        Err("未提供私钥路径或私钥内容".to_string())
    }
}

/// SSH 会话状态
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    /// 未连接
    Disconnected,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// 连接失败
    Failed(String),
}

/// russh 客户端 Handler 实现
struct SshHandler;

impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO: 实现 known_hosts 验证以达到真正的 StrictHostKeyChecking=accept-new 行为
        // 当前行为等同于 StrictHostKeyChecking=no，无条件接受所有服务器密钥
        Ok(true)
    }
}

/// 对已建立的 SSH 连接进行用户认证（密码、公钥或 none）
async fn authenticate(
    session: &mut client::Handle<SshHandler>,
    conn: &SSHConnection,
) -> Result<(), String> {
    if conn.auth_method == "none" {
        let auth_result = session
            .authenticate_none(&conn.username)
            .await
            .map_err(|e| format!("无认证失败: {}", e))?;
        if !auth_result.success() {
            return Err("无认证失败: 服务器不接受 none 认证".to_string());
        }
    } else if conn.auth_method == "password" && !conn.password.is_empty() {
        let auth_result = session
            .authenticate_password(&conn.username, &conn.password)
            .await
            .map_err(|e| format!("密码认证失败: {}", e))?;
        if !auth_result.success() {
            return Err("密码认证失败: 用户名或密码错误".to_string());
        }
    } else if conn.auth_method == "key" {
        let key_pair = load_private_key(conn)?;

        let auth_result = session
            .authenticate_publickey(
                &conn.username,
                russh::keys::PrivateKeyWithHashAlg::new(
                    Arc::new(key_pair),
                    session
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|e| format!("获取 RSA hash 算法失败: {}", e))?
                        .flatten(),
                ),
            )
            .await
            .map_err(|e| format!("公钥认证失败: {}", e))?;
        if !auth_result.success() {
            return Err("公钥认证失败: 密钥不被服务器接受".to_string());
        }
    } else {
        return Err(format!("不支持的认证方式: {}", conn.auth_method));
    }
    Ok(())
}

/// SSH 持久连接会话管理器
pub struct SshSession {
    /// 当前使用的连接信息
    conn: Option<SSHConnection>,
    /// russh 持久连接句柄
    handle: Option<client::Handle<SshHandler>>,
    /// 当前会话状态
    status: SessionStatus,
    /// 是否正在进行同步操作（防止并发）
    syncing: AtomicBool,
}

/// 全局 SSH 会话状态，注册到 Tauri State
pub struct SshSessionState(pub Arc<Mutex<SshSession>>);

impl SshSession {
    /// 创建新会话（不连接）
    pub fn new() -> Self {
        Self {
            conn: None,
            handle: None,
            status: SessionStatus::Disconnected,
            syncing: AtomicBool::new(false),
        }
    }

    /// 获取当前状态
    pub fn status(&self) -> &SessionStatus {
        &self.status
    }

    /// 获取当前连接信息
    pub fn conn(&self) -> Option<&SSHConnection> {
        self.conn.as_ref()
    }

    /// 建立持久连接
    pub async fn connect(&mut self, conn: &SSHConnection) -> Result<(), String> {
        // 如果已连接同一个目标，先检查是否存活
        if self.conn.as_ref().map(|c| &c.id) == Some(&conn.id) && self.is_alive() {
            self.status = SessionStatus::Connected;
            return Ok(());
        }

        // 如果之前连接了不同目标，先断开
        self.disconnect().await;

        self.status = SessionStatus::Connecting;
        self.conn = Some(conn.clone());

        match self.do_connect(conn).await {
            Ok(handle) => {
                self.handle = Some(handle);
                self.status = SessionStatus::Connected;
                info!(
                    "SSH 连接已建立: {}@{}:{}",
                    conn.username, conn.host, conn.port
                );
                Ok(())
            }
            Err(e) => {
                let err = format!("SSH 连接失败: {}", e);
                self.status = SessionStatus::Failed(err.clone());
                Err(err)
            }
        }
    }

    /// 内部连接逻辑
    async fn do_connect(&self, conn: &SSHConnection) -> Result<client::Handle<SshHandler>, String> {
        let config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(90)),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            ..Default::default()
        };

        let handler = SshHandler;
        let mut session = tokio::time::timeout(
            Duration::from_secs(30),
            client::connect(Arc::new(config), (conn.host.as_str(), conn.port), handler),
        )
        .await
        .map_err(|_| format!("连接超时: {}:{}", conn.host, conn.port))?
        .map_err(|e| format!("连接到 {}:{} 失败: {}", conn.host, conn.port, e))?;

        authenticate(&mut session, conn).await?;

        Ok(session)
    }

    /// 检查连接是否存活
    pub fn is_alive(&self) -> bool {
        match &self.handle {
            Some(handle) => !handle.is_closed(),
            None => false,
        }
    }

    /// 确保连接可用（不可用时自动重连）
    pub async fn ensure_connected(&mut self) -> Result<(), String> {
        if self.is_alive() {
            self.status = SessionStatus::Connected;
            return Ok(());
        }
        let conn = self
            .conn
            .clone()
            .ok_or("没有可用的 SSH 连接配置".to_string())?;
        warn!("SSH 连接已断开，正在重连...");
        self.connect(&conn).await
    }

    /// 断开连接
    pub async fn disconnect(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.disconnect(Disconnect::ByApplication, "", "").await;
            if let Some(conn) = &self.conn {
                info!(
                    "SSH 连接已断开: {}@{}:{}",
                    conn.username, conn.host, conn.port
                );
            }
        }
        self.conn = None;
        self.status = SessionStatus::Disconnected;
    }

    /// 在远程执行命令并返回 stdout
    pub async fn exec_command(&self, cmd: &str) -> Result<String, String> {
        let handle = self.handle.as_ref().ok_or("SSH 会话未建立")?;

        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| format!("打开 SSH channel 失败: {}", e))?;

        channel
            .exec(true, cmd)
            .await
            .map_err(|e| format!("执行远程命令失败: {}", e))?;

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut exit_code: Option<u32> = None;

        loop {
            let Some(msg) = channel.wait().await else {
                break;
            };
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout_buf.extend_from_slice(data);
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        // SSH_EXTENDED_DATA_STDERR
                        stderr_buf.extend_from_slice(data);
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = Some(exit_status);
                }
                _ => {}
            }
        }

        let stdout = String::from_utf8_lossy(&stdout_buf).to_string();

        match exit_code {
            Some(0) | None => Ok(stdout),
            Some(code) => {
                let stderr = String::from_utf8_lossy(&stderr_buf).to_string();
                let detail = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else {
                    stdout.trim().to_string()
                };
                Err(format!("远程命令退出码 {}: {}", code, detail))
            }
        }
    }

    /// 在远程执行命令，带 stdin 输入
    pub async fn exec_command_with_stdin(
        &self,
        cmd: &str,
        stdin_data: &[u8],
    ) -> Result<(), String> {
        let handle = self.handle.as_ref().ok_or("SSH 会话未建立")?;

        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| format!("打开 SSH channel 失败: {}", e))?;

        channel
            .exec(true, cmd)
            .await
            .map_err(|e| format!("执行远程命令失败: {}", e))?;

        channel
            .data(stdin_data)
            .await
            .map_err(|e| format!("写入 stdin 失败: {}", e))?;

        channel
            .eof()
            .await
            .map_err(|e| format!("发送 EOF 失败: {}", e))?;

        let mut stderr_buf = Vec::new();
        let mut exit_code: Option<u32> = None;
        loop {
            let Some(msg) = channel.wait().await else {
                break;
            };
            match msg {
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = Some(exit_status);
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        stderr_buf.extend_from_slice(data);
                    }
                }
                _ => {}
            }
        }

        match exit_code {
            Some(0) | None => Ok(()),
            Some(code) => {
                let stderr = String::from_utf8_lossy(&stderr_buf).to_string();
                let detail = if !stderr.trim().is_empty() {
                    format!(": {}", stderr.trim())
                } else {
                    String::new()
                };
                Err(format!("远程命令退出码 {}{}", code, detail))
            }
        }
    }

    /// 创建 SFTP 会话（供需要批量文件操作的调用方复用）
    pub async fn create_sftp_session(&self) -> Result<russh_sftp::client::SftpSession, String> {
        let handle = self.handle.as_ref().ok_or("SSH 会话未建立")?;

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| format!("打开 SFTP channel 失败: {}", e))?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| format!("请求 SFTP 子系统失败: {}", e))?;

        russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| format!("初始化 SFTP 会话失败: {}", e))
    }

    /// 通过 SFTP 上传单个文件
    pub async fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<(), String> {
        let sftp = self.create_sftp_session().await?;
        upload_file_via_sftp(&sftp, local_path, remote_path).await
    }

    /// 通过 SFTP 递归上传目录
    pub async fn upload_dir(&self, local_path: &str, remote_path: &str) -> Result<(), String> {
        self.upload_dir_with_excludes(local_path, remote_path, &[])
            .await
    }

    /// 通过 SFTP 递归上传目录，并按目录名跳过 excluded folders
    pub async fn upload_dir_with_excludes(
        &self,
        local_path: &str,
        remote_path: &str,
        excluded_names: &[String],
    ) -> Result<(), String> {
        self.upload_dir_with_excludes_and_progress(local_path, remote_path, excluded_names, None)
            .await
    }

    /// 通过 SFTP 递归上传目录，并在每个实际上传文件前报告相对路径
    pub async fn upload_dir_with_excludes_and_progress(
        &self,
        local_path: &str,
        remote_path: &str,
        excluded_names: &[String],
        current_file_reporter: Option<&(dyn Fn(String) + Send + Sync)>,
    ) -> Result<(), String> {
        let sftp = self.create_sftp_session().await?;

        // 将 ~ 展开为绝对路径
        let abs_remote_path = resolve_remote_path(&sftp, remote_path).await?;
        let excluded_names: HashSet<String> = excluded_names.iter().cloned().collect();
        let mut stats = DirectoryUploadStats::default();
        let local_root = std::path::Path::new(local_path);

        // 递归上传
        upload_dir_recursive(
            &sftp,
            local_root,
            local_root,
            &abs_remote_path,
            &excluded_names,
            &mut stats,
            current_file_reporter,
        )
        .await?;

        if stats.skipped_excluded_dirs > 0 {
            info!(
                "SSH directory upload skipped excluded directories: local_path={}, remote_path={}, skipped_count={}, excludes={:?}",
                local_path, remote_path, stats.skipped_excluded_dirs, excluded_names
            );
        }

        Ok(())
    }

    /// 获取 user@host 字符串
    pub fn target_str(&self) -> Result<String, String> {
        let conn = self.conn.as_ref().ok_or("SSH 会话未建立")?;
        Ok(format!("{}@{}", conn.username, conn.host))
    }

    /// 尝试获取同步锁（防止并发同步）
    pub fn try_acquire_sync_lock(&self) -> bool {
        self.syncing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// 释放同步锁
    pub fn release_sync_lock(&self) {
        self.syncing.store(false, Ordering::SeqCst);
    }
}

/// 创建一个独立的临时 SSH 连接并执行命令（用于测试连接）
/// 返回命令输出结果
pub async fn test_connection_with_command(
    conn: &SSHConnection,
    cmd: &str,
) -> Result<String, String> {
    let config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(10)),
        ..Default::default()
    };

    let handler = SshHandler;
    let mut session = tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(Arc::new(config), (conn.host.as_str(), conn.port), handler),
    )
    .await
    .map_err(|_| format!("连接超时: {}:{}", conn.host, conn.port))?
    .map_err(|e| format!("连接到 {}:{} 失败: {}", conn.host, conn.port, e))?;

    authenticate(&mut session, conn).await?;

    // 执行命令
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| format!("打开 channel 失败: {}", e))?;

    channel
        .exec(true, cmd)
        .await
        .map_err(|e| format!("执行命令失败: {}", e))?;

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    loop {
        let Some(msg) = channel.wait().await else {
            break;
        };
        match msg {
            ChannelMsg::Data { ref data } => {
                stdout_buf.extend_from_slice(data);
            }
            ChannelMsg::ExtendedData { ref data, ext } => {
                if ext == 1 {
                    stderr_buf.extend_from_slice(data);
                }
            }
            _ => {}
        }
    }

    let _ = session.disconnect(Disconnect::ByApplication, "", "").await;

    Ok(String::from_utf8_lossy(&stdout_buf).to_string())
}

/// 通过已有 SFTP 会话上传单个文件
pub async fn upload_file_via_sftp(
    sftp: &russh_sftp::client::SftpSession,
    local_path: &str,
    remote_path: &str,
) -> Result<(), String> {
    // 读取本地文件
    let data = tokio::fs::read(local_path)
        .await
        .map_err(|e| format!("读取本地文件失败 {}: {}", local_path, e))?;

    // 将 ~ 展开为绝对路径（SFTP 不支持 ~ 语法）
    let abs_remote_path = resolve_remote_path(sftp, remote_path).await?;

    // 确保远程父目录存在
    if let Some(parent) = parent_path(&abs_remote_path) {
        sftp_mkdir_p(sftp, &parent).await;
    }

    // 打开远程文件写入
    let mut remote_file = sftp
        .open_with_flags(
            &abs_remote_path,
            russh_sftp::protocol::OpenFlags::CREATE
                | russh_sftp::protocol::OpenFlags::TRUNCATE
                | russh_sftp::protocol::OpenFlags::WRITE,
        )
        .await
        .map_err(|e| format!("打开远程文件失败 {}: {}", abs_remote_path, e))?;

    remote_file
        .write_all(&data)
        .await
        .map_err(|e| format!("写入远程文件失败: {}", e))?;

    remote_file
        .flush()
        .await
        .map_err(|e| format!("刷新远程文件失败: {}", e))?;

    remote_file
        .shutdown()
        .await
        .map_err(|e| format!("关闭远程文件失败: {}", e))?;

    Ok(())
}

#[derive(Default)]
struct DirectoryUploadStats {
    skipped_excluded_dirs: usize,
}

const MAX_EXCLUDED_DIR_TRACE_LOGS: usize = 20;

fn is_excluded_dir_name(file_name: &str, excluded_names: &HashSet<String>) -> bool {
    excluded_names.contains(file_name)
}

fn relative_upload_path(base_dir: &std::path::Path, file_path: &std::path::Path) -> String {
    file_path
        .strip_prefix(base_dir)
        .unwrap_or(file_path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 递归上传目录内容到远程
/// 使用 tokio::fs::metadata 跟随符号链接，等同于 cp -rL 行为
async fn upload_dir_recursive(
    sftp: &russh_sftp::client::SftpSession,
    base_dir: &std::path::Path,
    local_dir: &std::path::Path,
    remote_dir: &str,
    excluded_names: &HashSet<String>,
    stats: &mut DirectoryUploadStats,
    current_file_reporter: Option<&(dyn Fn(String) + Send + Sync)>,
) -> Result<(), String> {
    // 创建远程目录（忽略已存在的错误）
    let _ = sftp.create_dir(remote_dir).await;

    let mut entries = tokio::fs::read_dir(local_dir)
        .await
        .map_err(|e| format!("读取本地目录失败 {}: {}", local_dir.display(), e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("读取目录项失败: {}", e))?
    {
        let path = entry.path();
        // 使用 metadata（而非 symlink_metadata）跟随符号链接，获取最终目标的类型
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| format!("获取文件元数据失败 {}: {}", path.display(), e))?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        let remote_child = format!("{}/{}", remote_dir, file_name);

        if metadata.is_dir() {
            if is_excluded_dir_name(&file_name, excluded_names) {
                stats.skipped_excluded_dirs += 1;
                if stats.skipped_excluded_dirs <= MAX_EXCLUDED_DIR_TRACE_LOGS {
                    log::trace!(
                        "SSH directory upload skipped excluded directory: local_path={}, remote_path={}, exclude_name={}",
                        path.display(),
                        remote_child,
                        file_name
                    );
                }
                continue;
            }

            Box::pin(upload_dir_recursive(
                sftp,
                base_dir,
                &path,
                &remote_child,
                excluded_names,
                stats,
                current_file_reporter,
            ))
            .await?;
        } else if metadata.is_file() {
            if let Some(reporter) = current_file_reporter {
                reporter(relative_upload_path(base_dir, &path));
            }

            let data = tokio::fs::read(&path)
                .await
                .map_err(|e| format!("读取文件失败 {}: {}", path.display(), e))?;

            let mut remote_file = sftp
                .open_with_flags(
                    &remote_child,
                    russh_sftp::protocol::OpenFlags::CREATE
                        | russh_sftp::protocol::OpenFlags::TRUNCATE
                        | russh_sftp::protocol::OpenFlags::WRITE,
                )
                .await
                .map_err(|e| format!("打开远程文件失败 {}: {}", remote_child, e))?;

            remote_file
                .write_all(&data)
                .await
                .map_err(|e| format!("写入远程文件失败 {}: {}", remote_child, e))?;

            remote_file
                .flush()
                .await
                .map_err(|e| format!("刷新远程文件失败: {}", e))?;

            remote_file
                .shutdown()
                .await
                .map_err(|e| format!("关闭远程文件失败: {}", e))?;
        }
    }

    Ok(())
}

/// 将远程路径中的 ~ 和 $HOME 展开为绝对路径
/// SFTP 协议不支持 shell 变量或 ~ 语法，需要用 canonicalize 获取 home 路径
async fn resolve_remote_path(
    sftp: &russh_sftp::client::SftpSession,
    path: &str,
) -> Result<String, String> {
    if path.starts_with("~/") || path == "~" || path.contains("$HOME") {
        let home = sftp
            .canonicalize(".")
            .await
            .map_err(|e| format!("获取远程 home 路径失败: {}", e))?;
        let result = path.replacen("~", &home, 1).replace("$HOME", &home);
        Ok(result)
    } else {
        Ok(path.to_string())
    }
}

/// 获取远程路径的父目录
fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if let Some(pos) = trimmed.rfind('/') {
        if pos == 0 {
            Some("/".to_string())
        } else {
            Some(trimmed[..pos].to_string())
        }
    } else {
        None
    }
}

/// 递归创建远程目录（类似 mkdir -p）
async fn sftp_mkdir_p(sftp: &russh_sftp::client::SftpSession, path: &str) {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut current = String::new();
    for part in parts {
        current = format!("{}/{}", current, part);
        let _ = sftp.create_dir(&current).await;
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        // 在 Drop 中不能 async，直接丢弃 handle 让 russh 自行清理
        self.handle.take();
    }
}

#[cfg(test)]
mod tests {
    use super::{is_excluded_dir_name, relative_upload_path};
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn matches_excluded_directory_by_name_segment_only() {
        let excluded = HashSet::from(["cache".to_string(), ".venv".to_string()]);

        assert!(is_excluded_dir_name("cache", &excluded));
        assert!(is_excluded_dir_name(".venv", &excluded));
        assert!(!is_excluded_dir_name("plugin-cache", &excluded));
        assert!(!is_excluded_dir_name("cache/nested", &excluded));
    }

    #[test]
    fn reports_directory_upload_paths_relative_to_mapping_root() {
        let base = Path::new("/tmp/plugins");
        let file = Path::new("/tmp/plugins/marketplace/cache.json");

        assert_eq!(relative_upload_path(base, file), "marketplace/cache.json");
    }
}
