//! The `models.json` view both pages load.
//!
//! Kept here rather than in either module so the two pages cannot drift on the
//! `availableModels` semantics: an absent list and an empty list both mean
//! "show every model", and the page must render that as "no restriction", not
//! as "nothing is available".

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use super::channel::{build_channel_views, ChannelView};
use super::models_json::{self, ModelsDocument};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyRuntimeConfig {
    pub providers: Vec<ChannelView>,
    /// The file's own `availableModels` when it declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_models: Option<Vec<String>>,
    pub exists: bool,
    pub path: String,
}

/// Builds the page payload from an already-loaded document.
pub fn build_runtime_config(root: &Path, document: &ModelsDocument) -> FamilyRuntimeConfig {
    let value = document.to_value();
    let available_models = value
        .get("availableModels")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
    FamilyRuntimeConfig {
        providers: build_channel_views(&document.models),
        available_models,
        exists: document.existed,
        path: models_json::models_path(root).to_string_lossy().into_owned(),
    }
}

/// Reads `models.json` under `root` and returns the page payload.
pub fn read_runtime_config(root: &Path) -> Result<FamilyRuntimeConfig, String> {
    let document = ModelsDocument::load(root)?;
    Ok(build_runtime_config(root, &document))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS_FILE: &str = models_json::MODELS_FILE_NAME;

    #[test]
    fn an_empty_available_models_list_is_reported_as_empty_not_as_no_models() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(MODELS_FILE),
            r#"{"models":[{"id":"a","url":"https://x/v1","apiKey":"k"}],"availableModels":[]}"#,
        )
        .expect("seed");

        let runtime = read_runtime_config(dir.path()).expect("read");
        assert_eq!(runtime.available_models, Some(Vec::new()));
        assert_eq!(runtime.providers.len(), 1);
    }

    #[test]
    fn a_file_without_available_models_reports_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(MODELS_FILE),
            r#"{"models":[{"id":"a","url":"https://x/v1","apiKey":"k"}]}"#,
        )
        .expect("seed");

        let runtime = read_runtime_config(dir.path()).expect("read");
        assert_eq!(runtime.available_models, None);
    }

    #[test]
    fn a_missing_file_reports_not_existing_and_an_empty_channel_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = read_runtime_config(dir.path()).expect("read");
        assert!(!runtime.exists);
        assert!(runtime.providers.is_empty());
        assert!(runtime.path.ends_with(MODELS_FILE));
    }
}
