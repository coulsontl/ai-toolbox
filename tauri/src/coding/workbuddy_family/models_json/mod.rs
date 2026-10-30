//! The `models.json` contract shared by WorkBuddy desktop and CodeBuddy Code.
//!
//! Both products read `<config root>/models.json`, and the desktop app's own
//! loader (`CustomModelsProductProvider.extractConfig`) accepts two shapes: a
//! bare array of model entries, or an object with `models` + `availableModels`.
//! Everything in this module is written against that observed behaviour:
//!
//! - **Never narrow `availableModels` on an unmanaged file.** An absent or empty
//!   list means "show every model, built-ins included". Writing a list that only
//!   names the user's custom models hides the product's own catalog.
//! - **Preserve unknown fields.** The desktop UI writes extra keys onto entries
//!   (`credits`, `tags`, `reasoning`, `iconUrl`, …) and rewrites the file itself;
//!   a full rebuild from our own struct would drop them.
//! - **Keep the on-disk shape.** A bare-array file stays a bare array.
//!
//! The file is also watched by the app with a 1s debounce, so writes are atomic
//! (temp file + rename) — a half-written document would be read as a parse error.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Filename of the custom-model config inside a product's config root.
pub const MODELS_FILE_NAME: &str = "models.json";

/// Suffix the products normalize every custom model URL to.
pub const CHAT_COMPLETIONS_SUFFIX: &str = "/chat/completions";

/// The top-level document shape found on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsDocumentShape {
    /// `{"models": [...], "availableModels": [...]}` (or any object form).
    Object,
    /// A bare `[...]` array of model entries.
    Array,
}

/// A parsed `models.json`, keeping the parts we do not own.
#[derive(Debug, Clone)]
pub struct ModelsDocument {
    pub shape: ModelsDocumentShape,
    /// Model entries, in file order.
    pub models: Vec<Value>,
    /// The original object, so non-`models` keys survive a write.
    object: Option<Map<String, Value>>,
    /// Whether the file existed when it was read.
    pub existed: bool,
}

impl Default for ModelsDocument {
    fn default() -> Self {
        Self {
            shape: ModelsDocumentShape::Object,
            models: Vec::new(),
            object: None,
            existed: false,
        }
    }
}

impl ModelsDocument {
    /// Reads `models.json` from `config_root`. A missing file is an empty
    /// document, not an error: the product creates it on first use.
    pub fn load(config_root: &Path) -> Result<Self, String> {
        let path = config_root.join(MODELS_FILE_NAME);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        Self::parse(&raw, true)
    }

    /// Parses raw `models.json` text.
    ///
    /// The desktop loader retries with comment stripping (`parseJsonWithRepair`),
    /// so a commented file does reach the models list; mirror that here instead
    /// of failing the whole page on a `//` the app itself tolerates.
    pub fn parse(raw: &str, existed: bool) -> Result<Self, String> {
        let value = parse_json_tolerant(raw)
            .ok_or_else(|| "models.json is not valid JSON".to_string())?;
        Ok(Self::from_value(value, existed))
    }

    fn from_value(value: Value, existed: bool) -> Self {
        match value {
            Value::Array(items) => Self {
                shape: ModelsDocumentShape::Array,
                models: items,
                object: None,
                existed,
            },
            Value::Object(mut map) => {
                let models = map
                    .remove("models")
                    .and_then(|models| models.as_array().cloned())
                    .unwrap_or_default();
                Self {
                    shape: ModelsDocumentShape::Object,
                    models,
                    object: Some(map),
                    existed,
                }
            }
            _ => Self::default(),
        }
    }

    /// Serializes back to the on-disk shape, preserving untouched object keys.
    pub fn to_value(&self) -> Value {
        match self.shape {
            ModelsDocumentShape::Array => Value::Array(self.models.clone()),
            ModelsDocumentShape::Object => {
                let mut map = self.object.clone().unwrap_or_default();
                map.insert("models".to_string(), Value::Array(self.models.clone()));
                Value::Object(map)
            }
        }
    }

    /// Writes the document to `config_root/models.json` atomically.
    pub fn save(&self, config_root: &Path) -> Result<(), String> {
        let path = config_root.join(MODELS_FILE_NAME);
        let mut body = serde_json::to_string_pretty(&self.to_value())
            .map_err(|error| format!("Failed to serialize models.json: {error}"))?;
        body.push('\n');
        atomic_write(&path, body.as_bytes())
    }

    /// Index of the entry whose `id` matches, if any.
    fn index_of(&self, model_id: &str) -> Option<usize> {
        self.models
            .iter()
            .position(|entry| entry_id(entry).as_deref() == Some(model_id))
    }

    /// Replaces the entry with the same `id`, or appends when it is new.
    ///
    /// The incoming entry is merged onto the existing one rather than replacing
    /// it, so product-owned keys (`credits`, `tags`, `reasoning`, …) survive an
    /// edit made in AI Toolbox.
    pub fn upsert_model(&mut self, entry: Value) {
        self.upsert_model_renaming(entry, None);
    }

    /// Same as [`Self::upsert_model`], but with an explicit "this entry used to
    /// have `previous_id`" rename.
    ///
    /// A rename must merge onto the entry the user actually edited. Removing the
    /// old entry and appending the new one would (a) drop every product-owned key
    /// on it and (b) move it to the end of the list. When `previous_id` names a
    /// different entry, that entry keeps its position, its extras, and only its
    /// `id` changes.
    pub fn upsert_model_renaming(&mut self, entry: Value, previous_id: Option<&str>) {
        let Some(id) = entry_id(&entry) else {
            return;
        };
        let rename_from = previous_id
            .map(str::trim)
            .filter(|previous| !previous.is_empty() && *previous != id);

        let target_index = rename_from
            .and_then(|previous| self.index_of(previous))
            .or_else(|| self.index_of(&id));

        match target_index {
            Some(index) => {
                let mut merged = self.models[index].clone();
                merge_object(&mut merged, &entry);
                self.models[index] = merged;
                // An entry already carrying the new id elsewhere would otherwise
                // appear twice; the edited one wins and the stray is dropped.
                self.drop_duplicate_ids(&id, index);
            }
            None => self.models.push(entry),
        }
    }

    /// Removes other entries that now share `id`, keeping `keep_index`.
    ///
    /// Walks from the end so no removal ever shifts an index that is still to be
    /// visited; `keep_index` therefore stays valid for the whole pass.
    fn drop_duplicate_ids(&mut self, id: &str, keep_index: usize) {
        let mut index = self.models.len();
        while index > 0 {
            index -= 1;
            if index != keep_index && entry_id(&self.models[index]).as_deref() == Some(id) {
                self.models.remove(index);
            }
        }
    }

    /// Removes every entry with this `id`. Returns whether anything changed.
    pub fn remove_model(&mut self, model_id: &str) -> bool {
        let before = self.models.len();
        self.models
            .retain(|entry| entry_id(entry).as_deref() != Some(model_id));
        before != self.models.len()
    }

    /// Entries in this document that belong to one channel.
    pub fn models_for_channel(&self, base_url: &str, api_key: &str) -> Vec<Value> {
        self.models
            .iter()
            .filter(|entry| {
                channel_key_of(entry).map(|(url, key)| url == base_url && key == api_key)
                    == Some(true)
            })
            .cloned()
            .collect()
    }
}

/// A model entry's `id`, trimmed and non-empty.
pub fn entry_id(entry: &Value) -> Option<String> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Strips a trailing `/chat/completions` so two entries pointing at the same
/// endpoint group together regardless of how the user typed the URL.
pub fn normalize_channel_base_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_suffix = trimmed
        .strip_suffix(CHAT_COMPLETIONS_SUFFIX)
        .unwrap_or(trimmed);
    without_suffix.trim_end_matches('/').to_string()
}

/// Appends `/chat/completions` unless the URL already names an endpoint.
///
/// The products run the same normalization (`normalizeChatCompletionsUrl`
/// strips repeated suffixes before appending), so a base URL written here lands
/// on disk exactly as the UI would have written it.
pub fn to_chat_completions_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let (head, tail) = split_query_or_fragment(trimmed);
    let mut base = head.trim_end_matches('/');
    while let Some(stripped) = base.strip_suffix(CHAT_COMPLETIONS_SUFFIX) {
        base = stripped.trim_end_matches('/');
    }
    format!("{base}{CHAT_COMPLETIONS_SUFFIX}{tail}")
}

fn split_query_or_fragment(url: &str) -> (&str, &str) {
    match url.find(['?', '#']) {
        Some(index) => (&url[..index], &url[index..]),
        None => (url, ""),
    }
}

/// The `(base_url, api_key)` pair that identifies an entry's channel.
///
/// `None` when the entry has no URL — an entry with only an id cannot be
/// grouped, and `models.json` allows `url` to be absent.
pub fn channel_key_of(entry: &Value) -> Option<(String, String)> {
    let url = entry
        .get("url")
        .and_then(Value::as_str)
        .map(normalize_channel_base_url)
        .filter(|url| !url.is_empty())?;
    let api_key = entry
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Some((url, api_key))
}

/// Builds a model entry in the exact field set the products accept.
///
/// `useCustomProtocol` is left alone on merge; only the fields AI Toolbox owns
/// are written so product-side extras survive.
pub fn build_model_entry(input: &ModelEntryInput<'_>) -> Value {
    let mut map = Map::new();
    map.insert("id".to_string(), json!(input.id));
    if let Some(name) = non_empty(input.name) {
        map.insert("name".to_string(), json!(name));
    }
    if let Some(vendor) = non_empty(input.vendor) {
        map.insert("vendor".to_string(), json!(vendor));
    }
    let url = to_chat_completions_url(input.base_url);
    if !url.is_empty() {
        map.insert("url".to_string(), json!(url));
    }
    if let Some(api_key) = non_empty(input.api_key) {
        map.insert("apiKey".to_string(), json!(api_key));
    }
    if let Some(max_input) = input.max_input_tokens {
        map.insert("maxInputTokens".to_string(), json!(max_input));
    }
    if let Some(max_output) = input.max_output_tokens {
        map.insert("maxOutputTokens".to_string(), json!(max_output));
    }
    if let Some(temperature) = input.temperature {
        map.insert("temperature".to_string(), json!(temperature));
    }
    map.insert(
        "supportsToolCall".to_string(),
        Value::Bool(input.supports_tool_call),
    );
    map.insert("supportsImages".to_string(), Value::Bool(input.supports_images));
    map.insert(
        "supportsReasoning".to_string(),
        Value::Bool(input.supports_reasoning),
    );
    // Written unconditionally, like the three flags above. Inserting only when
    // `true` (the first version) made turning either switch **off** a no-op:
    // `upsert_model_renaming` deep-merges onto the existing entry, and a key
    // absent from the patch keeps its old value — so the UI showed the switch
    // off, the save reported success, and the file still said `true`.
    map.insert(
        "onlyReasoning".to_string(),
        Value::Bool(input.only_reasoning),
    );
    map.insert(
        "useCustomProtocol".to_string(),
        Value::Bool(input.use_custom_protocol),
    );
    Value::Object(map)
}

/// Everything AI Toolbox writes onto one model entry.
#[derive(Debug, Clone, Copy)]
pub struct ModelEntryInput<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub vendor: Option<&'a str>,
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub supports_tool_call: bool,
    pub supports_images: bool,
    pub supports_reasoning: bool,
    pub only_reasoning: bool,
    pub use_custom_protocol: bool,
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Deep-merges `patch` into `target` for objects; other kinds replace outright.
fn merge_object(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                match target_map.get_mut(key) {
                    Some(existing) if existing.is_object() && value.is_object() => {
                        merge_object(existing, value);
                    }
                    _ => {
                        target_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, patch) => {
            *target = patch.clone();
        }
    }
}

/// JSON parse with the same leniency the desktop loader applies before giving up:
/// strip a UTF-8 BOM, drop `//` and `/* */` comments, then trailing commas.
pub fn parse_json_tolerant(raw: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str(raw) {
        return Some(value);
    }
    let repaired = repair_json(raw);
    serde_json::from_str(&repaired).ok()
}

fn repair_json(raw: &str) -> String {
    let mut without_comments = String::with_capacity(raw.len());
    let bytes: Vec<char> = raw.chars().collect();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let current = bytes[index];
        if in_string {
            without_comments.push(current);
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if current == '"' {
            in_string = true;
            without_comments.push(current);
            index += 1;
            continue;
        }
        if current == '/' && bytes.get(index + 1) == Some(&'/') {
            while index < bytes.len() && bytes[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if current == '/' && bytes.get(index + 1) == Some(&'*') {
            index += 2;
            while index + 1 < bytes.len()
                && !(bytes[index] == '*' && bytes[index + 1] == '/')
            {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        without_comments.push(current);
        index += 1;
    }
    let trimmed = without_comments.trim_start_matches('\u{feff}');
    remove_trailing_commas(trimmed)
}

fn remove_trailing_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut in_string = false;
    let mut escaped = false;
    for (index, current) in chars.iter().enumerate() {
        if in_string {
            output.push(*current);
            if escaped {
                escaped = false;
            } else if *current == '\\' {
                escaped = true;
            } else if *current == '"' {
                in_string = false;
            }
            continue;
        }
        if *current == '"' {
            in_string = true;
            output.push(*current);
            continue;
        }
        if *current == ',' {
            let next = chars[index + 1..]
                .iter()
                .find(|candidate| !candidate.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        output.push(*current);
    }
    output
}

/// Writes `content` to `path` through a sibling temp file and a rename.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Failed to resolve parent of {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;
    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "models.json".to_string()),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&temp_path, content)
        .map_err(|error| format!("Failed to write temp file {}: {error}", temp_path.display()))?;
    std::fs::rename(&temp_path, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        format!("Failed to rename temp file to {}: {error}", path.display())
    })
}

/// Path of the models file inside a config root.
pub fn models_path(config_root: &Path) -> PathBuf {
    config_root.join(MODELS_FILE_NAME)
}

#[cfg(test)]
mod tests;
