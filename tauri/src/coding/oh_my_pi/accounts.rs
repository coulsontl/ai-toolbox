use super::types::{OmpCodexAccount, OmpCodexAccountUsage, OmpCodexAccountsResult};
use crate::coding::codex::official_accounts::fetch_usage_snapshot;
use crate::coding::runtime_location::{self, RuntimeLocationInfo, RuntimeLocationMode};
use crate::db::SqliteDbState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde_json::{json, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

// Source contract: @oh-my-pi/pi-ai 18.8.7, auth/sqlite-credential-store.ts
// AUTH_SCHEMA_VERSION=8, serializeCredential, resolveCredentialIdentityKey,
// #createAuthCredentialsTable, #createAuthChangeTrackingObjects; Codex profile
// hook: registry/oauth/openai-codex.ts (orgId=accountId, orgName=planType).
const AUTH_SCHEMA_VERSION: i64 = 8;
const PROVIDER: &str = "openai-codex";
const INACTIVE: &str = "ai-toolbox:inactive";
const MAX_IMPORT_BYTES: usize = 256 * 1024;
const DB_ERROR: &str =
    "Unable to access OMP credential database; close competing writers and retry.";
const SCHEMA_ERROR: &str = "Unsupported OMP credential schema (requires native schema 8, verified against OMP 18.8.7). Run OMP to initialize/update its database; AI Toolbox will not create or migrate it.";

fn failure(error: impl Into<String>) -> OmpCodexAccountsResult {
    OmpCodexAccountsResult {
        accounts: vec![],
        can_write: false,
        error: Some(error.into()),
    }
}

fn checked_root(location: &RuntimeLocationInfo, supplied: &str) -> Result<PathBuf, String> {
    // Never open Windows UNC SQLite files, including read-only: the host may
    // apply the wrong locking semantics to a live Linux WAL database.
    if location.mode == RuntimeLocationMode::WslDirect
        || supplied.starts_with("\\\\")
        || supplied.starts_with("//")
        || location.host_path.to_string_lossy().starts_with("\\\\")
    {
        return Err("OMP account management is unavailable for WSL/UNC runtimes. Use native OMP inside the selected distro.".into());
    }
    let requested = Path::new(supplied);
    if supplied.contains('\0')
        || !requested.is_absolute()
        || requested
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || requested != location.host_path
    {
        return Err("Account root must exactly match the selected OMP runtime root.".into());
    }
    fs::canonicalize(&location.host_path)
        .map_err(|_| "Selected OMP runtime root is unavailable.".into())
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn jwt(token: &str) -> Result<Value, String> {
    let mut parts = token.split('.');
    let (header, payload, signature) = (parts.next(), parts.next(), parts.next());
    if header.is_none_or(str::is_empty)
        || payload.is_none_or(str::is_empty)
        || signature.is_none_or(str::is_empty)
        || parts.next().is_some()
    {
        return Err("auth.json contains an invalid JWT structure.".into());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.unwrap())
        .map_err(|_| "auth.json contains an invalid JWT payload.")?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "auth.json contains an invalid JWT payload.")?;
    if !value.is_object() {
        return Err("auth.json JWT payload must be an object.".into());
    }
    Ok(value)
}

fn claim<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    payload
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get(key))
}

fn email(payload: &Value) -> Option<String> {
    nonempty(
        payload
            .get("https://api.openai.com/profile")
            .and_then(|p| p.get("email")),
    )
    .or_else(|| nonempty(payload.get("email")))
    .map(|s| s.to_lowercase())
}

fn identity(data: &Value) -> Option<String> {
    let account = nonempty(data.get("accountId"));
    let address = nonempty(data.get("email")).map(|s| s.to_lowercase());
    let org = nonempty(data.get("orgId"));
    let base = address
        .map(|s| format!("email:{s}"))
        .or_else(|| account.map(|s| format!("account:{s}")));
    match (base, org) {
        (Some(base), Some(org)) => Some(format!("{base}|org:{org}")),
        (Some(base), None) => Some(base),
        (None, Some(org)) => Some(format!("org:{org}")),
        _ => None,
    }
}

// Decoding validates structure, not signatures or subscription validity. No
// token exchange, refresh, credential-file reads, or raw parse errors escape.
fn parse_import(raw: &str) -> Result<Value, String> {
    if raw.len() > MAX_IMPORT_BYTES {
        return Err("auth.json exceeds the 256 KiB import limit.".into());
    }
    let input: Value =
        serde_json::from_str(raw).map_err(|_| "Import requires valid auth.json JSON.")?;
    if !input.is_object() {
        return Err("auth.json must be an object.".into());
    }
    let tokens = input.get("tokens").unwrap_or(&input);
    if !tokens.is_object() {
        return Err("auth.json tokens must be an object.".into());
    }
    let access = nonempty(tokens.get("access_token")).ok_or("auth.json requires access_token.")?;
    let refresh =
        nonempty(tokens.get("refresh_token")).ok_or("auth.json requires refresh_token.")?;
    let access_claims = jwt(&access)?;
    let id_claims = nonempty(tokens.get("id_token"))
        .map(|token| jwt(&token))
        .transpose()?;
    let expires = access_claims
        .get("exp")
        .and_then(Value::as_f64)
        .map(|seconds| seconds * 1000.0)
        .filter(|ms| ms.is_finite() && *ms > 0.0 && *ms <= 9_007_199_254_740_991.0)
        .ok_or("access_token requires a finite positive exp in milliseconds.")?;
    let access_id = nonempty(claim(&access_claims, "chatgpt_account_id"));
    let id_id = id_claims
        .as_ref()
        .and_then(|p| nonempty(claim(p, "chatgpt_account_id")));
    let file_id = nonempty(tokens.get("account_id"));
    let root_file_id = nonempty(input.get("account_id"));
    let account = access_id
        .as_ref()
        .or(id_id.as_ref())
        .ok_or("auth.json tokens contain no Codex account ID.")?;
    if [
        access_id.as_ref(),
        id_id.as_ref(),
        file_id.as_ref(),
        root_file_id.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|id| id != account)
    {
        return Err("auth.json contains conflicting account IDs.".into());
    }
    let address = email(&access_claims).or_else(|| id_claims.as_ref().and_then(email));
    let plan = nonempty(claim(&access_claims, "chatgpt_plan_type"))
        .or_else(|| {
            id_claims
                .as_ref()
                .and_then(|p| nonempty(claim(p, "chatgpt_plan_type")))
        })
        .map(|s| s.to_lowercase());
    let mut data = json!({"access": access, "refresh": refresh, "expires": expires, "accountId": account, "orgId": account});
    if let Some(address) = address {
        data["email"] = json!(address);
    }
    if let Some(plan) = plan {
        data["orgName"] = json!(plan);
    }
    Ok(data)
}

fn schema_supported(conn: &Connection) -> bool {
    let version = conn.query_row(
        "SELECT version FROM auth_schema_version WHERE id=1",
        [],
        |r| r.get::<_, i64>(0),
    );
    if version.ok() != Some(AUTH_SCHEMA_VERSION) {
        return false;
    }
    let columns = conn
        .prepare("PRAGMA table_info(auth_credentials)")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
        });
    let Ok(columns) = columns else {
        return false;
    };
    let expected = [
        ("id", "INTEGER", 1),
        ("provider", "TEXT", 0),
        ("credential_type", "TEXT", 0),
        ("data", "TEXT", 0),
        ("disabled_cause", "TEXT", 0),
        ("identity_key", "TEXT", 0),
        ("created_at", "INTEGER", 0),
        ("updated_at", "INTEGER", 0),
    ];
    if columns.len() != expected.len()
        || columns.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || !actual.1.eq_ignore_ascii_case(expected.1)
                || actual.2 != expected.2
        })
    {
        return false;
    }
    // External writes must trigger native auth-pool invalidation, not bypass it.
    conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND tbl_name='auth_credentials' AND name IN ('auth_change_revision_auth_credentials_insert','auth_change_revision_auth_credentials_update','auth_change_revision_auth_credentials_delete')", [], |r| r.get::<_, i64>(0)).ok() == Some(3)
        && conn.query_row("SELECT revision FROM auth_change_revision WHERE id=1", [], |r| r.get::<_, i64>(0)).is_ok()
}

fn list_accounts(conn: &Connection) -> Result<Vec<OmpCodexAccount>, String> {
    let mut stmt = conn.prepare("SELECT id,data,disabled_cause FROM auth_credentials WHERE provider=?1 AND credential_type='oauth' ORDER BY id")
        .map_err(|_| DB_ERROR)?;
    let rows = stmt
        .query_map([PROVIDER], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|_| DB_ERROR)?;
    rows.map(|row| {
        let (id, raw, disabled) = row.map_err(|_| DB_ERROR)?;
        let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        // Native disabled_cause is verbatim error text; do not leak it to UI.
        Ok(OmpCodexAccount {
            id: id.to_string(),
            email: nonempty(data.get("email")).map(|s| s.to_lowercase()),
            account_id: nonempty(data.get("accountId")),
            plan: nonempty(data.get("orgName")),
            expires_at: data
                .get("expires")
                .and_then(Value::as_f64)
                .filter(|ms| ms.is_finite() && *ms > 0.0),
            is_enabled: disabled.is_none(),
            disabled_cause: disabled.map(|cause| {
                if cause == INACTIVE {
                    INACTIVE.into()
                } else {
                    "native-disabled".into()
                }
            }),
        })
    })
    .collect()
}

fn import_account(conn: &mut Connection, raw: &str) -> Result<(), String> {
    let data = parse_import(raw)?;
    let key = identity(&data).ok_or("Unable to determine Codex identity.")?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| DB_ERROR)?;
    let matching = {
        let mut stmt = tx.prepare("SELECT id,identity_key,data FROM auth_credentials WHERE provider=?1 AND credential_type='oauth'").map_err(|_| DB_ERROR)?;
        let rows = stmt
            .query_map([PROVIDER], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|_| DB_ERROR)?;
        let mut matches = vec![];
        for row in rows {
            let (id, stored_key, raw) = row.map_err(|_| DB_ERROR)?;
            let stored_key = stored_key.filter(|s| !s.trim().is_empty()).or_else(|| {
                serde_json::from_str::<Value>(&raw)
                    .ok()
                    .and_then(|v| identity(&v))
            });
            if stored_key.as_deref() == Some(&key) {
                matches.push((id, raw));
            }
        }
        matches
    };
    if matching.len() > 1 {
        return Err(
            "Multiple native rows share this identity; resolve them in OMP before importing."
                .into(),
        );
    }
    if let Some((id, old_raw)) = matching.first() {
        let mut old: Value = serde_json::from_str(old_raw)
            .map_err(|_| "Stored credential payload is invalid; import refused.")?;
        let old_obj = old
            .as_object_mut()
            .ok_or("Stored credential payload is invalid; import refused.")?;
        old_obj.extend(match data {
            Value::Object(fields) => fields,
            _ => unreachable!("parsed imports are objects"),
        });
        tx.execute("UPDATE auth_credentials SET data=?1,identity_key=?2,disabled_cause=CASE WHEN disabled_cause IS NULL THEN NULL ELSE ?4 END,updated_at=CAST(strftime('%s','now') AS INTEGER) WHERE id=?3", params![old.to_string(), key, id, INACTIVE]).map_err(|_| DB_ERROR)?;
    } else {
        let has_accounts: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM auth_credentials WHERE provider=?1 AND credential_type='oauth')", [PROVIDER], |r| r.get(0)).map_err(|_| DB_ERROR)?;
        tx.execute("INSERT INTO auth_credentials(provider,credential_type,data,identity_key,disabled_cause,created_at,updated_at) VALUES (?1,'oauth',?2,?3,?4,CAST(strftime('%s','now') AS INTEGER),CAST(strftime('%s','now') AS INTEGER))", params![PROVIDER, data.to_string(), key, has_accounts.then_some(INACTIVE)]).map_err(|_| DB_ERROR)?;
    }
    tx.commit().map_err(|_| DB_ERROR.into())
}

fn selected_account_id(target: &str) -> Result<i64, String> {
    target
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| "Invalid Codex account selection.".into())
}

fn check_account_disabled(disabled: Option<&str>) -> Result<(), String> {
    if disabled.is_some_and(|cause| cause != INACTIVE) {
        return Err(
            "OMP disabled this credential; authenticate again in OMP before selecting it.".into(),
        );
    }
    Ok(())
}

fn switch_account(conn: &mut Connection, target: &str) -> Result<(), String> {
    let id = selected_account_id(target)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| DB_ERROR)?;
    let disabled = tx.query_row("SELECT disabled_cause FROM auth_credentials WHERE id=?1 AND provider=?2 AND credential_type='oauth'", params![id, PROVIDER], |r| r.get::<_, Option<String>>(0))
        .map_err(|error| if error == rusqlite::Error::QueryReturnedNoRows { "Codex account no longer exists." } else { DB_ERROR })?;
    check_account_disabled(disabled.as_deref())?;
    tx.execute("UPDATE auth_credentials SET disabled_cause=CASE WHEN id=?1 THEN NULL ELSE ?2 END,updated_at=CAST(strftime('%s','now') AS INTEGER) WHERE provider=?3 AND credential_type='oauth' AND (disabled_cause IS NULL OR id=?1)", params![id, INACTIVE, PROVIDER]).map_err(|_| DB_ERROR)?;
    tx.commit().map_err(|_| DB_ERROR.into())
}

fn broker_configured(root: &Path, env_url: Option<&str>) -> Result<bool, String> {
    if env_url.is_some_and(|s| !s.is_empty()) {
        return Ok(true);
    }
    let path = super::commands::get_omp_config_path_from_root(root);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => {
            return Err("Cannot check OMP broker configuration; account writes refused.".into())
        }
    };
    let value: serde_yaml::Value = serde_yaml::from_str(&raw)
        .map_err(|_| "Cannot parse OMP configuration; account writes refused.")?;
    if value.is_null() {
        return Ok(false);
    }
    if !value.is_mapping() {
        return Err("OMP configuration must be a mapping; account writes refused.".into());
    }
    let nested = value
        .get("auth")
        .and_then(|v| v.get("broker"))
        .and_then(|v| v.get("url"));
    let url = nested.or_else(|| value.get("auth.broker.url"));
    Ok(url.is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty())))
}

fn open_native_readonly(root: &Path) -> Result<Connection, String> {
    let path = root.join("agent.db");
    // Existing, regular local database only; no CREATE or URI interpretation.
    let metadata = fs::symlink_metadata(&path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .ok_or("Native OMP agent.db is missing or not a regular file. Run OMP first; AI Toolbox will not create it.")?;
    if metadata.len() == 0 {
        return Err(SCHEMA_ERROR.into());
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| DB_ERROR)?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|_| DB_ERROR)?;
    Ok(conn)
}

// Intentionally no Debug/Serialize: tokens stay private to this explicit read.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageCredentials {
    access: String,
    account_id: String,
    #[serde(rename = "orgName")]
    plan: Option<String>,
}

fn usage_credentials(root: &Path, target: &str) -> Result<UsageCredentials, String> {
    let id = selected_account_id(target)?;
    let conn = open_native_readonly(root)?;
    if !schema_supported(&conn) {
        return Err(SCHEMA_ERROR.into());
    }
    let (raw, disabled) = conn.query_row(
        "SELECT data,disabled_cause FROM auth_credentials WHERE id=?1 AND provider=?2 AND credential_type='oauth'",
        params![id, PROVIDER],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    ).map_err(|error| if error == rusqlite::Error::QueryReturnedNoRows {
        "Codex account no longer exists."
    } else { DB_ERROR })?;
    check_account_disabled(disabled.as_deref())?;
    let credentials: UsageCredentials = serde_json::from_str(&raw)
        .map_err(|_| "Stored Codex credential payload is invalid; authenticate again in OMP.")?;
    if credentials.access.trim().is_empty() {
        return Err(
            "Stored Codex credential has no access token; authenticate again in OMP.".into(),
        );
    }
    if credentials.account_id.trim().is_empty() {
        return Err("Stored Codex credential has no account ID; authenticate again in OMP.".into());
    }
    Ok(credentials)
}

enum Action<'a> {
    List,
    Import(&'a str),
    Switch(&'a str),
}

fn account_operation(
    root: &Path,
    action: Action<'_>,
    blocked: Option<String>,
) -> OmpCodexAccountsResult {
    let path = root.join("agent.db");
    let mut conn = match open_native_readonly(root) {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    let accounts = match list_accounts(&conn) {
        Ok(accounts) => accounts,
        Err(_) => return failure(SCHEMA_ERROR),
    };
    if !schema_supported(&conn) {
        return OmpCodexAccountsResult {
            accounts,
            can_write: false,
            error: Some(SCHEMA_ERROR.into()),
        };
    }
    if let Some(error) = blocked {
        return OmpCodexAccountsResult {
            accounts,
            can_write: false,
            error: Some(error),
        };
    }
    // Opening read-write without CREATE checks write permission even for list.
    drop(conn);
    conn = match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE) {
        Ok(conn) => conn,
        Err(_) => {
            return OmpCodexAccountsResult {
                accounts,
                can_write: false,
                error: Some(DB_ERROR.into()),
            }
        }
    };
    if conn.busy_timeout(Duration::from_secs(5)).is_err() || !schema_supported(&conn) {
        return OmpCodexAccountsResult {
            accounts,
            can_write: false,
            error: Some(SCHEMA_ERROR.into()),
        };
    }
    let result = match action {
        Action::List => Ok(()),
        Action::Import(raw) => import_account(&mut conn, raw),
        Action::Switch(id) => switch_account(&mut conn, id),
    };
    match list_accounts(&conn) {
        Ok(accounts) => OmpCodexAccountsResult {
            accounts,
            can_write: true,
            error: result.err(),
        },
        Err(error) => failure(error),
    }
}

fn configured_account_block(root: &Path) -> Option<String> {
    let env_url = std::env::var("OMP_AUTH_BROKER_URL").ok().or_else(|| {
        crate::coding::open_code::shell_env::get_env_from_shell_config("OMP_AUTH_BROKER_URL")
    });
    // Native profiles/XDG can redirect agent.db independently of runtime root.
    let redirected = ["OMP_PROFILE", "PI_PROFILE", "XDG_DATA_HOME"]
        .iter()
        .any(|key| {
            std::env::var(key)
                .ok()
                .or_else(|| crate::coding::open_code::shell_env::get_env_from_shell_config(key))
                .is_some_and(|v| !v.trim().is_empty())
        });
    account_block(root, env_url.as_deref(), redirected)
}

fn account_block(root: &Path, env_url: Option<&str>, redirected: bool) -> Option<String> {
    let blocked = match broker_configured(root, env_url) {
        Ok(true) => Some("OMP auth broker is configured; local account writes and quota queries are unavailable. Manage accounts through native OMP/broker.".into()),
        Err(error) => Some(error),
        Ok(false) => None,
    };
    blocked.or_else(|| redirected.then(|| "OMP profile/XDG database redirection is unsupported for local account management. Use native OMP account management.".into()))
}

async fn run(db: &SqliteDbState, root_path: &str, action: Action<'_>) -> OmpCodexAccountsResult {
    let location = match runtime_location::get_oh_my_pi_runtime_location_async(db).await {
        Ok(location) => location,
        Err(_) => return failure("Unable to resolve the selected OMP runtime root."),
    };
    let root = match checked_root(&location, root_path) {
        Ok(root) => root,
        Err(error) => return failure(error),
    };
    account_operation(&root, action, configured_account_block(&root))
}

#[tauri::command]
pub async fn get_omp_codex_account_usage(
    state: tauri::State<'_, SqliteDbState>,
    root_path: String,
    account_id: String,
) -> Result<OmpCodexAccountUsage, String> {
    let location = runtime_location::get_oh_my_pi_runtime_location_async(state.db())
        .await
        .map_err(|_| "Unable to resolve the selected OMP runtime root.")?;
    let root = checked_root(&location, &root_path)?;
    if let Some(error) = configured_account_block(&root) {
        return Err(error);
    }
    // Inactive rows are queried without enabling them; drop SQLite before HTTP.
    let credentials = usage_credentials(&root, &account_id)?;
    let snapshot = fetch_usage_snapshot(
        state.db(),
        credentials.access.trim(),
        Some(&credentials.account_id),
        credentials.plan.as_deref(),
    )
    .await
    .map_err(|_| "Unable to query Codex quota. Check the connection or authenticate again in OMP, then retry manually.")?;
    Ok(OmpCodexAccountUsage {
        account_id,
        has_five_hour_limit: snapshot.limit_short_label.is_some(),
        limit_5h_text: snapshot.limit_5h_text,
        limit_weekly_text: snapshot.limit_weekly_text,
        limit_5h_reset_at: snapshot.limit_5h_reset_at,
        limit_weekly_reset_at: snapshot.limit_weekly_reset_at,
    })
}

#[tauri::command]
pub async fn list_omp_codex_accounts(
    state: tauri::State<'_, SqliteDbState>,
    root_path: String,
) -> Result<OmpCodexAccountsResult, String> {
    Ok(run(state.db(), &root_path, Action::List).await)
}

#[tauri::command]
pub async fn import_omp_codex_account(
    state: tauri::State<'_, SqliteDbState>,
    root_path: String,
    auth_json: String,
) -> Result<OmpCodexAccountsResult, String> {
    Ok(run(state.db(), &root_path, Action::Import(&auth_json)).await)
}

#[tauri::command]
pub async fn switch_omp_codex_account(
    state: tauri::State<'_, SqliteDbState>,
    root_path: String,
    account_id: String,
) -> Result<OmpCodexAccountsResult, String> {
    Ok(run(state.db(), &root_path, Action::Switch(&account_id)).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    // Native 18.8.7 schema/trigger SQL, scoped to tables these operations use.
    fn fixture() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("agent.db")).unwrap();
        conn.execute_batch("CREATE TABLE auth_schema_version(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);
            INSERT INTO auth_schema_version VALUES(1,8);
            CREATE TABLE auth_credentials(id INTEGER PRIMARY KEY AUTOINCREMENT,provider TEXT NOT NULL,credential_type TEXT NOT NULL,data TEXT NOT NULL,disabled_cause TEXT DEFAULT NULL,identity_key TEXT DEFAULT NULL,created_at INTEGER NOT NULL DEFAULT(CAST(strftime('%s','now') AS INTEGER)),updated_at INTEGER NOT NULL DEFAULT(CAST(strftime('%s','now') AS INTEGER)));
            CREATE INDEX idx_auth_provider ON auth_credentials(provider);
            CREATE INDEX idx_auth_provider_identity ON auth_credentials(provider,identity_key) WHERE identity_key IS NOT NULL;
            CREATE TABLE auth_change_revision(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL);
            INSERT INTO auth_change_revision VALUES(1,0);").unwrap();
        for event in ["insert", "update", "delete"] {
            conn.execute_batch(&format!("CREATE TRIGGER auth_change_revision_auth_credentials_{event} AFTER {event} ON auth_credentials BEGIN UPDATE auth_change_revision SET revision=revision+1 WHERE id=1; END;")).unwrap();
        }
        assert!(schema_supported(&conn));
        (dir, conn)
    }

    fn token(payload: Value) -> String {
        format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD.encode(payload.to_string())
        )
    }

    fn auth(account: &str, address: Option<&str>, refresh: &str) -> String {
        json!({"tokens": {"access_token": token(json!({"exp": 2_000_000_000,
            "https://api.openai.com/auth": {"chatgpt_account_id": account,"chatgpt_plan_type":" TEAM "},
            "https://api.openai.com/profile": {"email":address}})),"refresh_token":refresh,"account_id":account}}).to_string()
    }

    fn rows(conn: &Connection) -> Vec<(i64, String, String, Option<String>, Option<String>)> {
        let mut stmt = conn.prepare("SELECT id,provider,data,disabled_cause,identity_key FROM auth_credentials ORDER BY id").unwrap();
        stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn sibling_import_preserves_payloads_and_reimport_status() {
        let (dir, mut conn) = fixture();
        let first = auth("personal", Some(" Alice@Example.Com "), "first-secret");
        import_account(&mut conn, &first).unwrap();
        conn.execute("UPDATE auth_credentials SET data=json_set(data,'$.nativeExtra','preserved') WHERE id=1", []).unwrap();
        let before = rows(&conn)[0].clone();
        import_account(
            &mut conn,
            &auth("work", Some("alice@example.com"), "second-secret"),
        )
        .unwrap();
        assert_eq!(rows(&conn)[0], before);
        assert_eq!(rows(&conn)[1].3.as_deref(), Some(INACTIVE));
        import_account(
            &mut conn,
            &auth("work", Some("ALICE@example.com"), "rotated-secret"),
        )
        .unwrap();
        assert_eq!(rows(&conn).len(), 2);
        assert_eq!(rows(&conn)[1].3.as_deref(), Some(INACTIVE));
        assert!(rows(&conn)[1].2.contains("rotated-secret"));
        import_account(&mut conn, &first).unwrap();
        assert!(rows(&conn)[0].2.contains("nativeExtra"));
        assert_eq!(rows(&conn)[0].3, None);
        let result = account_operation(dir.path(), Action::List, None);
        assert!(result.can_write);
        let safe = serde_json::to_string(&result).unwrap();
        assert!(!safe.contains("secret") && !safe.contains("access") && !safe.contains("refresh"));
        assert_eq!(
            result.accounts[0].email.as_deref(),
            Some("alice@example.com")
        );
        assert_eq!(result.accounts[0].expires_at, Some(2_000_000_000_000.0));
        assert_eq!(result.accounts[0].plan.as_deref(), Some("team"));
    }

    #[test]
    fn switch_matches_native_active_selection_and_preserves_other_providers() {
        let (dir, mut conn) = fixture();
        for id in ["one", "two", "three"] {
            import_account(&mut conn, &auth(id, Some("a@example.com"), id)).unwrap();
        }
        conn.execute(
            "UPDATE auth_credentials SET disabled_cause=NULL WHERE id=2",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO auth_credentials(provider,credential_type,data) VALUES('other','oauth','other-secret')", []).unwrap();
        conn.execute("INSERT INTO auth_credentials(provider,credential_type,data) VALUES('openai-codex','api_key','key-secret')", []).unwrap();
        let before = rows(&conn);
        let revision: i64 = conn
            .query_row("SELECT revision FROM auth_change_revision", [], |r| {
                r.get(0)
            })
            .unwrap();
        switch_account(&mut conn, "3").unwrap();
        // Exact native #listActiveByProviderStmt predicate (v18.8.7).
        let selected: Vec<i64> = conn.prepare("SELECT id FROM auth_credentials WHERE provider='openai-codex' AND disabled_cause IS NULL ORDER BY id ASC").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(selected, [3, 5]); // unrelated api_key is deliberately untouched.
        let after = rows(&conn);
        for (old, new) in before.iter().zip(&after) {
            assert_eq!(
                (&old.0, &old.1, &old.2, &old.4),
                (&new.0, &new.1, &new.2, &new.4)
            );
        }
        assert_eq!(&before[3..], &after[3..]);
        assert_eq!(after[0].3.as_deref(), Some(INACTIVE));
        assert_eq!(after[1].3.as_deref(), Some(INACTIVE));
        assert!(
            conn.query_row("SELECT revision FROM auth_change_revision", [], |r| r
                .get::<_, i64>(0))
                .unwrap()
                > revision
        );
        let result = account_operation(dir.path(), Action::List, None);
        assert_eq!(
            result
                .accounts
                .iter()
                .filter(|a| a.is_enabled)
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["3"]
        );
    }

    #[test]
    fn native_identity_keeps_email_and_org_collisions_distinct() {
        let (_dir, mut conn) = fixture();
        for (org, address) in [
            ("org-one", Some("a@example.com")),
            ("org-two", Some("a@example.com")),
            ("org-one", Some("b@example.com")),
            ("org-three", None),
        ] {
            import_account(&mut conn, &auth(org, address, "secret")).unwrap();
        }
        let keys: Vec<_> = rows(&conn).into_iter().map(|r| r.4.unwrap()).collect();
        assert_eq!(
            keys,
            [
                "email:a@example.com|org:org-one",
                "email:a@example.com|org:org-two",
                "email:b@example.com|org:org-one",
                "account:org-three|org:org-three"
            ]
        );
    }

    #[test]
    fn unknown_or_native_disabled_target_rolls_back_without_leaks() {
        let (dir, mut conn) = fixture();
        import_account(&mut conn, &auth("one", None, "secret")).unwrap();
        import_account(&mut conn, &auth("two", None, "secret")).unwrap();
        conn.execute(
            "UPDATE auth_credentials SET disabled_cause='oauth refresh failed: secret' WHERE id=2",
            [],
        )
        .unwrap();
        let before = rows(&conn);
        assert!(switch_account(&mut conn, "999").is_err());
        assert!(switch_account(&mut conn, "2").is_err());
        assert_eq!(rows(&conn), before);
        let result = account_operation(dir.path(), Action::Switch("2"), None);
        assert!(result.can_write && result.error.is_some());
        assert_eq!(
            result.accounts[1].disabled_cause.as_deref(),
            Some("native-disabled")
        );
        assert!(!serde_json::to_string(&result).unwrap().contains("secret"));
    }

    #[test]
    fn invalid_import_json_and_claims_never_mutate() {
        let (dir, mut conn) = fixture();
        import_account(&mut conn, &auth("one", None, "secret")).unwrap();
        let before = rows(&conn);
        let mut conflicting: Value = serde_json::from_str(&auth("two", None, "secret")).unwrap();
        conflicting["tokens"]["account_id"] = json!("different");
        let mut fallback: Value = serde_json::from_str(&auth("two", None, "secret")).unwrap();
        fallback["tokens"]["id_token"] = json!(token(
            json!({"https://api.openai.com/auth":{"chatgpt_account_id":"different"}})
        ));
        let invalid_exp = json!({"access_token":token(json!({"exp":-1,"https://api.openai.com/auth":{"chatgpt_account_id":"one"}})),"refresh_token":"secret"}).to_string();
        for invalid in [
            "{secret",
            "[]",
            "{}",
            &conflicting.to_string(),
            &fallback.to_string(),
            &invalid_exp,
            &" ".repeat(MAX_IMPORT_BYTES + 1),
        ] {
            let result = account_operation(dir.path(), Action::Import(invalid), None);
            assert!(result.can_write && result.error.is_some());
            assert!(!result.error.unwrap().contains("secret"));
            assert_eq!(rows(&conn), before);
        }
        let fallback = json!({"tokens":{"access_token":token(json!({"exp":123})),"refresh_token":"secret","id_token":token(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"fallback"},"https://api.openai.com/profile":{"email":" B@EXAMPLE.COM "}}))}});
        let parsed = parse_import(&fallback.to_string()).unwrap();
        assert_eq!(parsed["accountId"], "fallback");
        assert_eq!(parsed["email"], "b@example.com");
        assert_eq!(parsed["expires"], 123000.0);
    }

    #[test]
    fn unsupported_schema_and_broker_are_read_only_and_missing_db_not_created() {
        let (dir, mut conn) = fixture();
        import_account(&mut conn, &auth("one", None, "secret")).unwrap();
        let before = rows(&conn);
        let blocked = account_operation(
            dir.path(),
            Action::Import(&auth("two", None, "secret")),
            Some("broker unavailable".into()),
        );
        assert!(!blocked.can_write && blocked.accounts.len() == 1);
        assert_eq!(rows(&conn), before);
        conn.execute("UPDATE auth_schema_version SET version=9", [])
            .unwrap();
        let blocked = account_operation(dir.path(), Action::Switch("1"), None);
        assert!(!blocked.can_write && blocked.accounts.len() == 1 && blocked.error.is_some());
        assert_eq!(rows(&conn), before);
        let missing = tempfile::tempdir().unwrap();
        assert!(!account_operation(missing.path(), Action::List, None).can_write);
        assert!(!missing.path().join("agent.db").exists());
        for config in [
            "auth.broker.url: https://broker.example",
            "auth:\n  broker:\n    url: '!get-broker'",
            "auth: []\nauth.broker.url: https://broker.example",
        ] {
            fs::write(missing.path().join("config.yml"), config).unwrap();
            assert!(broker_configured(missing.path(), None).unwrap());
        }
        assert!(broker_configured(missing.path(), Some("https://broker.example")).unwrap());
    }

    #[test]
    fn usage_reads_inactive_native_credentials_without_writes_or_refresh() {
        let (dir, conn) = fixture();
        // Exact schema-8 data; no refresh token is needed for a readonly query.
        let data = json!({"access":"synthetic-access", "accountId":"synthetic-account",
            "orgName":"prolite", "expires":1, "nativeExtra":"preserved"});
        conn.execute(
            "INSERT INTO auth_credentials(provider,credential_type,data,disabled_cause) VALUES(?1,'oauth',?2,?3)",
            params![PROVIDER, data.to_string(), INACTIVE],
        ).unwrap();
        let before = rows(&conn);
        let revision: i64 = conn
            .query_row("SELECT revision FROM auth_change_revision", [], |r| {
                r.get(0)
            })
            .unwrap();
        let timestamps: (i64, i64) = conn
            .query_row(
                "SELECT created_at,updated_at FROM auth_credentials",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let bytes = fs::read(dir.path().join("agent.db")).unwrap();
        let credentials = usage_credentials(dir.path(), "1").unwrap();
        assert_eq!(credentials.access, "synthetic-access");
        assert_eq!(credentials.account_id, "synthetic-account");
        assert_eq!(credentials.plan.as_deref(), Some("prolite"));
        assert_eq!(rows(&conn), before);
        assert_eq!(
            conn.query_row("SELECT revision FROM auth_change_revision", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            revision
        );
        assert_eq!(
            conn.query_row(
                "SELECT created_at,updated_at FROM auth_credentials",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
            timestamps
        );
        assert_eq!(fs::read(dir.path().join("agent.db")).unwrap(), bytes);
        let readonly = open_native_readonly(dir.path()).unwrap();
        assert!(readonly
            .execute("UPDATE auth_credentials SET disabled_cause=NULL", [])
            .is_err());
    }

    #[test]
    fn usage_refuses_invalid_selection_schema_and_payload_without_leaks() {
        let (dir, conn) = fixture();
        conn.execute("INSERT INTO auth_credentials(provider,credential_type,data) VALUES('other','oauth','synthetic-secret')", []).unwrap();
        conn.execute("INSERT INTO auth_credentials(provider,credential_type,data) VALUES('openai-codex','api_key','synthetic-secret')", []).unwrap();
        conn.execute("INSERT INTO auth_credentials(provider,credential_type,data,disabled_cause) VALUES('openai-codex','oauth','synthetic-secret','invalid_grant synthetic-secret')", []).unwrap();
        conn.execute("INSERT INTO auth_credentials(provider,credential_type,data) VALUES('openai-codex','oauth','{synthetic-secret')", []).unwrap();
        for data in [
            json!({"accountId":"synthetic-secret"}),
            json!({"access":"synthetic-secret"}),
        ] {
            conn.execute("INSERT INTO auth_credentials(provider,credential_type,data) VALUES('openai-codex','oauth',?1)", [data.to_string()]).unwrap();
        }
        let before = rows(&conn);
        for id in [
            "", "-1", "0", "1 OR 1=1", "999", "1", "2", "3", "4", "5", "6",
        ] {
            let error = usage_credentials(dir.path(), id).err().unwrap();
            assert!(!error.contains("synthetic-secret"));
        }
        conn.execute("UPDATE auth_schema_version SET version=9", [])
            .unwrap();
        assert_eq!(
            usage_credentials(dir.path(), "4").err().unwrap(),
            SCHEMA_ERROR
        );
        assert_eq!(rows(&conn), before);
        let missing = tempfile::tempdir().unwrap();
        assert!(usage_credentials(missing.path(), "1").is_err());
        assert!(!missing.path().join("agent.db").exists());
    }

    #[test]
    fn usage_reuses_broker_and_profile_fail_closed_guards() {
        let dir = tempfile::tempdir().unwrap();
        assert!(account_block(dir.path(), None, false).is_none());
        assert!(account_block(dir.path(), Some("https://broker.example"), false).is_some());
        assert!(account_block(dir.path(), None, true).is_some());
        fs::write(
            dir.path().join("config.yml"),
            "auth.broker.url: https://broker.example",
        )
        .unwrap();
        assert!(account_block(dir.path(), None, false).is_some());
        fs::write(dir.path().join("config.yml"), "[synthetic-secret").unwrap();
        let error = account_block(dir.path(), None, false).unwrap();
        assert!(!error.contains("synthetic-secret"));
        assert!(!dir.path().join("agent.db").exists());
    }

    #[test]
    fn root_selection_rejects_injection_and_wsl_before_opening() {
        let dir = tempfile::tempdir().unwrap();
        let mut location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::LocalWindows,
            source: "db".into(),
            host_path: dir.path().to_path_buf(),
            wsl: None,
        };
        assert!(checked_root(&location, dir.path().to_str().unwrap()).is_ok());
        for supplied in [
            "relative",
            "/another-runtime",
            "file:/tmp/agent.db",
            &format!("{}/../anything", dir.path().display()),
            "\\\\wsl.localhost\\Ubuntu\\home\\test",
        ] {
            assert!(checked_root(&location, supplied).is_err());
        }
        location.mode = RuntimeLocationMode::WslDirect;
        assert!(checked_root(&location, dir.path().to_str().unwrap()).is_err());
    }
}

#[cfg(test)]
mod replacement_tests {
    use super::*;

    #[test]
    fn ambiguous_identity_refuses_import_and_fresh_tokens_recover_disabled_account() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE auth_credentials(id INTEGER PRIMARY KEY,provider TEXT,credential_type TEXT,data TEXT,identity_key TEXT,disabled_cause TEXT,created_at INTEGER,updated_at INTEGER);").unwrap();
        let access = format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(
                json!({"exp":123,"https://api.openai.com/auth":{"chatgpt_account_id":"one"}})
                    .to_string()
            )
        );
        let raw = json!({"access_token":access,"refresh_token":"new-refresh"}).to_string();
        let data = parse_import(&raw).unwrap();
        conn.execute("INSERT INTO auth_credentials(id,provider,credential_type,data,identity_key,disabled_cause) VALUES(1,?1,'oauth',?2,?3,'invalid_grant')",params![PROVIDER,data.to_string(),identity(&data)]).unwrap();
        assert!(switch_account(&mut conn, "1").is_err());
        import_account(&mut conn, &raw).unwrap();
        let inactive: String = conn
            .query_row("SELECT disabled_cause FROM auth_credentials", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(inactive, INACTIVE);
        switch_account(&mut conn, "1").unwrap();
        conn.execute("INSERT INTO auth_credentials(id,provider,credential_type,data,identity_key) SELECT 2,provider,credential_type,data,identity_key FROM auth_credentials WHERE id=1",[]).unwrap();
        let before: i64 = conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert!(import_account(&mut conn, &raw)
            .unwrap_err()
            .contains("Multiple native rows"));
        assert_eq!(
            conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
    }
}
