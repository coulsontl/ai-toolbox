use serde::Deserialize;
use tauri::State;

use super::store;
use super::types::{
    RelayAccessKey, RelayChannel, RelayCheckinRecord, RelayCheckinSchedule, RelayModel,
    RelayPlugin, RelayPluginAccount, RelayPluginProvider,
};
use crate::SqliteDbState;

const DEFAULT_CHECKIN_RECORD_LIMIT: usize = 200;

#[tauri::command]
pub async fn relay_list_channels(
    state: State<'_, SqliteDbState>,
) -> Result<Vec<RelayChannel>, String> {
    store::list_channels(&state).await
}

#[tauri::command]
pub async fn relay_upsert_channel(
    state: State<'_, SqliteDbState>,
    channel: RelayChannel,
) -> Result<RelayChannel, String> {
    store::upsert_channel(&state, channel).await
}

#[tauri::command]
pub async fn relay_delete_channel(
    state: State<'_, SqliteDbState>,
    id: String,
) -> Result<(), String> {
    store::delete_channel(&state, &id).await
}

#[derive(Debug, Deserialize)]
pub struct ListModelsInput {
    #[serde(default)]
    pub channel_id: Option<String>,
}

#[tauri::command]
pub async fn relay_list_models(
    state: State<'_, SqliteDbState>,
    input: Option<ListModelsInput>,
) -> Result<Vec<RelayModel>, String> {
    let channel_id = input.and_then(|input| input.channel_id);
    store::list_models(&state, channel_id.as_deref()).await
}

#[tauri::command]
pub async fn relay_replace_channel_models(
    state: State<'_, SqliteDbState>,
    channel_id: String,
    models: Vec<RelayModel>,
) -> Result<Vec<RelayModel>, String> {
    store::replace_channel_models(&state, &channel_id, models).await
}

#[tauri::command]
pub async fn relay_list_plugins(
    state: State<'_, SqliteDbState>,
) -> Result<Vec<RelayPlugin>, String> {
    store::list_plugins(&state).await
}

#[tauri::command]
pub async fn relay_upsert_plugin(
    state: State<'_, SqliteDbState>,
    plugin: RelayPlugin,
) -> Result<RelayPlugin, String> {
    store::upsert_plugin(&state, plugin).await
}

#[tauri::command]
pub async fn relay_delete_plugin(
    state: State<'_, SqliteDbState>,
    id: String,
) -> Result<(), String> {
    store::delete_plugin(&state, &id).await
}

#[derive(Debug, Deserialize)]
pub struct ListPluginProvidersInput {
    #[serde(default)]
    pub plugin_id: Option<String>,
}

#[tauri::command]
pub async fn relay_list_plugin_providers(
    state: State<'_, SqliteDbState>,
    input: Option<ListPluginProvidersInput>,
) -> Result<Vec<RelayPluginProvider>, String> {
    let plugin_id = input.and_then(|input| input.plugin_id);
    store::list_plugin_providers(&state, plugin_id.as_deref()).await
}

#[tauri::command]
pub async fn relay_upsert_plugin_provider(
    state: State<'_, SqliteDbState>,
    provider: RelayPluginProvider,
) -> Result<RelayPluginProvider, String> {
    store::upsert_plugin_provider(&state, provider).await
}

#[derive(Debug, Deserialize)]
pub struct ListPluginAccountsInput {
    #[serde(default)]
    pub plugin_provider_id: Option<String>,
}

#[tauri::command]
pub async fn relay_list_plugin_accounts(
    state: State<'_, SqliteDbState>,
    input: Option<ListPluginAccountsInput>,
) -> Result<Vec<RelayPluginAccount>, String> {
    let plugin_provider_id = input.and_then(|input| input.plugin_provider_id);
    store::list_plugin_accounts(&state, plugin_provider_id.as_deref()).await
}

#[tauri::command]
pub async fn relay_upsert_plugin_account(
    state: State<'_, SqliteDbState>,
    account: RelayPluginAccount,
) -> Result<RelayPluginAccount, String> {
    store::upsert_plugin_account(&state, account).await
}

#[tauri::command]
pub async fn relay_delete_plugin_account(
    state: State<'_, SqliteDbState>,
    id: String,
) -> Result<(), String> {
    store::delete_plugin_account(&state, &id).await
}

#[tauri::command]
pub async fn relay_list_access_keys(
    state: State<'_, SqliteDbState>,
) -> Result<Vec<RelayAccessKey>, String> {
    store::list_access_keys(&state).await
}

#[tauri::command]
pub async fn relay_upsert_access_key(
    state: State<'_, SqliteDbState>,
    key: RelayAccessKey,
) -> Result<RelayAccessKey, String> {
    store::upsert_access_key(&state, key).await
}

#[tauri::command]
pub async fn relay_delete_access_key(
    state: State<'_, SqliteDbState>,
    id: String,
) -> Result<(), String> {
    store::delete_access_key(&state, &id).await
}

#[tauri::command]
pub async fn relay_list_checkin_schedules(
    state: State<'_, SqliteDbState>,
) -> Result<Vec<RelayCheckinSchedule>, String> {
    store::list_checkin_schedules(&state).await
}

#[tauri::command]
pub async fn relay_upsert_checkin_schedule(
    state: State<'_, SqliteDbState>,
    schedule: RelayCheckinSchedule,
) -> Result<RelayCheckinSchedule, String> {
    store::upsert_checkin_schedule(&state, schedule).await
}

#[tauri::command]
pub async fn relay_list_checkin_records(
    state: State<'_, SqliteDbState>,
    limit: Option<usize>,
) -> Result<Vec<RelayCheckinRecord>, String> {
    let limit = limit.unwrap_or(DEFAULT_CHECKIN_RECORD_LIMIT);
    store::list_checkin_records(&state, limit).await
}
