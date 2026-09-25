use chrono::Local;

/// Shared backup filename contract across local / WebDAV / repository storage.
///
/// Recognized layouts (all ending in `.zip` or `.zip.enc`):
/// - new:      `ai-toolbox-backup-<YYYYMMDD>-<HHMMSS>-<unique8>[_<host>].zip[.enc]`
/// - current:  `ai-toolbox-backup-<YYYYMMDD>-<HHMMSS>[_<host>].zip`
/// - legacy:   `ai-toolbox-backup-<anything>-<YYYYMMDD>-<HHMMSS>.zip`
///
/// The unique id keeps same-second or multi-device backups from overwriting each other
/// (repository Contents APIs refuse same-name writes, so the generator must not emit
/// colliding names in the first place).
pub const BACKUP_FILENAME_PREFIX: &str = "ai-toolbox-backup-";
/// `YYYYMMDD-HHMMSS` = 8 digits + separator + 6 digits.
const TIMESTAMP_LENGTH: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFileNameInfo {
    /// `YYYYMMDD-HHMMSS` extracted from the name; string comparison is time order.
    pub timestamp: String,
    /// Short unique suffix on new-format files.
    pub unique_id: Option<String>,
    /// Host label for the current/new-with-host layouts.
    pub host_label: Option<String>,
    /// True when the file carries the `.zip.enc` suffix.
    pub encrypted: bool,
}

/// Generate the full backup filename for a new backup. Every generated name carries a
/// unique id so two backups created within the same second never collide.
pub fn generate_backup_filename(host_label: Option<&str>, encrypted: bool) -> String {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let unique = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let host = host_label.map(str::trim).filter(|host| !host.is_empty());
    let stem = match host {
        Some(host) => format!("{BACKUP_FILENAME_PREFIX}{timestamp}-{unique}_{host}"),
        None => format!("{BACKUP_FILENAME_PREFIX}{timestamp}-{unique}"),
    };
    if encrypted {
        format!("{stem}.zip.enc")
    } else {
        format!("{stem}.zip")
    }
}

/// True when the filename is a managed backup file (any historical layout, with or
/// without the `.enc` outer suffix). Repository listing and local retention cleanup
/// must use this instead of a bare `.zip` suffix check.
pub fn is_managed_backup_filename(filename: &str) -> bool {
    parse_backup_filename(filename).is_some()
}

/// Parse a managed backup filename. Returns `None` for foreign files (they must never
/// be listed, cleaned up, or offered for restore). Host labels may contain `-`, `_`,
/// and non-ASCII characters, so parsing anchors on the fixed-width timestamp instead
/// of splitting on separators. All slicing goes through `str::get`, which returns
/// `None` on a non-character boundary — a multibyte host label can never panic here.
pub fn parse_backup_filename(filename: &str) -> Option<BackupFileNameInfo> {
    let (stem, encrypted) = match filename.strip_suffix(".zip.enc") {
        Some(stem) => (stem, true),
        None => filename.strip_suffix(".zip").map(|stem| (stem, false))?,
    };
    if !stem.starts_with(BACKUP_FILENAME_PREFIX) {
        return None;
    }
    let body = &stem[BACKUP_FILENAME_PREFIX.len()..];

    // New/current layouts open with the fixed-width timestamp.
    if let Some(timestamp) = body.get(..TIMESTAMP_LENGTH) {
        if is_timestamp(timestamp) {
            return match body.get(TIMESTAMP_LENGTH..) {
                None | Some("") => Some(BackupFileNameInfo {
                    timestamp: timestamp.to_string(),
                    unique_id: None,
                    host_label: None,
                    encrypted,
                }),
                Some(rest) if rest.starts_with('-') => {
                    // New layout: -<unique8>[_<host>]
                    let suffix = &rest[1..];
                    let (unique_id, host_label) = match suffix.split_once('_') {
                        Some((unique, host)) => (unique, Some(host)),
                        None => (suffix, None),
                    };
                    if !is_unique_id(unique_id) {
                        return None;
                    }
                    Some(BackupFileNameInfo {
                        timestamp: timestamp.to_string(),
                        unique_id: Some(unique_id.to_string()),
                        host_label: host_label
                            .map(str::trim)
                            .filter(|h| !h.is_empty())
                            .map(str::to_string),
                        encrypted,
                    })
                }
                Some(rest) if rest.starts_with('_') => {
                    // Current layout with host: _<host>
                    let host = rest[1..].trim();
                    Some(BackupFileNameInfo {
                        timestamp: timestamp.to_string(),
                        unique_id: None,
                        host_label: (!host.is_empty()).then(|| host.to_string()),
                        encrypted,
                    })
                }
                _ => None,
            };
        }
    }

    // Legacy layout: <anything>-<timestamp>. Anchor on the trailing timestamp so a
    // dash inside the legacy prefix cannot split it.
    if body.len() > TIMESTAMP_LENGTH + 1 {
        let split = body.len() - TIMESTAMP_LENGTH;
        if let (Some(prefix), Some(timestamp)) = (body.get(..split), body.get(split..)) {
            if let Some(prefix) = prefix.strip_suffix('-') {
                if !prefix.is_empty() && is_timestamp(timestamp) {
                    return Some(BackupFileNameInfo {
                        timestamp: timestamp.to_string(),
                        unique_id: None,
                        host_label: None,
                        encrypted,
                    });
                }
            }
        }
    }
    None
}

fn is_unique_id(value: &str) -> bool {
    value.len() == 8 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != TIMESTAMP_LENGTH || bytes[8] != b'-' {
        return false;
    }
    value
        .chars()
        .enumerate()
        .all(|(index, character)| index == 8 || character.is_ascii_digit())
}

/// Sort key so mixed old/new and plain/encrypted backups order by creation time.
/// New files in the same second sort after each other by unique id.
pub fn backup_sort_key(filename: &str) -> (String, String) {
    match parse_backup_filename(filename) {
        Some(info) => (info.timestamp, info.unique_id.unwrap_or_default()),
        None => (String::new(), filename.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_are_unique_within_the_same_second() {
        let first = generate_backup_filename(None, false);
        let second = generate_backup_filename(None, false);
        assert_ne!(first, second);
        assert!(first.starts_with(BACKUP_FILENAME_PREFIX) && first.ends_with(".zip"));
    }

    #[test]
    fn generated_encrypted_name_keeps_base_and_enc_suffix() {
        let name = generate_backup_filename(Some("Office PC"), true);
        assert!(name.ends_with(".zip.enc"));
        assert!(name.contains("_Office PC"));
        let info = parse_backup_filename(&name).unwrap();
        assert!(info.encrypted);
        assert_eq!(info.host_label.as_deref(), Some("Office PC"));
        assert!(info.unique_id.is_some());
    }

    #[test]
    fn generated_name_with_separators_in_host_label_round_trips() {
        let name = generate_backup_filename(Some("my-host_v2"), false);
        let info = parse_backup_filename(&name).unwrap();
        assert_eq!(info.host_label.as_deref(), Some("my-host_v2"));
        assert!(!info.encrypted);
    }

    #[test]
    fn parses_current_legacy_and_new_layouts() {
        let current = parse_backup_filename("ai-toolbox-backup-20260913-120000.zip").unwrap();
        assert_eq!(current.timestamp, "20260913-120000");
        assert!(!current.encrypted);
        assert_eq!(current.unique_id, None);
        assert_eq!(current.host_label, None);

        let with_host =
            parse_backup_filename("ai-toolbox-backup-20260913-120000_HomeNAS.zip").unwrap();
        assert_eq!(with_host.host_label.as_deref(), Some("HomeNAS"));

        let legacy =
            parse_backup_filename("ai-toolbox-backup-app-1.2.3-20260102-030405.zip").unwrap();
        assert_eq!(legacy.timestamp, "20260102-030405");
        assert_eq!(legacy.unique_id, None);
        assert_eq!(legacy.host_label, None);

        let new_format =
            parse_backup_filename("ai-toolbox-backup-20260913-120000-abc123ef.zip.enc").unwrap();
        assert_eq!(new_format.timestamp, "20260913-120000");
        assert_eq!(new_format.unique_id.as_deref(), Some("abc123ef"));
        assert!(new_format.encrypted);
    }

    #[test]
    fn foreign_filenames_are_not_managed() {
        assert!(!is_managed_backup_filename("configuration.aitsync"));
        assert!(!is_managed_backup_filename(
            "other-backup-20260913-120000.zip"
        ));
        assert!(!is_managed_backup_filename(
            "ai-toolbox-backup-notatime.zip"
        ));
        assert!(!is_managed_backup_filename(
            "ai-toolbox-backup-20260913-120000.tar.gz"
        ));
        assert!(!is_managed_backup_filename(
            "ai-toolbox-backup-20260913-120000.zip.enc.enc"
        ));
        assert!(!is_managed_backup_filename(
            "ai-toolbox-backup-20260913-120000-badid!.zip"
        ));
    }

    #[test]
    fn sort_key_orders_by_timestamp_then_unique_id() {
        let old = backup_sort_key("ai-toolbox-backup-20260101-000000.zip");
        let new = backup_sort_key("ai-toolbox-backup-20260913-120000-ffff0001.zip");
        let newer = backup_sort_key("ai-toolbox-backup-20260913-120000-ffff0002.zip");
        let unlabeled = backup_sort_key("unrelated.zip");
        assert!(old < new);
        assert!(new < newer);
        assert_eq!(unlabeled, (String::new(), "unrelated.zip".to_string()));
    }

    #[test]
    fn empty_and_whitespace_host_labels_round_trip_to_none() {
        let name = generate_backup_filename(Some("  "), false);
        let info = parse_backup_filename(&name).unwrap();
        assert_eq!(info.host_label, None);
    }

    #[test]
    fn multibyte_legacy_prefix_parses_without_panicking() {
        // Regression: slicing the fixed-width timestamp at byte offsets used to
        // panic when a multibyte host label straddled the boundary.
        let legacy = "ai-toolbox-backup-Windows中文备份-20260102-030405.zip";
        let info = parse_backup_filename(legacy).expect("multibyte legacy name must parse");
        assert_eq!(info.timestamp, "20260102-030405");
        assert!(!info.encrypted);

        let with_host =
            parse_backup_filename("ai-toolbox-backup-20260913-120000-abc123ef_工作机.zip.enc")
                .expect("multibyte host label on the new layout must parse");
        assert_eq!(with_host.timestamp, "20260913-120000");
        assert_eq!(with_host.unique_id.as_deref(), Some("abc123ef"));
        assert_eq!(with_host.host_label.as_deref(), Some("工作机"));
        assert!(with_host.encrypted);
    }

    #[test]
    fn multibyte_foreign_names_return_none_without_panicking() {
        // Non-boundary slicing must degrade to None, never panic, for anything
        // that is not a managed backup name.
        assert_eq!(
            parse_backup_filename("ai-toolbox-backup-中文备份.zip"),
            None
        );
        assert_eq!(
            parse_backup_filename("备份-ai-toolbox-backup-20260913-120000.zip"),
            None
        );
        assert_eq!(
            parse_backup_filename("ai-toolbox-backup-工作-20260102-030405.zip.enc.enc"),
            None
        );
        assert!(is_managed_backup_filename(
            "ai-toolbox-backup-中文-20260102-030405.zip"
        ));
    }
}
