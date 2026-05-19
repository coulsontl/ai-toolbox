use std::collections::HashSet;
use std::error::Error as _;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Instant;

use base64::Engine;
use image::ImageReader;
use log::{debug, error, warn};
use reqwest::multipart::{Form, Part};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

use super::store;
use super::types::{
    now_ms, CreateImageJobInput, DeleteImageChannelInput, DeleteImageJobInput, ImageAssetDto,
    ImageAssetRecord, ImageChannelDto, ImageChannelModel, ImageChannelRecord, ImageJobDto,
    ImageJobMode, ImageJobRecord, ImageJobStatus, ImageReferenceInput, ImageWorkspaceDto,
    ListImageChannelsInput, ListImageJobsInput, ReorderImageChannelsInput, UpsertImageChannelInput,
};
use crate::coding::db_id::db_clean_id;
use crate::http_client;
use crate::SqliteDbState;

const DEFAULT_CHANNEL_LIST_LIMIT: usize = 200;
const PROVIDER_KIND_OPENAI_COMPATIBLE: &str = "openai_compatible";
const PROVIDER_KIND_GEMINI: &str = "gemini";
const PROVIDER_KIND_OPENAI_RESPONSES: &str = "openai_responses";
const IMAGE_REQUEST_ACCEPT_ENCODING: &str = "identity";
const IMAGE_JOB_PROGRESS_EVENT: &str = "image-job-progress";
const IMAGE_REQUEST_MAX_RETRIES: usize = 3;
const IMAGE_REQUEST_MAX_ATTEMPTS: usize = IMAGE_REQUEST_MAX_RETRIES + 1;
const IMAGE_REQUEST_RETRY_DELAYS_MS: [u64; 3] = [1500, 3000, 5000];
const RESPONSES_PROMPT_REWRITE_GUARD_PREFIX: &str =
    "Use the following text as the complete prompt. Do not rewrite it:";

#[derive(Clone, Serialize)]
struct ImageJobProgressPayload {
    job_id: String,
    stage: &'static str,
    attempt: usize,
    max_attempts: usize,
    retry_count: usize,
    max_retries: usize,
    delay_ms: Option<u64>,
    timeout_seconds: u64,
    provider_kind: String,
    mode: String,
    channel_name: String,
    model_id: String,
    plan: Option<String>,
    reference_input_mode: Option<String>,
    message: Option<String>,
}

struct ImageJobProgressContext<'a> {
    app: Option<&'a AppHandle>,
    job_id: &'a str,
    provider_kind: &'a str,
    mode: &'a str,
    channel_name: &'a str,
    model_id: &'a str,
    timeout_seconds: u64,
}

struct ImageJobRequestSnapshot {
    request_url: String,
    request_headers_json: String,
    request_body_json: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageProviderAdapter {
    OpenAiCompatible,
    Gemini,
    OpenAiResponses,
}

impl ImageProviderAdapter {
    fn from_kind(provider_kind: &str) -> Result<Self, String> {
        match provider_kind.trim() {
            PROVIDER_KIND_OPENAI_COMPATIBLE => Ok(Self::OpenAiCompatible),
            PROVIDER_KIND_GEMINI => Ok(Self::Gemini),
            PROVIDER_KIND_OPENAI_RESPONSES => Ok(Self::OpenAiResponses),
            value => Err(format!("Unsupported image provider kind: {}", value)),
        }
    }

    fn supports_custom_paths(self) -> bool {
        matches!(self, Self::OpenAiCompatible)
    }

    fn default_request_path(self, mode: &str, model_id: &str) -> Result<String, String> {
        match self {
            Self::OpenAiCompatible if mode == ImageJobMode::TextToImage.as_str() => {
                Ok("images/generations".to_string())
            }
            Self::OpenAiCompatible if mode == ImageJobMode::ImageToImage.as_str() => {
                Ok("images/edits".to_string())
            }
            Self::OpenAiCompatible => Err(format!("Unsupported image job mode: {}", mode)),
            Self::Gemini => Ok(format!("models/{}:generateContent", model_id.trim())),
            Self::OpenAiResponses => Ok("responses".to_string()),
        }
    }

    fn resolve_request_path(
        self,
        channel: &ImageChannelDto,
        mode: &str,
        model_id: &str,
    ) -> Result<String, String> {
        if !self.supports_custom_paths() {
            return self.default_request_path(mode, model_id);
        }

        let custom_path = if mode == ImageJobMode::TextToImage.as_str() {
            channel.generation_path.clone()
        } else {
            channel.edit_path.clone()
        };

        if let Some(path) = sanitize_channel_path(custom_path) {
            return Ok(path);
        }

        self.default_request_path(mode, model_id)
    }

    fn build_request_url(
        self,
        channel: &ImageChannelDto,
        mode: &str,
        model_id: &str,
    ) -> Result<String, String> {
        let request_path = self.resolve_request_path(channel, mode, model_id)?;
        match self {
            Self::OpenAiCompatible | Self::OpenAiResponses => {
                Ok(build_image_api_url(&channel.base_url, &request_path))
            }
            Self::Gemini => Ok(build_gemini_api_url(&channel.base_url, &request_path)),
        }
    }

    fn request_headers_snapshot(self, input: &CreateImageJobInput) -> serde_json::Value {
        match self {
            Self::OpenAiCompatible => {
                let content_type = if input.mode == ImageJobMode::ImageToImage.as_str() {
                    "multipart/form-data"
                } else {
                    "application/json"
                };
                json!({
                    "Authorization": "Bearer ***",
                    "Content-Type": content_type,
                    "Accept-Encoding": IMAGE_REQUEST_ACCEPT_ENCODING,
                })
            }
            Self::Gemini => json!({
                "x-goog-api-key": "***",
                "Content-Type": "application/json",
                "Accept-Encoding": IMAGE_REQUEST_ACCEPT_ENCODING,
            }),
            Self::OpenAiResponses => json!({
                "Authorization": "Bearer ***",
                "Content-Type": "application/json",
                "Accept-Encoding": IMAGE_REQUEST_ACCEPT_ENCODING,
            }),
        }
    }

    fn request_body_snapshot(
        self,
        input: &CreateImageJobInput,
    ) -> Result<serde_json::Value, String> {
        let output_format = input.params.output_format.trim().to_lowercase();
        match self {
            Self::OpenAiCompatible => {
                if input.mode == ImageJobMode::ImageToImage.as_str() {
                    Ok(build_image_to_image_request_body_snapshot(
                        input,
                        &output_format,
                    ))
                } else {
                    Ok(build_text_to_image_request_body(input, &output_format))
                }
            }
            Self::Gemini => build_gemini_request_body(input, false),
            Self::OpenAiResponses => Ok(build_responses_request_body_snapshot(input, false)),
        }
    }

    async fn execute_generation_request(
        self,
        state: &SqliteDbState,
        channel: &ImageChannelDto,
        input: &CreateImageJobInput,
        request_url: &str,
        progress_context: &ImageJobProgressContext<'_>,
    ) -> Result<Vec<GeneratedImageResult>, String> {
        match self {
            Self::OpenAiCompatible => {
                execute_openai_compatible_generation_request(
                    state,
                    channel,
                    input,
                    request_url,
                    progress_context,
                )
                .await
            }
            Self::Gemini => {
                execute_gemini_generation_request(
                    state,
                    channel,
                    input,
                    request_url,
                    progress_context,
                )
                .await
            }
            Self::OpenAiResponses => {
                execute_responses_generation_request(
                    state,
                    channel,
                    input,
                    request_url,
                    progress_context,
                )
                .await
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponsesInputPayloadMode {
    CompactString,
    MessageList,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponsesTransportKind {
    Json,
    Stream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponsesToolChoiceMode {
    Required,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponsesReferenceInputMode {
    InlineDataUrl,
    FileId,
}

fn responses_reference_input_mode_label(mode: ResponsesReferenceInputMode) -> &'static str {
    match mode {
        ResponsesReferenceInputMode::InlineDataUrl => "inline_data_url",
        ResponsesReferenceInputMode::FileId => "file_id",
    }
}

#[derive(Clone, Copy, Debug)]
struct ResponsesRequestPlan {
    id: &'static str,
    input_payload_mode: ResponsesInputPayloadMode,
    transport: ResponsesTransportKind,
    tool_choice_mode: ResponsesToolChoiceMode,
}

struct PreparedResponsesReferenceInputs {
    input_images: Vec<serde_json::Value>,
    uploaded_file_ids: Vec<String>,
}

struct GeneratedImageResult {
    bytes: Vec<u8>,
    mime_type: String,
    response_metadata: Option<serde_json::Value>,
}

fn generated_image_result(bytes: Vec<u8>, mime_type: String) -> GeneratedImageResult {
    GeneratedImageResult {
        bytes,
        mime_type,
        response_metadata: None,
    }
}

fn image_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(app_data_dir.join("image-studio"))
}

pub fn image_assets_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(image_data_dir(app)?.join("assets"))
}

fn ensure_image_assets_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = image_assets_dir(app)?;
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create image assets dir: {}", e))?;
    }
    Ok(dir)
}

fn sanitize_file_name(file_name: &str) -> String {
    let trimmed = file_name.trim();
    let fallback = "image.png";
    let candidate = if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    };
    candidate
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => ch,
        })
        .collect()
}

fn sanitize_channel_path(raw_path: Option<String>) -> Option<String> {
    raw_path.and_then(|value| {
        let trimmed = value.trim().trim_matches('/').to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn file_extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

fn mime_from_output_format(output_format: &str) -> &'static str {
    match output_format {
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn decode_base64_bytes(raw: &str) -> Result<Vec<u8>, String> {
    let data = raw
        .split_once(',')
        .map(|(_, rest)| rest)
        .unwrap_or(raw)
        .trim()
        .replace(['\r', '\n', ' '], "");
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("Failed to decode base64 image data: {}", e))
}

fn build_image_api_url(base_url: &str, path: &str) -> String {
    let normalized_base = base_url.trim().trim_end_matches('/');
    let normalized_path = path.trim().trim_start_matches('/');
    if normalized_base.ends_with("/v1") {
        format!("{normalized_base}/{normalized_path}")
    } else {
        format!("{normalized_base}/v1/{normalized_path}")
    }
}

fn build_gemini_api_url(base_url: &str, path: &str) -> String {
    let normalized_base = base_url.trim().trim_end_matches('/');
    let normalized_path = path.trim().trim_start_matches('/');
    if normalized_base.ends_with("/v1beta")
        || normalized_base.ends_with("/v1alpha")
        || normalized_base.ends_with("/v1")
    {
        format!("{normalized_base}/{normalized_path}")
    } else {
        format!("{normalized_base}/v1beta/{normalized_path}")
    }
}

fn normalize_reference_data_url(reference: &ImageReferenceInput) -> String {
    let trimmed_data = reference.base64_data.trim();
    if trimmed_data.starts_with("data:") {
        return trimmed_data.to_string();
    }

    let normalized_base64 = trimmed_data.replace(['\r', '\n', ' '], "");
    format!("data:{};base64,{}", reference.mime_type, normalized_base64)
}

fn serialize_json_pretty(value: &serde_json::Value, error_context: &str) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map_err(|e| format!("Failed to serialize {}: {}", error_context, e))
}

fn summarize_response_headers(headers: &reqwest::header::HeaderMap) -> String {
    let interesting_headers = [
        "content-type",
        "content-length",
        "content-encoding",
        "transfer-encoding",
        "connection",
        "server",
        "cf-ray",
    ];

    let parts = interesting_headers
        .iter()
        .filter_map(|header_name| {
            headers
                .get(*header_name)
                .and_then(|value| value.to_str().ok())
                .map(|value| format!("{header_name}={value}"))
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(", ")
    }
}

fn build_image_result_http_error(
    mode: &str,
    channel_name: &str,
    request_url: &str,
    image_url: &str,
    status: reqwest::StatusCode,
    headers: &str,
    body_bytes: &[u8],
) -> String {
    let body_preview = String::from_utf8_lossy(body_bytes);
    let preview = truncate_for_log(&body_preview, 240);
    format!(
        "Image result fetch failed: mode={} channel={} url={} image_url={} HTTP {} headers={} body_preview={}",
        mode, channel_name, request_url, image_url, status, headers, preview
    )
}

fn format_reqwest_error(error: &reqwest::Error) -> String {
    let mut parts = vec![error.to_string()];

    let mut kind_flags = Vec::new();
    if error.is_timeout() {
        kind_flags.push("timeout");
    }
    if error.is_connect() {
        kind_flags.push("connect");
    }
    if error.is_request() {
        kind_flags.push("request");
    }
    if error.is_body() {
        kind_flags.push("body");
    }
    if error.is_decode() {
        kind_flags.push("decode");
    }

    if !kind_flags.is_empty() {
        parts.push(format!("kind={}", kind_flags.join("|")));
    }

    let mut source = error.source();
    let mut chain = Vec::new();
    while let Some(current) = source {
        chain.push(current.to_string());
        source = current.source();
    }
    if !chain.is_empty() {
        parts.push(format!("sources={}", chain.join(" <- ")));
    }

    parts.join(" ")
}

fn should_retry_image_request_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request() || error.is_body()
}

fn should_retry_image_response_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::REQUEST_TIMEOUT
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

fn image_request_retry_delay_ms(attempt: usize) -> u64 {
    IMAGE_REQUEST_RETRY_DELAYS_MS
        .get(attempt.saturating_sub(1))
        .copied()
        .unwrap_or(*IMAGE_REQUEST_RETRY_DELAYS_MS.last().unwrap_or(&3000))
}

fn emit_image_job_progress(
    context: &ImageJobProgressContext<'_>,
    stage: &'static str,
    attempt: usize,
    delay_ms: Option<u64>,
    plan: Option<&str>,
    reference_input_mode: Option<&str>,
    message: Option<String>,
) {
    let Some(app) = context.app else {
        return;
    };
    let payload = ImageJobProgressPayload {
        job_id: context.job_id.to_string(),
        stage,
        attempt,
        max_attempts: IMAGE_REQUEST_MAX_ATTEMPTS,
        retry_count: attempt.saturating_sub(1),
        max_retries: IMAGE_REQUEST_MAX_RETRIES,
        delay_ms,
        timeout_seconds: context.timeout_seconds,
        provider_kind: context.provider_kind.to_string(),
        mode: context.mode.to_string(),
        channel_name: context.channel_name.to_string(),
        model_id: context.model_id.to_string(),
        plan: plan.map(str::to_string),
        reference_input_mode: reference_input_mode.map(str::to_string),
        message,
    };
    let _ = app.emit(IMAGE_JOB_PROGRESS_EVENT, payload);
}

fn truncate_for_log(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let preview = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

fn detect_dimensions(bytes: &[u8]) -> (Option<i64>, Option<i64>) {
    let reader = match ImageReader::new(Cursor::new(bytes)).with_guessed_format() {
        Ok(reader) => reader,
        Err(_) => return (None, None),
    };

    match reader.into_dimensions() {
        Ok((width, height)) => (Some(width as i64), Some(height as i64)),
        Err(_) => (None, None),
    }
}

fn parse_channel_models(models_json: &str) -> Result<Vec<ImageChannelModel>, String> {
    if models_json.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(models_json)
        .map_err(|e| format!("Failed to parse image channel models: {}", e))
}

fn serialize_channel_models(models: &[ImageChannelModel]) -> Result<String, String> {
    serde_json::to_string(models)
        .map_err(|e| format!("Failed to serialize image channel models: {}", e))
}

fn channel_to_dto(record: ImageChannelRecord) -> Result<ImageChannelDto, String> {
    Ok(ImageChannelDto {
        id: db_clean_id(&record.id),
        name: record.name,
        provider_kind: record.provider_kind,
        base_url: record.base_url,
        api_key: record.api_key,
        generation_path: record.generation_path,
        edit_path: record.edit_path,
        timeout_seconds: record.timeout_seconds,
        enabled: record.enabled,
        sort_order: record.sort_order,
        models: parse_channel_models(&record.models_json)?,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn resolve_channel_timeout_seconds(channel: &ImageChannelDto) -> u64 {
    channel.timeout_seconds.unwrap_or(300).max(1)
}

fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

fn infer_gemini_aspect_ratio(size: &str) -> Option<String> {
    let trimmed = size.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        return None;
    }

    let (width_raw, height_raw) = trimmed
        .split_once(['x', 'X', '×'])
        .map(|(width, height)| (width.trim(), height.trim()))?;
    let width = width_raw.parse::<u32>().ok()?;
    let height = height_raw.parse::<u32>().ok()?;
    if width == 0 || height == 0 {
        return None;
    }

    let divisor = greatest_common_divisor(width, height);
    Some(format!("{}:{}", width / divisor, height / divisor))
}

fn supports_gemini_image_size(model_id: &str) -> bool {
    let normalized_model_id = model_id.trim().to_lowercase();
    normalized_model_id == "gemini-3.1-flash-image-preview"
        || normalized_model_id == "gemini-3-pro-image-preview"
}

fn infer_gemini_image_size(model_id: &str, size: &str) -> Option<&'static str> {
    if !supports_gemini_image_size(model_id) {
        return None;
    }

    let trimmed = size.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        return None;
    }

    let (width_raw, height_raw) = trimmed
        .split_once(['x', 'X', '×'])
        .map(|(width, height)| (width.trim(), height.trim()))?;
    let width = width_raw.parse::<u32>().ok()?;
    let height = height_raw.parse::<u32>().ok()?;
    let longer_side = width.max(height);

    if longer_side >= 3072 {
        Some("4K")
    } else if longer_side >= 1536 {
        Some("2K")
    } else {
        Some("1K")
    }
}

fn build_gemini_generation_config(input: &CreateImageJobInput) -> serde_json::Value {
    let mut generation_config = json!({
        "responseModalities": ["IMAGE"]
    });

    let mut image_config = serde_json::Map::new();
    if let Some(aspect_ratio) = infer_gemini_aspect_ratio(&input.params.size) {
        image_config.insert("aspectRatio".to_string(), json!(aspect_ratio));
    }
    if let Some(image_size) = infer_gemini_image_size(&input.model_id, &input.params.size) {
        image_config.insert("imageSize".to_string(), json!(image_size));
    }
    if !image_config.is_empty() {
        generation_config["imageConfig"] = serde_json::Value::Object(image_config);
    }

    generation_config
}

fn build_gemini_request_body(
    input: &CreateImageJobInput,
    include_inline_data: bool,
) -> Result<serde_json::Value, String> {
    let mut parts = vec![json!({ "text": input.prompt })];
    for reference in &input.references {
        if include_inline_data {
            let normalized_data = reference
                .base64_data
                .split_once(',')
                .map(|(_, rest)| rest)
                .unwrap_or(&reference.base64_data)
                .trim()
                .replace(['\r', '\n', ' '], "");
            parts.push(json!({
                "inlineData": {
                    "mimeType": reference.mime_type,
                    "data": normalized_data,
                }
            }));
        } else {
            parts.push(json!({
                "inlineData": {
                    "mimeType": reference.mime_type,
                    "data": "***",
                },
                "fileName": sanitize_file_name(&reference.file_name),
            }));
        }
    }

    Ok(json!({
        "contents": [{
            "parts": parts,
        }],
        "generationConfig": build_gemini_generation_config(input),
    }))
}

fn build_responses_prompt_text(prompt: &str) -> String {
    format!(
        "{}\n{}",
        RESPONSES_PROMPT_REWRITE_GUARD_PREFIX,
        prompt.trim()
    )
}

fn pick_responses_image_metadata(item: &serde_json::Value) -> Option<serde_json::Value> {
    let mut metadata = serde_json::Map::new();

    for key in [
        "size",
        "quality",
        "output_format",
        "moderation",
        "revised_prompt",
    ] {
        if let Some(value) = item
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            metadata.insert(key.to_string(), json!(value));
        }
    }

    if let Some(value) = item
        .get("output_compression")
        .and_then(|value| value.as_u64())
    {
        metadata.insert("output_compression".to_string(), json!(value));
    }

    if metadata.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(metadata))
    }
}

fn extract_responses_image_base64(item: &serde_json::Value) -> Option<&str> {
    if let Some(value) = item.get("b64_json").and_then(|value| value.as_str()) {
        return Some(value);
    }
    if let Some(value) = item.get("image").and_then(|value| value.as_str()) {
        return Some(value);
    }

    match item.get("result") {
        Some(serde_json::Value::String(value)) => Some(value),
        Some(serde_json::Value::Object(result)) => result
            .get("b64_json")
            .or_else(|| result.get("image"))
            .or_else(|| result.get("data"))
            .and_then(|value| value.as_str()),
        _ => None,
    }
}

fn extract_responses_image_url(item: &serde_json::Value) -> Option<&str> {
    if let Some(value) = item
        .get("url")
        .or_else(|| item.get("image_url"))
        .and_then(|value| value.as_str())
    {
        return Some(value);
    }

    match item.get("result") {
        Some(serde_json::Value::Object(result)) => result
            .get("url")
            .or_else(|| result.get("image_url"))
            .and_then(|value| value.as_str()),
        _ => None,
    }
}

fn build_responses_tool_body(
    input: &CreateImageJobInput,
) -> serde_json::Map<String, serde_json::Value> {
    let mut tool = serde_json::Map::from_iter([("type".to_string(), json!("image_generation"))]);

    let trimmed_size = input.params.size.trim();
    if !trimmed_size.is_empty() {
        tool.insert("size".to_string(), json!(trimmed_size));
    }

    let trimmed_quality = input.params.quality.trim();
    if !trimmed_quality.is_empty() {
        tool.insert("quality".to_string(), json!(trimmed_quality));
    }

    let trimmed_output_format = input.params.output_format.trim().to_lowercase();
    if !trimmed_output_format.is_empty() {
        tool.insert("output_format".to_string(), json!(trimmed_output_format));
    }

    if let Some(output_compression) = input.params.output_compression {
        if trimmed_output_format != "png" {
            tool.insert("output_compression".to_string(), json!(output_compression));
        }
    }

    if input.mode == ImageJobMode::ImageToImage.as_str() {
        tool.insert("action".to_string(), json!("edit"));
    } else {
        tool.insert("action".to_string(), json!("generate"));
    }

    tool
}

fn build_responses_input_content(
    input: &CreateImageJobInput,
    prepared_input_images: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut content = Vec::new();
    if !input.prompt.trim().is_empty() {
        content.push(json!({
            "type": "input_text",
            "text": build_responses_prompt_text(&input.prompt),
        }));
    }

    content.extend(prepared_input_images.iter().cloned());
    content
}

fn build_responses_input_payload(
    input: &CreateImageJobInput,
    prepared_input_images: &[serde_json::Value],
    input_payload_mode: ResponsesInputPayloadMode,
) -> serde_json::Value {
    if input_payload_mode == ResponsesInputPayloadMode::CompactString
        && prepared_input_images.is_empty()
        && !input.prompt.trim().is_empty()
    {
        return json!(build_responses_prompt_text(&input.prompt));
    }

    json!([{
        "role": "user",
        "content": build_responses_input_content(input, prepared_input_images),
    }])
}

fn build_responses_request_plans(input: &CreateImageJobInput) -> Vec<ResponsesRequestPlan> {
    let has_reference_images = !input.references.is_empty();
    let default_input_payload_mode = if has_reference_images {
        ResponsesInputPayloadMode::MessageList
    } else {
        ResponsesInputPayloadMode::CompactString
    };
    vec![ResponsesRequestPlan {
        id: if has_reference_images {
            "json-message-list"
        } else {
            "json-compact-string"
        },
        input_payload_mode: default_input_payload_mode,
        transport: ResponsesTransportKind::Json,
        tool_choice_mode: ResponsesToolChoiceMode::Required,
    }]
}

fn build_responses_request_body(
    input: &CreateImageJobInput,
    prepared_input_images: &[serde_json::Value],
    plan: ResponsesRequestPlan,
) -> serde_json::Value {
    let mut tool = build_responses_tool_body(input);
    if plan.transport == ResponsesTransportKind::Stream {
        tool.insert("partial_images".to_string(), json!(1));
    }

    let mut body = serde_json::Map::from_iter([
        ("model".to_string(), json!(input.model_id)),
        (
            "input".to_string(),
            build_responses_input_payload(input, prepared_input_images, plan.input_payload_mode),
        ),
        (
            "tools".to_string(),
            json!([serde_json::Value::Object(tool)]),
        ),
    ]);

    if plan.transport == ResponsesTransportKind::Stream {
        body.insert("stream".to_string(), json!(true));
    }

    if plan.tool_choice_mode == ResponsesToolChoiceMode::Required {
        body.insert("tool_choice".to_string(), json!("required"));
    }

    serde_json::Value::Object(body)
}

fn build_responses_request_body_snapshot(
    input: &CreateImageJobInput,
    include_reference_data: bool,
) -> serde_json::Value {
    let prepared_input_images = input
        .references
        .iter()
        .map(|reference| {
            let image_value = if include_reference_data {
                normalize_reference_data_url(reference)
            } else {
                "***".to_string()
            };
            json!({
                "type": "input_image",
                "image_url": image_value,
            })
        })
        .collect::<Vec<_>>();

    build_responses_request_body(
        input,
        &prepared_input_images,
        ResponsesRequestPlan {
            id: "snapshot",
            input_payload_mode: if input.references.is_empty() {
                ResponsesInputPayloadMode::CompactString
            } else {
                ResponsesInputPayloadMode::MessageList
            },
            transport: ResponsesTransportKind::Json,
            tool_choice_mode: ResponsesToolChoiceMode::Required,
        },
    )
}

fn build_text_to_image_request_body(
    input: &CreateImageJobInput,
    output_format: &str,
) -> serde_json::Value {
    let mut request_body = json!({
        "model": input.model_id,
        "prompt": input.prompt,
        "size": input.params.size,
        "quality": input.params.quality,
        "output_format": output_format,
    });

    if let Some(moderation) = input
        .params
        .moderation
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        request_body["moderation"] = json!(moderation);
    }

    if let Some(output_compression) = input.params.output_compression {
        if output_format != "png" {
            request_body["output_compression"] = json!(output_compression);
        }
    }

    request_body
}

fn build_image_to_image_request_body_snapshot(
    input: &CreateImageJobInput,
    output_format: &str,
) -> serde_json::Value {
    let mut request_body = json!({
        "model": input.model_id,
        "prompt": input.prompt,
        "size": input.params.size,
        "quality": input.params.quality,
        "output_format": output_format,
        "image_field": if input.references.len() > 1 { "image[]" } else { "image" },
        "reference_count": input.references.len(),
        "references": input
            .references
            .iter()
            .map(|reference| json!({
                "file_name": reference.file_name,
                "mime_type": reference.mime_type,
            }))
            .collect::<Vec<_>>(),
    });

    if let Some(moderation) = input
        .params
        .moderation
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        request_body["moderation"] = json!(moderation);
    }

    if let Some(output_compression) = input.params.output_compression {
        if output_format != "png" {
            request_body["output_compression"] = json!(output_compression);
        }
    }

    request_body
}

fn build_request_snapshot(
    channel: &ImageChannelDto,
    input: &CreateImageJobInput,
) -> Result<ImageJobRequestSnapshot, String> {
    let provider_adapter = ImageProviderAdapter::from_kind(&channel.provider_kind)?;
    let request_url = provider_adapter.build_request_url(channel, &input.mode, &input.model_id)?;
    let request_headers_value = provider_adapter.request_headers_snapshot(input);
    let request_headers_json =
        serialize_json_pretty(&request_headers_value, "image request headers")?;

    let request_body_value = provider_adapter.request_body_snapshot(input)?;
    let request_body_json = serialize_json_pretty(&request_body_value, "image request body")?;

    Ok(ImageJobRequestSnapshot {
        request_url,
        request_headers_json,
        request_body_json,
    })
}

fn find_channel_model<'a>(
    channel: &'a ImageChannelDto,
    model_id: &str,
) -> Option<&'a ImageChannelModel> {
    channel.models.iter().find(|model| model.id == model_id)
}

fn validate_channel_model_support(
    channel: &ImageChannelDto,
    model: &ImageChannelModel,
    mode: &str,
) -> Result<(), String> {
    if !channel.enabled {
        return Err(format!("Image channel is disabled: {}", channel.name));
    }
    if !model.enabled {
        return Err(format!("Image model is disabled: {}", model.id));
    }

    if mode == ImageJobMode::TextToImage.as_str() && !model.supports_text_to_image {
        return Err(format!(
            "Model {} does not support text-to-image on channel {}",
            model.id, channel.name
        ));
    }

    if mode == ImageJobMode::ImageToImage.as_str() && !model.supports_image_to_image {
        return Err(format!(
            "Model {} does not support image-to-image on channel {}",
            model.id, channel.name
        ));
    }

    Ok(())
}

fn validate_channel_input(input: &UpsertImageChannelInput) -> Result<(), String> {
    let channel_name = input.name.trim();
    if channel_name.is_empty() {
        return Err("Image channel name is required".to_string());
    }

    let base_url = input.base_url.trim();
    if base_url.is_empty() {
        return Err("Image channel base URL is required".to_string());
    }

    let provider_adapter = ImageProviderAdapter::from_kind(&input.provider_kind)?;

    let mut model_ids = HashSet::new();
    for model in &input.models {
        let model_id = model.id.trim();
        if model_id.is_empty() {
            return Err("Image channel model ID is required".to_string());
        }
        if !model_ids.insert(model_id.to_string()) {
            return Err(format!("Duplicate image channel model ID: {}", model_id));
        }
        if !model.supports_text_to_image && !model.supports_image_to_image {
            return Err(format!(
                "Image channel model must support at least one mode: {}",
                model.id
            ));
        }
    }

    if provider_adapter.supports_custom_paths() {
        for raw_path in [&input.generation_path, &input.edit_path] {
            if let Some(path) = raw_path
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                if path.contains("://") {
                    return Err(format!("Image channel path must be relative: {}", path));
                }
            }
        }
    }

    Ok(())
}

fn normalize_channel_models(models: &[ImageChannelModel]) -> Vec<ImageChannelModel> {
    models
        .iter()
        .map(|model| ImageChannelModel {
            id: model.id.trim().to_string(),
            name: model
                .name
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            supports_text_to_image: model.supports_text_to_image,
            supports_image_to_image: model.supports_image_to_image,
            enabled: model.enabled,
        })
        .collect()
}

fn to_asset_dto(app: &AppHandle, record: &ImageAssetRecord) -> Result<ImageAssetDto, String> {
    let full_path = image_data_dir(app)?.join(&record.relative_path);
    Ok(ImageAssetDto {
        id: record.id.clone(),
        job_id: record.job_id.clone(),
        role: record.role.clone(),
        mime_type: record.mime_type.clone(),
        file_name: record.file_name.clone(),
        relative_path: record.relative_path.clone(),
        bytes: record.bytes,
        width: record.width,
        height: record.height,
        created_at: record.created_at,
        file_path: full_path.to_string_lossy().to_string(),
    })
}

fn remove_asset_files(app: &AppHandle, assets: &[ImageAssetRecord]) -> Result<(), String> {
    let image_root_dir = image_data_dir(app)?;
    for asset in assets {
        let asset_path = image_root_dir.join(&asset.relative_path);
        if !asset_path.exists() {
            continue;
        }

        fs::remove_file(&asset_path).map_err(|e| {
            format!(
                "Failed to remove image asset file {}: {}",
                asset_path.display(),
                e
            )
        })?;
    }

    Ok(())
}

async fn persist_asset_file(
    app: &AppHandle,
    state: &SqliteDbState,
    job_id: Option<String>,
    role: &str,
    file_name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> Result<ImageAssetRecord, String> {
    let started_at = Instant::now();
    let assets_dir = ensure_image_assets_dir(app)?;
    let asset_id = crate::coding::db_new_id();
    let extension = Path::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_string())
        .unwrap_or_else(|| file_extension_for_mime(mime_type).to_string());
    let stored_file_name = format!("{asset_id}.{extension}");
    let relative_path = format!("assets/{stored_file_name}");
    let full_path = assets_dir.join(&stored_file_name);
    fs::write(&full_path, bytes).map_err(|e| format!("Failed to write image asset file: {}", e))?;

    let (width, height) = detect_dimensions(bytes);
    let asset = ImageAssetRecord {
        id: asset_id,
        job_id,
        role: role.to_string(),
        mime_type: mime_type.to_string(),
        file_name: sanitize_file_name(file_name),
        relative_path,
        bytes: bytes.len() as i64,
        width,
        height,
        created_at: now_ms(),
    };

    let created_id = store::create_image_asset(state, &asset).await?;
    debug!(
        "Image asset persisted: asset_id={} job_id={} role={} bytes={} mime_type={} file_name={} elapsed_ms={}",
        created_id,
        asset.job_id.as_deref().unwrap_or("none"),
        role,
        bytes.len(),
        mime_type,
        stored_file_name,
        started_at.elapsed().as_millis()
    );
    Ok(ImageAssetRecord {
        id: created_id,
        ..asset
    })
}

async fn persist_reference_assets(
    app: &AppHandle,
    state: &SqliteDbState,
    job_id: &str,
    references: &[ImageReferenceInput],
) -> Result<Vec<ImageAssetRecord>, String> {
    let mut assets = Vec::with_capacity(references.len());
    for reference in references {
        let bytes = decode_base64_bytes(&reference.base64_data)?;
        let asset = persist_asset_file(
            app,
            state,
            Some(job_id.to_string()),
            "input",
            &reference.file_name,
            &reference.mime_type,
            &bytes,
        )
        .await?;
        assets.push(asset);
    }
    Ok(assets)
}

async fn execute_generation_request(
    app: Option<&AppHandle>,
    state: &SqliteDbState,
    job_id: &str,
    channel: &ImageChannelDto,
    input: &CreateImageJobInput,
    request_url: &str,
) -> Result<Vec<GeneratedImageResult>, String> {
    let timeout_seconds = resolve_channel_timeout_seconds(channel);
    let progress_context = ImageJobProgressContext {
        app,
        job_id,
        provider_kind: &channel.provider_kind,
        mode: &input.mode,
        channel_name: &channel.name,
        model_id: &input.model_id,
        timeout_seconds,
    };

    ImageProviderAdapter::from_kind(&channel.provider_kind)?
        .execute_generation_request(state, channel, input, request_url, &progress_context)
        .await
}

async fn execute_openai_compatible_generation_request(
    state: &SqliteDbState,
    channel: &ImageChannelDto,
    input: &CreateImageJobInput,
    request_url: &str,
    progress_context: &ImageJobProgressContext<'_>,
) -> Result<Vec<GeneratedImageResult>, String> {
    let timeout_seconds = progress_context.timeout_seconds;
    let client = http_client::client_with_timeout_no_compression(state, timeout_seconds).await?;
    let authorization = format!("Bearer {}", channel.api_key.trim());
    let output_format = input.params.output_format.trim().to_lowercase();
    let mime_type = mime_from_output_format(&output_format).to_string();

    if input.mode == ImageJobMode::ImageToImage.as_str() {
        for attempt in 1..=IMAGE_REQUEST_MAX_ATTEMPTS {
            let request_started_at = Instant::now();
            emit_image_job_progress(
                &progress_context,
                "request_start",
                attempt,
                None,
                None,
                None,
                None,
            );
            debug!(
                "Image request start: mode={} channel={} model={} url={} timeout={}s output_format={} reference_count={} attempt={}/{}",
                input.mode,
                channel.name,
                input.model_id,
                request_url,
                timeout_seconds,
                output_format,
                input.references.len(),
                attempt,
                IMAGE_REQUEST_MAX_ATTEMPTS
            );

            let mut form = Form::new()
                .text("model", input.model_id.clone())
                .text("prompt", input.prompt.clone())
                .text("size", input.params.size.clone())
                .text("quality", input.params.quality.clone())
                .text("output_format", output_format.clone());

            if let Some(moderation) = input
                .params
                .moderation
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                form = form.text("moderation", moderation.to_string());
            }

            if let Some(output_compression) = input.params.output_compression {
                if output_format != "png" {
                    form = form.text("output_compression", output_compression.to_string());
                }
            }

            for reference in &input.references {
                let bytes = decode_base64_bytes(&reference.base64_data)?;
                let part = Part::bytes(bytes)
                    .file_name(sanitize_file_name(&reference.file_name))
                    .mime_str(&reference.mime_type)
                    .map_err(|e| format!("Invalid image mime type: {}", e))?;
                let field_name = if input.references.len() > 1 {
                    "image[]"
                } else {
                    "image"
                };
                form = form.part(field_name.to_string(), part);
            }

            let response = match client
                .post(request_url)
                .header("Authorization", &authorization)
                .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
                .multipart(form)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error)
                    if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
                        && should_retry_image_request_error(&error) =>
                {
                    let delay_ms = image_request_retry_delay_ms(attempt);
                    emit_image_job_progress(
                        &progress_context,
                        "retry_scheduled",
                        attempt,
                        Some(delay_ms),
                        None,
                        None,
                        Some(format_reqwest_error(&error)),
                    );
                    warn!(
                        "Image request retry scheduled after transport error: mode={} channel={} model={} url={} attempt={}/{} delay_ms={} error={}",
                        input.mode,
                        channel.name,
                        input.model_id,
                        request_url,
                        attempt,
                        IMAGE_REQUEST_MAX_ATTEMPTS,
                        delay_ms,
                        format_reqwest_error(&error)
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    continue;
                }
                Err(error) => {
                    let message = format!(
                        "Image edit request failed: mode={} channel={} model={} url={} timeout={}s error={}",
                        input.mode,
                        channel.name,
                        input.model_id,
                        request_url,
                        timeout_seconds,
                        format_reqwest_error(&error)
                    );
                    error!("{}", message);
                    return Err(message);
                }
            };

            debug!(
                "Image request headers received: mode={} channel={} model={} url={} elapsed_ms={} status={} headers={} attempt={}/{}",
                input.mode,
                channel.name,
                input.model_id,
                request_url,
                request_started_at.elapsed().as_millis(),
                response.status(),
                summarize_response_headers(response.headers()),
                attempt,
                IMAGE_REQUEST_MAX_ATTEMPTS
            );

            if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
                && should_retry_image_response_status(response.status())
            {
                let retry_status = response.status();
                let retry_body = match response.text().await {
                    Ok(body) => truncate_for_log(&body.replace(['\r', '\n'], " "), 240),
                    Err(error) => format!("<failed to read retry body: {}>", error),
                };
                let delay_ms = image_request_retry_delay_ms(attempt);
                emit_image_job_progress(
                    &progress_context,
                    "retry_scheduled",
                    attempt,
                    Some(delay_ms),
                    None,
                    None,
                    Some(format!("HTTP {retry_status} {retry_body}")),
                );
                warn!(
                    "Image request retry scheduled after upstream status: mode={} channel={} model={} url={} attempt={}/{} delay_ms={} status={} body_preview={}",
                    input.mode,
                    channel.name,
                    input.model_id,
                    request_url,
                    attempt,
                    IMAGE_REQUEST_MAX_ATTEMPTS,
                    delay_ms,
                    retry_status,
                    retry_body
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            return parse_image_response(
                state,
                timeout_seconds,
                response,
                &mime_type,
                request_url,
                &channel.name,
                &input.mode,
                request_started_at,
            )
            .await
            .map(|results| {
                results
                    .into_iter()
                    .map(|(bytes, mime_type)| generated_image_result(bytes, mime_type))
                    .collect()
            });
        }

        return Err("Image edit request exhausted retries unexpectedly".to_string());
    }

    let request_body = build_text_to_image_request_body(input, &output_format);
    for attempt in 1..=IMAGE_REQUEST_MAX_ATTEMPTS {
        let request_started_at = Instant::now();
        emit_image_job_progress(
            &progress_context,
            "request_start",
            attempt,
            None,
            None,
            None,
            None,
        );
        debug!(
            "Image request start: mode={} channel={} model={} url={} timeout={}s output_format={} reference_count={} attempt={}/{}",
            input.mode,
            channel.name,
            input.model_id,
            request_url,
            timeout_seconds,
            output_format,
            input.references.len(),
            attempt,
            IMAGE_REQUEST_MAX_ATTEMPTS
        );

        let response = match client
            .post(request_url)
            .header("Authorization", &authorization)
            .header("Content-Type", "application/json")
            .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
            .json(&request_body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error)
                if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
                    && should_retry_image_request_error(&error) =>
            {
                let delay_ms = image_request_retry_delay_ms(attempt);
                emit_image_job_progress(
                    &progress_context,
                    "retry_scheduled",
                    attempt,
                    Some(delay_ms),
                    None,
                    None,
                    Some(format_reqwest_error(&error)),
                );
                warn!(
                    "Image request retry scheduled after transport error: mode={} channel={} model={} url={} attempt={}/{} delay_ms={} error={}",
                    input.mode,
                    channel.name,
                    input.model_id,
                    request_url,
                    attempt,
                    IMAGE_REQUEST_MAX_ATTEMPTS,
                    delay_ms,
                    format_reqwest_error(&error)
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }
            Err(error) => {
                let message = format!(
                    "Image generation request failed: mode={} channel={} model={} url={} timeout={}s error={}",
                    input.mode,
                    channel.name,
                    input.model_id,
                    request_url,
                    timeout_seconds,
                    format_reqwest_error(&error)
                );
                error!("{}", message);
                return Err(message);
            }
        };

        debug!(
            "Image request headers received: mode={} channel={} model={} url={} elapsed_ms={} status={} headers={} attempt={}/{}",
            input.mode,
            channel.name,
            input.model_id,
            request_url,
            request_started_at.elapsed().as_millis(),
            response.status(),
            summarize_response_headers(response.headers()),
            attempt,
            IMAGE_REQUEST_MAX_ATTEMPTS
        );

        if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
            && should_retry_image_response_status(response.status())
        {
            let retry_status = response.status();
            let retry_body = match response.text().await {
                Ok(body) => truncate_for_log(&body.replace(['\r', '\n'], " "), 240),
                Err(error) => format!("<failed to read retry body: {}>", error),
            };
            let delay_ms = image_request_retry_delay_ms(attempt);
            emit_image_job_progress(
                &progress_context,
                "retry_scheduled",
                attempt,
                Some(delay_ms),
                None,
                None,
                Some(format!("HTTP {retry_status} {retry_body}")),
            );
            warn!(
                "Image request retry scheduled after upstream status: mode={} channel={} model={} url={} attempt={}/{} delay_ms={} status={} body_preview={}",
                input.mode,
                channel.name,
                input.model_id,
                request_url,
                attempt,
                IMAGE_REQUEST_MAX_ATTEMPTS,
                delay_ms,
                retry_status,
                retry_body
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            continue;
        }

        return parse_image_response(
            state,
            timeout_seconds,
            response,
            &mime_type,
            request_url,
            &channel.name,
            &input.mode,
            request_started_at,
        )
        .await
        .map(|results| {
            results
                .into_iter()
                .map(|(bytes, mime_type)| generated_image_result(bytes, mime_type))
                .collect()
        });
    }

    Err("Image generation request exhausted retries unexpectedly".to_string())
}

async fn parse_image_response(
    state: &SqliteDbState,
    timeout_seconds: u64,
    response: reqwest::Response,
    fallback_mime_type: &str,
    request_url: &str,
    channel_name: &str,
    mode: &str,
    request_started_at: Instant,
) -> Result<Vec<(Vec<u8>, String)>, String> {
    let status = response.status();
    let response_headers = summarize_response_headers(response.headers());
    let body_read_started_at = Instant::now();
    let response_bytes = response.bytes().await.map_err(|e| {
        let message = format!(
            "Failed to read image API response body: mode={} channel={} url={} status={} elapsed_ms={} body_read_ms={} error={}",
            mode,
            channel_name,
            request_url,
            status,
            request_started_at.elapsed().as_millis(),
            body_read_started_at.elapsed().as_millis(),
            format_reqwest_error(&e)
        );
        error!("{}", message);
        message
    })?;

    debug!(
        "Image response body read: mode={} channel={} url={} status={} elapsed_ms={} body_read_ms={} bytes={} headers={}",
        mode,
        channel_name,
        request_url,
        status,
        request_started_at.elapsed().as_millis(),
        body_read_started_at.elapsed().as_millis(),
        response_bytes.len(),
        response_headers
    );

    if !status.is_success() {
        let body = String::from_utf8_lossy(&response_bytes);
        let mut message = format!(
            "Image API failed: mode={mode} channel={channel_name} url={request_url} HTTP {status} {body}"
        );

        if body.contains("image is not supported") {
            if mode == ImageJobMode::ImageToImage.as_str() {
                message.push_str(
                    " Hint: current request is image-to-image, but the upstream channel or edit path does not accept image inputs. Check whether the channel edit path points to a real images/edits-compatible endpoint and whether the upstream gateway actually supports image edits for this model."
                );
            } else {
                message.push_str(
                    " Hint: current request is text-to-image, so this error usually means the upstream gateway routed the request to a path that expects a different payload. Check generation path and upstream request transformation."
                );
            }
        }

        error!("{}", message);
        return Err(message);
    }

    let json_parse_started_at = Instant::now();
    let payload: serde_json::Value = serde_json::from_slice(&response_bytes).map_err(|e| {
        let body_preview = String::from_utf8_lossy(&response_bytes);
        let preview = body_preview.chars().take(240).collect::<String>();
        let message = format!(
            "Failed to parse image API response: mode={} channel={} url={} elapsed_ms={} json_parse_ms={} bytes={} error={} body_preview={}",
            mode,
            channel_name,
            request_url,
            request_started_at.elapsed().as_millis(),
            json_parse_started_at.elapsed().as_millis(),
            response_bytes.len(),
            e,
            preview
        );
        error!("{}", message);
        message
    })?;

    debug!(
        "Image response json parsed: mode={} channel={} url={} elapsed_ms={} json_parse_ms={}",
        mode,
        channel_name,
        request_url,
        request_started_at.elapsed().as_millis(),
        json_parse_started_at.elapsed().as_millis()
    );

    let data = payload
        .get("data")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "Image API returned no data array".to_string())?;

    let mut results = Vec::new();
    for item in data {
        if let Some(base64_data) = item.get("b64_json").and_then(|value| value.as_str()) {
            results.push((
                decode_base64_bytes(base64_data)?,
                fallback_mime_type.to_string(),
            ));
            continue;
        }

        if let Some(image_url) = item.get("url").and_then(|value| value.as_str()) {
            let client =
                http_client::client_with_timeout_no_compression(state, timeout_seconds).await?;
            let image_url_started_at = Instant::now();
            debug!(
                "Image result fetch start: mode={} channel={} request_url={} image_url={} timeout={}s",
                mode, channel_name, request_url, image_url, timeout_seconds
            );
            let bytes = client
                .get(image_url)
                .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
                .send()
                .await
                .map_err(|e| {
                    let message = format!(
                        "Failed to fetch image URL result: mode={} channel={} url={} image_url={} timeout={}s error={}",
                        mode,
                        channel_name,
                        request_url,
                        image_url,
                        timeout_seconds,
                        format_reqwest_error(&e)
                    );
                    error!("{}", message);
                    message
                })?;

            debug!(
                "Image result headers received: mode={} channel={} request_url={} image_url={} elapsed_ms={} status={} headers={}",
                mode,
                channel_name,
                request_url,
                image_url,
                image_url_started_at.elapsed().as_millis(),
                bytes.status(),
                summarize_response_headers(bytes.headers())
            );

            let status = bytes.status();
            let headers = summarize_response_headers(bytes.headers());
            let image_body_read_started_at = Instant::now();
            let bytes = bytes
                .bytes()
                .await
                .map_err(|e| {
                    let message = format!(
                        "Failed to read image URL bytes: mode={} channel={} url={} image_url={} elapsed_ms={} body_read_ms={} error={}",
                        mode,
                        channel_name,
                        request_url,
                        image_url,
                        image_url_started_at.elapsed().as_millis(),
                        image_body_read_started_at.elapsed().as_millis(),
                        format_reqwest_error(&e)
                    );
                    error!("{}", message);
                    message
                })?;

            debug!(
                "Image result body read: mode={} channel={} request_url={} image_url={} status={} elapsed_ms={} body_read_ms={} bytes={} headers={}",
                mode,
                channel_name,
                request_url,
                image_url,
                status,
                image_url_started_at.elapsed().as_millis(),
                image_body_read_started_at.elapsed().as_millis(),
                bytes.len(),
                headers
            );

            if !status.is_success() {
                let message = build_image_result_http_error(
                    mode,
                    channel_name,
                    request_url,
                    image_url,
                    status,
                    &headers,
                    &bytes,
                );
                error!("{}", message);
                return Err(message);
            }
            results.push((bytes.to_vec(), fallback_mime_type.to_string()));
        }
    }

    if results.is_empty() {
        let message = format!(
            "Image API returned no usable image payload: mode={} channel={} url={}",
            mode, channel_name, request_url
        );
        error!("{}", message);
        return Err(message);
    }

    debug!(
        "Image response processed: mode={} channel={} url={} elapsed_ms={} result_count={}",
        mode,
        channel_name,
        request_url,
        request_started_at.elapsed().as_millis(),
        results.len()
    );

    Ok(results)
}

async fn execute_gemini_generation_request(
    state: &SqliteDbState,
    channel: &ImageChannelDto,
    input: &CreateImageJobInput,
    request_url: &str,
    progress_context: &ImageJobProgressContext<'_>,
) -> Result<Vec<GeneratedImageResult>, String> {
    let timeout_seconds = resolve_channel_timeout_seconds(channel);
    let client = http_client::client_with_timeout_no_compression(state, timeout_seconds).await?;
    let request_body = build_gemini_request_body(input, true)?;

    for attempt in 1..=IMAGE_REQUEST_MAX_ATTEMPTS {
        let request_started_at = Instant::now();
        emit_image_job_progress(
            progress_context,
            "request_start",
            attempt,
            None,
            None,
            None,
            None,
        );
        debug!(
            "Gemini image request start: mode={} channel={} model={} url={} timeout={}s reference_count={} attempt={}/{}",
            input.mode,
            channel.name,
            input.model_id,
            request_url,
            timeout_seconds,
            input.references.len(),
            attempt,
            IMAGE_REQUEST_MAX_ATTEMPTS
        );

        let response = match client
            .post(request_url)
            .header("x-goog-api-key", channel.api_key.trim())
            .header("Content-Type", "application/json")
            .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
            .json(&request_body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error)
                if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
                    && should_retry_image_request_error(&error) =>
            {
                let delay_ms = image_request_retry_delay_ms(attempt);
                emit_image_job_progress(
                    progress_context,
                    "retry_scheduled",
                    attempt,
                    Some(delay_ms),
                    None,
                    None,
                    Some(format_reqwest_error(&error)),
                );
                warn!(
                    "Gemini image request retry scheduled after transport error: mode={} channel={} model={} url={} attempt={}/{} delay_ms={} error={}",
                    input.mode,
                    channel.name,
                    input.model_id,
                    request_url,
                    attempt,
                    IMAGE_REQUEST_MAX_ATTEMPTS,
                    delay_ms,
                    format_reqwest_error(&error)
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }
            Err(error) => {
                let message = format!(
                    "Gemini image request failed: mode={} channel={} model={} url={} timeout={}s error={}",
                    input.mode,
                    channel.name,
                    input.model_id,
                    request_url,
                    timeout_seconds,
                    format_reqwest_error(&error)
                );
                error!("{}", message);
                return Err(message);
            }
        };

        debug!(
            "Gemini image request headers received: mode={} channel={} model={} url={} elapsed_ms={} status={} headers={} attempt={}/{}",
            input.mode,
            channel.name,
            input.model_id,
            request_url,
            request_started_at.elapsed().as_millis(),
            response.status(),
            summarize_response_headers(response.headers()),
            attempt,
            IMAGE_REQUEST_MAX_ATTEMPTS
        );

        if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
            && should_retry_image_response_status(response.status())
        {
            let retry_status = response.status();
            let retry_body = match response.text().await {
                Ok(body) => truncate_for_log(&body.replace(['\r', '\n'], " "), 240),
                Err(error) => format!("<failed to read retry body: {}>", error),
            };
            let delay_ms = image_request_retry_delay_ms(attempt);
            emit_image_job_progress(
                progress_context,
                "retry_scheduled",
                attempt,
                Some(delay_ms),
                None,
                None,
                Some(format!("HTTP {retry_status} {retry_body}")),
            );
            warn!(
                "Gemini image request retry scheduled after upstream status: mode={} channel={} model={} url={} attempt={}/{} delay_ms={} status={} body_preview={}",
                input.mode,
                channel.name,
                input.model_id,
                request_url,
                attempt,
                IMAGE_REQUEST_MAX_ATTEMPTS,
                delay_ms,
                retry_status,
                retry_body
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            continue;
        }

        return parse_gemini_image_response(
            response,
            request_url,
            &channel.name,
            &input.mode,
            request_started_at,
        )
        .await
        .map(|results| {
            results
                .into_iter()
                .map(|(bytes, mime_type)| generated_image_result(bytes, mime_type))
                .collect()
        });
    }

    Err("Gemini image request exhausted retries unexpectedly".to_string())
}

async fn parse_gemini_image_response(
    response: reqwest::Response,
    request_url: &str,
    channel_name: &str,
    mode: &str,
    request_started_at: Instant,
) -> Result<Vec<(Vec<u8>, String)>, String> {
    let status = response.status();
    let response_headers = summarize_response_headers(response.headers());
    let body_read_started_at = Instant::now();
    let response_bytes = response.bytes().await.map_err(|e| {
        let message = format!(
            "Failed to read Gemini image response body: mode={} channel={} url={} status={} elapsed_ms={} body_read_ms={} error={}",
            mode,
            channel_name,
            request_url,
            status,
            request_started_at.elapsed().as_millis(),
            body_read_started_at.elapsed().as_millis(),
            format_reqwest_error(&e)
        );
        error!("{}", message);
        message
    })?;

    debug!(
        "Gemini image response body read: mode={} channel={} url={} status={} elapsed_ms={} body_read_ms={} bytes={} headers={}",
        mode,
        channel_name,
        request_url,
        status,
        request_started_at.elapsed().as_millis(),
        body_read_started_at.elapsed().as_millis(),
        response_bytes.len(),
        response_headers
    );

    if !status.is_success() {
        let body = String::from_utf8_lossy(&response_bytes);
        let message = format!(
            "Gemini image API failed: mode={mode} channel={channel_name} url={request_url} HTTP {status} {body}"
        );
        error!("{}", message);
        return Err(message);
    }

    let json_parse_started_at = Instant::now();
    let payload: serde_json::Value = serde_json::from_slice(&response_bytes).map_err(|e| {
        let body_preview = String::from_utf8_lossy(&response_bytes);
        let preview = body_preview.chars().take(240).collect::<String>();
        let message = format!(
            "Failed to parse Gemini image response: mode={} channel={} url={} elapsed_ms={} json_parse_ms={} bytes={} error={} body_preview={}",
            mode,
            channel_name,
            request_url,
            request_started_at.elapsed().as_millis(),
            json_parse_started_at.elapsed().as_millis(),
            response_bytes.len(),
            e,
            preview
        );
        error!("{}", message);
        message
    })?;

    debug!(
        "Gemini image response json parsed: mode={} channel={} url={} elapsed_ms={} json_parse_ms={}",
        mode,
        channel_name,
        request_url,
        request_started_at.elapsed().as_millis(),
        json_parse_started_at.elapsed().as_millis()
    );

    let mut results = Vec::new();
    if let Some(candidates) = payload.get("candidates").and_then(|value| value.as_array()) {
        for candidate in candidates {
            let Some(parts) = candidate
                .get("content")
                .and_then(|content| content.get("parts"))
                .and_then(|value| value.as_array())
            else {
                continue;
            };

            for part in parts {
                let Some(inline_data) = part.get("inlineData").or_else(|| part.get("inline_data"))
                else {
                    continue;
                };
                let Some(base64_data) = inline_data.get("data").and_then(|value| value.as_str())
                else {
                    continue;
                };
                let mime_type = inline_data
                    .get("mimeType")
                    .or_else(|| inline_data.get("mime_type"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("image/png")
                    .to_string();
                results.push((decode_base64_bytes(base64_data)?, mime_type));
            }
        }
    }

    if results.is_empty() {
        let message = format!(
            "Gemini image API returned no usable image payload: mode={} channel={} url={}",
            mode, channel_name, request_url
        );
        error!("{}", message);
        return Err(message);
    }

    debug!(
        "Gemini image response processed: mode={} channel={} url={} elapsed_ms={} result_count={}",
        mode,
        channel_name,
        request_url,
        request_started_at.elapsed().as_millis(),
        results.len()
    );

    Ok(results)
}

async fn append_image_result_from_response_item(
    state: &SqliteDbState,
    timeout_seconds: u64,
    results: &mut Vec<GeneratedImageResult>,
    item: &serde_json::Value,
    fallback_mime_type: &str,
    request_url: &str,
    channel_name: &str,
    mode: &str,
) -> Result<(), String> {
    let response_metadata = pick_responses_image_metadata(item);

    if let Some(base64_data) = extract_responses_image_base64(item) {
        results.push(GeneratedImageResult {
            bytes: decode_base64_bytes(base64_data)?,
            mime_type: fallback_mime_type.to_string(),
            response_metadata,
        });
        return Ok(());
    }

    if let Some(image_url) = extract_responses_image_url(item) {
        let client =
            http_client::client_with_timeout_no_compression(state, timeout_seconds).await?;
        let image_url_started_at = Instant::now();
        debug!(
            "Responses image result fetch start: mode={} channel={} request_url={} image_url={} timeout={}s",
            mode, channel_name, request_url, image_url, timeout_seconds
        );
        let response = client
            .get(image_url)
            .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
            .send()
            .await
            .map_err(|e| {
                let message = format!(
                    "Failed to fetch Responses image URL result: mode={} channel={} url={} image_url={} timeout={}s error={}",
                    mode,
                    channel_name,
                    request_url,
                    image_url,
                    timeout_seconds,
                    format_reqwest_error(&e)
                );
                error!("{}", message);
                message
            })?;

        let status = response.status();
        let headers = summarize_response_headers(response.headers());
        let body_read_started_at = Instant::now();
        let bytes = response.bytes().await.map_err(|e| {
            let message = format!(
                "Failed to read Responses image URL bytes: mode={} channel={} url={} image_url={} elapsed_ms={} body_read_ms={} error={}",
                mode,
                channel_name,
                request_url,
                image_url,
                image_url_started_at.elapsed().as_millis(),
                body_read_started_at.elapsed().as_millis(),
                format_reqwest_error(&e)
            );
            error!("{}", message);
            message
        })?;

        if !status.is_success() {
            let message = build_image_result_http_error(
                mode,
                channel_name,
                request_url,
                image_url,
                status,
                &headers,
                &bytes,
            );
            error!("{}", message);
            return Err(message);
        }

        results.push(GeneratedImageResult {
            bytes: bytes.to_vec(),
            mime_type: fallback_mime_type.to_string(),
            response_metadata,
        });
    }

    Ok(())
}

async fn append_images_from_responses_payload(
    state: &SqliteDbState,
    timeout_seconds: u64,
    results: &mut Vec<GeneratedImageResult>,
    payload: &serde_json::Value,
    fallback_mime_type: &str,
    request_url: &str,
    channel_name: &str,
    mode: &str,
) -> Result<(), String> {
    let mut payload_stack: Vec<&serde_json::Value> = vec![payload];

    while let Some(current_payload) = payload_stack.pop() {
        let mut item_stack: Vec<&serde_json::Value> = Vec::new();

        if let Some(data_items) = current_payload
            .get("data")
            .and_then(|value| value.as_array())
        {
            for item in data_items.iter().rev() {
                item_stack.push(item);
            }
        }

        if let Some(output_items) = current_payload
            .get("output")
            .and_then(|value| value.as_array())
        {
            for item in output_items.iter().rev() {
                item_stack.push(item);
            }
        }

        if let Some(item) = current_payload.get("item") {
            item_stack.push(item);
        }

        if let Some(response_payload) = current_payload.get("response") {
            payload_stack.push(response_payload);
        }

        while let Some(item) = item_stack.pop() {
            append_image_result_from_response_item(
                state,
                timeout_seconds,
                results,
                item,
                fallback_mime_type,
                request_url,
                channel_name,
                mode,
            )
            .await?;

            if let Some(content_items) = item.get("content").and_then(|value| value.as_array()) {
                for content_item in content_items.iter().rev() {
                    item_stack.push(content_item);
                }
            }
        }
    }

    Ok(())
}

fn parse_json_value(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn extract_error_message_from_payload(payload: &serde_json::Value) -> Option<String> {
    let direct_message = payload.get("message").and_then(|value| value.as_str());
    if let Some(message) = direct_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(message.to_string());
    }

    let direct_detail = payload.get("detail");
    if let Some(detail_text) = direct_detail.and_then(|value| value.as_str()) {
        let trimmed = detail_text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(detail_items) = direct_detail.and_then(|value| value.as_array()) {
        let detail_message = detail_items
            .iter()
            .filter_map(|item| {
                if let Some(detail_text) = item.as_str() {
                    return Some(detail_text.trim().to_string());
                }
                item.get("msg")
                    .and_then(|value| value.as_str())
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("；");
        if !detail_message.is_empty() {
            return Some(detail_message);
        }
    }

    if let Some(nested_message) = payload
        .get("error")
        .and_then(|value| value.get("message"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(nested_message.to_string());
    }

    None
}

fn is_payload_too_large_error(status: reqwest::StatusCode, body_text: &str) -> bool {
    status == reqwest::StatusCode::PAYLOAD_TOO_LARGE
        || body_text.contains("payload too large")
        || body_text.contains("request entity too large")
        || body_text.contains("content too large")
        || body_text.contains("body too large")
        || body_text.contains("HTTP 413")
}

fn should_retry_responses_with_compatibility(message: &str) -> bool {
    let normalized_message = message.to_lowercase();
    if is_non_retryable_responses_request_error(&normalized_message) {
        return false;
    }

    normalized_message.contains("tool_choice")
        || normalized_message.contains("image_generation")
        || normalized_message.contains("response")
        || normalized_message.contains("internal")
        || normalized_message.contains("server error")
        || normalized_message.contains("input must be a list")
        || normalized_message.contains("input") && normalized_message.contains("array")
        || normalized_message.contains("expected")
            && (normalized_message.contains("list") || normalized_message.contains("array"))
        || normalized_message.contains("multipart")
        || normalized_message.contains("stream")
        || normalized_message.contains("sse")
        || normalized_message.contains("file_id")
}

fn is_non_retryable_responses_request_error(normalized_message: &str) -> bool {
    normalized_message.contains("unknown parameter")
        || normalized_message.contains("unsupported parameter")
        || normalized_message.contains("unrecognized parameter")
        || normalized_message.contains("unrecognized request argument")
        || normalized_message.contains("invalid parameter")
}

fn should_fallback_responses_stream_to_json(
    message: &str,
    current_plan: ResponsesRequestPlan,
    next_plan: Option<ResponsesRequestPlan>,
) -> bool {
    if current_plan.transport != ResponsesTransportKind::Stream {
        return false;
    }
    let Some(next_plan) = next_plan else {
        return false;
    };
    if next_plan.transport != ResponsesTransportKind::Json {
        return false;
    }

    let normalized_message = message.to_lowercase();
    if is_non_retryable_responses_request_error(&normalized_message) {
        return false;
    }

    !(normalized_message.contains("auth_not_found")
        || normalized_message.contains("no auth available")
        || normalized_message.contains("invalid api key")
        || normalized_message.contains("insufficient")
        || normalized_message.contains("quota"))
}

fn parse_sse_events(text: &str) -> Vec<(String, String, Option<serde_json::Value>)> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut data_lines: Vec<String> = Vec::new();

    let flush = |events: &mut Vec<(String, String, Option<serde_json::Value>)>,
                 current_event: &mut String,
                 data_lines: &mut Vec<String>| {
        if current_event.is_empty() && data_lines.is_empty() {
            return;
        }

        let data_text = data_lines.join("\n");
        let json_payload = parse_json_value(&data_text);
        events.push((current_event.clone(), data_text, json_payload));
        current_event.clear();
        data_lines.clear();
    };

    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut events, &mut current_event, &mut data_lines);
            continue;
        }

        if let Some(event_name) = line.strip_prefix("event:") {
            current_event = event_name.trim().to_string();
            continue;
        }

        if let Some(data_line) = line.strip_prefix("data:") {
            data_lines.push(data_line.trim_start().to_string());
        }
    }

    flush(&mut events, &mut current_event, &mut data_lines);
    events
}

fn read_responses_payload_from_text(
    text: &str,
    status: reqwest::StatusCode,
) -> Result<serde_json::Value, String> {
    if let Some(direct_json) = parse_json_value(text) {
        return Ok(direct_json);
    }

    let sse_events = parse_sse_events(text);
    if sse_events.is_empty() {
        return Err(format!(
            "Responses API returned non-JSON payload and it was not parseable SSE: HTTP {} body_preview={}",
            status,
            truncate_for_log(text, 240)
        ));
    }

    let json_payloads = sse_events
        .iter()
        .filter_map(|(_, _, payload)| payload.as_ref())
        .collect::<Vec<_>>();

    if let Some(failed_payload) = json_payloads.iter().rev().find(|payload| {
        payload
            .get("type")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == "response.failed")
            || payload
                .get("response")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str())
                .is_some_and(|value| value == "failed")
    }) {
        let nested_response = failed_payload.get("response");
        let message = extract_error_message_from_payload(failed_payload)
            .or_else(|| nested_response.and_then(extract_error_message_from_payload))
            .unwrap_or_else(|| "Responses API processing failed".to_string());
        return Err(message);
    }

    let output_items = json_payloads
        .iter()
        .filter(|payload| {
            payload
                .get("type")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value == "response.output_item.done")
                && payload.get("item").is_some()
        })
        .filter_map(|payload| payload.get("item").cloned())
        .collect::<Vec<_>>();

    if let Some(completed_payload) = json_payloads.iter().rev().find(|payload| {
        payload
            .get("type")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == "response.completed")
            && payload.get("response").is_some()
    }) {
        if let Some(completed_response) = completed_payload.get("response") {
            let mut completed_response = completed_response.clone();
            let has_existing_output = completed_response
                .get("output")
                .and_then(|value| value.as_array())
                .map(|value| !value.is_empty())
                .unwrap_or(false);
            if !has_existing_output && !output_items.is_empty() {
                if let Some(object) = completed_response.as_object_mut() {
                    object.insert("output".to_string(), serde_json::Value::Array(output_items));
                }
            }
            return Ok(completed_response);
        }
    }

    if !output_items.is_empty() {
        return Ok(json!({ "output": output_items }));
    }

    if let Some(last_json_payload) = json_payloads.last() {
        return Ok((*last_json_payload).clone());
    }

    Err(format!(
        "Responses API returned SSE events without parseable JSON payload: HTTP {} body_preview={}",
        status,
        truncate_for_log(text, 240)
    ))
}

async fn upload_responses_input_image_as_file_id(
    state: &SqliteDbState,
    channel: &ImageChannelDto,
    timeout_seconds: u64,
    reference: &ImageReferenceInput,
    index: usize,
) -> Result<String, String> {
    let client = http_client::client_with_timeout_no_compression(state, timeout_seconds).await?;
    let authorization = format!("Bearer {}", channel.api_key.trim());
    let file_request_url = build_image_api_url(&channel.base_url, "files");
    let bytes = decode_base64_bytes(&reference.base64_data)?;
    let part = Part::bytes(bytes)
        .file_name(format!(
            "input-{}.{}",
            index + 1,
            file_extension_for_mime(&reference.mime_type)
        ))
        .mime_str(&reference.mime_type)
        .map_err(|e| format!("Invalid image mime type: {}", e))?;
    let form = Form::new().text("purpose", "vision").part("file", part);

    let response = client
        .post(&file_request_url)
        .header("Authorization", &authorization)
        .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
        .multipart(form)
        .send()
        .await
        .map_err(|error| {
            format!(
                "Responses image file upload failed: channel={} url={} timeout={}s error={}",
                channel.name,
                file_request_url,
                timeout_seconds,
                format_reqwest_error(&error)
            )
        })?;

    let status = response.status();
    let response_text = response.text().await.map_err(|error| {
        format!(
            "Failed to read Responses file upload response: channel={} url={} status={} error={}",
            channel.name,
            file_request_url,
            status,
            format_reqwest_error(&error)
        )
    })?;

    if !status.is_success() {
        return Err(format!(
            "Responses file upload failed: channel={} url={} HTTP {} {}",
            channel.name, file_request_url, status, response_text
        ));
    }

    let payload = parse_json_value(&response_text).ok_or_else(|| {
        format!(
            "Responses file upload returned non-JSON payload: channel={} url={} body_preview={}",
            channel.name,
            file_request_url,
            truncate_for_log(&response_text, 240)
        )
    })?;
    let file_id = payload
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "Responses file upload succeeded but no file id was returned: channel={} url={}",
                channel.name, file_request_url
            )
        })?;

    Ok(file_id.to_string())
}

async fn delete_uploaded_responses_file(
    state: &SqliteDbState,
    channel: &ImageChannelDto,
    timeout_seconds: u64,
    file_id: &str,
) {
    let Ok(client) = http_client::client_with_timeout_no_compression(state, timeout_seconds).await
    else {
        return;
    };
    let authorization = format!("Bearer {}", channel.api_key.trim());
    let file_request_url = build_image_api_url(&channel.base_url, &format!("files/{}", file_id));

    let _ = client
        .delete(&file_request_url)
        .header("Authorization", &authorization)
        .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
        .send()
        .await;
}

async fn prepare_responses_reference_inputs(
    state: &SqliteDbState,
    channel: &ImageChannelDto,
    timeout_seconds: u64,
    input: &CreateImageJobInput,
    reference_input_mode: ResponsesReferenceInputMode,
    include_reference_data: bool,
) -> Result<PreparedResponsesReferenceInputs, String> {
    let mut input_images = Vec::with_capacity(input.references.len());
    let mut uploaded_file_ids = Vec::new();

    for (index, reference) in input.references.iter().enumerate() {
        match reference_input_mode {
            ResponsesReferenceInputMode::InlineDataUrl => {
                let image_url = if include_reference_data {
                    normalize_reference_data_url(reference)
                } else {
                    "***".to_string()
                };
                input_images.push(json!({
                    "type": "input_image",
                    "image_url": image_url,
                }));
            }
            ResponsesReferenceInputMode::FileId => {
                let file_id = upload_responses_input_image_as_file_id(
                    state,
                    channel,
                    timeout_seconds,
                    reference,
                    index,
                )
                .await?;
                uploaded_file_ids.push(file_id.clone());
                input_images.push(json!({
                    "type": "input_image",
                    "file_id": file_id,
                }));
            }
        }
    }

    Ok(PreparedResponsesReferenceInputs {
        input_images,
        uploaded_file_ids,
    })
}

async fn execute_responses_generation_request(
    state: &SqliteDbState,
    channel: &ImageChannelDto,
    input: &CreateImageJobInput,
    request_url: &str,
    progress_context: &ImageJobProgressContext<'_>,
) -> Result<Vec<GeneratedImageResult>, String> {
    let timeout_seconds = resolve_channel_timeout_seconds(channel);
    let client = http_client::client_with_timeout_no_compression(state, timeout_seconds).await?;
    let authorization = format!("Bearer {}", channel.api_key.trim());
    let output_format = input.params.output_format.trim().to_lowercase();
    let fallback_mime_type = mime_from_output_format(&output_format).to_string();
    let request_plans = build_responses_request_plans(input);
    let mut reference_input_mode = ResponsesReferenceInputMode::InlineDataUrl;
    let mut last_error_message = None;

    'reference_mode: loop {
        let prepared_inputs = prepare_responses_reference_inputs(
            state,
            channel,
            timeout_seconds,
            input,
            reference_input_mode,
            true,
        )
        .await;

        let PreparedResponsesReferenceInputs {
            input_images,
            uploaded_file_ids,
        } = match prepared_inputs {
            Ok(value) => value,
            Err(error) => {
                if reference_input_mode == ResponsesReferenceInputMode::FileId {
                    return Err(format!(
                        "Responses provider rejected both inline image input and /v1/files fallback: {}",
                        error
                    ));
                }
                return Err(error);
            }
        };

        let mut should_retry_with_file_id = false;
        let mut response_result: Option<Vec<GeneratedImageResult>> = None;

        for (plan_index, plan) in request_plans.iter().copied().enumerate() {
            let request_body = build_responses_request_body(input, &input_images, plan);
            let mut last_plan_error: Option<String> = None;
            let reference_input_mode_label =
                responses_reference_input_mode_label(reference_input_mode);

            for attempt in 1..=IMAGE_REQUEST_MAX_ATTEMPTS {
                let request_started_at = Instant::now();
                emit_image_job_progress(
                    progress_context,
                    "request_start",
                    attempt,
                    None,
                    Some(plan.id),
                    Some(reference_input_mode_label),
                    None,
                );
                debug!(
                    "Responses image request start: mode={} channel={} model={} url={} timeout={}s reference_count={} attempt={}/{} plan={} reference_input_mode={}",
                    input.mode,
                    channel.name,
                    input.model_id,
                    request_url,
                    timeout_seconds,
                    input.references.len(),
                    attempt,
                    IMAGE_REQUEST_MAX_ATTEMPTS,
                    plan.id,
                    reference_input_mode_label
                );

                let response = match client
                    .post(request_url)
                    .header("Authorization", &authorization)
                    .header("Content-Type", "application/json")
                    .header("Accept-Encoding", IMAGE_REQUEST_ACCEPT_ENCODING)
                    .json(&request_body)
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error)
                        if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
                            && should_retry_image_request_error(&error) =>
                    {
                        let delay_ms = image_request_retry_delay_ms(attempt);
                        emit_image_job_progress(
                            progress_context,
                            "retry_scheduled",
                            attempt,
                            Some(delay_ms),
                            Some(plan.id),
                            Some(reference_input_mode_label),
                            Some(format_reqwest_error(&error)),
                        );
                        warn!(
                            "Responses image request retry scheduled after transport error: mode={} channel={} model={} url={} plan={} attempt={}/{} delay_ms={} error={}",
                            input.mode,
                            channel.name,
                            input.model_id,
                            request_url,
                            plan.id,
                            attempt,
                            IMAGE_REQUEST_MAX_ATTEMPTS,
                            delay_ms,
                            format_reqwest_error(&error)
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    Err(error) => {
                        let message = format!(
                            "Responses image request failed: mode={} channel={} model={} url={} timeout={}s plan={} error={}",
                            input.mode,
                            channel.name,
                            input.model_id,
                            request_url,
                            timeout_seconds,
                            plan.id,
                            format_reqwest_error(&error)
                        );
                        error!("{}", message);
                        last_plan_error = Some(message);
                        break;
                    }
                };

                debug!(
                    "Responses image request headers received: mode={} channel={} model={} url={} elapsed_ms={} status={} headers={} attempt={}/{} plan={}",
                    input.mode,
                    channel.name,
                    input.model_id,
                    request_url,
                    request_started_at.elapsed().as_millis(),
                    response.status(),
                    summarize_response_headers(response.headers()),
                    attempt,
                    IMAGE_REQUEST_MAX_ATTEMPTS,
                    plan.id
                );

                if attempt < IMAGE_REQUEST_MAX_ATTEMPTS
                    && should_retry_image_response_status(response.status())
                {
                    let retry_status = response.status();
                    let retry_body = match response.text().await {
                        Ok(body) => truncate_for_log(&body.replace(['\r', '\n'], " "), 240),
                        Err(error) => format!("<failed to read retry body: {}>", error),
                    };
                    let delay_ms = image_request_retry_delay_ms(attempt);
                    emit_image_job_progress(
                        progress_context,
                        "retry_scheduled",
                        attempt,
                        Some(delay_ms),
                        Some(plan.id),
                        Some(reference_input_mode_label),
                        Some(format!("HTTP {retry_status} {retry_body}")),
                    );
                    warn!(
                        "Responses image request retry scheduled after upstream status: mode={} channel={} model={} url={} plan={} attempt={}/{} delay_ms={} status={} body_preview={}",
                        input.mode,
                        channel.name,
                        input.model_id,
                        request_url,
                        plan.id,
                        attempt,
                        IMAGE_REQUEST_MAX_ATTEMPTS,
                        delay_ms,
                        retry_status,
                        retry_body
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    continue;
                }

                match parse_responses_image_response(
                    state,
                    timeout_seconds,
                    response,
                    &fallback_mime_type,
                    request_url,
                    &channel.name,
                    &input.mode,
                    request_started_at,
                )
                .await
                {
                    Ok(results) => {
                        response_result = Some(results);
                        break;
                    }
                    Err(error_message) => {
                        let next_plan = request_plans.get(plan_index + 1).copied();
                        if reference_input_mode == ResponsesReferenceInputMode::InlineDataUrl
                            && !input.references.is_empty()
                            && is_payload_too_large_error(reqwest::StatusCode::OK, &error_message)
                        {
                            warn!(
                                "Responses image request switching to /v1/files fallback after payload-too-large error: mode={} channel={} model={} url={} plan={}",
                                input.mode, channel.name, input.model_id, request_url, plan.id
                            );
                            emit_image_job_progress(
                                progress_context,
                                "fallback_file_id",
                                attempt,
                                None,
                                Some(plan.id),
                                Some(reference_input_mode_label),
                                Some("Payload too large; switching to file_id input".to_string()),
                            );
                            should_retry_with_file_id = true;
                            last_plan_error = Some(error_message);
                            break;
                        }

                        if should_fallback_responses_stream_to_json(&error_message, plan, next_plan)
                            || should_retry_responses_with_compatibility(&error_message)
                        {
                            last_plan_error = Some(error_message);
                            break;
                        }

                        for file_id in &uploaded_file_ids {
                            delete_uploaded_responses_file(
                                state,
                                channel,
                                timeout_seconds,
                                file_id,
                            )
                            .await;
                        }
                        return Err(error_message);
                    }
                }
            }

            if let Some(results) = response_result {
                for file_id in &uploaded_file_ids {
                    delete_uploaded_responses_file(state, channel, timeout_seconds, file_id).await;
                }
                return Ok(results);
            }

            if should_retry_with_file_id {
                for file_id in &uploaded_file_ids {
                    delete_uploaded_responses_file(state, channel, timeout_seconds, file_id).await;
                }
                reference_input_mode = ResponsesReferenceInputMode::FileId;
                continue 'reference_mode;
            }

            if let Some(error_message) = last_plan_error {
                last_error_message = Some(error_message);
                continue;
            }
        }

        for file_id in &uploaded_file_ids {
            delete_uploaded_responses_file(state, channel, timeout_seconds, file_id).await;
        }

        if reference_input_mode == ResponsesReferenceInputMode::InlineDataUrl
            && should_retry_with_file_id
        {
            reference_input_mode = ResponsesReferenceInputMode::FileId;
            continue;
        }

        break;
    }

    Err(last_error_message.unwrap_or_else(|| {
        "Responses image request exhausted compatibility plans without a usable image".to_string()
    }))
}

async fn parse_responses_image_response(
    state: &SqliteDbState,
    timeout_seconds: u64,
    response: reqwest::Response,
    fallback_mime_type: &str,
    request_url: &str,
    channel_name: &str,
    mode: &str,
    request_started_at: Instant,
) -> Result<Vec<GeneratedImageResult>, String> {
    let status = response.status();
    let response_headers = summarize_response_headers(response.headers());
    let body_read_started_at = Instant::now();
    let response_bytes = response.bytes().await.map_err(|e| {
        let message = format!(
            "Failed to read Responses image response body: mode={} channel={} url={} status={} elapsed_ms={} body_read_ms={} error={}",
            mode,
            channel_name,
            request_url,
            status,
            request_started_at.elapsed().as_millis(),
            body_read_started_at.elapsed().as_millis(),
            format_reqwest_error(&e)
        );
        error!("{}", message);
        message
    })?;

    debug!(
        "Responses image response body read: mode={} channel={} url={} status={} elapsed_ms={} body_read_ms={} bytes={} headers={}",
        mode,
        channel_name,
        request_url,
        status,
        request_started_at.elapsed().as_millis(),
        body_read_started_at.elapsed().as_millis(),
        response_bytes.len(),
        response_headers
    );

    if !status.is_success() {
        let body = String::from_utf8_lossy(&response_bytes).to_string();
        let payload_message = parse_json_value(&body)
            .as_ref()
            .and_then(extract_error_message_from_payload);
        let response_message = payload_message.unwrap_or_else(|| truncate_for_log(&body, 240));
        let message = format!(
            "Responses image API failed: mode={mode} channel={channel_name} url={request_url} HTTP {status} {response_message}"
        );
        error!("{}", message);
        return Err(message);
    }

    let json_parse_started_at = Instant::now();
    let response_text = String::from_utf8_lossy(&response_bytes).to_string();
    let payload = read_responses_payload_from_text(&response_text, status).map_err(|error_message| {
        let message = format!(
            "Failed to parse Responses image response: mode={} channel={} url={} elapsed_ms={} json_parse_ms={} bytes={} error={}",
            mode,
            channel_name,
            request_url,
            request_started_at.elapsed().as_millis(),
            json_parse_started_at.elapsed().as_millis(),
            response_bytes.len(),
            error_message
        );
        error!("{}", message);
        message
    })?;

    debug!(
        "Responses image response json parsed: mode={} channel={} url={} elapsed_ms={} json_parse_ms={}",
        mode,
        channel_name,
        request_url,
        request_started_at.elapsed().as_millis(),
        json_parse_started_at.elapsed().as_millis()
    );

    let mut results = Vec::new();
    append_images_from_responses_payload(
        state,
        timeout_seconds,
        &mut results,
        &payload,
        fallback_mime_type,
        request_url,
        channel_name,
        mode,
    )
    .await?;

    if results.is_empty() {
        let message = format!(
            "Responses image API returned no usable image payload: mode={} channel={} url={}",
            mode, channel_name, request_url
        );
        error!("{}", message);
        return Err(message);
    }

    debug!(
        "Responses image response processed: mode={} channel={} url={} elapsed_ms={} result_count={}",
        mode,
        channel_name,
        request_url,
        request_started_at.elapsed().as_millis(),
        results.len()
    );

    Ok(results)
}

async fn to_job_dto(
    app: &AppHandle,
    state: &SqliteDbState,
    record: ImageJobRecord,
) -> Result<ImageJobDto, String> {
    let input_assets = store::list_image_assets_by_ids(state, &record.input_asset_ids).await?;
    let output_assets = store::list_image_assets_by_ids(state, &record.output_asset_ids).await?;

    Ok(ImageJobDto {
        id: record.id,
        mode: record.mode,
        prompt: record.prompt,
        channel_id: record.channel_id,
        channel_name_snapshot: record.channel_name_snapshot,
        provider_kind_snapshot: record.provider_kind_snapshot,
        model_id: record.model_id,
        model_name_snapshot: record.model_name_snapshot,
        params_json: record.params_json,
        status: record.status,
        error_message: record.error_message,
        request_url: record.request_url,
        request_headers_json: record.request_headers_json,
        request_body_json: record.request_body_json,
        response_metadata_json: record.response_metadata_json,
        input_assets: input_assets
            .iter()
            .map(|asset| to_asset_dto(app, asset))
            .collect::<Result<Vec<_>, _>>()?,
        output_assets: output_assets
            .iter()
            .map(|asset| to_asset_dto(app, asset))
            .collect::<Result<Vec<_>, _>>()?,
        created_at: record.created_at,
        finished_at: record.finished_at,
        elapsed_ms: record.elapsed_ms,
    })
}

async fn mark_job_as_error(
    state: &SqliteDbState,
    job_record: &mut ImageJobRecord,
    created_at: i64,
    error_message: String,
) -> Result<(), String> {
    job_record.status = ImageJobStatus::Error.as_str().to_string();
    job_record.error_message = Some(error_message);
    job_record.finished_at = Some(now_ms());
    job_record.elapsed_ms = job_record
        .finished_at
        .map(|finished_at| finished_at - created_at);
    store::update_image_job(state, job_record).await
}

#[tauri::command]
pub async fn image_get_workspace(
    app: AppHandle,
    state: State<'_, SqliteDbState>,
) -> Result<ImageWorkspaceDto, String> {
    let started_at = Instant::now();
    debug!("Image workspace load start");
    let channels = store::list_image_channels(&state, DEFAULT_CHANNEL_LIST_LIMIT).await?;
    let jobs = store::list_image_jobs(&state, 20).await?;
    let mut job_dtos = Vec::with_capacity(jobs.len());
    for job in jobs {
        match to_job_dto(&app, &state, job.clone()).await {
            Ok(job_dto) => job_dtos.push(job_dto),
            Err(error) => {
                error!("Image workspace skipped invalid job dto: {}", error);
            }
        }
    }

    let mut channel_dtos = Vec::with_capacity(channels.len());
    for channel in channels {
        match channel_to_dto(channel) {
            Ok(channel_dto) => channel_dtos.push(channel_dto),
            Err(error) => {
                error!("Image workspace skipped invalid channel dto: {}", error);
            }
        }
    }

    Ok(ImageWorkspaceDto {
        channels: channel_dtos,
        jobs: job_dtos,
    })
    .map(|workspace| {
        debug!(
            "Image workspace load complete: channels={} jobs={} elapsed_ms={}",
            workspace.channels.len(),
            workspace.jobs.len(),
            started_at.elapsed().as_millis()
        );
        workspace
    })
}

#[tauri::command]
pub async fn image_list_channels(
    state: State<'_, SqliteDbState>,
    input: Option<ListImageChannelsInput>,
) -> Result<Vec<ImageChannelDto>, String> {
    let limit = input
        .map(|value| value.limit)
        .unwrap_or(DEFAULT_CHANNEL_LIST_LIMIT);
    let channels = store::list_image_channels(&state, limit).await?;
    let mut channel_dtos = Vec::with_capacity(channels.len());
    for channel in channels {
        match channel_to_dto(channel) {
            Ok(channel_dto) => channel_dtos.push(channel_dto),
            Err(error) => {
                error!("Image channels skipped invalid channel dto: {}", error);
            }
        }
    }
    Ok(channel_dtos)
}

#[tauri::command]
pub async fn image_update_channel(
    state: State<'_, SqliteDbState>,
    input: UpsertImageChannelInput,
) -> Result<ImageChannelDto, String> {
    validate_channel_input(&input)?;

    let now = now_ms();
    let normalized_models = normalize_channel_models(&input.models);
    let models_json = serialize_channel_models(&normalized_models)?;

    let next_record =
        if let Some(channel_id) = input.id.clone().filter(|value| !value.trim().is_empty()) {
            let clean_channel_id = db_clean_id(&channel_id);
            let existing_channel = store::get_image_channel_by_id(&state, &clean_channel_id)
                .await?
                .ok_or_else(|| format!("Image channel not found: {}", clean_channel_id))?;

            ImageChannelRecord {
                id: existing_channel.id,
                name: input.name.trim().to_string(),
                provider_kind: input.provider_kind.trim().to_string(),
                base_url: input.base_url.trim().to_string(),
                api_key: input.api_key.trim().to_string(),
                generation_path: sanitize_channel_path(input.generation_path),
                edit_path: sanitize_channel_path(input.edit_path),
                timeout_seconds: input.timeout_seconds.map(|value| value.max(1)),
                enabled: input.enabled,
                sort_order: existing_channel.sort_order,
                models_json,
                created_at: existing_channel.created_at,
                updated_at: now,
            }
        } else {
            let next_sort_order = store::get_max_image_channel_sort_order(&state).await? + 1;
            ImageChannelRecord {
                id: crate::coding::db_new_id(),
                name: input.name.trim().to_string(),
                provider_kind: input.provider_kind.trim().to_string(),
                base_url: input.base_url.trim().to_string(),
                api_key: input.api_key.trim().to_string(),
                generation_path: sanitize_channel_path(input.generation_path),
                edit_path: sanitize_channel_path(input.edit_path),
                timeout_seconds: input.timeout_seconds.map(|value| value.max(1)),
                enabled: input.enabled,
                sort_order: next_sort_order,
                models_json,
                created_at: now,
                updated_at: now,
            }
        };

    let saved_record = store::upsert_image_channel(&state, &next_record).await?;
    channel_to_dto(saved_record)
}

#[tauri::command]
pub async fn image_delete_channel(
    state: State<'_, SqliteDbState>,
    input: DeleteImageChannelInput,
) -> Result<(), String> {
    let clean_channel_id = db_clean_id(&input.id);
    store::delete_image_channel(&state, &clean_channel_id).await
}

#[tauri::command]
pub async fn image_delete_job(
    app: AppHandle,
    state: State<'_, SqliteDbState>,
    input: DeleteImageJobInput,
) -> Result<(), String> {
    let clean_job_id = db_clean_id(&input.id);
    let job = store::get_image_job_by_id(&state, &clean_job_id)
        .await?
        .ok_or_else(|| format!("Image job not found: {}", clean_job_id))?;

    let mut related_asset_ids = job.input_asset_ids.clone();
    related_asset_ids.extend(job.output_asset_ids.clone());
    let related_assets = store::list_image_assets_by_ids(&state, &related_asset_ids).await?;

    if input.delete_local_assets {
        remove_asset_files(&app, &related_assets)?;
    }

    store::delete_image_assets_by_ids(&state, &related_asset_ids).await?;
    store::delete_image_job(&state, &clean_job_id).await
}

#[tauri::command]
pub async fn image_reorder_channels(
    state: State<'_, SqliteDbState>,
    input: ReorderImageChannelsInput,
) -> Result<Vec<ImageChannelDto>, String> {
    let ordered_ids = input
        .ordered_ids
        .into_iter()
        .map(|channel_id| db_clean_id(&channel_id))
        .collect::<Vec<_>>();
    let reordered = store::update_image_channel_sort_orders(&state, &ordered_ids).await?;
    reordered
        .into_iter()
        .map(channel_to_dto)
        .collect::<Result<Vec<_>, _>>()
}

#[tauri::command]
pub async fn image_list_jobs(
    app: AppHandle,
    state: State<'_, SqliteDbState>,
    input: Option<ListImageJobsInput>,
) -> Result<Vec<ImageJobDto>, String> {
    let started_at = Instant::now();
    let limit = input.and_then(|value| value.limit).unwrap_or(50);
    debug!("Image list jobs start: limit={}", limit);
    let jobs = store::list_image_jobs(&state, limit).await?;
    let mut job_dtos = Vec::with_capacity(jobs.len());
    for job in jobs {
        match to_job_dto(&app, &state, job.clone()).await {
            Ok(job_dto) => job_dtos.push(job_dto),
            Err(error) => {
                error!("Image jobs skipped invalid job dto: {}", error);
            }
        }
    }
    debug!(
        "Image list jobs complete: limit={} jobs={} elapsed_ms={}",
        limit,
        job_dtos.len(),
        started_at.elapsed().as_millis()
    );
    Ok(job_dtos)
}

#[tauri::command]
pub async fn image_create_job(
    app: AppHandle,
    state: State<'_, SqliteDbState>,
    input: CreateImageJobInput,
) -> Result<ImageJobDto, String> {
    let command_started_at = Instant::now();
    let prompt = input.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("Prompt is required".to_string());
    }

    let mode = input.mode.trim().to_string();
    if mode != ImageJobMode::TextToImage.as_str() && mode != ImageJobMode::ImageToImage.as_str() {
        return Err(format!("Unsupported image mode: {}", input.mode));
    }

    if mode == ImageJobMode::ImageToImage.as_str() && input.references.is_empty() {
        return Err("At least one reference image is required for image-to-image".to_string());
    }

    let clean_channel_id = db_clean_id(input.channel_id.trim());
    let channel = store::get_image_channel_by_id(&state, &clean_channel_id)
        .await?
        .ok_or_else(|| format!("Image channel not found: {}", clean_channel_id))?;
    let channel_dto = channel_to_dto(channel)?;

    debug!(
        "Image job command start: mode={} channel={} model={} prompt_len={} reference_count={} elapsed_ms={}",
        mode,
        channel_dto.name,
        input.model_id.trim(),
        prompt.chars().count(),
        input.references.len(),
        command_started_at.elapsed().as_millis()
    );

    if channel_dto.api_key.trim().is_empty() {
        return Err(format!(
            "Image channel API key is not configured: {}",
            channel_dto.name
        ));
    }

    let model = find_channel_model(&channel_dto, input.model_id.trim()).ok_or_else(|| {
        format!(
            "Image model not found on channel {}: {}",
            channel_dto.name, input.model_id
        )
    })?;
    validate_channel_model_support(&channel_dto, model, &mode)?;
    let request_snapshot = build_request_snapshot(&channel_dto, &input)?;

    let created_at = now_ms();
    let job_id = crate::coding::db_new_id();
    let reference_assets =
        persist_reference_assets(&app, &state, &job_id, &input.references).await?;
    debug!(
        "Image job references persisted: job_id={} count={} elapsed_ms={}",
        job_id,
        reference_assets.len(),
        command_started_at.elapsed().as_millis()
    );
    let mut job_record = ImageJobRecord {
        id: job_id,
        mode,
        prompt,
        channel_id: channel_dto.id.clone(),
        channel_name_snapshot: channel_dto.name.clone(),
        provider_kind_snapshot: Some(channel_dto.provider_kind.clone()),
        model_id: model.id.clone(),
        model_name_snapshot: model
            .name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| model.id.clone()),
        params_json: serde_json::to_string(&input.params).map_err(|e| e.to_string())?,
        status: ImageJobStatus::Running.as_str().to_string(),
        error_message: None,
        request_url: Some(request_snapshot.request_url.clone()),
        request_headers_json: Some(request_snapshot.request_headers_json.clone()),
        request_body_json: Some(request_snapshot.request_body_json.clone()),
        response_metadata_json: None,
        input_asset_ids: reference_assets
            .iter()
            .map(|asset| asset.id.clone())
            .collect(),
        output_asset_ids: Vec::new(),
        created_at,
        finished_at: None,
        elapsed_ms: None,
    };

    let created_job_id = store::create_image_job(&state, &job_record).await?;
    job_record.id = created_job_id;
    debug!(
        "Image job db record created: job_id={} elapsed_ms={}",
        job_record.id,
        command_started_at.elapsed().as_millis()
    );

    match execute_generation_request(
        Some(&app),
        &state,
        &job_record.id,
        &channel_dto,
        &input,
        &request_snapshot.request_url,
    )
    .await
    {
        Ok(result_images) => {
            debug!(
                "Image generation finished, persisting outputs: job_id={} output_count={} elapsed_ms={}",
                job_record.id,
                result_images.len(),
                command_started_at.elapsed().as_millis()
            );
            let persist_result: Result<(), String> = async {
                let mut output_asset_ids = Vec::with_capacity(result_images.len());
                let mut response_metadata_items = Vec::new();
                for (index, result_image) in result_images.into_iter().enumerate() {
                    let file_name = format!(
                        "result-{}.{}",
                        index + 1,
                        file_extension_for_mime(&result_image.mime_type)
                    );
                    debug!(
                        "Image output persist start: job_id={} index={} bytes={} mime_type={} elapsed_ms={}",
                        job_record.id,
                        index + 1,
                        result_image.bytes.len(),
                        result_image.mime_type,
                        command_started_at.elapsed().as_millis()
                    );
                    let asset = persist_asset_file(
                        &app,
                        &state,
                        Some(job_record.id.clone()),
                        "output",
                        &file_name,
                        &result_image.mime_type,
                        &result_image.bytes,
                    )
                    .await?;
                    let output_asset_id = asset.id.clone();
                    if let Some(response_metadata) = result_image.response_metadata {
                        response_metadata_items.push(json!({
                            "index": index + 1,
                            "asset_id": output_asset_id,
                            "metadata": response_metadata,
                        }));
                    }
                    output_asset_ids.push(asset.id);
                }

                job_record.output_asset_ids = output_asset_ids;
                job_record.response_metadata_json = if response_metadata_items.is_empty() {
                    None
                } else {
                    Some(serialize_json_pretty(
                        &json!({ "outputs": response_metadata_items }),
                        "image response metadata",
                    )?)
                };
                job_record.status = ImageJobStatus::Done.as_str().to_string();
                job_record.error_message = None;
                job_record.finished_at = Some(now_ms());
                job_record.elapsed_ms = job_record.finished_at.map(|finished_at| finished_at - created_at);
                store::update_image_job(&state, &job_record).await?;
                Ok(())
            }
            .await;

            match persist_result {
                Ok(()) => {
                    debug!(
                        "Image job db record marked done: job_id={} output_assets={} elapsed_ms={}",
                        job_record.id,
                        job_record.output_asset_ids.len(),
                        command_started_at.elapsed().as_millis()
                    );
                }
                Err(error_message) => {
                    error!(
                        "Image job output persistence failed: id={} mode={} channel={} model={} error={}",
                        job_record.id,
                        job_record.mode,
                        job_record.channel_name_snapshot,
                        job_record.model_name_snapshot,
                        error_message
                    );
                    mark_job_as_error(&state, &mut job_record, created_at, error_message.clone())
                        .await
                        .map_err(|update_error| {
                            format!(
                                "Failed to mark image job as error after output persistence failure: job_id={} original_error={} update_error={}",
                                job_record.id,
                                error_message,
                                update_error
                            )
                        })?;
                    debug!(
                        "Image job db record marked error after output persistence failure: job_id={} elapsed_ms={}",
                        job_record.id,
                        command_started_at.elapsed().as_millis()
                    );
                }
            }
        }
        Err(error_message) => {
            error!(
                "Image job failed: id={} mode={} channel={} model={} error={}",
                job_record.id,
                job_record.mode,
                job_record.channel_name_snapshot,
                job_record.model_name_snapshot,
                error_message
            );
            mark_job_as_error(&state, &mut job_record, created_at, error_message).await?;
            debug!(
                "Image job db record marked error: job_id={} elapsed_ms={}",
                job_record.id,
                command_started_at.elapsed().as_millis()
            );
        }
    }

    debug!(
        "Image job reload start: job_id={} elapsed_ms={}",
        job_record.id,
        command_started_at.elapsed().as_millis()
    );
    let saved_job = store::get_image_job_by_id(&state, &job_record.id)
        .await?
        .ok_or_else(|| "Created image job not found".to_string())?;
    debug!(
        "Image job dto build start: job_id={} elapsed_ms={}",
        job_record.id,
        command_started_at.elapsed().as_millis()
    );
    let job_dto = to_job_dto(&app, &state, saved_job).await?;
    debug!(
        "Image job command complete: job_id={} status={} output_assets={} elapsed_ms={}",
        job_dto.id,
        job_dto.status,
        job_dto.output_assets.len(),
        command_started_at.elapsed().as_millis()
    );
    Ok(job_dto)
}

#[tauri::command]
pub async fn image_reveal_assets_dir(app: AppHandle) -> Result<String, String> {
    let dir = ensure_image_assets_dir(&app)?;
    Ok(dir.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::image::types::ImageTaskParams;
    use tempfile::TempDir;

    struct TestDbState {
        _temp_dir: TempDir,
        state: SqliteDbState,
    }

    async fn create_test_db_state() -> TestDbState {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_path = temp_dir.path().join("test.db");
        let state = SqliteDbState::open(db_path).expect("open sqlite test db");

        TestDbState {
            _temp_dir: temp_dir,
            state,
        }
    }

    fn sample_channel_with_provider(
        provider_kind: &str,
        base_url: &str,
        api_key: &str,
        model_id: &str,
        generation_path: Option<&str>,
        edit_path: Option<&str>,
    ) -> ImageChannelDto {
        ImageChannelDto {
            id: "channel-live-smoke".to_string(),
            name: "Live Smoke".to_string(),
            provider_kind: provider_kind.to_string(),
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            generation_path: generation_path.map(str::to_string),
            edit_path: edit_path.map(str::to_string),
            timeout_seconds: Some(300),
            enabled: true,
            sort_order: 0,
            models: vec![ImageChannelModel {
                id: model_id.to_string(),
                name: Some(model_id.to_string()),
                supports_text_to_image: true,
                supports_image_to_image: true,
                enabled: true,
            }],
            created_at: 0,
            updated_at: 0,
        }
    }

    fn sample_channel(
        base_url: &str,
        api_key: &str,
        model_id: &str,
        generation_path: Option<&str>,
        edit_path: Option<&str>,
    ) -> ImageChannelDto {
        sample_channel_with_provider(
            PROVIDER_KIND_OPENAI_COMPATIBLE,
            base_url,
            api_key,
            model_id,
            generation_path,
            edit_path,
        )
    }

    fn sample_text_to_image_input(model_id: &str) -> CreateImageJobInput {
        CreateImageJobInput {
            mode: ImageJobMode::TextToImage.as_str().to_string(),
            prompt: "A tiny red square icon on a plain white background".to_string(),
            channel_id: "channel-live-smoke".to_string(),
            model_id: model_id.to_string(),
            params: ImageTaskParams {
                size: "auto".to_string(),
                quality: "auto".to_string(),
                output_format: "png".to_string(),
                output_compression: Some(80),
                moderation: Some("low".to_string()),
            },
            references: Vec::new(),
        }
    }

    fn sample_image_to_image_input(model_id: &str) -> CreateImageJobInput {
        CreateImageJobInput {
            mode: ImageJobMode::ImageToImage.as_str().to_string(),
            prompt: "Turn the reference into a flat monochrome icon".to_string(),
            channel_id: "channel-live-smoke".to_string(),
            model_id: model_id.to_string(),
            params: ImageTaskParams {
                size: "1024x1024".to_string(),
                quality: "high".to_string(),
                output_format: "webp".to_string(),
                output_compression: Some(65),
                moderation: Some("low".to_string()),
            },
            references: vec![
                ImageReferenceInput {
                    file_name: "alpha.png".to_string(),
                    mime_type: "image/png".to_string(),
                    base64_data: "data:image/png;base64,QUJD".to_string(),
                },
                ImageReferenceInput {
                    file_name: "beta.png".to_string(),
                    mime_type: "image/png".to_string(),
                    base64_data: "data:image/png;base64,REVG".to_string(),
                },
            ],
        }
    }

    fn require_live_env(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("missing required env var: {name}"))
    }

    #[test]
    fn build_image_result_http_error_contains_status_and_preview() {
        let message = build_image_result_http_error(
            "text_to_image",
            "Demo Channel",
            "https://gateway.example/v1/images/generations",
            "https://cdn.example/result.png",
            reqwest::StatusCode::FORBIDDEN,
            "content-type=text/html",
            b"<html>signature expired</html>",
        );

        assert!(message.contains("HTTP 403 Forbidden"));
        assert!(message.contains("content-type=text/html"));
        assert!(message.contains("signature expired"));
    }

    #[tokio::test]
    async fn mark_job_as_error_updates_status_message_and_elapsed_time() {
        let test_db_state = create_test_db_state().await;
        let created_at = now_ms().saturating_sub(25);
        let mut record = ImageJobRecord {
            id: "job-mark-error".to_string(),
            mode: ImageJobMode::TextToImage.as_str().to_string(),
            prompt: "prompt".to_string(),
            channel_id: "channel-1".to_string(),
            channel_name_snapshot: "Channel 1".to_string(),
            provider_kind_snapshot: Some(PROVIDER_KIND_OPENAI_COMPATIBLE.to_string()),
            model_id: "gpt-image-2".to_string(),
            model_name_snapshot: "gpt-image-2".to_string(),
            params_json: "{}".to_string(),
            status: ImageJobStatus::Running.as_str().to_string(),
            error_message: None,
            request_url: None,
            request_headers_json: None,
            request_body_json: None,
            response_metadata_json: None,
            input_asset_ids: Vec::new(),
            output_asset_ids: Vec::new(),
            created_at,
            finished_at: None,
            elapsed_ms: None,
        };

        store::create_image_job(&test_db_state.state, &record)
            .await
            .expect("create job record");

        mark_job_as_error(
            &test_db_state.state,
            &mut record,
            created_at,
            "persist output failed".to_string(),
        )
        .await
        .expect("mark job as error");

        let saved_job = store::get_image_job_by_id(&test_db_state.state, &record.id)
            .await
            .expect("load saved job")
            .expect("saved job exists");

        assert_eq!(saved_job.status, ImageJobStatus::Error.as_str());
        assert_eq!(
            saved_job.error_message.as_deref(),
            Some("persist output failed")
        );
        assert!(saved_job.finished_at.is_some());
        assert!(saved_job.elapsed_ms.unwrap_or_default() >= 0);
    }

    #[test]
    fn detect_dimensions_reads_png_size() {
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO7Z0mQAAAAASUVORK5CYII=")
            .expect("decode png bytes");

        let dimensions = detect_dimensions(&png_bytes);

        assert_eq!(dimensions, (Some(1), Some(1)));
    }

    #[test]
    fn image_build_request_snapshot_for_text_to_image_uses_identity_encoding() {
        let channel = sample_channel(
            "https://example.com/",
            "test-key",
            "gpt-image-2",
            Some("/custom/generations/"),
            None,
        );
        let input = sample_text_to_image_input("gpt-image-2");

        let snapshot = build_request_snapshot(&channel, &input).expect("build request snapshot");
        let request_headers: serde_json::Value =
            serde_json::from_str(&snapshot.request_headers_json)
                .expect("parse request headers json");
        let request_body: serde_json::Value =
            serde_json::from_str(&snapshot.request_body_json).expect("parse request body json");

        assert_eq!(
            snapshot.request_url,
            "https://example.com/v1/custom/generations"
        );
        assert_eq!(
            request_headers["Accept-Encoding"],
            serde_json::Value::String(IMAGE_REQUEST_ACCEPT_ENCODING.to_string())
        );
        assert_eq!(
            request_headers["Content-Type"],
            serde_json::Value::String("application/json".to_string())
        );
        assert_eq!(
            request_body["model"],
            serde_json::Value::String("gpt-image-2".to_string())
        );
        assert_eq!(
            request_body["output_format"],
            serde_json::Value::String("png".to_string())
        );
        assert_eq!(
            request_body["moderation"],
            serde_json::Value::String("low".to_string())
        );
        assert!(
            request_body.get("output_compression").is_none(),
            "png output should not include output_compression"
        );
    }

    #[test]
    fn image_build_request_snapshot_for_image_to_image_uses_multipart_shape() {
        let channel = sample_channel(
            "https://example.com",
            "test-key",
            "gpt-image-2",
            None,
            Some("custom/edits"),
        );
        let input = sample_image_to_image_input("gpt-image-2");

        let snapshot = build_request_snapshot(&channel, &input).expect("build request snapshot");
        let request_headers: serde_json::Value =
            serde_json::from_str(&snapshot.request_headers_json)
                .expect("parse request headers json");
        let request_body: serde_json::Value =
            serde_json::from_str(&snapshot.request_body_json).expect("parse request body json");

        assert_eq!(snapshot.request_url, "https://example.com/v1/custom/edits");
        assert_eq!(
            request_headers["Accept-Encoding"],
            serde_json::Value::String(IMAGE_REQUEST_ACCEPT_ENCODING.to_string())
        );
        assert_eq!(
            request_headers["Content-Type"],
            serde_json::Value::String("multipart/form-data".to_string())
        );
        assert_eq!(
            request_body["image_field"],
            serde_json::Value::String("image[]".to_string())
        );
        assert_eq!(request_body["reference_count"], serde_json::Value::from(2));
        assert_eq!(
            request_body["output_compression"],
            serde_json::Value::from(65)
        );
        assert_eq!(
            request_body["moderation"],
            serde_json::Value::String("low".to_string())
        );
    }

    #[test]
    fn image_build_request_snapshot_omits_moderation_when_not_provided() {
        let channel = sample_channel(
            "https://example.com/",
            "test-key",
            "google/nano-banana",
            Some("/custom/generations/"),
            None,
        );
        let mut input = sample_text_to_image_input("google/nano-banana");
        input.params.moderation = None;

        let snapshot = build_request_snapshot(&channel, &input).expect("build request snapshot");
        let request_body: serde_json::Value =
            serde_json::from_str(&snapshot.request_body_json).expect("parse request body json");

        assert!(
            request_body.get("moderation").is_none(),
            "banana-compatible requests should omit moderation when not provided"
        );
    }

    #[test]
    fn image_build_request_snapshot_for_gemini_uses_generate_content_shape() {
        let channel = sample_channel_with_provider(
            PROVIDER_KIND_GEMINI,
            "https://generativelanguage.googleapis.com/v1beta",
            "test-key",
            "gemini-2.5-flash-image",
            Some("ignored/custom/path"),
            Some("ignored/custom/edit"),
        );
        let mut input = sample_text_to_image_input("gemini-2.5-flash-image");
        input.params.size = "1024x1024".to_string();

        let snapshot = build_request_snapshot(&channel, &input).expect("build request snapshot");
        let request_headers: serde_json::Value =
            serde_json::from_str(&snapshot.request_headers_json)
                .expect("parse request headers json");
        let request_body: serde_json::Value =
            serde_json::from_str(&snapshot.request_body_json).expect("parse request body json");

        assert_eq!(
            snapshot.request_url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-image:generateContent"
        );
        assert_eq!(
            request_headers["x-goog-api-key"],
            serde_json::Value::String("***".to_string())
        );
        assert_eq!(
            request_body["generationConfig"]["responseModalities"][0],
            serde_json::Value::String("IMAGE".to_string())
        );
        assert_eq!(
            request_body["contents"][0]["parts"][0]["text"],
            serde_json::Value::String(
                "A tiny red square icon on a plain white background".to_string()
            )
        );
        assert_eq!(
            request_body["generationConfig"]["imageConfig"]["aspectRatio"],
            serde_json::Value::String("1:1".to_string())
        );
        assert!(request_body.get("model").is_none());
    }

    #[test]
    fn image_build_request_snapshot_for_gemini_image_to_image_masks_inline_data() {
        let channel = sample_channel_with_provider(
            PROVIDER_KIND_GEMINI,
            "https://generativelanguage.googleapis.com",
            "test-key",
            "gemini-3.1-flash-image-preview",
            None,
            None,
        );
        let input = sample_image_to_image_input("gemini-3.1-flash-image-preview");

        let snapshot = build_request_snapshot(&channel, &input).expect("build request snapshot");
        let request_body: serde_json::Value =
            serde_json::from_str(&snapshot.request_body_json).expect("parse request body json");
        let parts = request_body["contents"][0]["parts"]
            .as_array()
            .expect("parts array");

        assert_eq!(
            snapshot.request_url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-image-preview:generateContent"
        );
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts[1]["inlineData"]["data"],
            serde_json::Value::String("***".to_string())
        );
        assert_eq!(
            request_body["generationConfig"]["imageConfig"]["imageSize"],
            serde_json::Value::String("1K".to_string())
        );
    }

    #[test]
    fn image_build_request_snapshot_for_responses_uses_responses_endpoint() {
        let channel = sample_channel_with_provider(
            PROVIDER_KIND_OPENAI_RESPONSES,
            "https://api.openai.com",
            "test-key",
            "gpt-image-1",
            Some("ignored/custom/path"),
            Some("ignored/custom/edit"),
        );
        let input = sample_text_to_image_input("gpt-image-1");

        let snapshot = build_request_snapshot(&channel, &input).expect("build request snapshot");
        let request_headers: serde_json::Value =
            serde_json::from_str(&snapshot.request_headers_json)
                .expect("parse request headers json");
        let request_body: serde_json::Value =
            serde_json::from_str(&snapshot.request_body_json).expect("parse request body json");
        let tools = request_body["tools"].as_array().expect("tools array");

        assert_eq!(snapshot.request_url, "https://api.openai.com/v1/responses");
        assert_eq!(
            request_headers["Authorization"],
            serde_json::Value::String("Bearer ***".to_string())
        );
        assert_eq!(
            request_body["model"],
            serde_json::Value::String("gpt-image-1".to_string())
        );
        assert_eq!(
            request_body["input"],
            serde_json::Value::String(
                "Use the following text as the complete prompt. Do not rewrite it:\nA tiny red square icon on a plain white background".to_string()
            )
        );
        assert!(
            tools[0].get("moderation").is_none(),
            "Responses image tool should omit moderation"
        );
        assert_eq!(
            tools[0]["type"],
            serde_json::Value::String("image_generation".to_string())
        );
        assert_eq!(
            request_body["tool_choice"],
            serde_json::Value::String("required".to_string())
        );
        assert_eq!(
            tools[0]["action"],
            serde_json::Value::String("generate".to_string())
        );
        assert!(
            tools[0].get("model").is_none(),
            "Responses image tool should not repeat the root model"
        );
    }

    #[test]
    fn image_build_request_snapshot_for_responses_image_to_image_masks_data_url() {
        let channel = sample_channel_with_provider(
            PROVIDER_KIND_OPENAI_RESPONSES,
            "https://api.openai.com/v1",
            "test-key",
            "gpt-image-1",
            None,
            None,
        );
        let input = sample_image_to_image_input("gpt-image-1");

        let snapshot = build_request_snapshot(&channel, &input).expect("build request snapshot");
        let request_body: serde_json::Value =
            serde_json::from_str(&snapshot.request_body_json).expect("parse request body json");
        let content = request_body["input"][0]["content"]
            .as_array()
            .expect("content array");
        let tools = request_body["tools"].as_array().expect("tools array");

        assert_eq!(snapshot.request_url, "https://api.openai.com/v1/responses");
        assert_eq!(content.len(), 3);
        assert_eq!(
            content[0]["text"],
            serde_json::Value::String(
                "Use the following text as the complete prompt. Do not rewrite it:\nTurn the reference into a flat monochrome icon".to_string()
            )
        );
        assert_eq!(
            content[1]["type"],
            serde_json::Value::String("input_image".to_string())
        );
        assert_eq!(
            content[1]["image_url"],
            serde_json::Value::String("***".to_string())
        );
        for item in content.iter().skip(1) {
            assert!(
                item.get("file_name").is_none(),
                "Responses input_image items should omit file_name"
            );
        }
        assert_eq!(
            tools[0]["action"],
            serde_json::Value::String("edit".to_string())
        );
        assert!(
            tools[0].get("model").is_none(),
            "Responses image tool should not repeat the root model"
        );
    }

    #[test]
    fn responses_request_body_uses_reference_json_shape() {
        let input = sample_text_to_image_input("gpt-image-1");
        let plan = build_responses_request_plans(&input)
            .into_iter()
            .next()
            .expect("responses request plan");
        let body = build_responses_request_body(&input, &[], plan);

        assert!(body.get("stream").is_none());
        assert!(body["tools"][0].get("partial_images").is_none());
        assert!(body["tools"][0].get("model").is_none());
        assert_eq!(
            body["tool_choice"],
            serde_json::Value::String("required".to_string())
        );
        assert_eq!(
            body["tools"][0]["action"],
            serde_json::Value::String("generate".to_string())
        );
        assert_eq!(
            body["input"],
            serde_json::Value::String(
                "Use the following text as the complete prompt. Do not rewrite it:\nA tiny red square icon on a plain white background".to_string()
            )
        );
    }

    #[test]
    fn responses_unknown_parameter_errors_do_not_trigger_compatibility_retry() {
        let message = concat!(
            "Responses image API failed: HTTP 400 Unknown parameter: ",
            "'input[0].content[1].file_name'."
        );
        let stream_plan = ResponsesRequestPlan {
            id: "stream-message-list",
            input_payload_mode: ResponsesInputPayloadMode::MessageList,
            transport: ResponsesTransportKind::Stream,
            tool_choice_mode: ResponsesToolChoiceMode::Required,
        };
        let json_plan = ResponsesRequestPlan {
            id: "json-message-list",
            input_payload_mode: ResponsesInputPayloadMode::MessageList,
            transport: ResponsesTransportKind::Json,
            tool_choice_mode: ResponsesToolChoiceMode::Required,
        };

        assert!(!should_retry_responses_with_compatibility(message));
        assert!(!should_fallback_responses_stream_to_json(
            message,
            stream_plan,
            Some(json_plan)
        ));
    }

    #[test]
    fn pick_responses_image_metadata_records_actual_params_and_revised_prompt() {
        let item = json!({
            "type": "image_generation_call",
            "result": "aGVsbG8=",
            "size": "1024x1536",
            "quality": "high",
            "output_format": "webp",
            "output_compression": 75,
            "moderation": "auto",
            "revised_prompt": "A revised prompt"
        });

        let metadata = pick_responses_image_metadata(&item).expect("metadata");

        assert_eq!(
            metadata["size"],
            serde_json::Value::String("1024x1536".to_string())
        );
        assert_eq!(
            metadata["quality"],
            serde_json::Value::String("high".to_string())
        );
        assert_eq!(
            metadata["output_format"],
            serde_json::Value::String("webp".to_string())
        );
        assert_eq!(metadata["output_compression"], serde_json::Value::from(75));
        assert_eq!(
            metadata["revised_prompt"],
            serde_json::Value::String("A revised prompt".to_string())
        );
    }

    #[test]
    fn read_responses_payload_from_sse_merges_completed_response_and_output_items() {
        let sse_text = concat!(
            "event: response.output_item.done\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"image_generation_call\",\"result\":\"aGVsbG8=\"}}\n",
            "\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"status\":\"completed\"}}\n",
            "\n"
        );

        let payload = read_responses_payload_from_text(sse_text, reqwest::StatusCode::OK)
            .expect("parse responses sse payload");

        assert_eq!(
            payload["id"],
            serde_json::Value::String("resp_123".to_string())
        );
        assert_eq!(
            payload["output"][0]["type"],
            serde_json::Value::String("image_generation_call".to_string())
        );
        assert_eq!(
            payload["output"][0]["result"],
            serde_json::Value::String("aGVsbG8=".to_string())
        );
    }

    #[tokio::test]
    #[ignore = "requires real image gateway credentials"]
    async fn image_execute_generation_live_smoke_works_with_openai_compatible_gateway() {
        let test_db_state = create_test_db_state().await;
        let base_url = require_live_env("AI_TOOLBOX_IMAGE_LIVE_BASE_URL");
        let api_key = require_live_env("AI_TOOLBOX_IMAGE_LIVE_API_KEY");
        let model_id = require_live_env("AI_TOOLBOX_IMAGE_LIVE_MODEL_ID");
        let prompt = std::env::var("AI_TOOLBOX_IMAGE_LIVE_PROMPT")
            .unwrap_or_else(|_| "A tiny red square icon on a plain white background".to_string());

        let channel = sample_channel(&base_url, &api_key, &model_id, None, None);
        let mut input = sample_text_to_image_input(&model_id);
        input.prompt = prompt;

        let request_url = ImageProviderAdapter::from_kind(&channel.provider_kind)
            .expect("provider adapter")
            .build_request_url(&channel, &input.mode, &input.model_id)
            .expect("build request url");
        let results = execute_generation_request(
            None,
            &test_db_state.state,
            "live-smoke-job",
            &channel,
            &input,
            &request_url,
        )
        .await
        .expect("execute real image generation request");

        assert!(
            !results.is_empty(),
            "live gateway returned no images for request_url={request_url}"
        );

        let first_image = &results[0];
        assert!(
            !first_image.bytes.is_empty(),
            "live gateway returned an empty image payload"
        );
        assert_eq!(first_image.mime_type, "image/png");

        println!(
            "live image smoke test ok: url={} images={} first_image_bytes={}",
            request_url,
            results.len(),
            first_image.bytes.len()
        );
    }
}
