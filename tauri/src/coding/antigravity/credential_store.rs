//! Antigravity CLI (`agy`) credential-store bridge.
//!
//! ## Why this module exists
//!
//! `agy` does NOT read `~/.gemini/antigravity-cli/oauth_creds.json`. Empirical
//! evidence from a real Windows install:
//!
//! * Moving `oauth_creds.json` away did not break `agy models` (still
//!   authenticated), so the file is not the runtime credential source.
//! * The credential store holds `gemini:antigravity` (Windows Credential
//!   Manager, `LegacyGeneric:target=gemini:antigravity`, `UserName`
//!   `antigravity`, `Persist` LocalMachine), and overwriting that blob with a
//!   bogus value makes `agy models` fail with `401 UNAUTHENTICATED`.
//!
//! So account switching only works by rewriting that credential. `agy` refreshes
//! it itself on the next run when the stored `token.expiry` is in the past, which
//! is why writing a snapshot and letting the CLI refresh is sufficient.
//!
//! ## Gotchas (do not "simplify" these away)
//!
//! * `keyring` v4's `Entry::new(service, user)` always builds the Windows target
//!   `service.user`, so it cannot address `gemini:antigravity`. We use
//!   `keyring-core` plus the platform store crates and pass the explicit
//!   `target` attribute `agy` itself relies on.
//! * The blob is written as raw bytes. `set_password()` would encode UTF-16LE,
//!   which `agy` cannot read; `set_secret()`/`get_secret()` are mandatory.
//! * The payload must keep `agy`'s own field shape (a nested `token` object plus
//!   `auth_method`) and must preserve `refresh_token`/`id_token` verbatim.
//! * `token.expiry` must stay RFC3339 (`+08:00` offset and `Z` are both fine).

use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

const GO_KEYRING_BASE64_PREFIX: &str = "go-keyring-base64:";
const WINDOWS_CREDENTIAL_MAX_BLOB_BYTES: usize = 5 * 512;
const CLI_FALLBACK_TOKEN_FILE: &str = "antigravity-oauth-token";

/// Injectable boundary for account-switch round trips without touching real logins.
pub(super) trait CredentialStore: Send + Sync {
    fn read(&self) -> Result<Option<String>, String>;
    fn replace(&self, snapshot: Option<&str>) -> Result<(), String>;
}

pub(super) struct OsCredentialStore;

impl CredentialStore for OsCredentialStore {
    fn read(&self) -> Result<Option<String>, String> {
        read_credential_text()
    }

    fn replace(&self, snapshot: Option<&str>) -> Result<(), String> {
        match snapshot {
            Some(snapshot) => write_credential_text(snapshot),
            None => delete_credential(),
        }
    }
}

/// Credential-store target/account name used by `agy`.
pub const ANTIGRAVITY_CREDENTIAL_TARGET: &str = "gemini:antigravity";
/// Service part of the macOS Keychain `service`/`account` pair.
pub const ANTIGRAVITY_CREDENTIAL_SERVICE: &str = "gemini";
/// Account part of the macOS Keychain `service`/`account` pair.
pub const ANTIGRAVITY_CREDENTIAL_ACCOUNT: &str = "antigravity";

/// Read the raw credential blob. `Ok(None)` means no credential is stored.
pub fn read_credential_blob() -> Result<Option<Vec<u8>>, String> {
    with_entry(|entry| match entry.get_secret() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(error) => Err(credential_error("read", &error)),
    })
}

/// Decode `go-keyring-base64:` payloads (written by Go's keyring on macOS) and
/// trim surrounding whitespace.
fn normalize_credential_secret(secret: &str) -> Result<String, String> {
    let trimmed = secret.trim();
    if let Some(encoded) = trimmed.strip_prefix(GO_KEYRING_BASE64_PREFIX) {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|error| format!("Antigravity credential base64 decode failed: {error}"))?;
        let text = String::from_utf8(decoded)
            .map_err(|error| format!("Antigravity decoded credential is not valid UTF-8: {error}"))?;
        return Ok(text.trim().to_string());
    }
    Ok(trimmed.to_string())
}

/// Windows Credential Manager enforces a 2560-byte (5 * 512) blob limit.
/// When a full OAuth snapshot (often carrying a large JWT `id_token`) exceeds
/// that limit, strip non-essential fields while preserving everything `agy`
/// needs to authenticate and refresh (`token.{access_token,refresh_token,token_type,expiry}`
/// + `auth_method`).
#[cfg(any(target_os = "windows", test))]
fn fit_windows_credential_payload(contents: &str) -> Result<String, String> {
    if contents.len() <= WINDOWS_CREDENTIAL_MAX_BLOB_BYTES {
        return Ok(contents.to_string());
    }
    let parsed = serde_json::from_str::<Value>(contents)
        .map_err(|error| format!("Antigravity credential exceeds Windows limit and is not JSON: {error}"))?;
    let mut token = serde_json::Map::new();
    for field in ["access_token", "refresh_token", "token_type", "expiry"] {
        if let Some(val) = parsed
            .get("token")
            .and_then(|t| t.get(field))
            .or_else(|| parsed.get(field))
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
        {
            token.insert(field.to_string(), json!(val));
        }
    }
    let auth_method = parsed
        .get("auth_method")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("consumer");
    let compact = json!({
        "token": Value::Object(token),
        "auth_method": auth_method,
    })
    .to_string();
    if compact.len() > WINDOWS_CREDENTIAL_MAX_BLOB_BYTES {
        return Err(format!(
            "Antigravity credential payload ({} bytes) exceeds Windows Credential Manager limit ({} bytes)",
            compact.len(),
            WINDOWS_CREDENTIAL_MAX_BLOB_BYTES
        ));
    }
    Ok(compact)
}

/// Sync `<runtime_root>/antigravity-oauth-token` only if it already exists, so
/// an old CLI fallback file cannot resurrect a previous account.
pub fn sync_existing_cli_fallback_file(root_dir: &Path, snapshot: Option<&str>) {
    let fallback_path = root_dir.join(CLI_FALLBACK_TOKEN_FILE);
    if !fallback_path.is_file() {
        return;
    }
    match snapshot {
        Some(content) => {
            let _ = std::fs::write(&fallback_path, content);
        }
        None => {
            let _ = std::fs::remove_file(&fallback_path);
        }
    }
}

/// Read the credential blob and decode it as UTF-8 text.
pub fn read_credential_text() -> Result<Option<String>, String> {
    let Some(bytes) = read_credential_blob()? else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes)
        .map_err(|error| format!("Antigravity credential is not valid UTF-8: {error}"))?;
    let normalized = normalize_credential_secret(&text)?;
    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

/// Overwrite the credential blob with raw UTF-8 bytes.
///
/// Never use the credential store's password API here: it encodes UTF-16LE and
/// `agy` would then fail to parse its own credential.
pub fn write_credential_text(contents: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let prepared = fit_windows_credential_payload(contents)?;
    #[cfg(target_os = "macos")]
    let prepared = format!(
        "{GO_KEYRING_BASE64_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(contents.as_bytes())
    );
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let prepared = contents.to_string();

    write_credential_blob(prepared.as_bytes())
}

/// Overwrite the credential blob with arbitrary bytes.
pub fn write_credential_blob(secret: &[u8]) -> Result<(), String> {
    with_entry(|entry| {
        entry
            .set_secret(secret)
            .map_err(|error| credential_error("write", &error))
    })
}

/// Delete the credential, treating "already gone" as success.
pub fn delete_credential() -> Result<(), String> {
    with_entry(|entry| match entry.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(credential_error("delete", &error)),
    })
}

/// Whether a credential currently exists in the store.
pub fn credential_exists() -> bool {
    matches!(read_credential_blob(), Ok(Some(_)))
}

fn credential_error(action: &str, error: &keyring_core::Error) -> String {
    format!(
        "Failed to {action} the Antigravity CLI credential \
         ('{ANTIGRAVITY_CREDENTIAL_TARGET}'): {error}"
    )
}

fn with_entry<T>(
    operation: impl FnOnce(&keyring_core::Entry) -> Result<T, String>,
) -> Result<T, String> {
    let entry = build_entry()?;
    operation(&entry)
}

/// Build an entry that resolves to the exact credential `agy` uses.
///
/// The store is created once per process; repeated calls only rebuild the
/// (cheap) entry wrapper.
fn build_entry() -> Result<keyring_core::Entry, String> {
    let (store, modifiers) = default_store()?;
    store
        .build(
            ANTIGRAVITY_CREDENTIAL_SERVICE,
            ANTIGRAVITY_CREDENTIAL_ACCOUNT,
            Some(&modifiers),
        )
        .map_err(|error| format!("Failed to open the Antigravity CLI credential entry: {error}"))
}

#[allow(clippy::type_complexity)]
fn default_store() -> Result<
    (
        std::sync::Arc<keyring_core::CredentialStore>,
        HashMap<&'static str, &'static str>,
    ),
    String,
> {
    static STORE: std::sync::OnceLock<
        Result<std::sync::Arc<keyring_core::CredentialStore>, String>,
    > = std::sync::OnceLock::new();
    static MODIFIERS: std::sync::OnceLock<HashMap<&'static str, &'static str>> =
        std::sync::OnceLock::new();

    let modifiers = MODIFIERS.get_or_init(platform_modifiers).clone();
    let store = STORE.get_or_init(create_store).clone()?;
    Ok((store, modifiers))
}

#[cfg(target_os = "windows")]
fn platform_modifiers() -> HashMap<&'static str, &'static str> {
    // Explicit target: overrides the store's default `user.service` composition
    // so we address the same generic credential `agy` writes.
    HashMap::from([("target", ANTIGRAVITY_CREDENTIAL_TARGET)])
}

#[cfg(not(target_os = "windows"))]
fn platform_modifiers() -> HashMap<&'static str, &'static str> {
    // Empty by design: the native keychain/Secret Service store maps the
    // service/account pair we pass to `build_entry` unchanged.
    HashMap::new()
}

#[cfg(target_os = "windows")]
fn create_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, String> {
    let store = windows_native_keyring_store::Store::new()
        .map_err(|error| format!("Failed to open the Windows credential store: {error}"))?;
    Ok(store)
}

#[cfg(target_os = "macos")]
fn create_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, String> {
    let store = apple_native_keyring_store::keychain::Store::new()
        .map_err(|error| format!("Failed to open the macOS Keychain: {error}"))?;
    Ok(store)
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn create_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, String> {
    let store = zbus_secret_service_keyring_store::Store::new()
        .map_err(|error| format!("Failed to open the Linux Secret Service store: {error}"))?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_names_match_agy_layout() {
        // The Windows target is a raw `service:user` pair, not keyring v4's
        // `service.user`; the macOS pair splits the same string.
        assert_eq!(ANTIGRAVITY_CREDENTIAL_TARGET, "gemini:antigravity");
        assert_eq!(ANTIGRAVITY_CREDENTIAL_SERVICE, "gemini");
        assert_eq!(ANTIGRAVITY_CREDENTIAL_ACCOUNT, "antigravity");
        assert!(!ANTIGRAVITY_CREDENTIAL_TARGET.contains('.'));
    }

    #[test]
    fn normalize_decodes_go_keyring_base64_and_keeps_plain_json() {
        let raw = r#"{"auth_method":"consumer","token":{"access_token":"abc"}}"#;
        assert_eq!(normalize_credential_secret(raw).unwrap(), raw);
        let encoded = format!(
            "{GO_KEYRING_BASE64_PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        );
        assert_eq!(normalize_credential_secret(&encoded).unwrap(), raw);
    }

    #[test]
    fn windows_payload_compacts_oversized_id_token() {
        let huge_id_token = "x".repeat(3000);
        let payload = json!({
            "auth_method": "consumer",
            "extra_field": "drop-me",
            "token": {
                "access_token": "acc-123",
                "refresh_token": "ref-456",
                "token_type": "Bearer",
                "expiry": "2099-01-01T00:00:00Z",
                "id_token": huge_id_token
            }
        })
        .to_string();
        assert!(payload.len() > WINDOWS_CREDENTIAL_MAX_BLOB_BYTES);
        let compacted = fit_windows_credential_payload(&payload).unwrap();
        assert!(compacted.len() <= WINDOWS_CREDENTIAL_MAX_BLOB_BYTES);
        let parsed: Value = serde_json::from_str(&compacted).unwrap();
        assert_eq!(parsed["auth_method"], "consumer");
        assert_eq!(parsed["token"]["access_token"], "acc-123");
        assert_eq!(parsed["token"]["refresh_token"], "ref-456");
        assert!(parsed["token"].get("id_token").is_none());
    }
}
