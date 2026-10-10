//! Read-only native OMP subscription metadata, never OAuth credentials.
//!
//! Contract: oh-my-pi b07a1c146d0d12cfc855a2c65d52f892ef319040,
//! sqlite-credential-store.ts, model-cache.ts and utils/dirs.ts. Unknown
//! schemas are deliberately treated as unknown rather than as logged out.
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use super::types::OmpCodexSubscription;
use crate::coding::cli_resolver::resolve_local_omp_program;
use crate::coding::runtime_location::{RuntimeLocationInfo, RuntimeLocationMode};

fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn quote_powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn command_for(root: &str, program: &str, powershell: bool, action: &str) -> String {
    // Do not render a misleading multi-line command for unusual filesystem
    // paths. No command is executed here; the user remains in their terminal.
    if root.chars().chain(program.chars()).any(char::is_control) {
        return String::new();
    }
    if powershell {
        format!(
            "& {{ $previous = $env:PI_CODING_AGENT_DIR; try {{ $env:PI_CODING_AGENT_DIR={}; & {} --profile default {action} }} finally {{ $env:PI_CODING_AGENT_DIR=$previous }} }}",
            quote_powershell(root),
            quote_powershell(program)
        )
    } else {
        format!(
            "PI_CODING_AGENT_DIR={} {} --profile default {action}",
            quote_posix(root),
            quote_posix(program)
        )
    }
}

fn open_read_only(path: &Path) -> Result<Connection, rusqlite::Error> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(100))?;
    Ok(connection)
}

fn read_status(root: &Path) -> &'static str {
    let path = root.join("agent.db");
    match path.try_exists() {
        Ok(false) => return "not_configured",
        Err(_) => return "unknown",
        Ok(true) => {}
    }
    let result = open_read_only(&path).and_then(|connection| {
        // Do not select `data`, deserialize tokens, open OMP AuthStorage (which
        // migrates/writes), refresh credentials, or query the remote provider.
        connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM auth_credentials WHERE provider = ?1 \
             AND credential_type = 'oauth' AND disabled_cause IS NULL)",
            ["openai-codex"],
            |row| row.get::<_, bool>(0),
        )
    });
    match result {
        Ok(true) => "configured",
        Ok(false) => "not_configured",
        Err(_) => "unknown",
    }
}

fn read_model_ids(root: &Path) -> Vec<String> {
    let Ok(connection) = open_read_only(&root.join("models.db")) else {
        return Vec::new();
    };
    // Only the newest native Codex cache, never a hashed custom-base-URL cache.
    // Project only IDs in SQL: older cache payloads can contain secret headers.
    // An unknown cache schema or invalid JSON is simply an unavailable catalog.
    let result = (|| -> Result<Vec<String>, rusqlite::Error> {
        let mut statement = connection.prepare(
            "SELECT DISTINCT json_extract(entry.value, '$.id') FROM \
             (SELECT models FROM model_cache WHERE version BETWEEN 11 AND 13 \
              AND provider_id LIKE 'openai-codex:%' \
              AND instr(substr(provider_id, 14), ':') = 0 \
              ORDER BY updated_at DESC LIMIT 1) AS cache, json_each(cache.models) AS entry \
             WHERE json_type(entry.value, '$.id') = 'text' \
              AND json_extract(entry.value, '$.provider') = 'openai-codex' \
             LIMIT 1000",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    })();
    result
        .unwrap_or_default()
        .into_iter()
        .filter(|id| !id.trim().is_empty() && !id.contains(['\n', '\r']))
        .collect()
}

fn data_root(location: &RuntimeLocationInfo) -> Option<PathBuf> {
    if location.mode == RuntimeLocationMode::WslDirect {
        let wsl = location.wsl.as_ref()?;
        // The Windows process cannot infer a Linux shell's XDG_DATA_HOME.
        // Default-root WSL metadata is unknown rather than reading a possibly
        // stale agent.db next to config.yml. Custom roots are unambiguous.
        let home = wsl.linux_user_root.as_ref()?;
        if wsl.linux_path.trim_end_matches('/')
            == format!("{}/.omp/agent", home.trim_end_matches('/'))
        {
            return None;
        }
        return Some(location.host_path.clone());
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(home) = dirs::home_dir() {
        let config_dir = std::env::var("PI_CONFIG_DIR").ok().or_else(|| {
            crate::coding::open_code::shell_env::get_env_from_shell_config("PI_CONFIG_DIR")
        });
        if config_dir
            .filter(|value| !value.is_empty() && value != ".omp")
            .is_some()
        {
            // A different native config root changes which agent directory
            // qualifies for XDG routing. Do not guess its credential location.
            return None;
        }
        if location.host_path == home.join(".omp").join("agent") {
            let xdg = std::env::var("XDG_DATA_HOME").ok();
            if xdg.is_none()
                && crate::coding::open_code::shell_env::get_env_from_shell_config("XDG_DATA_HOME")
                    .filter(|value| !value.is_empty())
                    .is_some()
            {
                // GUI and shell environments differ; static shell parsing
                // cannot tell whether a conditional assignment actually ran.
                return None;
            }
            if let Some(xdg) = xdg.filter(|value| !value.is_empty()) {
                let candidate = PathBuf::from(xdg).join("omp");
                match candidate.try_exists() {
                    Ok(true) => return Some(candidate),
                    Err(_) => return None,
                    Ok(false) => {}
                }
            }
        }
    }
    Some(location.host_path.clone())
}

pub async fn read_subscription(
    location: &RuntimeLocationInfo,
    has_api_key_override: bool,
) -> (OmpCodexSubscription, Vec<String>) {
    let is_wsl = location.mode == RuntimeLocationMode::WslDirect;
    let powershell = cfg!(target_os = "windows") && !is_wsl;
    let root = location
        .wsl
        .as_ref()
        .filter(|_| is_wsl)
        .map(|wsl| wsl.linux_path.clone())
        .unwrap_or_else(|| location.host_path.to_string_lossy().into_owned());
    // A timed-out filesystem lookup must not block the configuration editor.
    let program = "omp";
    let mut summary = OmpCodexSubscription {
        status: "unknown".to_string(),
        has_api_key_override,
        login_command: command_for(&root, program, powershell, "login openai-codex"),
        models_command: command_for(
            &root,
            program,
            powershell,
            "models openai-codex --json --no-extensions",
        ),
        shell: if powershell { "powershell" } else { "posix" }.to_string(),
        wsl_distro: location
            .wsl
            .as_ref()
            .filter(|_| is_wsl)
            .map(|wsl| wsl.distro.clone()),
    };
    let location = location.clone();
    let metadata = tokio::task::spawn_blocking(move || {
        // Resolve the executable only for guidance; no login is started.
        let program = if is_wsl {
            "omp".to_string()
        } else {
            resolve_local_omp_program()
                .path
                .to_string_lossy()
                .into_owned()
        };
        let Some(root) = data_root(&location) else {
            return ("unknown", Vec::new(), program);
        };
        (read_status(&root), read_model_ids(&root), program)
    });
    match tokio::time::timeout(Duration::from_secs(2), metadata).await {
        Ok(Ok((status, models, program))) => {
            summary.status = status.to_string();
            summary.login_command = command_for(&root, &program, powershell, "login openai-codex");
            summary.models_command = command_for(
                &root,
                &program,
                powershell,
                "models openai-codex --json --no-extensions",
            );
            (summary, models)
        }
        _ => (summary, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_status_does_not_create_database_and_invalid_schema_is_unknown() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(read_status(directory.path()), "not_configured");
        assert!(!directory.path().join("agent.db").exists());
        assert!(read_model_ids(directory.path()).is_empty());
        assert!(!directory.path().join("models.db").exists());
        fs::write(directory.path().join("agent.db"), b"invalid database").unwrap();
        assert_eq!(read_status(directory.path()), "unknown");
    }

    #[test]
    fn status_reads_only_active_oauth_and_never_changes_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE auth_credentials(provider TEXT, credential_type TEXT, disabled_cause TEXT, data TEXT);
                 INSERT INTO auth_credentials VALUES
                 ('openai-codex', 'api_key', NULL, 'fake-key'),
                 ('openai-codex', 'oauth', 'disabled', 'fake-disabled'),
                 ('other', 'oauth', NULL, 'fake-other');",
            )
            .unwrap();
        assert_eq!(read_status(directory.path()), "not_configured");
        connection
            .execute(
                "INSERT INTO auth_credentials VALUES ('openai-codex', 'oauth', NULL, ?1)",
                ["not-even-valid-json-secret-fixture"],
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        assert_eq!(read_status(directory.path()), "configured");
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn model_cache_excludes_old_and_custom_endpoint_entries() {
        let directory = tempfile::tempdir().unwrap();
        let connection = Connection::open(directory.path().join("models.db")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE model_cache(provider_id TEXT, version INTEGER, updated_at INTEGER, models TEXT);",
            )
            .unwrap();
        for (key, version, timestamp, id) in [
            ("openai-codex:old", 10, 1, "old"),
            ("openai-codex:1.0", 13, 2, "native-model"),
            ("openai-codex:1.0:hash", 13, 3, "proxy-model"),
            ("openai-codex:future", 14, 4, "future-model"),
        ] {
            let models = serde_json::json!([{
                "id": id, "provider": "openai-codex",
                "headers": {"Authorization": "fake-secret"}
            }]);
            connection
                .execute(
                    "INSERT INTO model_cache VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![key, version, timestamp, models.to_string()],
                )
                .unwrap();
        }
        assert_eq!(read_model_ids(directory.path()), vec!["native-model"]);
    }

    #[test]
    fn guidance_quotes_roots_and_selects_profile_before_subcommand() {
        assert_eq!(command_for("/tmp/a'b $x", "/bin/omp", false, "login openai-codex"), "PI_CODING_AGENT_DIR='/tmp/a'\\''b $x' '/bin/omp' --profile default login openai-codex");
        assert_eq!(command_for("C:\\a'b", "C:\\Program Files\\omp.cmd", true, "login openai-codex"), "& { $previous = $env:PI_CODING_AGENT_DIR; try { $env:PI_CODING_AGENT_DIR='C:\\a''b'; & 'C:\\Program Files\\omp.cmd' --profile default login openai-codex } finally { $env:PI_CODING_AGENT_DIR=$previous } }");
        assert!(command_for("/tmp/a\nb", "omp", false, "login openai-codex").is_empty());
    }

    #[test]
    fn unsupported_auth_schema_is_unknown_not_logged_out() {
        let directory = tempfile::tempdir().unwrap();
        let connection = Connection::open(directory.path().join("agent.db")).unwrap();
        connection
            .execute_batch("CREATE TABLE auth_credentials(provider TEXT, disabled INTEGER);")
            .unwrap();
        assert_eq!(read_status(directory.path()), "unknown");
    }

    #[test]
    fn locked_auth_database_is_unknown_not_logged_out() {
        let directory = tempfile::tempdir().unwrap();
        let connection = Connection::open(directory.path().join("agent.db")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE auth_credentials(provider TEXT, credential_type TEXT, disabled_cause TEXT);
                 BEGIN EXCLUSIVE;",
            )
            .unwrap();
        assert_eq!(read_status(directory.path()), "unknown");
    }

    #[test]
    fn wsl_default_root_does_not_guess_linux_xdg_from_windows_environment() {
        use crate::coding::runtime_location::WslLocationInfo;
        let mut location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from(r"\\wsl.localhost\Ubuntu\home\test\.omp\agent"),
            wsl: Some(WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/test/.omp/agent".to_string(),
                linux_user_root: Some("/home/test".to_string()),
            }),
        };
        assert!(data_root(&location).is_none());
        location.wsl.as_mut().unwrap().linux_path = "/home/test/custom-omp".to_string();
        location.host_path = PathBuf::from(r"\\wsl.localhost\Ubuntu\home\test\custom-omp");
        assert_eq!(data_root(&location), Some(location.host_path));
    }
}
