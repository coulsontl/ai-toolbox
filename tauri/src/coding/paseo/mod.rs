//! Paseo model-list synchronization.
//!
//! Paseo stores provider overrides in `<paseo home>/config.json`. The field
//! managed here is `agents.providers.<provider>.models`: Paseo treats that as a
//! replacement list (`profileModelsAreAdditive = false`), while
//! `additionalModels` is append-only. AI Toolbox only ever overwrites `models`
//! and leaves every other field (including `additionalModels`) untouched.

pub mod config;
pub mod local;
pub mod models;

pub use config::patch_paseo_config_text;
pub use local::{
    local_paseo_config_path, local_paseo_home_path, sync_local_models, PaseoLocalSyncOutcome,
};
pub use models::{
    build_codex_models, build_claude_models, build_models_for_provider, PaseoModelEntry,
    PaseoThinkingOption,
};

/// The built-in Paseo provider ids whose model lists AI Toolbox can mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaseoProvider {
    Claude,
    Codex,
}

impl PaseoProvider {
    pub fn key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}
