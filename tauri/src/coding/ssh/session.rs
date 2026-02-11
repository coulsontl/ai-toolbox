//! SSH ControlMaster 长连接会话管理
//!
//! 维护一个持久的 SSH 主连接，所有后续 ssh/scp 命令复用该连接。
//! 网络断开后自动重连。

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use log::{info, warn};

use super::key_file;
use super::types::SSHConnection;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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

/// SSH 长连接会话管理器
pub struct SshSession {
    /// 当前使用的连接信息
    conn: Option<SSHConnection>,
    /// ControlMaster socket 路径（String 而非 PathBuf，因为 Windows Named Pipe 路径会被 PathBuf 破坏）
    control_path: String,
    /// 应用数据目录（用于存储私钥文件）
    app_data_dir: PathBuf,
    /// 当前会话状态
    status: SessionStatus,
    /// 是否正在进行同步操作（防止并发）
    syncing: AtomicBool,
}

/// 全局 SSH 会话状态，注册到 Tauri State
pub struct SshSessionState(pub Arc<Mutex<SshSession>>);

impl SshSession {
    /// 创建新会话（不连接）
    pub fn new(app_data_dir: PathBuf) -> Self {
        // %C 是 OpenSSH 内置的 hash，自动根据 host:port:user 生成唯一值
        // Windows: 使用 Named Pipe（\\.\pipe\...），因为 Windows 不支持 Unix domain socket
        // Unix: 使用临时目录下的 socket 文件
        let control_path = if cfg!(target_os = "windows") {
            r"\\.\pipe\ai-toolbox-ssh-ctrl-%C".to_string()
        } else {
            std::env::temp_dir()
                .join("ai-toolbox-ssh-ctrl-%C")
                .to_string_lossy()
                .to_string()
        };
        Self {
            conn: None,
            control_path,
            app_data_dir,
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

    /// 建立主连接
    /// 启动一个 ssh -M（ControlMaster=yes）后台进程，保持长连接
    pub fn connect(&mut self, conn: &SSHConnection) -> Result<(), String> {
        // 如果已连接同一个目标，先检查是否存活
        if self.conn.as_ref().map(|c| &c.id) == Some(&conn.id) && self.is_alive() {
            self.status = SessionStatus::Connected;
            return Ok(());
        }

        // 如果之前连接了不同目标，先断开
        self.disconnect();

        self.status = SessionStatus::Connecting;
        self.conn = Some(conn.clone());

        let target = format!("{}@{}", conn.username, conn.host);

        // 构建主连接命令
        let mut cmd = self.build_base_ssh_command(conn);
        cmd.args([
            "-M",                              // 启动 ControlMaster
            "-S", &self.control_path,          // socket 路径
            "-o", "ControlPersist=yes",        // 前台 ssh 退出后主连接仍保持
            "-o", "ServerAliveInterval=30",    // 每30秒发心跳
            "-o", "ServerAliveCountMax=3",     // 3次无响应断开
            "-N",                              // 不执行远程命令
            "-f",                              // 后台运行
            &target,
        ]);

        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let output = cmd.output()
            .map_err(|e| format!("启动 SSH 主连接失败: {}", e))?;

        if output.status.success() {
            self.status = SessionStatus::Connected;
            info!("SSH 主连接已建立: {}@{}:{}", conn.username, conn.host, conn.port);
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let err = format!("SSH 主连接失败: {}", stderr);
            self.status = SessionStatus::Failed(err.clone());
            Err(err)
        }
    }

    /// 检查主连接是否存活
    pub fn is_alive(&self) -> bool {
        let conn = match &self.conn {
            Some(c) => c,
            None => return false,
        };
        let target = format!("{}@{}", conn.username, conn.host);

        let mut cmd = Command::new("ssh");
        cmd.args([
            "-S", &self.control_path,
            "-O", "check",            // 检查主连接状态
            "-p", &conn.port.to_string(),
            &target,
        ]);

        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }

    /// 确保连接可用（不可用时自动重连）
    /// 所有同步操作前应先调用此方法
    pub fn ensure_connected(&mut self) -> Result<(), String> {
        if self.is_alive() {
            self.status = SessionStatus::Connected;
            return Ok(());
        }
        // 不存活则重连
        let conn = self.conn.clone()
            .ok_or("没有可用的 SSH 连接配置".to_string())?;
        warn!("SSH 主连接已断开，正在重连...");
        self.connect(&conn)
    }

    /// 断开主连接
    pub fn disconnect(&mut self) {
        if let Some(conn) = &self.conn {
            let target = format!("{}@{}", conn.username, conn.host);

            let mut cmd = Command::new("ssh");
            cmd.args([
                "-S", &self.control_path,
                "-O", "exit",
                "-p", &conn.port.to_string(),
                &target,
            ]);

            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);

            let _ = cmd.output(); // 忽略结果，可能本来就没连接
            info!("SSH 主连接已断开: {}@{}:{}", conn.username, conn.host, conn.port);
        }
        self.conn = None;
        self.status = SessionStatus::Disconnected;
    }

    /// 创建复用主连接的 SSH 命令（供 sync.rs 使用）
    pub fn create_ssh_command(&self) -> Result<Command, String> {
        let conn = self.conn.as_ref()
            .ok_or("SSH 会话未建立")?;
        let target = format!("{}@{}", conn.username, conn.host);

        let mut cmd = Command::new("ssh");
        cmd.args([
            "-S", &self.control_path,          // 复用主连接
            "-o", "ControlMaster=no",          // 不尝试成为 master
            "-p", &conn.port.to_string(),
            "-o", "ConnectTimeout=10",
            "-o", "StrictHostKeyChecking=accept-new",
            &target,
        ]);

        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        Ok(cmd)
    }

    /// 创建复用主连接的 SCP 命令（供 sync.rs 使用）
    pub fn create_scp_command(&self) -> Result<Command, String> {
        let conn = self.conn.as_ref()
            .ok_or("SSH 会话未建立")?;

        let mut cmd = Command::new("scp");
        cmd.args([
            "-o", &format!("ControlPath={}", self.control_path),  // 复用主连接
            "-o", "ControlMaster=no",
            "-P", &conn.port.to_string(),
            "-o", "ConnectTimeout=10",
            "-o", "StrictHostKeyChecking=accept-new",
        ]);

        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        Ok(cmd)
    }

    /// 获取 user@host 字符串
    pub fn target_str(&self) -> Result<String, String> {
        let conn = self.conn.as_ref().ok_or("SSH 会话未建立")?;
        Ok(format!("{}@{}", conn.username, conn.host))
    }

    /// 尝试获取同步锁（防止并发同步）
    pub fn try_acquire_sync_lock(&self) -> bool {
        self.syncing.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    /// 释放同步锁
    pub fn release_sync_lock(&self) {
        self.syncing.store(false, Ordering::SeqCst);
    }

    // === 私有辅助方法 ===

    /// 构建基础 SSH 命令（仅用于建立主连接时，处理密码认证）
    fn build_base_ssh_command(&self, conn: &SSHConnection) -> Command {
        if conn.auth_method == "password" && !conn.password.is_empty() {
            // 关键修复：使用 sshpass -e（环境变量）而非 -p（命令行参数）
            let mut cmd = Command::new("sshpass");
            cmd.args(["-e", "ssh"]);  // -e 从 SSHPASS 环境变量读取密码
            cmd.env("SSHPASS", &conn.password);  // 设置环境变量
            self.add_ssh_base_args(&mut cmd, conn);
            cmd
        } else {
            let mut cmd = Command::new("ssh");
            self.add_ssh_base_args(&mut cmd, conn);
            cmd
        }
    }

    /// 添加基础 SSH 参数
    fn add_ssh_base_args(&self, cmd: &mut Command, conn: &SSHConnection) {
        cmd.args(["-p", &conn.port.to_string()]);
        cmd.args(["-o", "StrictHostKeyChecking=accept-new"]);
        cmd.args(["-o", "ConnectTimeout=10"]);
        if conn.auth_method == "key" {
            let key_path = key_file::resolve_key_path(
                &self.app_data_dir,
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
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        self.disconnect();
    }
}
