use ai_toolbox_lib::coding::codex::history_sync;
use rusqlite::Connection;
use std::fs;
use tempfile::TempDir;

struct TestCodexHistory {
    _temp_dir: TempDir,
    root: std::path::PathBuf,
}

impl TestCodexHistory {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let root = temp_dir.path().join("codex-home");
        fs::create_dir_all(root.join("sessions/2026/05/21")).expect("create sessions");
        fs::write(
            root.join("config.toml"),
            r#"model_provider = "new-provider"
model = "gpt-new"
"#,
        )
        .expect("write config");
        let conn = Connection::open(root.join("state_5.sqlite")).expect("open db");
        conn.execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                model TEXT,
                title TEXT,
                updated_at INTEGER,
                archived INTEGER
            );
            INSERT INTO threads (id, model_provider, model, title, updated_at, archived)
            VALUES ('11111111-1111-1111-1111-111111111111', 'old-provider', 'gpt-old', 'Old thread', 1700000000, 0);
            "#,
        )
        .expect("create db");
        fs::write(
            root.join("sessions/2026/05/21/rollout-2026-05-21T00-00-00-11111111-1111-1111-1111-111111111111.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::json!({
                    "type": "session_meta",
                    "payload": {
                        "id": "11111111-1111-1111-1111-111111111111",
                        "cwd": "/tmp/original-project",
                        "model_provider": "old-provider",
                        "model": "gpt-old"
                    }
                }),
                serde_json::json!({
                    "type": "response_item",
                    "payload": {"type": "message", "role": "user", "content": "hello"}
                })
            ),
        )
        .expect("write session");
        Self {
            _temp_dir: temp_dir,
            root,
        }
    }
}

#[test]
fn codex_history_sync_round_trip_restores_latest_backup() {
    let env = TestCodexHistory::new();

    let status = history_sync::get_status(&env.root).expect("status");
    assert!(status.has_work);
    assert_eq!(status.provider_mismatch_threads, 1);

    let sync_result = history_sync::sync(&env.root, None).expect("sync");
    assert_eq!(sync_result.updated_thread_rows, 1);
    assert_eq!(sync_result.updated_session_files, 1);
    assert!(!sync_result.partial_success);
    let conn = Connection::open(env.root.join("state_5.sqlite")).expect("open db");
    let model: String = conn
        .query_row("SELECT model FROM threads", [], |row| row.get(0))
        .expect("model");
    assert_eq!(model, "gpt-old");
    let first_line = fs::read_to_string(env.root.join("sessions/2026/05/21/rollout-2026-05-21T00-00-00-11111111-1111-1111-1111-111111111111.jsonl"))
        .expect("read session")
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(first_line.contains("new-provider"));
    assert!(first_line.contains("gpt-old"));
    assert!(!first_line.contains("gpt-new"));

    let restored = history_sync::restore_latest(&env.root).expect("restore latest");
    assert!(restored.safety_backup_path.contains("pre-restore"));
    let conn = Connection::open(env.root.join("state_5.sqlite")).expect("open db");
    let provider: String = conn
        .query_row("SELECT model_provider FROM threads", [], |row| row.get(0))
        .expect("provider");
    assert_eq!(provider, "old-provider");
}
