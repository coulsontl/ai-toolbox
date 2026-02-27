//! Common Database ID Utilities for SurrealDB Record IDs
//!
//! Provides standardized functions for handling SurrealDB record IDs:
//! - Converting raw database IDs to clean business IDs
//! - Stripping table prefixes and SurrealDB wrapper characters
//!
//! **Usage**:
//! ```rust
//! use super::db_id::{db_clean_id, db_extract_id};
//!
//! let id = db_extract_id(&record);
//! ```

use serde_json::Value;

/// Clean a SurrealDB record ID by stripping table prefix and wrapper characters.
///
/// **Purpose**: SurrealDB returns IDs in formats like:
/// - `"claude_provider:c6bs..."` (with table prefix)
/// - `"claude_provider:⟨uuid⟩"` (with table prefix and wrapper)
/// - `"⟨uuid⟩"` (with wrapper only)
///
/// **Output**: Clean ID suitable for frontend and business logic: `"c6bs..."` or `"uuid"`
///
/// # Example
/// ```rust
/// let raw_id = "claude_provider:⟨abc-123⟩";
/// let clean = db_clean_id(raw_id);
/// assert_eq!(clean, "abc-123");
/// ```
pub fn db_clean_id(raw_id: &str) -> String {
    // Strip table prefix if present (e.g., "claude_provider:xxx" -> "xxx")
    let without_prefix = if let Some(pos) = raw_id.find(':') {
        &raw_id[pos + 1..]
    } else {
        raw_id
    };
    // Strip SurrealDB wrapper characters ⟨⟩ if present
    without_prefix
        .trim_start_matches('⟨')
        .trim_end_matches('⟩')
        .to_string()
}

/// Extract a clean ID from a database record Value.
///
/// **Purpose**: Safely extract and clean the ID from a SurrealDB record.
///
/// # Example
/// ```rust
/// let record = json!({
///     "id": "claude_provider:⟨abc-123⟩",
///     "name": "Test"
/// });
/// let id = db_extract_id(&record);
/// ```
pub fn db_extract_id(record: &Value) -> String {
    record
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| db_clean_id(s))
        .unwrap_or_default()
}

/// Extract a clean ID from a database record, returning None if not found.
pub fn db_extract_id_opt(record: &Value) -> Option<String> {
    record
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| db_clean_id(s))
}

/// Build a SurrealDB record ID from table name and ID.
///
/// **Purpose**: Create a proper Thing ID string for queries.
///
/// # Example
/// ```rust
/// let thing_id = db_build_id("claude_provider", "abc-123");
/// assert_eq!(thing_id, "claude_provider:abc-123");
/// ```
pub fn db_build_id(table: &str, id: &str) -> String {
    format!("{}:{}", table, id)
}

/// Build a backtick-escaped record reference for use in SurrealQL queries.
///
/// Returns format: `` table:`id` `` which ensures the ID is treated as a literal
/// string regardless of its content (hyphens, slashes, etc.).
///
/// **Purpose**: Avoids `type::thing()` which may interpret UUID-format strings
/// differently across SurrealDB versions (e.g., 2.4 vs 2.6). Backtick-escaped
/// IDs are the safest way to reference records with special characters.
///
/// **Security**: Input ID is sanitized to only allow safe characters.
///
/// # Example
/// ```rust
/// let ref_id = db_record_id("mcp_server", "100dcf2a-3718-457f-b1ef-31d48c3478f8");
/// assert_eq!(ref_id, "mcp_server:`100dcf2a-3718-457f-b1ef-31d48c3478f8`");
/// ```
/// Generate a new database record ID (UUID v4 without hyphens).
///
/// # Example
/// ```rust
/// let id = db_new_id(); // e.g. "a1b2c3d4e5f6..."
/// ```
pub fn db_new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn db_record_id(table: &str, id: &str) -> String {
    // Sanitize: only allow safe characters to prevent query injection
    let clean: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '/' || *c == '.')
        .collect();
    format!("{}:`{}`", table, clean)
}
