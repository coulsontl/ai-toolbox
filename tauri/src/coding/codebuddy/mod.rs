//! CodeBuddy Code CLI backend.
//!
//! Same engine as WorkBuddy desktop, so the `models.json` contract and the
//! channel view come from [`crate::coding::workbuddy_family`]. What is specific
//! to the CLI lives here: its own config root, its `settings.json`, its
//! `CODEBUDDY.md` presets and its MCP file precedence.

pub mod adapter;
pub mod commands;
pub mod constants;
pub mod paths;
pub mod types;

pub use commands::*;
