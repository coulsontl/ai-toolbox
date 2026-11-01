//! Official ZCode accounts: snapshot, switch, log in.
//!
//! ZCode keeps its official login in `v2/credentials.json` — one flat map
//! naming every provider namespace it holds tokens for, with
//! `oauth:active_provider` deciding which one is live. There is no per-account
//! directory to point at, so an account *is* a copy of that file: capturing one
//! means reading the file, applying one means writing it back. Nothing is
//! decrypted on the way through, which is what keeps a snapshot byte-identical
//! to what ZCode wrote.
//!
//! Two files matter, and only one of them usually does:
//!
//! - `v2/credentials.json`, always.
//! - `v2/config.json`, only for installations that have not migrated to
//!   `v2/provider_config.json` yet. ZCode stops reading `config.json` entirely
//!   once the new registry exists, so a snapshot of it is taken — and restored
//!   — only when it is still the live source of truth.
//!
//! Switching also realigns `setting.json`'s `providerFamilyDomain`. That field
//! is the desktop's "which family am I signed into" marker; leaving it pointing
//! at the family the previous account belonged to makes the sidebar report the
//! wrong plan.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::Local;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::Mutex as AsyncMutex;

use crate::db::helpers::{db_delete, db_get, db_list, db_max_i64, db_put, db_update_applied_status};
use crate::db::schema::{Cli, DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;

use super::adapter;
use super::commands::resolve_zcode_root_dir;
use super::constants::*;
use super::credential_cipher::{self, ZcodeLoginIdentity};
use super::oauth_login::{self, ZcodeLoginMaterial, ZcodeOauthFlow};
use super::types::{ZcodeOfficialAccountApplyResult, ZcodeOfficialAccountContent};

/// Id of the entry that mirrors the live login rather than a stored row.
pub const LOCAL_ACCOUNT_ID: &str = crate::coding::local_bridge::LOCAL_CONFIG_ID;

/// Serializes read-modify-write passes over the live credential files.
///
/// Switching and logging in both read the live file and write it back; letting
/// two of them interleave would drop whichever read the older document.
static LIVE_WRITE_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

/// Set while a login flow is waiting, so the page can abandon it. Reset when a
/// flow starts, which is what keeps a cancel from killing the next attempt.
static LOGIN_CANCELLED: AtomicBool = AtomicBool::new(false);

fn live_credentials_path(root_dir: &Path) -> PathBuf {
    root_dir.join(ZCODE_CREDENTIALS_RELATIVE_PATH)
}

fn legacy_config_path(root_dir: &Path) -> PathBuf {
    root_dir.join(ZCODE_LEGACY_CONFIG_RELATIVE_PATH)
}

/// Whether ZCode has migrated past the legacy `config.json` provider map.
fn is_new_generation(root_dir: &Path) -> bool {
    root_dir.join(ZCODE_PROVIDER_CONFIG_RELATIVE_PATH).exists()
}

fn read_json_object(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn read_optional_text(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .filter(|text| !text.trim().is_empty())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).unwrap_or_default() + "\n";
    super::commands::write_text_atomic(path, &body)
}

fn now() -> String {
    Local::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// Reading the live login
// ---------------------------------------------------------------------------

/// The live credentials document, decrypted only far enough to identify it.
struct LiveLogin {
    credentials: Value,
    identity: ZcodeLoginIdentity,
    logged_in: bool,
}

fn read_live_login(root_dir: &Path) -> LiveLogin {
    let credentials = read_json_object(&live_credentials_path(root_dir));
    let secret = credential_cipher::credential_secret(&home_dir());
    LiveLogin {
        identity: credential_cipher::read_login_identity(&credentials, &secret),
        logged_in: credential_cipher::is_logged_in(&credentials),
        credentials,
    }
}

/// The directory the credential key is derived from — the user's home, not the
/// data root, because that is what ZCode derives from.
fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn snapshot_name(identity: &ZcodeLoginIdentity, provider: &str) -> String {
    credential_cipher::identity_label(identity)
        .unwrap_or_else(|| provider.to_string())
}

/// Whether a stored account describes the same login the live file holds.
///
/// The snapshot itself is compared first, byte for byte. That is the one check
/// that still works when nothing can be decrypted — and it is the case that
/// needs it most, because an identity that cannot be read would otherwise make
/// every "save current login" add another row for the same login.
///
/// Beyond that, matching runs strongest signal first: the provider's user id,
/// then the email, then the provider namespace alone.
fn same_login(
    content: &ZcodeOfficialAccountContent,
    identity: &ZcodeLoginIdentity,
    live_credentials: &Value,
) -> bool {
    if content.credentials_snapshot == live_credentials.to_string() {
        return true;
    }
    identity_matches(
        &content.provider_id,
        content.account_id.as_deref(),
        content.email.as_deref(),
        identity,
    )
}

/// The identity half of [`same_login`], for callers holding only the fields.
///
/// A stored account and the live login are the same login when the provider
/// namespace agrees and the strongest shared signal agrees with it: the
/// provider's user id, else the email, else nothing more to compare.
fn identity_matches(
    provider_id: &str,
    account_id: Option<&str>,
    email: Option<&str>,
    identity: &ZcodeLoginIdentity,
) -> bool {
    if !identity.has_signal() || provider_id != identity.provider {
        return false;
    }
    if let (Some(stored), Some(live)) = (account_id, identity.user_id.as_deref()) {
        return stored == live;
    }
    if let (Some(stored), Some(live)) = (email, identity.email.as_deref()) {
        return stored.eq_ignore_ascii_case(live);
    }
    // Only the namespace is known on both sides. Two logins into the same
    // provider that cannot be told apart are not worth listing twice.
    true
}

fn build_content(
    credentials: &Value,
    config_snapshot: Option<String>,
    identity: &ZcodeLoginIdentity,
    provider: &str,
    name: String,
    existing: Option<&ZcodeOfficialAccountContent>,
) -> ZcodeOfficialAccountContent {
    let timestamp = now();
    ZcodeOfficialAccountContent {
        provider_id: provider.to_string(),
        name,
        kind: "oauth".to_string(),
        email: identity.email.clone(),
        account_id: identity.user_id.clone(),
        credentials_snapshot: credentials.to_string(),
        config_snapshot,
        sort_index: existing.and_then(|record| record.sort_index),
        is_applied: existing.map(|record| record.is_applied).unwrap_or(false),
        created_at: existing
            .map(|record| record.created_at.clone())
            .unwrap_or_else(|| timestamp.clone()),
        updated_at: timestamp,
    }
}

// ---------------------------------------------------------------------------
// Database access
// ---------------------------------------------------------------------------

fn account_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::created_at(OrderDirection::Asc),
    ]))
}

fn list_stored_accounts(db: &SqliteDbState) -> Result<Vec<(String, ZcodeOfficialAccountContent)>, String> {
    let order = account_order()?;
    db.with_conn(|conn| {
        Ok(db_list(conn, DbTable::CliOfficialAccount(Cli::Zcode), Some(&order))?
            .into_iter()
            .map(|value| {
                let content = adapter::from_db_value_official_account(value.clone());
                let id = value
                    .get("id")
                    .and_then(|id| id.as_str())
                    .unwrap_or_default()
                    .to_string();
                (id, content)
            })
            .collect())
    })
}

fn load_stored_account(
    db: &SqliteDbState,
    account_id: &str,
) -> Result<ZcodeOfficialAccountContent, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Zcode), account_id))?
        .map(adapter::from_db_value_official_account)
        .ok_or_else(|| format!("Official account '{account_id}' not found"))
}

fn next_sort_index(db: &SqliteDbState) -> Result<i64, String> {
    let field = JsonFieldPath::new("sort_index")?;
    db.with_conn(|conn| db_max_i64(conn, DbTable::CliOfficialAccount(Cli::Zcode), &field))
        .map(|max| max.unwrap_or(-1) + 1)
}

fn store_account(
    db: &SqliteDbState,
    account_id: &str,
    content: &ZcodeOfficialAccountContent,
) -> Result<(), String> {
    let payload = adapter::to_db_value_official_account(content);
    db.with_conn(|conn| db_put(conn, DbTable::CliOfficialAccount(Cli::Zcode), account_id, &payload))
}

/// Saves the live login, replacing any stored snapshot of the same login.
///
/// Recognising an existing row matters more than it looks: re-saving the same
/// login would otherwise grow the list by one every time, and every copy would
/// claim to be "the" snapshot of an identical identity.
fn capture_live_login(
    db: &SqliteDbState,
    root_dir: &Path,
    live: &LiveLogin,
) -> Result<(String, ZcodeOfficialAccountContent), String> {
    if !live.logged_in {
        return Err("ZCode is not logged in, so there is nothing to save".to_string());
    }

    let provider = live.identity.provider.clone();
    let name = snapshot_name(&live.identity, &provider);
    let config_snapshot = legacy_config_snapshot(root_dir);

    let stored = list_stored_accounts(db)?;
    let existing = stored
        .iter()
        .find(|(_, content)| same_login(content, &live.identity, &live.credentials));

    let (account_id, previous) = match existing {
        Some((id, content)) => (id.clone(), Some(content)),
        None => (uuid::Uuid::new_v4().to_string(), None),
    };

    let mut content = build_content(
        &live.credentials,
        config_snapshot,
        &live.identity,
        &provider,
        name,
        previous,
    );
    if content.sort_index.is_none() {
        content.sort_index = Some(next_sort_index(db)?);
    }

    store_account(db, &account_id, &content)?;
    Ok((account_id, content))
}

/// `config.json` as a snapshot, but only while ZCode still reads it.
fn legacy_config_snapshot(root_dir: &Path) -> Option<String> {
    if is_new_generation(root_dir) {
        return None;
    }
    read_optional_text(&legacy_config_path(root_dir))
}

// ---------------------------------------------------------------------------
// Writing the live login
// ---------------------------------------------------------------------------

/// Writes a credentials document to the live path.
fn write_live_credentials(root_dir: &Path, credentials: &Value) -> Result<(), String> {
    write_json(&live_credentials_path(root_dir), credentials)
}

/// Points `setting.json` at the family the newly applied account belongs to.
///
/// Best-effort on purpose: the field drives plan display, not authentication,
/// and a missing or unreadable `setting.json` is a normal state on a machine
/// that has never opened the desktop app.
fn align_provider_family_domain(provider: &str) {
    if provider.is_empty() {
        return;
    }
    let Some(path) = super::commands::zcode_setting_path() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.get("providerFamilyDomain").and_then(|v| v.as_str()) == Some(provider) {
        return;
    }
    object.insert(
        "providerFamilyDomain".to_string(),
        Value::String(provider.to_string()),
    );
    object.insert(
        "providerFamilyDomainUpdatedAt".to_string(),
        Value::from(Local::now().timestamp_millis()),
    );
    let _ = write_json(&path, &value);
}

/// Installs a snapshot as the live login.
///
/// Runs under [`LIVE_WRITE_LOCK`].
async fn apply_snapshot(
    db: &SqliteDbState,
    app: &tauri::AppHandle,
    root_dir: &Path,
    account_id: &str,
    content: &ZcodeOfficialAccountContent,
) -> Result<(), String> {
    let credentials: Value = serde_json::from_str(&content.credentials_snapshot)
        .map_err(|error| format!("Official account snapshot is not valid JSON: {error}"))?;
    if !credentials.is_object() {
        return Err("Official account snapshot is not a credentials map".to_string());
    }

    // The legacy provider map is only written while ZCode reads it; restoring
    // it on a migrated installation would hand back a file nothing looks at.
    if let Some(config_snapshot) = content.config_snapshot.as_deref() {
        if !is_new_generation(root_dir) {
            let config: Value = serde_json::from_str(config_snapshot)
                .map_err(|error| format!("Legacy config snapshot is not valid JSON: {error}"))?;
            write_json(&legacy_config_path(root_dir), &config)?;
        }
    }

    write_live_credentials(root_dir, &credentials)?;
    align_provider_family_domain(&content.provider_id);

    let timestamp = now();
    db.with_conn_mut(|conn| {
        db_update_applied_status(
            conn,
            DbTable::CliOfficialAccount(Cli::Zcode),
            Some(account_id),
            &timestamp,
        )
    })?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Every saved account, plus the live login when it has not been saved.
///
/// The virtual entry is how the page stays honest: the live `credentials.json`
/// is the truth about what is applied, and an account the user just logged into
/// through ZCode itself belongs in the list even though AI Toolbox never
/// captured it.
#[tauri::command]
pub async fn list_zcode_official_accounts(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<super::types::ZcodeOfficialAccount>, String> {
    let db = state.db();
    let root_dir = resolve_zcode_root_dir(&db)?;
    let live = read_live_login(&root_dir);

    let stored = list_stored_accounts(&db)?;
    let live_is_stored = stored
        .iter()
        .any(|(_, content)| same_login(content, &live.identity, &live.credentials));

    let mut accounts: Vec<super::types::ZcodeOfficialAccount> = stored
        .into_iter()
        .map(|(id, content)| content.into_api(id, false))
        .collect();

    if live.logged_in && !live_is_stored {
        let provider = live.identity.provider.clone();
        let content = build_content(
            &live.credentials,
            None,
            &live.identity,
            &provider,
            snapshot_name(&live.identity, &provider),
            None,
        );
        accounts.insert(0, content.into_api(LOCAL_ACCOUNT_ID.to_string(), true));
    }

    // The live file decides which account is running, not the stored flag.
    //
    // That flag is only written by the switch command, so it says nothing about
    // the two other ways the live login changes: saving the current login never
    // applies anything, and a login performed in ZCode itself bypasses this app
    // entirely. Deriving the mark here also clears it when the live login moves
    // elsewhere, which is what keeps exactly one row marked.
    for account in accounts.iter_mut() {
        account.is_applied = live.logged_in
            && identity_matches(
                &account.provider_id,
                account.account_id.as_deref(),
                account.email.as_deref(),
                &live.identity,
            );
    }

    Ok(accounts)
}

/// Saves the current login so it can be switched back to later.
#[tauri::command]
pub async fn save_zcode_official_local_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<super::types::ZcodeOfficialAccount, String> {
    let db = state.db();
    let _guard = LIVE_WRITE_LOCK.lock().await;
    let root_dir = resolve_zcode_root_dir(&db)?;
    let live = read_live_login(&root_dir);
    let (account_id, content) = capture_live_login(&db, &root_dir, &live)?;
    let _ = app.emit("config-changed", "window");
    Ok(content.into_api(account_id, false))
}

/// Switches ZCode to a saved account.
///
/// The login being replaced is saved first. Losing it would mean re-running the
/// whole browser flow to get back, and the user has no way to know it was about
/// to be discarded.
#[tauri::command]
pub async fn apply_zcode_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<ZcodeOfficialAccountApplyResult, String> {
    let db = state.db();
    let _guard = LIVE_WRITE_LOCK.lock().await;
    let root_dir = resolve_zcode_root_dir(&db)?;

    if account_id == LOCAL_ACCOUNT_ID {
        return Err("The live login is already applied".to_string());
    }

    let target = load_stored_account(&db, &account_id)?;

    // Capture before overwriting, and only when the outgoing login is neither
    // the target nor already stored.
    let live = read_live_login(&root_dir);
    let mut preserved_as = None;
    if live.logged_in
        && !same_login(&target, &live.identity, &live.credentials)
        && !list_stored_accounts(&db)?
            .iter()
            .any(|(_, content)| same_login(content, &live.identity, &live.credentials))
    {
        if let Ok((_, content)) = capture_live_login(&db, &root_dir, &live) {
            preserved_as = Some(content.name);
        }
    }

    apply_snapshot(&db, &app, &root_dir, &account_id, &target).await?;

    Ok(ZcodeOfficialAccountApplyResult { preserved_as })
}

/// Forgets a saved account. The live login is left alone: deleting a row is not
/// a log-out, and the entry that mirrors the live login reappears on its own.
#[tauri::command]
pub async fn delete_zcode_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
    if account_id == LOCAL_ACCOUNT_ID {
        return Err("The live login cannot be deleted".to_string());
    }
    db.with_conn(|conn| db_delete(conn, DbTable::CliOfficialAccount(Cli::Zcode), &account_id))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Runs a browser login and applies what it returns.
///
/// The command resolves when the flow does, so the page can hold a modal open
/// on the promise and abandon it through [`cancel_zcode_official_account_oauth`].
#[tauri::command]
pub async fn start_zcode_official_account_oauth(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<super::types::ZcodeOfficialAccount, String> {
    let db = state.db();
    if !oauth_login::is_supported_provider(&provider_id) {
        return Err(format!("Unsupported ZCode login provider: {provider_id}"));
    }

    let root_dir = resolve_zcode_root_dir(&db)?;
    let device_mid = oauth_login::read_device_mid(&root_dir);

    LOGIN_CANCELLED.store(false, Ordering::SeqCst);
    let flow = oauth_login::start_flow(&db, &provider_id, device_mid.as_deref()).await?;
    open_in_browser(&flow.authorize_url)?;

    let material = wait_for_login(&db, &flow, device_mid.as_deref()).await?;
    apply_login(&db, &app, &root_dir, &provider_id, &material).await
}

/// Polls until the browser hands the flow back.
async fn wait_for_login(
    db: &SqliteDbState,
    flow: &ZcodeOauthFlow,
    device_mid: Option<&str>,
) -> Result<ZcodeLoginMaterial, String> {
    // The flow's own expiry is the real deadline; the local timeout is a
    // backstop for a flow that outlives it.
    let lifetime = Duration::from_secs(flow.lifetime_seconds.max(1));
    let deadline = Instant::now() + lifetime.min(oauth_login::FLOW_TIMEOUT);
    let interval = Duration::from_secs(flow.interval_seconds.max(1));

    loop {
        if LOGIN_CANCELLED.load(Ordering::SeqCst) {
            return Err("ZCode login was cancelled".to_string());
        }
        if Instant::now() >= deadline {
            return Err("ZCode login timed out waiting for the browser".to_string());
        }
        tokio::time::sleep(interval).await;
        match oauth_login::poll_flow_once(db, flow, device_mid).await? {
            oauth_login::ZcodePollOutcome::Pending => continue,
            oauth_login::ZcodePollOutcome::Ready(material) => return Ok(material),
        }
    }
}

/// Folds a completed login into the live credentials and saves it.
async fn apply_login(
    db: &SqliteDbState,
    app: &tauri::AppHandle,
    root_dir: &Path,
    provider: &str,
    material: &ZcodeLoginMaterial,
) -> Result<super::types::ZcodeOfficialAccount, String> {
    let _guard = LIVE_WRITE_LOCK.lock().await;

    let mut credentials = read_json_object(&live_credentials_path(root_dir));
    let Some(object) = credentials.as_object_mut() else {
        return Err("Live credentials file is not a JSON object".to_string());
    };

    // Merge rather than replace, matching ZCode's own repo: the document is
    // shared by every provider namespace, and `account-provider:*` entries for
    // a coding-plan connection have no business being dropped by a login.
    for (key, value) in oauth_login::credentials_from_material(provider, material)
        .as_object()
        .cloned()
        .unwrap_or_default()
    {
        object.insert(key, value);
    }
    // ZCode deletes the key outright when a login carries no refresh token,
    // rather than storing an empty one.
    if material.refresh_token.is_none() {
        object.remove(&format!("oauth:{provider}:refresh_token"));
    }

    write_live_credentials(root_dir, &credentials)?;
    align_provider_family_domain(provider);

    let live = LiveLogin {
        identity: credential_cipher::read_login_identity(
            &credentials,
            &credential_cipher::credential_secret(&home_dir()),
        ),
        logged_in: true,
        credentials,
    };
    let (account_id, content) = capture_live_login(db, root_dir, &live)?;

    // The login just wrote the live file, so this account is the applied one.
    let timestamp = now();
    db.with_conn_mut(|conn| {
        db_update_applied_status(
            conn,
            DbTable::CliOfficialAccount(Cli::Zcode),
            Some(&account_id),
            &timestamp,
        )
    })?;

    let _ = app.emit("config-changed", "window");
    Ok(content.into_api(account_id, false))
}

/// Abandons a login flow that is waiting on the browser.
#[tauri::command]
pub async fn cancel_zcode_official_account_oauth() -> Result<(), String> {
    LOGIN_CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}

fn open_in_browser(url: &str) -> Result<(), String> {
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|error| format!("Failed to open the browser for ZCode login: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(provider: &str, user_id: Option<&str>, email: Option<&str>) -> ZcodeLoginIdentity {
        ZcodeLoginIdentity {
            provider: provider.to_string(),
            user_id: user_id.map(str::to_string),
            email: email.map(str::to_string),
            username: None,
        }
    }

    fn content(provider: &str, user_id: Option<&str>, email: Option<&str>) -> ZcodeOfficialAccountContent {
        ZcodeOfficialAccountContent {
            provider_id: provider.to_string(),
            name: "Account".to_string(),
            kind: "oauth".to_string(),
            email: email.map(str::to_string),
            account_id: user_id.map(str::to_string),
            credentials_snapshot: "{}".to_string(),
            config_snapshot: None,
            sort_index: Some(0),
            is_applied: false,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// Stands in for a live credentials file. Deliberately not the empty
    /// object: `content` stores `{}` as its snapshot, and the byte-for-byte
    /// comparison would otherwise match everything.
    fn live() -> Value {
        json!({ "oauth:active_provider": "unrelated" })
    }

    #[test]
    fn an_identical_snapshot_matches_before_anything_is_decrypted() {
        let mut stored = content("", None, None);
        stored.credentials_snapshot = live().to_string();
        assert!(same_login(&stored, &identity("", None, None), &live()));
    }

    #[test]
    fn the_user_id_decides_when_both_sides_have_one() {
        assert!(same_login(
            &content("zai", Some("u-1"), Some("a@example.com")),
            &identity("zai", Some("u-1"), Some("a@example.com")),
            &live(),
        ));
        assert!(!same_login(
            &content("zai", Some("u-1"), Some("a@example.com")),
            &identity("zai", Some("u-2"), Some("a@example.com")),
            &live(),
        ));
    }

    #[test]
    fn the_email_decides_when_the_user_id_is_missing() {
        assert!(same_login(
            &content("zai", None, Some("A@Example.com")),
            &identity("zai", None, Some("a@example.com")),
            &live(),
        ));
        assert!(!same_login(
            &content("zai", None, Some("a@example.com")),
            &identity("zai", None, Some("b@example.com")),
            &live(),
        ));
    }

    #[test]
    fn a_different_provider_is_never_the_same_login() {
        assert!(!same_login(
            &content("bigmodel", Some("u-1"), None),
            &identity("zai", Some("u-1"), None),
            &live(),
        ));
    }

    #[test]
    fn an_identity_with_no_signal_matches_nothing() {
        assert!(!same_login(
            &content("zai", None, None),
            &identity("", None, None),
            &live(),
        ));
    }

    #[test]
    fn an_unreadable_identity_still_matches_only_its_own_namespace() {
        // Only the provider survived decryption, so it is all that can be
        // compared — and it must still be compared.
        assert!(same_login(
            &content("zai", Some("u-1"), None),
            &identity("zai", None, None),
            &live(),
        ));
    }
}
