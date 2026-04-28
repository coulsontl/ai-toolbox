use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageChannelModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub supports_text_to_image: bool,
    pub supports_image_to_image: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageChannelRecord {
    pub id: String,
    pub name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    pub enabled: bool,
    pub sort_order: i64,
    pub models_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageChannelDto {
    pub id: String,
    pub name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    pub enabled: bool,
    pub sort_order: i64,
    pub models: Vec<ImageChannelModel>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageJobMode {
    TextToImage,
    ImageToImage,
}

impl ImageJobMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TextToImage => "text_to_image",
            Self::ImageToImage => "image_to_image",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageJobStatus {
    Running,
    Done,
    Error,
}

impl ImageJobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageTaskParams {
    pub size: String,
    pub quality: String,
    pub output_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_compression: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderation: Option<String>,
}

impl Default for ImageTaskParams {
    fn default() -> Self {
        Self {
            size: "auto".to_string(),
            quality: "auto".to_string(),
            output_format: "png".to_string(),
            output_compression: None,
            moderation: Some("low".to_string()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageJobRecord {
    pub id: String,
    pub mode: String,
    pub prompt: String,
    pub channel_id: String,
    pub channel_name_snapshot: String,
    pub model_id: String,
    pub model_name_snapshot: String,
    pub params_json: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_headers_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_json: Option<String>,
    pub input_asset_ids: Vec<String>,
    pub output_asset_ids: Vec<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageAssetRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub role: String,
    pub mime_type: String,
    pub file_name: String,
    pub relative_path: String,
    pub bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageReferenceInput {
    pub file_name: String,
    pub mime_type: String,
    pub base64_data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateImageJobInput {
    pub mode: String,
    pub prompt: String,
    pub channel_id: String,
    pub model_id: String,
    pub params: ImageTaskParams,
    #[serde(default)]
    pub references: Vec<ImageReferenceInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpsertImageChannelInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<ImageChannelModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteImageChannelInput {
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteImageJobInput {
    pub id: String,
    pub delete_local_assets: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReorderImageChannelsInput {
    pub ordered_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageAssetDto {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub role: String,
    pub mime_type: String,
    pub file_name: String,
    pub relative_path: String,
    pub bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    pub created_at: i64,
    pub file_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageJobDto {
    pub id: String,
    pub mode: String,
    pub prompt: String,
    pub channel_id: String,
    pub channel_name_snapshot: String,
    pub model_id: String,
    pub model_name_snapshot: String,
    pub params_json: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_headers_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_json: Option<String>,
    pub input_assets: Vec<ImageAssetDto>,
    pub output_assets: Vec<ImageAssetDto>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageWorkspaceDto {
    pub channels: Vec<ImageChannelDto>,
    pub jobs: Vec<ImageJobDto>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListImageJobsInput {
    #[serde(default)]
    pub limit: Option<usize>,
}

fn default_list_limit() -> usize {
    50
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListImageChannelsInput {
    #[serde(default = "default_list_limit")]
    pub limit: usize,
}

pub fn now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
