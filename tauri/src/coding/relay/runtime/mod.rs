//! 插件宿主运行时：Bun 运行时下载、子进程监管、RPC 帧协议。
//!
//! 这三块放在一起是因为它们**只能一起用**：没有 Bun 起不了宿主进程，没有帧
//! 协议进程起来了也没法对话。拆成三个 crate 或三处目录只会让「插件不可用」的
//! 降级判断分散在三个地方。

pub mod bun;
pub mod host;
pub mod protocol;

pub use bun::{BunDownloadProgress, BunRuntimeStatus};
pub use host::{
    HostExport, HostInitResult, HostLoadedPlugin, HostPluginSpec, HostProvider, HostStatus,
};
pub use protocol::{HostLink, Reply};
