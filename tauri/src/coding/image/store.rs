use serde_json::json;

use super::types::{ImageAssetRecord, ImageChannelRecord, ImageJobRecord};
use crate::coding::db_id::{db_clean_id, db_new_id};
use crate::db::helpers::{db_delete, db_get, db_list, db_max_i64, db_put};
use crate::db::schema::{
    DbTable, ImageEntityKind, JsonFieldPath, OrderDirection, OrderField, OrderSpec,
};
use crate::SqliteDbState;

fn normalize_image_channel_record(mut record: ImageChannelRecord) -> ImageChannelRecord {
    record.id = db_clean_id(&record.id);
    record
}

fn normalize_image_job_record(mut record: ImageJobRecord) -> ImageJobRecord {
    record.id = db_clean_id(&record.id);
    record.channel_id = db_clean_id(&record.channel_id);
    record
}

fn normalize_image_asset_record(mut record: ImageAssetRecord) -> ImageAssetRecord {
    record.id = db_clean_id(&record.id);
    record.job_id = record.job_id.map(|job_id| db_clean_id(&job_id));
    record
}

fn image_channel_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_order", OrderDirection::Asc)?,
        OrderField::json_integer("created_at", OrderDirection::Asc)?,
    ]))
}

fn image_job_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::single(OrderField::json_integer(
        "created_at",
        OrderDirection::Desc,
    )?))
}

fn sqlite_value_to_image_channel(value: serde_json::Value) -> Result<ImageChannelRecord, String> {
    serde_json::from_value(value)
        .map(normalize_image_channel_record)
        .map_err(|error| format!("Failed to parse SQLite image channel: {error}"))
}

fn sqlite_value_to_image_job(value: serde_json::Value) -> Result<ImageJobRecord, String> {
    serde_json::from_value(value)
        .map(normalize_image_job_record)
        .map_err(|error| format!("Failed to parse SQLite image job: {error}"))
}

fn sqlite_value_to_image_asset(value: serde_json::Value) -> Result<ImageAssetRecord, String> {
    serde_json::from_value(value)
        .map(normalize_image_asset_record)
        .map_err(|error| format!("Failed to parse SQLite image asset: {error}"))
}

pub async fn list_image_channels(
    state: &SqliteDbState,
    limit: usize,
) -> Result<Vec<ImageChannelRecord>, String> {
    let order = image_channel_order()?;
    state.with_conn(|conn| {
        db_list(conn, DbTable::ImageEntity(ImageEntityKind::Channel), Some(&order))?
            .into_iter()
            .take(limit)
            .map(sqlite_value_to_image_channel)
            .collect()
    })
}

pub async fn get_image_channel_by_id(
    state: &SqliteDbState,
    channel_id: &str,
) -> Result<Option<ImageChannelRecord>, String> {
    state.with_conn(|conn| {
        db_get(conn, DbTable::ImageEntity(ImageEntityKind::Channel), &db_clean_id(channel_id))?
            .map(sqlite_value_to_image_channel)
            .transpose()
    })
}

pub async fn get_max_image_channel_sort_order(state: &SqliteDbState) -> Result<i64, String> {
    state.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::ImageEntity(ImageEntityKind::Channel),
            &JsonFieldPath::new("sort_order")?,
        )?
        .unwrap_or(-1))
    })
}

pub async fn upsert_image_channel(
    state: &SqliteDbState,
    channel: &ImageChannelRecord,
) -> Result<ImageChannelRecord, String> {
    let payload = json!({
        "id": channel.id,
        "name": channel.name,
        "provider_kind": channel.provider_kind,
        "base_url": channel.base_url,
        "api_key": channel.api_key,
        "generation_path": channel.generation_path,
        "edit_path": channel.edit_path,
        "timeout_seconds": channel.timeout_seconds,
        "enabled": channel.enabled,
        "sort_order": channel.sort_order,
        "models_json": channel.models_json,
        "created_at": channel.created_at,
        "updated_at": channel.updated_at,
    });
    state.with_conn(|conn| db_put(conn, DbTable::ImageEntity(ImageEntityKind::Channel), &channel.id, &payload))?;

    get_image_channel_by_id(state, &channel.id)
        .await?
        .ok_or_else(|| "Saved image channel not found".to_string())
}

pub async fn delete_image_channel(state: &SqliteDbState, channel_id: &str) -> Result<(), String> {
    state.with_conn(|conn| {
        db_delete(conn, DbTable::ImageEntity(ImageEntityKind::Channel), &db_clean_id(channel_id)).map(|_| ())
    })
}

pub async fn update_image_channel_sort_orders(
    state: &SqliteDbState,
    ordered_ids: &[String],
) -> Result<Vec<ImageChannelRecord>, String> {
    for (index, channel_id) in ordered_ids.iter().enumerate() {
        let existing_channel = get_image_channel_by_id(state, channel_id)
            .await?
            .ok_or_else(|| format!("Image channel not found: {}", channel_id))?;

        let updated_channel = ImageChannelRecord {
            sort_order: index as i64,
            ..existing_channel
        };
        upsert_image_channel(state, &updated_channel).await?;
    }

    list_image_channels(state, ordered_ids.len().max(50)).await
}

pub async fn create_image_job(
    state: &SqliteDbState,
    record: &ImageJobRecord,
) -> Result<String, String> {
    let id = if record.id.is_empty() {
        db_new_id()
    } else {
        record.id.clone()
    };
    let mut record_with_id = record.clone();
    record_with_id.id = id.clone();
    let payload = serde_json::to_value(&record_with_id).map_err(|e| e.to_string())?;
    state.with_conn(|conn| db_put(conn, DbTable::ImageEntity(ImageEntityKind::Job), &id, &payload))?;
    Ok(id)
}

pub async fn update_image_job(
    state: &SqliteDbState,
    record: &ImageJobRecord,
) -> Result<(), String> {
    let payload = serde_json::to_value(record).map_err(|e| e.to_string())?;
    state.with_conn(|conn| db_put(conn, DbTable::ImageEntity(ImageEntityKind::Job), &record.id, &payload))
}

pub async fn list_image_jobs(
    state: &SqliteDbState,
    limit: usize,
) -> Result<Vec<ImageJobRecord>, String> {
    let order = image_job_order()?;
    state.with_conn(|conn| {
        db_list(conn, DbTable::ImageEntity(ImageEntityKind::Job), Some(&order))?
            .into_iter()
            .take(limit)
            .map(sqlite_value_to_image_job)
            .collect()
    })
}

pub async fn get_image_job_by_id(
    state: &SqliteDbState,
    job_id: &str,
) -> Result<Option<ImageJobRecord>, String> {
    state.with_conn(|conn| {
        db_get(conn, DbTable::ImageEntity(ImageEntityKind::Job), &db_clean_id(job_id))?
            .map(sqlite_value_to_image_job)
            .transpose()
    })
}

pub async fn delete_image_job(state: &SqliteDbState, job_id: &str) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::ImageEntity(ImageEntityKind::Job), &db_clean_id(job_id)).map(|_| ()))
}

pub async fn create_image_asset(
    state: &SqliteDbState,
    asset: &ImageAssetRecord,
) -> Result<String, String> {
    let id = if asset.id.is_empty() {
        db_new_id()
    } else {
        asset.id.clone()
    };
    let mut asset_with_id = asset.clone();
    asset_with_id.id = id.clone();
    let payload = serde_json::to_value(&asset_with_id).map_err(|e| e.to_string())?;
    state.with_conn(|conn| db_put(conn, DbTable::ImageEntity(ImageEntityKind::Asset), &id, &payload))?;
    Ok(id)
}

pub async fn get_image_asset_by_id(
    state: &SqliteDbState,
    asset_id: &str,
) -> Result<Option<ImageAssetRecord>, String> {
    state.with_conn(|conn| {
        db_get(conn, DbTable::ImageEntity(ImageEntityKind::Asset), &db_clean_id(asset_id))?
            .map(sqlite_value_to_image_asset)
            .transpose()
    })
}

pub async fn list_image_assets_by_ids(
    state: &SqliteDbState,
    asset_ids: &[String],
) -> Result<Vec<ImageAssetRecord>, String> {
    if asset_ids.is_empty() {
        return Ok(Vec::new());
    }

    state.with_conn(|conn| {
        let mut assets = Vec::with_capacity(asset_ids.len());
        for asset_id in asset_ids {
            if let Some(asset) = db_get(conn, DbTable::ImageEntity(ImageEntityKind::Asset), &db_clean_id(asset_id))?
                .map(sqlite_value_to_image_asset)
                .transpose()?
            {
                assets.push(asset);
            }
        }
        Ok(assets)
    })
}

pub async fn delete_image_assets_by_ids(
    state: &SqliteDbState,
    asset_ids: &[String],
) -> Result<(), String> {
    if asset_ids.is_empty() {
        return Ok(());
    }

    state.with_conn(|conn| {
        for asset_id in asset_ids {
            db_delete(conn, DbTable::ImageEntity(ImageEntityKind::Asset), &db_clean_id(asset_id))?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db_state() -> (tempfile::TempDir, SqliteDbState) {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_state = SqliteDbState::open(temp_dir.path().join("ai-toolbox.db"))
            .expect("open sqlite test db");
        (temp_dir, db_state)
    }

    fn sample_asset(asset_id: &str, job_id: &str, file_name: &str) -> ImageAssetRecord {
        ImageAssetRecord {
            id: asset_id.to_string(),
            job_id: Some(job_id.to_string()),
            role: "output".to_string(),
            mime_type: "image/png".to_string(),
            file_name: file_name.to_string(),
            relative_path: format!("assets/{asset_id}.png"),
            bytes: 123,
            width: None,
            height: None,
            created_at: 1,
        }
    }

    #[tokio::test]
    async fn list_image_assets_by_ids_preserves_input_order_and_skips_missing_records() {
        let (_temp_dir, db_state) = create_test_db_state();

        let first_asset = sample_asset("asset-first", "job-1", "first.png");
        let second_asset = sample_asset("asset-second", "job-1", "second.png");
        create_image_asset(&db_state, &first_asset)
            .await
            .expect("create first image asset");
        create_image_asset(&db_state, &second_asset)
            .await
            .expect("create second image asset");

        let assets = list_image_assets_by_ids(
            &db_state,
            &[
                "asset-second".to_string(),
                "asset-missing".to_string(),
                "asset-first".to_string(),
                "asset-second".to_string(),
            ],
        )
        .await
        .expect("list image assets by ids");

        assert_eq!(assets.len(), 3);
        assert_eq!(assets[0].id, "asset-second");
        assert_eq!(assets[1].id, "asset-first");
        assert_eq!(assets[2].id, "asset-second");
    }

    #[tokio::test]
    async fn channels_jobs_and_assets_round_trip_in_their_own_kinds() {
        let (_temp_dir, db_state) = create_test_db_state();

        let channel = ImageChannelRecord {
            id: "channel-a".to_string(),
            name: "Channel A".to_string(),
            provider_kind: "openai_compatible".to_string(),
            base_url: "https://example.com/v1".to_string(),
            api_key: "test-key".to_string(),
            generation_path: None,
            edit_path: None,
            timeout_seconds: None,
            enabled: true,
            sort_order: 3,
            models_json: "[]".to_string(),
            created_at: 1,
            updated_at: 2,
        };
        let saved_channel = upsert_image_channel(&db_state, &channel)
            .await
            .expect("save image channel");
        assert_eq!(saved_channel.id, "channel-a");
        assert_eq!(
            get_max_image_channel_sort_order(&db_state)
                .await
                .expect("max channel sort order"),
            3
        );
        assert_eq!(
            list_image_channels(&db_state, 10)
                .await
                .expect("list image channels")
                .len(),
            1
        );

        let job = ImageJobRecord {
            id: String::new(),
            mode: "text_to_image".to_string(),
            prompt: "draw a cat".to_string(),
            channel_id: "channel-a".to_string(),
            channel_name_snapshot: "Channel A".to_string(),
            provider_kind_snapshot: None,
            model_id: "model-a".to_string(),
            model_name_snapshot: "Model A".to_string(),
            params_json: "{}".to_string(),
            status: "running".to_string(),
            error_message: None,
            request_url: None,
            request_headers_json: None,
            request_body_json: None,
            response_metadata_json: None,
            input_asset_ids: vec![],
            output_asset_ids: vec!["asset-a".to_string()],
            created_at: 10,
            finished_at: None,
            elapsed_ms: None,
        };
        let job_id = create_image_job(&db_state, &job)
            .await
            .expect("create image job");
        let loaded_job = get_image_job_by_id(&db_state, &job_id)
            .await
            .expect("read image job")
            .expect("image job exists");
        assert_eq!(loaded_job.channel_id, "channel-a");
        assert_eq!(
            list_image_jobs(&db_state, 10)
                .await
                .expect("list image jobs")
                .len(),
            1
        );
        let mut finished_job = loaded_job.clone();
        finished_job.status = "done".to_string();
        update_image_job(&db_state, &finished_job)
            .await
            .expect("update image job");
        assert_eq!(
            get_image_job_by_id(&db_state, &job_id)
                .await
                .expect("read updated image job")
                .expect("updated image job exists")
                .status,
            "done"
        );
        assert!(
            get_image_channel_by_id(&db_state, &job_id)
                .await
                .expect("read channel by job id")
                .is_none(),
            "a job row must not resolve as a channel"
        );

        let asset = sample_asset("asset-a", &job_id, "a.png");
        create_image_asset(&db_state, &asset)
            .await
            .expect("create image asset");
        assert!(get_image_asset_by_id(&db_state, "asset-a")
            .await
            .expect("read image asset")
            .is_some());

        delete_image_job(&db_state, &job_id)
            .await
            .expect("delete image job");
        assert!(get_image_job_by_id(&db_state, &job_id)
            .await
            .expect("read deleted image job")
            .is_none());
        delete_image_assets_by_ids(&db_state, &["asset-a".to_string()])
            .await
            .expect("delete image asset");
        assert!(get_image_asset_by_id(&db_state, "asset-a")
            .await
            .expect("read deleted image asset")
            .is_none());
        delete_image_channel(&db_state, "channel-a")
            .await
            .expect("delete image channel");
        assert!(get_image_channel_by_id(&db_state, "channel-a")
            .await
            .expect("read deleted image channel")
            .is_none());
    }
}
