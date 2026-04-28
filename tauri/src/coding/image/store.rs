use std::collections::HashMap;

use serde_json::json;
use surrealdb::sql::Thing;

use super::types::{ImageAssetRecord, ImageChannelRecord, ImageJobRecord};
use crate::coding::db_id::{db_clean_id, db_new_id, db_record_id};
use crate::DbState;

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

pub async fn list_image_channels(
    state: &DbState,
    limit: usize,
) -> Result<Vec<ImageChannelRecord>, String> {
    let db = state.db();
    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM image_channel ORDER BY sort_order ASC, created_at ASC LIMIT $limit",
        )
        .bind(("limit", limit))
        .await
        .map_err(|e| format!("Failed to list image channels: {}", e))?;

    let records: Vec<ImageChannelRecord> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records
        .into_iter()
        .map(normalize_image_channel_record)
        .collect())
}

pub async fn get_image_channel_by_id(
    state: &DbState,
    channel_id: &str,
) -> Result<Option<ImageChannelRecord>, String> {
    let db = state.db();
    let record_id = db_record_id("image_channel", channel_id);
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to get image channel: {}", e))?;

    let records: Vec<ImageChannelRecord> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records
        .into_iter()
        .map(normalize_image_channel_record)
        .next())
}

pub async fn get_max_image_channel_sort_order(state: &DbState) -> Result<i64, String> {
    let db = state.db();
    let mut result = db
        .query(
            "SELECT sort_order FROM image_channel ORDER BY sort_order DESC LIMIT 1",
        )
        .await
        .map_err(|e| format!("Failed to query image channel sort order: {}", e))?;

    let rows: Vec<serde_json::Value> = result.take(0).map_err(|e| e.to_string())?;
    let max_sort_order = rows
        .first()
        .and_then(|row| row.get("sort_order"))
        .and_then(|value| value.as_i64())
        .unwrap_or(-1);
    Ok(max_sort_order)
}

pub async fn upsert_image_channel(
    state: &DbState,
    channel: &ImageChannelRecord,
) -> Result<ImageChannelRecord, String> {
    let db = state.db();
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
    let record_id = db_record_id("image_channel", &channel.id);

    db.query(&format!("UPSERT {} CONTENT $data", record_id))
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to upsert image channel: {}", e))?;

    get_image_channel_by_id(state, &channel.id)
        .await?
        .ok_or_else(|| "Saved image channel not found".to_string())
}

pub async fn delete_image_channel(state: &DbState, channel_id: &str) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("image_channel", channel_id);
    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete image channel: {}", e))?;
    Ok(())
}

pub async fn update_image_channel_sort_orders(
    state: &DbState,
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
    state: &DbState,
    record: &ImageJobRecord,
) -> Result<String, String> {
    let db = state.db();
    let id = if record.id.is_empty() {
        db_new_id()
    } else {
        record.id.clone()
    };
    let payload = serde_json::to_value(record).map_err(|e| e.to_string())?;
    let record_id = db_record_id("image_job", &id);
    db.query(&format!("CREATE {} CONTENT $data", record_id))
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to create image job: {}", e))?;
    Ok(id)
}

pub async fn update_image_job(
    state: &DbState,
    record: &ImageJobRecord,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("image_job", &record.id);
    let payload = serde_json::to_value(record).map_err(|e| e.to_string())?;
    db.query(&format!("UPDATE {} CONTENT $data", record_id))
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to update image job: {}", e))?;
    Ok(())
}

pub async fn list_image_jobs(
    state: &DbState,
    limit: usize,
) -> Result<Vec<ImageJobRecord>, String> {
    let db = state.db();
    let mut result = db
        .query("SELECT *, type::string(id) as id FROM image_job ORDER BY created_at DESC LIMIT $limit")
        .bind(("limit", limit))
        .await
        .map_err(|e| format!("Failed to list image jobs: {}", e))?;

    let records: Vec<ImageJobRecord> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(normalize_image_job_record).collect())
}

pub async fn get_image_job_by_id(
    state: &DbState,
    job_id: &str,
) -> Result<Option<ImageJobRecord>, String> {
    let db = state.db();
    let record_id = db_record_id("image_job", job_id);
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to get image job: {}", e))?;

    let records: Vec<ImageJobRecord> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(normalize_image_job_record).next())
}

pub async fn delete_image_job(state: &DbState, job_id: &str) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("image_job", job_id);
    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete image job: {}", e))?;
    Ok(())
}

pub async fn create_image_asset(
    state: &DbState,
    asset: &ImageAssetRecord,
) -> Result<String, String> {
    let db = state.db();
    let id = if asset.id.is_empty() {
        db_new_id()
    } else {
        asset.id.clone()
    };
    let payload = serde_json::to_value(asset).map_err(|e| e.to_string())?;
    let record_id = db_record_id("image_asset", &id);
    db.query(&format!("CREATE {} CONTENT $data", record_id))
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to create image asset: {}", e))?;
    Ok(id)
}

pub async fn get_image_asset_by_id(
    state: &DbState,
    asset_id: &str,
) -> Result<Option<ImageAssetRecord>, String> {
    let db = state.db();
    let record_id = db_record_id("image_asset", asset_id);
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to get image asset: {}", e))?;

    let records: Vec<ImageAssetRecord> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(normalize_image_asset_record).next())
}

pub async fn list_image_assets_by_ids(
    state: &DbState,
    asset_ids: &[String],
) -> Result<Vec<ImageAssetRecord>, String> {
    if asset_ids.is_empty() {
        return Ok(Vec::new());
    }

    let db = state.db();
    let record_refs = asset_ids
        .iter()
        .map(|asset_id| Thing::from(("image_asset", db_clean_id(asset_id).as_str())))
        .collect::<Vec<_>>();

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM image_asset WHERE id INSIDE $asset_ids")
        .bind(("asset_ids", record_refs))
        .await
        .map_err(|e| format!("Failed to list image assets by ids: {}", e))?;

    let records: Vec<ImageAssetRecord> = result.take(0).map_err(|e| e.to_string())?;
    let records_by_id = records
        .into_iter()
        .map(normalize_image_asset_record)
        .map(|record| (record.id.clone(), record))
        .collect::<HashMap<_, _>>();

    let mut assets = Vec::with_capacity(asset_ids.len());
    for asset_id in asset_ids {
        let clean_asset_id = db_clean_id(asset_id);
        if let Some(asset) = records_by_id.get(&clean_asset_id) {
            assets.push(asset.clone());
        }
    }
    Ok(assets)
}

pub async fn delete_image_assets_by_ids(state: &DbState, asset_ids: &[String]) -> Result<(), String> {
    if asset_ids.is_empty() {
        return Ok(());
    }

    let db = state.db();
    let record_ids: Vec<String> = asset_ids
        .iter()
        .map(|asset_id| db_record_id("image_asset", asset_id))
        .collect();

    db.query("DELETE $asset_ids")
        .bind(("asset_ids", record_ids))
        .await
        .map_err(|e| format!("Failed to delete image assets: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::local::SurrealKv;
    use surrealdb::Surreal;

    async fn create_test_db_state() -> (tempfile::TempDir, DbState) {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_path = temp_dir.path().join("surreal");
        let db = Surreal::new::<SurrealKv>(db_path)
            .await
            .expect("open surreal test db");
        db.use_ns("ai_toolbox")
            .use_db("main")
            .await
            .expect("select surreal test namespace");
        (temp_dir, DbState(db))
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
        let (_temp_dir, db_state) = create_test_db_state().await;

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
}
