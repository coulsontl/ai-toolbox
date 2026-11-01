//! 2API：一个独立的 API 网关入口，渠道由用户自建，能力由 OpenCode 插件扩展。
//!
//! ── 与现有 Gateway 的关系 ──────────────────────────────────────
//! 复用同一个网关进程、同一套 `transformer` 协议转换和上游发送链路，但**不复用**
//! CLI 渠道模型：现有渠道按 CLI 分物理表（`ClaudeProvider`、`CodexProvider`…），
//! 每条记录都绑死某个 CLI 的 runtime 配置。2API 面对的是任意 API 客户端，
//! 没有 CLI 可言，所以渠道自成一表（`relay_entity`）。
//!
//! ── 插件为什么是核心 ────────────────────────────────────────────
//! 「支持某某订阅」这类能力不该由本仓库内置——每加一家就要改一次网关。插件把
//! 登录、模型目录和请求出口都搬到用户自己安装的包里，本仓库只提供宿主：
//! Bun 运行时、子进程监管、RPC 帧协议，以及 `config` / `provider.models` /
//! `auth.loader` / `auth.checkin` 四类钩子的调用点。
//!
//! 插件协议直接对齐 OpenCode 的插件 API（magpie 跑的就是同一套），因此 npm 上
//! 现成的插件可以直接装。
//!
//! ── 模块划分 ────────────────────────────────────────────────────
//! - `types`：落库结构与 DTO；
//! - `store`：SQLite JSONB 读写；
//! - `commands`：Tauri 命令；
//! - `runtime`：Bun 运行时下载、插件宿主子进程监管、RPC 帧协议。

pub mod commands;
pub mod commands_runtime;
pub mod runtime;
pub mod store;
pub mod types;

pub use commands::*;
pub use commands_runtime::*;
