use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::{
    RelayAccessKey, RelayChannel, RelayCheckinRecord, RelayCheckinSchedule, RelayModel,
    RelayPlugin, RelayPluginAccount, RelayPluginProvider,
};
use crate::coding::db_id::{db_clean_id, db_new_id};
use crate::db::helpers::{db_delete, db_get, db_list, db_prune_before, db_put};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec, RelayKind};
use crate::SqliteDbState;

/// Everything a channel read returns: the channel plus its models.
///
/// Models are their own rows (a channel can list hundreds), so the list view
/// reads them separately and the channel DTO only carries the count.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelayChannelDto {
    #[serde(flatten)]
    pub channel: RelayChannel,
    pub models: Vec<RelayModel>,
}

fn channel_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_integer("created_at", OrderDirection::Asc)?,
    ]))
}

fn model_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
    ]))
}

fn record_order(field: &str) -> Result<OrderSpec, String> {
    Ok(OrderSpec::single(OrderField::json_integer(
        field,
        OrderDirection::Desc,
    )?))
}

fn normalize_channel(mut channel: RelayChannel) -> RelayChannel {
    channel.id = db_clean_id(&channel.id);
    channel
}

fn normalize_model(mut model: RelayModel) -> RelayModel {
    model.id = db_clean_id(&model.id);
    model.channel_id = db_clean_id(&model.channel_id);
    model
}

fn parse<T: serde::de::DeserializeOwned>(value: Value, label: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Failed to parse {label}: {error}"))
}

// ---- channels ----------------------------------------------------------------

pub async fn list_channels(state: &SqliteDbState) -> Result<Vec<RelayChannel>, String> {
    let order = channel_order()?;
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::Channel), Some(&order))?
            .into_iter()
            .map(|value| parse::<RelayChannel>(value, "2API channel").map(normalize_channel))
            .collect()
    })
}

pub async fn get_channel(
    state: &SqliteDbState,
    id: &str,
) -> Result<Option<RelayChannel>, String> {
    state.with_conn(|conn| {
        db_get(conn, DbTable::RelayEntity(RelayKind::Channel), id)?
            .map(|value| parse::<RelayChannel>(value, "2API channel").map(normalize_channel))
            .transpose()
    })
}

pub async fn upsert_channel(
    state: &SqliteDbState,
    mut channel: RelayChannel,
) -> Result<RelayChannel, String> {
    let now = super::types::now_ms();
    state.with_conn(|conn| {
        if channel.id.trim().is_empty() {
            channel.id = db_new_id();
            channel.created_at = now;
        } else {
            // Preserve the original creation time across updates.
            let created = db_get(conn, DbTable::RelayEntity(RelayKind::Channel), &channel.id)?
                .and_then(|value| value.get("created_at").and_then(Value::as_i64));
            channel.created_at = created.unwrap_or(now);
        }
        channel.updated_at = now;
        let payload = serde_json::to_value(&channel)
            .map_err(|error| format!("Failed to serialize 2API channel: {error}"))?;
        db_put(conn, DbTable::RelayEntity(RelayKind::Channel), &channel.id, &payload)?;
        Ok(channel)
    })
}

pub async fn delete_channel(state: &SqliteDbState, id: &str) -> Result<(), String> {
    state.with_conn(|conn| {
        // Models belong to their channel; deleting one without its models
        // would leave rows nothing can reach.
        let models = db_list(conn, DbTable::RelayEntity(RelayKind::Model), None)?;
        for value in models {
            let belongs = value.get("channel_id").and_then(Value::as_str) == Some(id);
            if belongs {
                if let Some(model_id) = value.get("id").and_then(Value::as_str) {
                    db_delete(conn, DbTable::RelayEntity(RelayKind::Model), model_id)?;
                }
            }
        }
        db_delete(conn, DbTable::RelayEntity(RelayKind::Channel), id).map(|_| ())
    })
}

// ---- models ------------------------------------------------------------------

pub async fn list_models(
    state: &SqliteDbState,
    channel_id: Option<&str>,
) -> Result<Vec<RelayModel>, String> {
    let order = model_order()?;
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::Model), Some(&order))?
            .into_iter()
            .map(|value| parse::<RelayModel>(value, "2API model").map(normalize_model))
            .filter(|model| match (model, channel_id) {
                (Ok(model), Some(channel_id)) => model.channel_id == channel_id,
                (_, None) => true,
                (Err(_), Some(_)) => true,
            })
            .collect()
    })
}

/// Replace every model of a channel in one pass.
///
/// A refresh can reorder, rename or drop models; writing row by row would leave
/// half of the old list behind if it stopped midway.
pub async fn replace_channel_models(
    state: &SqliteDbState,
    channel_id: &str,
    mut models: Vec<RelayModel>,
) -> Result<Vec<RelayModel>, String> {
    state.with_conn(|conn| {
        let existing = db_list(conn, DbTable::RelayEntity(RelayKind::Model), None)?;
        for value in existing {
            if value.get("channel_id").and_then(Value::as_str) != Some(channel_id) {
                continue;
            }
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                db_delete(conn, DbTable::RelayEntity(RelayKind::Model), id)?;
            }
        }
        for (index, model) in models.iter_mut().enumerate() {
            if model.id.trim().is_empty() {
                model.id = db_new_id();
            }
            model.channel_id = channel_id.to_string();
            model.sort_index = index as i64;
            let payload = serde_json::to_value(&*model)
                .map_err(|error| format!("Failed to serialize 2API model: {error}"))?;
            db_put(conn, DbTable::RelayEntity(RelayKind::Model), &model.id, &payload)?;
        }
        Ok(models)
    })
}

// ---- plugins -----------------------------------------------------------------

pub async fn list_plugins(state: &SqliteDbState) -> Result<Vec<RelayPlugin>, String> {
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::Plugin), None)?
            .into_iter()
            .map(|value| parse::<RelayPlugin>(value, "2API plugin"))
            .collect()
    })
}

pub async fn upsert_plugin(
    state: &SqliteDbState,
    mut plugin: RelayPlugin,
) -> Result<RelayPlugin, String> {
    state.with_conn(|conn| {
        if plugin.id.trim().is_empty() {
            plugin.id = db_new_id();
            plugin.installed_at = super::types::now_ms();
        }
        let payload = serde_json::to_value(&plugin)
            .map_err(|error| format!("Failed to serialize 2API plugin: {error}"))?;
        db_put(conn, DbTable::RelayEntity(RelayKind::Plugin), &plugin.id, &payload)?;
        Ok(plugin)
    })
}

pub async fn delete_plugin(state: &SqliteDbState, id: &str) -> Result<(), String> {
    state.with_conn(|conn| {
        let providers = db_list(conn, DbTable::RelayEntity(RelayKind::PluginProvider), None)?;
        for value in providers {
            if value.get("plugin_id").and_then(Value::as_str) != Some(id) {
                continue;
            }
            if let Some(provider_row_id) = value.get("id").and_then(Value::as_str) {
                db_delete(conn, DbTable::RelayEntity(RelayKind::PluginProvider), provider_row_id)?;
            }
        }
        db_delete(conn, DbTable::RelayEntity(RelayKind::Plugin), id).map(|_| ())
    })
}

pub async fn list_plugin_providers(
    state: &SqliteDbState,
    plugin_id: Option<&str>,
) -> Result<Vec<RelayPluginProvider>, String> {
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::PluginProvider), None)?
            .into_iter()
            .map(|value| parse::<RelayPluginProvider>(value, "2API plugin provider"))
            .filter(|provider| match (provider, plugin_id) {
                (Ok(provider), Some(plugin_id)) => provider.plugin_id == plugin_id,
                (_, None) => true,
                (Err(_), Some(_)) => true,
            })
            .collect()
    })
}

pub async fn upsert_plugin_provider(
    state: &SqliteDbState,
    mut provider: RelayPluginProvider,
) -> Result<RelayPluginProvider, String> {
    state.with_conn(|conn| {
        if provider.id.trim().is_empty() {
            provider.id = db_new_id();
        }
        let payload = serde_json::to_value(&provider)
            .map_err(|error| format!("Failed to serialize 2API plugin provider: {error}"))?;
        db_put(conn, DbTable::RelayEntity(RelayKind::PluginProvider), &provider.id, &payload)?;
        Ok(provider)
    })
}

// ---- plugin accounts ---------------------------------------------------------

pub async fn list_plugin_accounts(
    state: &SqliteDbState,
    plugin_provider_id: Option<&str>,
) -> Result<Vec<RelayPluginAccount>, String> {
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::PluginAccount), None)?
            .into_iter()
            .map(|value| parse::<RelayPluginAccount>(value, "2API plugin account"))
            .filter(|account| match (account, plugin_provider_id) {
                (Ok(account), Some(id)) => account.plugin_provider_id == id,
                (_, None) => true,
                (Err(_), Some(_)) => true,
            })
            .collect()
    })
}

pub async fn upsert_plugin_account(
    state: &SqliteDbState,
    mut account: RelayPluginAccount,
) -> Result<RelayPluginAccount, String> {
    state.with_conn(|conn| {
        if account.id.trim().is_empty() {
            account.id = db_new_id();
        }
        let payload = serde_json::to_value(&account)
            .map_err(|error| format!("Failed to serialize 2API plugin account: {error}"))?;
        db_put(conn, DbTable::RelayEntity(RelayKind::PluginAccount), &account.id, &payload)?;
        Ok(account)
    })
}

pub async fn delete_plugin_account(state: &SqliteDbState, id: &str) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::RelayEntity(RelayKind::PluginAccount), id).map(|_| ()))
}

// ---- access keys -------------------------------------------------------------

pub async fn list_access_keys(state: &SqliteDbState) -> Result<Vec<RelayAccessKey>, String> {
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::AccessKey), None)?
            .into_iter()
            .map(|value| parse::<RelayAccessKey>(value, "2API access key"))
            .collect()
    })
}

pub async fn upsert_access_key(
    state: &SqliteDbState,
    mut key: RelayAccessKey,
) -> Result<RelayAccessKey, String> {
    state.with_conn(|conn| {
        if key.id.trim().is_empty() {
            key.id = db_new_id();
            key.created_at = super::types::now_ms();
        }
        let payload = serde_json::to_value(&key)
            .map_err(|error| format!("Failed to serialize 2API access key: {error}"))?;
        db_put(conn, DbTable::RelayEntity(RelayKind::AccessKey), &key.id, &payload)?;
        Ok(key)
    })
}

pub async fn delete_access_key(state: &SqliteDbState, id: &str) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::RelayEntity(RelayKind::AccessKey), id).map(|_| ()))
}

// ---- check-in ----------------------------------------------------------------

pub async fn list_checkin_schedules(
    state: &SqliteDbState,
) -> Result<Vec<RelayCheckinSchedule>, String> {
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::CheckinSchedule), None)?
            .into_iter()
            .map(|value| parse::<RelayCheckinSchedule>(value, "2API check-in schedule"))
            .collect()
    })
}

pub async fn upsert_checkin_schedule(
    state: &SqliteDbState,
    mut schedule: RelayCheckinSchedule,
) -> Result<RelayCheckinSchedule, String> {
    state.with_conn(|conn| {
        if schedule.id.trim().is_empty() {
            schedule.id = db_new_id();
        }
        let payload = serde_json::to_value(&schedule)
            .map_err(|error| format!("Failed to serialize 2API check-in schedule: {error}"))?;
        db_put(conn, DbTable::RelayEntity(RelayKind::CheckinSchedule), &schedule.id, &payload)?;
        Ok(schedule)
    })
}

pub async fn list_checkin_records(
    state: &SqliteDbState,
    limit: usize,
) -> Result<Vec<RelayCheckinRecord>, String> {
    let order = record_order("ts")?;
    state.with_conn(|conn| {
        db_list(conn, DbTable::RelayEntity(RelayKind::CheckinRecord), Some(&order))?
            .into_iter()
            .take(limit)
            .map(|value| parse::<RelayCheckinRecord>(value, "2API check-in record"))
            .collect()
    })
}

/// How long the check-in ledger is kept. Three months covers every question the
/// page is asked (this week's streak, last month's claims) while bounding a
/// table that is appended to once per account per day and never read in bulk.
pub const CHECKIN_RECORD_RETENTION_DAYS: i64 = 90;

pub async fn append_checkin_record(
    state: &SqliteDbState,
    mut record: RelayCheckinRecord,
) -> Result<RelayCheckinRecord, String> {
    state.with_conn(|conn| {
        if record.id.trim().is_empty() {
            record.id = db_new_id();
        }
        if record.ts == 0 {
            record.ts = super::types::now_ms();
        }
        let payload = serde_json::to_value(&record)
            .map_err(|error| format!("Failed to serialize 2API check-in record: {error}"))?;
        db_put(conn, DbTable::RelayEntity(RelayKind::CheckinRecord), &record.id, &payload)?;
        // Trim on append rather than on a timer: the ledger only grows by
        // appending, so that is exactly when trimming is worth doing, and no
        // background task has to be owned and cancelled with the app. A failed
        // prune is maintenance debt, not a reason to report the check-in as
        // failed -- the row is already written and the vendor already paid out.
        if let Err(error) = prune_checkin_records(conn) {
            log::warn!("Failed to prune 2API check-in records: {error}");
        }
        Ok(record)
    })
}

/// Drop ledger rows older than [`CHECKIN_RECORD_RETENTION_DAYS`], inside the
/// check-in scope only.
fn prune_checkin_records(conn: &rusqlite::Connection) -> Result<usize, String> {
    let cutoff = super::types::now_ms() - CHECKIN_RECORD_RETENTION_DAYS * 24 * 60 * 60 * 1000;
    let field = JsonFieldPath::new("ts")?;
    db_prune_before(
        conn,
        DbTable::RelayEntity(RelayKind::CheckinRecord),
        &field,
        cutoff,
    )
}

#[allow(dead_code)]
fn _json_path_marker(_: JsonFieldPath) {}
