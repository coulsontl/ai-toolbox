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
use serde_json::json;
use tauri::{AppHandle, Manager, State};

use super::store;
use super::types::{
    now_ms, CreateImageJobInput, DeleteImageChannelInput, DeleteImageJobInput, ImageAssetDto,
    ImageAssetRecord, ImageChannelDto, ImageChannelModel, ImageChannelRecord, ImageJobDto,
    ImageJobMode, ImageJobRecord, ImageJobStatus, ImageReferenceInput, ImageWorkspaceDto,
    ListImageChannelsInput, ListImageJobsInput, ReorderImageChannelsInput, UpsertImageChannelInput,
};
use crate::coding::db_id::db_clean_id;
use crate::http_client;
use crate::DbState;

const DEFAULT_CHANNEL_LIST_LIMIT: usize = 200;
const PROVIDER_KIND_OPENAI_COMPATIBLE: &str = "openai_compatible";
const IMAGE_REQUEST_ACCEPT_ENCODING: &str = "identity";
const IMAGE_REQUEST_MAX_ATTEMPTS: usize = 4;
const IMAGE_REQUEST_RETRY_DELAYS_MS: [u64; 3] = [1500, 3000, 5000];

struct ImageJobRequestSnapshot {
    request_url: String,
    request_headers_json: String,
    request_body_json: String,
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
        mode,
        channel_name,
        request_url,
        image_url,
        status,
        headers,
        preview
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

fn resolve_default_channel_path(provider_kind: &str, mode: &str) -> Result<&'static str, String> {
    match (provider_kind, mode) {
        (PROVIDER_KIND_OPENAI_COMPATIBLE, value) if value == ImageJobMode::TextToImage.as_str() => {
            Ok("images/generations")
        }
        (PROVIDER_KIND_OPENAI_COMPATIBLE, value)
            if value == ImageJobMode::ImageToImage.as_str() =>
        {
            Ok("images/edits")
        }
        _ => Err(format!(
            "Unsupported image provider kind: {}",
            provider_kind
        )),
    }
}

fn resolve_channel_request_path(channel: &ImageChannelDto, mode: &str) -> Result<String, String> {
    let custom_path = if mode == ImageJobMode::TextToImage.as_str() {
        channel.generation_path.clone()
    } else {
        channel.edit_path.clone()
    };

    if let Some(path) = sanitize_channel_path(custom_path) {
        return Ok(path);
    }

    resolve_default_channel_path(&channel.provider_kind, mode).map(str::to_string)
}

fn resolve_channel_timeout_seconds(channel: &ImageChannelDto) -> u64 {
    channel.timeout_seconds.unwrap_or(300).max(1)
}

fn build_request_url(channel: &ImageChannelDto, mode: &str) -> Result<String, String> {
    let request_path = resolve_channel_request_path(channel, mode)?;
    Ok(build_image_api_url(&channel.base_url, &request_path))
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
        "moderation": input.params.moderation,
    });

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
        "moderation": input.params.moderation,
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
    let request_url = build_request_url(channel, &input.mode)?;
    let output_format = input.params.output_format.trim().to_lowercase();
    let content_type = if input.mode == ImageJobMode::ImageToImage.as_str() {
        "multipart/form-data"
    } else {
        "application/json"
    };
    let request_headers_json = serialize_json_pretty(
        &json!({
            "Authorization": "Bearer ***",
            "Content-Type": content_type,
            "Accept-Encoding": IMAGE_REQUEST_ACCEPT_ENCODING,
        }),
        "image request headers",
    )?;

    let request_body_value = if input.mode == ImageJobMode::ImageToImage.as_str() {
        build_image_to_image_request_body_snapshot(input, &output_format)
    } else {
        build_text_to_image_request_body(input, &output_format)
    };
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

    let provider_kind = input.provider_kind.trim();
    if provider_kind != PROVIDER_KIND_OPENAI_COMPATIBLE {
        return Err(format!(
            "Unsupported image provider kind: {}",
            provider_kind
        ));
    }

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
    state: &DbState,
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
        asset
            .job_id
            .as_deref()
            .unwrap_or("none"),
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
    state: &DbState,
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
    state: &DbState,
    channel: &ImageChannelDto,
    input: &CreateImageJobInput,
    request_url: &str,
) -> Result<Vec<(Vec<u8>, String)>, String> {
    let timeout_seconds = resolve_channel_timeout_seconds(channel);
    let client = http_client::client_with_timeout_no_compression(state, timeout_seconds).await?;
    let authorization = format!("Bearer {}", channel.api_key.trim());
    let output_format = input.params.output_format.trim().to_lowercase();
    let mime_type = mime_from_output_format(&output_format).to_string();

    if input.mode == ImageJobMode::ImageToImage.as_str() {
        for attempt in 1..=IMAGE_REQUEST_MAX_ATTEMPTS {
            let request_started_at = Instant::now();
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
                .text("output_format", output_format.clone())
                .text("moderation", input.params.moderation.clone());

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
            .await;
        }

        return Err("Image edit request exhausted retries unexpectedly".to_string());
    }

    let request_body = build_text_to_image_request_body(input, &output_format);
    for attempt in 1..=IMAGE_REQUEST_MAX_ATTEMPTS {
        let request_started_at = Instant::now();
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
        .await;
    }

    Err("Image generation request exhausted retries unexpectedly".to_string())
}

async fn parse_image_response(
    state: &DbState,
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
                mode,
                channel_name,
                request_url,
                image_url,
                timeout_seconds
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

async fn to_job_dto(
    app: &AppHandle,
    state: &DbState,
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
        model_id: record.model_id,
        model_name_snapshot: record.model_name_snapshot,
        params_json: record.params_json,
        status: record.status,
        error_message: record.error_message,
        request_url: record.request_url,
        request_headers_json: record.request_headers_json,
        request_body_json: record.request_body_json,
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
    state: &DbState,
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
    state: State<'_, DbState>,
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
    state: State<'_, DbState>,
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
    state: State<'_, DbState>,
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
    state: State<'_, DbState>,
    input: DeleteImageChannelInput,
) -> Result<(), String> {
    let clean_channel_id = db_clean_id(&input.id);
    store::delete_image_channel(&state, &clean_channel_id).await
}

#[tauri::command]
pub async fn image_delete_job(
    app: AppHandle,
    state: State<'_, DbState>,
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
    state: State<'_, DbState>,
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
    state: State<'_, DbState>,
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
    state: State<'_, DbState>,
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

    match execute_generation_request(&state, &channel_dto, &input, &request_snapshot.request_url)
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
                for (index, (bytes, mime_type)) in result_images.into_iter().enumerate() {
                    let file_name = format!("result-{}.{}", index + 1, file_extension_for_mime(&mime_type));
                    debug!(
                        "Image output persist start: job_id={} index={} bytes={} mime_type={} elapsed_ms={}",
                        job_record.id,
                        index + 1,
                        bytes.len(),
                        mime_type,
                        command_started_at.elapsed().as_millis()
                    );
                    let asset = persist_asset_file(
                        &app,
                        &state,
                        Some(job_record.id.clone()),
                        "output",
                        &file_name,
                        &mime_type,
                        &bytes,
                    )
                    .await?;
                    output_asset_ids.push(asset.id);
                }

                job_record.output_asset_ids = output_asset_ids;
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
    use surrealdb::engine::local::SurrealKv;
    use surrealdb::Surreal;
    use tempfile::TempDir;

    struct TestDbState {
        _temp_dir: TempDir,
        state: DbState,
    }

    async fn create_test_db_state() -> TestDbState {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_path = temp_dir.path().join("surreal");
        let db = Surreal::new::<SurrealKv>(db_path)
            .await
            .expect("open surreal test db");
        db.use_ns("ai_toolbox")
            .use_db("main")
            .await
            .expect("select surreal test namespace");

        TestDbState {
            _temp_dir: temp_dir,
            state: DbState(db),
        }
    }

    fn sample_channel(
        base_url: &str,
        api_key: &str,
        model_id: &str,
        generation_path: Option<&str>,
        edit_path: Option<&str>,
    ) -> ImageChannelDto {
        ImageChannelDto {
            id: "channel-live-smoke".to_string(),
            name: "Live Smoke".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI_COMPATIBLE.to_string(),
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
                moderation: "low".to_string(),
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
                moderation: "low".to_string(),
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
            model_id: "gpt-image-2".to_string(),
            model_name_snapshot: "gpt-image-2".to_string(),
            params_json: "{}".to_string(),
            status: ImageJobStatus::Running.as_str().to_string(),
            error_message: None,
            request_url: None,
            request_headers_json: None,
            request_body_json: None,
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

        let request_url = build_request_url(&channel, &input.mode).expect("build request url");
        let results =
            execute_generation_request(&test_db_state.state, &channel, &input, &request_url)
                .await
                .expect("execute real image generation request");

        assert!(
            !results.is_empty(),
            "live gateway returned no images for request_url={request_url}"
        );

        let (first_image_bytes, first_image_mime) = &results[0];
        assert!(
            !first_image_bytes.is_empty(),
            "live gateway returned an empty image payload"
        );
        assert_eq!(first_image_mime, "image/png");

        println!(
            "live image smoke test ok: url={} images={} first_image_bytes={}",
            request_url,
            results.len(),
            first_image_bytes.len()
        );
    }
}
