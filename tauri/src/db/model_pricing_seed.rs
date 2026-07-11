use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;

use rusqlite::{params, Connection};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::db::SqliteDbState;
use crate::http_client;

const CACHE_FILE_NAME: &str = "model_pricing.json";
const BUNDLED_MODEL_PRICING_JSON: &str = include_str!("../../resources/model_pricing.json");

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ModelPricingSeedItem {
    model_id: String,
    display_name: String,
    input_cost_per_million: String,
    output_cost_per_million: String,
    cache_read_cost_per_million: String,
    cache_creation_cost_per_million: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelPricingSeedResult {
    pub inserted_count: usize,
}

pub fn set_cache_dir(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

fn get_cache_file_path() -> Option<PathBuf> {
    CACHE_DIR.get().map(|dir| dir.join(CACHE_FILE_NAME))
}

pub fn get_model_pricing_cache_path() -> Option<PathBuf> {
    get_cache_file_path()
}

pub fn ensure_seeded_from_cache(conn: &Connection) -> Result<usize, String> {
    let Some(path) = get_cache_file_path() else {
        return Ok(0);
    };
    if !path.exists() {
        return Ok(0);
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            log::warn!("[ModelPricing] Ignoring unreadable cache: {}", error);
            return Ok(0);
        }
    };
    match seed_from_json_str(conn, &content, "cached model pricing") {
        Ok(inserted_count) => Ok(inserted_count),
        Err(error) => {
            log::warn!("[ModelPricing] Ignoring invalid cache: {}", error);
            Ok(0)
        }
    }
}

pub fn ensure_seeded_from_bundled(conn: &Connection) -> Result<usize, String> {
    seed_from_json_str(conn, BUNDLED_MODEL_PRICING_JSON, "bundled model pricing")
}

pub fn ensure_seeded(conn: &Connection) -> Result<usize, String> {
    let cache_inserted_count = ensure_seeded_from_cache(conn)?;
    let bundled_inserted_count = ensure_seeded_from_bundled(conn)?;
    Ok(cache_inserted_count + bundled_inserted_count)
}

pub async fn fetch_remote_model_pricing(
    db_state: &SqliteDbState,
    url: String,
) -> Result<ModelPricingSeedResult, String> {
    let client = http_client::client_with_timeout(db_state, 30).await?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("Failed to fetch remote model pricing: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Remote model pricing request failed: {}",
            response.status()
        ));
    }

    let content = response
        .text()
        .await
        .map_err(|error| format!("Failed to read remote model pricing response: {error}"))?;
    let pricing_items = parse_pricing_items(&content, "remote model pricing")?;

    if let Err(error) = write_cache_file(&pricing_items) {
        log::warn!("[ModelPricing] Failed to write cache: {}", error);
    }

    let inserted_count = db_state
        .with_conn(|conn| insert_pricing_items(conn, &pricing_items, "remote model pricing"))?;
    Ok(ModelPricingSeedResult { inserted_count })
}

fn seed_from_json_str(
    conn: &Connection,
    json_content: &str,
    source_label: &str,
) -> Result<usize, String> {
    let pricing_items = parse_pricing_items(json_content, source_label)?;
    insert_pricing_items(conn, &pricing_items, source_label)
}

fn parse_pricing_items(
    json_content: &str,
    source_label: &str,
) -> Result<Vec<ModelPricingSeedItem>, String> {
    let mut pricing_items: Vec<ModelPricingSeedItem> = serde_json::from_str(json_content)
        .map_err(|error| format!("Failed to parse {source_label} JSON: {error}"))?;
    if pricing_items.is_empty() {
        return Err(format!(
            "{source_label} JSON must contain at least one pricing item"
        ));
    }

    let mut seen_model_ids = HashSet::new();
    for item in &mut pricing_items {
        item.model_id = item.model_id.trim().to_string();
        if item.model_id.is_empty() {
            return Err(format!("{source_label} contains an empty model_id"));
        }

        if !seen_model_ids.insert(item.model_id.clone()) {
            return Err(format!(
                "{source_label} contains duplicate model_id {}",
                item.model_id
            ));
        }

        item.display_name = item.display_name.trim().to_string();
        if item.display_name.is_empty() {
            return Err(format!(
                "{source_label} contains empty display_name for {}",
                item.model_id
            ));
        }

        item.input_cost_per_million = validate_cost(
            source_label,
            &item.model_id,
            "input_cost_per_million",
            &item.input_cost_per_million,
        )?;
        item.output_cost_per_million = validate_cost(
            source_label,
            &item.model_id,
            "output_cost_per_million",
            &item.output_cost_per_million,
        )?;
        item.cache_read_cost_per_million = validate_cost(
            source_label,
            &item.model_id,
            "cache_read_cost_per_million",
            &item.cache_read_cost_per_million,
        )?;
        item.cache_creation_cost_per_million = validate_cost(
            source_label,
            &item.model_id,
            "cache_creation_cost_per_million",
            &item.cache_creation_cost_per_million,
        )?;
    }

    Ok(pricing_items)
}

fn validate_cost(
    source_label: &str,
    model_id: &str,
    field_name: &str,
    value: &str,
) -> Result<String, String> {
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return Err(format!(
            "{source_label} has empty {field_name} for {model_id}"
        ));
    }

    let parsed = Decimal::from_str(trimmed_value).map_err(|error| {
        format!("{source_label} has invalid {field_name} for {model_id}: {error}")
    })?;
    if parsed < Decimal::ZERO {
        return Err(format!(
            "{source_label} has negative {field_name} for {model_id}"
        ));
    }

    Ok(trimmed_value.to_string())
}

fn insert_pricing_items(
    conn: &Connection,
    pricing_items: &[ModelPricingSeedItem],
    source_label: &str,
) -> Result<usize, String> {
    let mut statement = conn
        .prepare(
            "INSERT OR IGNORE INTO model_pricing (
                model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|error| format!("Failed to prepare {source_label} seed statement: {error}"))?;

    let mut inserted_count = 0;
    for item in pricing_items {
        inserted_count += statement
            .execute(params![
                item.model_id,
                item.display_name,
                item.input_cost_per_million,
                item.output_cost_per_million,
                item.cache_read_cost_per_million,
                item.cache_creation_cost_per_million,
            ])
            .map_err(|error| {
                format!(
                    "Failed to seed model pricing {} from {source_label}: {error}",
                    item.model_id
                )
            })?;
    }

    Ok(inserted_count)
}

fn write_cache_file(pricing_items: &[ModelPricingSeedItem]) -> Result<(), String> {
    let path = get_cache_file_path()
        .ok_or_else(|| "Model pricing cache directory not initialized".to_string())?;
    let tmp_path = path.with_extension("json.tmp");

    let json = serde_json::to_string_pretty(pricing_items)
        .map_err(|error| format!("Failed to serialize model pricing cache: {error}"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create model pricing cache directory: {error}"))?;
    }

    fs::write(&tmp_path, json)
        .map_err(|error| format!("Failed to write model pricing tmp cache: {error}"))?;
    fs::rename(&tmp_path, &path)
        .map_err(|error| format!("Failed to rename model pricing tmp cache: {error}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE model_pricing (
                model_id TEXT PRIMARY KEY NOT NULL,
                display_name TEXT NOT NULL,
                input_cost_per_million TEXT NOT NULL DEFAULT '0',
                output_cost_per_million TEXT NOT NULL DEFAULT '0',
                cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
                cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
            );",
        )
        .expect("create model_pricing table");
    }

    #[test]
    fn bundled_model_pricing_json_is_valid() {
        let pricing_items =
            parse_pricing_items(BUNDLED_MODEL_PRICING_JSON, "bundled model pricing")
                .expect("bundled pricing");
        assert!(!pricing_items.is_empty());
    }

    #[test]
    fn bundled_model_pricing_includes_gpt_5_6_and_grok_4_5() {
        let pricing_items =
            parse_pricing_items(BUNDLED_MODEL_PRICING_JSON, "bundled model pricing")
                .expect("bundled pricing");

        for (model_id, input, output, cache_read, cache_creation) in [
            ("gpt-5.6", "5", "30", "0.50", "6.25"),
            ("gpt-5.6-sol", "5", "30", "0.50", "6.25"),
            ("gpt-5.6-terra", "2.50", "15", "0.25", "3.125"),
            ("gpt-5.6-luna", "1", "6", "0.10", "1.25"),
            ("grok-4.5", "2", "6", "0.50", "0"),
            ("grok-4.5-latest", "2", "6", "0.50", "0"),
            ("grok-build-latest", "2", "6", "0.50", "0"),
        ] {
            let pricing = pricing_items
                .iter()
                .find(|pricing| pricing.model_id == model_id)
                .unwrap_or_else(|| panic!("bundled pricing should include {model_id}"));

            assert_eq!(pricing.input_cost_per_million, input);
            assert_eq!(pricing.output_cost_per_million, output);
            assert_eq!(pricing.cache_read_cost_per_million, cache_read);
            assert_eq!(pricing.cache_creation_cost_per_million, cache_creation);
        }
    }

    #[test]
    fn seed_inserts_missing_rows_and_preserves_existing_rows() {
        let conn = Connection::open_in_memory().expect("sqlite");
        create_test_table(&conn);

        let json = r#"[
          {
            "model_id": "seed-model-a",
            "display_name": "Seed Model A",
            "input_cost_per_million": "1",
            "output_cost_per_million": "2",
            "cache_read_cost_per_million": "0.1",
            "cache_creation_cost_per_million": "0.2"
          },
          {
            "model_id": "seed-model-b",
            "display_name": "Seed Model B",
            "input_cost_per_million": "3",
            "output_cost_per_million": "4",
            "cache_read_cost_per_million": "0.3",
            "cache_creation_cost_per_million": "0.4"
          }
        ]"#;

        assert_eq!(seed_from_json_str(&conn, json, "test pricing").unwrap(), 2);
        conn.execute(
            "UPDATE model_pricing SET input_cost_per_million = ?1 WHERE model_id = ?2",
            params!["9", "seed-model-a"],
        )
        .expect("update custom price");

        assert_eq!(seed_from_json_str(&conn, json, "test pricing").unwrap(), 0);
        let preserved_input: String = conn
            .query_row(
                "SELECT input_cost_per_million FROM model_pricing WHERE model_id = ?1",
                params!["seed-model-a"],
                |row| row.get(0),
            )
            .expect("query preserved price");
        assert_eq!(preserved_input, "9");
    }

    #[test]
    fn seed_rejects_duplicate_model_ids() {
        let json = r#"[
          {
            "model_id": "duplicate-model",
            "display_name": "Duplicate Model",
            "input_cost_per_million": "1",
            "output_cost_per_million": "2",
            "cache_read_cost_per_million": "0.1",
            "cache_creation_cost_per_million": "0.2"
          },
          {
            "model_id": "duplicate-model",
            "display_name": "Duplicate Model Again",
            "input_cost_per_million": "3",
            "output_cost_per_million": "4",
            "cache_read_cost_per_million": "0.3",
            "cache_creation_cost_per_million": "0.4"
          }
        ]"#;

        let error = parse_pricing_items(json, "test pricing").expect_err("duplicate rejected");
        assert!(error.contains("duplicate model_id duplicate-model"));
    }
}
