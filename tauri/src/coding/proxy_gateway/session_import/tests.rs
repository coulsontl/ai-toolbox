use super::*;
use crate::coding::proxy_gateway::types::GatewayCliKey;
use crate::coding::proxy_gateway::{
    pricing,
    types::{GatewayRequestLogFilters, ModelPricing},
    usage_stats,
};
use serde_json::{json, Value};
use std::io::Write;

#[path = "native_tests.rs"]
mod native_tests;

#[path = "websocket_tests.rs"]
mod websocket_tests;

const NOW: i64 = 1_800_000_100;
const THEN: i64 = NOW - 60;
const PARENT: &str = "11111111-1111-4111-8111-111111111111";
const CHILD: &str = "22222222-2222-4222-8222-222222222222";

fn write_jsonl(path: &Path, values: &[Value]) {
    let mut file = fs::File::create(path).unwrap();
    for value in values {
        writeln!(file, "{value}").unwrap();
    }
}

fn claude_message(id: &str, output: u64) -> Value {
    json!({"type":"assistant", "sessionId":"claude-session", "timestamp":THEN,
        "message":{"id":id,"model":"usage-test-model",
            "usage":{"input_tokens":100,"output_tokens":output,"cache_read_input_tokens":80,"cache_creation_input_tokens":20}}})
}

fn run_sync(
    db: &SqliteDbState,
    cli: impl Into<GatewayUsageTool>,
    root: &Path,
) -> GatewaySessionUsageImportResult {
    sync_sources(db, &[(cli.into(), root.to_path_buf())], NOW).unwrap()
}

fn count(db: &SqliteDbState) -> u64 {
    usage_stats::usage_summary(db, None, None, None, true)
        .unwrap()
        .total_requests
}

#[test]
fn claude_sync_is_incremental_updates_final_usage_and_prices_without_gateway() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session.jsonl");
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO model_pricing (model_id, display_name, input_cost_per_million,
            output_cost_per_million, cache_read_cost_per_million, cache_creation_cost_per_million)
            VALUES ('usage-test-model','Test','2','4','0.2','2.5')",
            [],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
    write_jsonl(&file, &[claude_message("msg-1", 10)]);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_tokens, 210);
    assert_eq!(summary.total_cost_usd, "0.000306");
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).parsed_records,
        0
    );

    let mut append = fs::OpenOptions::new().append(true).open(&file).unwrap();
    writeln!(append, "{}", claude_message("msg-1", 30)).unwrap();
    drop(append);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).updated_records,
        1
    );
    assert_eq!(count(&db), 1);
    let logs =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(logs.data[0].output_tokens, 30);
    assert_eq!(logs.data[0].data_source, "session");
    let detail = usage_stats::request_log_detail_from_summary(&db, "SESSION:msg-1")
        .unwrap()
        .unwrap();
    assert_eq!(detail.summary.data_source.as_deref(), Some("session"));
    assert_eq!(detail.summary.status_code, None);
    assert!(detail.request_body.is_none());
    let http_successes = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            status_code: Some(200),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(http_successes.total, 0);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None, true)
            .unwrap()
            .total_cost_usd,
        "0.000386"
    );
}

#[test]
fn claude_zero_token_responses_are_counted_once_and_can_receive_final_usage() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("zero-usage.jsonl");
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let message = json!({
        "type": "assistant", "timestamp": THEN, "sessionId": "glm-session",
        "message": {"id": "chatcmpl-zero", "role": "assistant", "model": "glm-5.2",
            "usage": {"input_tokens": 0, "output_tokens": 0,
                      "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}
    });
    let mut synthetic = message.clone();
    synthetic["message"]["id"] = json!("synthetic-response");
    synthetic["message"]["model"] = json!("<synthetic>");
    let mut empty_usage = message.clone();
    empty_usage["message"]["id"] = json!("missing-usage");
    empty_usage["message"]["usage"] = json!({});
    let mut user_message = message.clone();
    user_message["type"] = json!("user");
    user_message["message"]["role"] = json!("user");
    user_message["message"]["id"] = json!("user-message");
    write_jsonl(
        &file,
        &[
            message.clone(),
            message.clone(),
            synthetic,
            empty_usage,
            user_message,
        ],
    );
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
    let models = usage_stats::model_stats(
        &db,
        Some(THEN - 1),
        Some(NOW),
        Some(GatewayCliKey::Claude.into()),
        true,
    )
    .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model, "glm-5.2");
    assert_eq!(models[0].request_count, 1);
    assert_eq!(models[0].total_tokens, 0);
    assert_eq!(models[0].total_cost_usd, "0.000000");
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).parsed_records,
        0
    );

    let mut final_message = message.clone();
    final_message["message"]["usage"]["input_tokens"] = json!(100);
    final_message["message"]["usage"]["output_tokens"] = json!(20);
    write_jsonl(&file, &[message, final_message]);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).updated_records,
        1
    );
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 1);
    assert_eq!(summary.total_tokens, 120);
}

#[test]
fn parser_revision_revisits_unchanged_claude_files_without_discarding_the_ledger() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("previously-scanned.jsonl");
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let known_message = claude_message("already-imported", 10);
    write_jsonl(&file, &[known_message.clone()]);
    run_sync(&db, GatewayCliKey::Claude, root.path());
    let mut zero_message = claude_message("previously-skipped-zero", 0);
    zero_message["message"]["model"] = json!("glm-5.2");
    zero_message["message"]["usage"] = json!({"input_tokens": 0, "output_tokens": 0});
    write_jsonl(&file, &[known_message, zero_message]);
    let source_id = "claude:previously-scanned";
    let mut old_state = load_states(&db).unwrap().remove(source_id).unwrap();
    let metadata = fs::metadata(&file).unwrap();
    old_state.parser_revision = 0;
    old_state.modified_nanos = modified_nanos(&metadata);
    old_state.size = metadata.len();
    db.with_conn(|conn| save_state(conn, source_id, &old_state))
        .unwrap();

    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
    assert_eq!(count(&db), 2);
    let state = load_states(&db).unwrap().remove(source_id).unwrap();
    assert_eq!(
        state.parser_revision,
        parsers::revision(GatewayCliKey::Claude.into())
    );
    assert_eq!(state.records.len(), 2);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).parsed_records,
        0
    );
}

#[test]
fn persisted_ledger_does_not_reinsert_pruned_history_when_a_file_changes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session.jsonl");
    let db_path = root.path().join("usage.sqlite");
    let db = SqliteDbState::open(db_path.clone()).unwrap();
    write_jsonl(&file, &[claude_message("old", 10)]);
    run_sync(&db, GatewayCliKey::Claude, root.path());
    db.with_conn(|conn| {
        conn.execute("DELETE FROM proxy_request_logs", [])
            .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
    drop(db);
    let db = SqliteDbState::open(db_path).unwrap();
    write_jsonl(
        &file,
        &[claude_message("old", 10), claude_message("new", 11)],
    );
    let result = run_sync(&db, GatewayCliKey::Claude, root.path());
    assert_eq!(result.inserted_records, 1);
    assert_eq!(count(&db), 1);
    let old_exists = db
        .with_conn(|conn| usage_stats::request_exists(conn, "SESSION:old"))
        .unwrap();
    assert!(!old_exists);
}

#[test]
fn usage_and_sync_ledger_roll_back_together() {
    let root = tempfile::tempdir().unwrap();
    write_jsonl(
        &root.path().join("session.jsonl"),
        &[claude_message("atomic", 10)],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_session_state BEFORE INSERT ON gateway_session_usage_state
         BEGIN SELECT RAISE(ABORT, 'simulated cursor failure'); END;",
        )
        .map_err(|error| error.to_string())
    })
    .unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).failed_files,
        1
    );
    assert_eq!(count(&db), 0);
    assert!(load_states(&db).unwrap().is_empty());
    db.with_conn(|conn| {
        conn.execute_batch("DROP TRIGGER fail_session_state")
            .map_err(|error| error.to_string())
    })
    .unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
}

fn token_event(total: u64, input: u64, output: u64, at: i64) -> Value {
    json!({"type":"event_msg","timestamp":at,"payload":{"type":"token_count","info":{
        "total_token_usage":{"input_tokens":total,"cached_input_tokens":0,"output_tokens":total / 10},
        "last_token_usage":{"input_tokens":input,"cached_input_tokens":input / 2,"output_tokens":output,"reasoning_output_tokens":output / 2}
    },"rate_limits":{"limit_id":"codex"}}})
}

fn codex_cache_write_event(input: u64, cached: u64, cache_creation: u64, output: u64) -> Value {
    let usage = json!({
        "input_tokens": input,
        "cached_input_tokens": cached,
        "cache_write_input_tokens": cache_creation,
        "output_tokens": output,
        "reasoning_output_tokens": 0,
        "total_tokens": input + output,
    });
    json!({"type":"event_msg","timestamp":THEN,"payload":{"type":"token_count","info":{
        "total_token_usage":usage,"last_token_usage":usage
    },"rate_limits":{"limit_id":"codex"}}})
}

fn write_codex_cache_write_session(path: &Path, events: &[Value]) {
    let mut records = vec![
        json!({"type":"session_meta","timestamp":THEN - 10,"payload":{"id":PARENT}}),
        json!({"type":"turn_context","payload":{"model":"gpt-6-astra"}}),
    ];
    records.extend_from_slice(events);
    write_jsonl(path, &records);
}

fn insert_codex_cache_write_proxy(db: &SqliteDbState) {
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                created_at, duration_ms, status_code, stream_outcome, data_source, total_cost_usd)
             VALUES ('codex-cache-write-proxy', 'provider', 'codex', 'gpt-6-astra',
                3, 808, 223222, 7211, ?1, 123590, 200, 'completed', 'proxy', '0.353790')",
            [THEN - 1],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
}

fn insert_codex_cache_write_pricing(db: &SqliteDbState) {
    pricing::upsert_model_pricing(
        db,
        ModelPricing {
            model_id: "gpt-6-astra".into(),
            display_name: "GPT-6 Astra".into(),
            input_cost_per_million: "10".into(),
            output_cost_per_million: "50".into(),
            cache_read_cost_per_million: "1".into(),
            cache_creation_cost_per_million: "12.5".into(),
        },
    )
    .unwrap();
}

#[test]
fn codex_cache_writes_round_trip_through_native_usage_and_pricing() {
    let root = tempfile::tempdir().unwrap();
    write_codex_cache_write_session(
        &root.path().join(format!("rollout-{PARENT}.jsonl")),
        &[codex_cache_write_event(230436, 223222, 7211, 808)],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    insert_codex_cache_write_pricing(&db);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Codex, root.path()).inserted_records,
        1
    );
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_input_tokens, 3);
    assert_eq!(summary.total_cache_read_tokens, 223222);
    assert_eq!(summary.total_cache_creation_tokens, 7211);
    assert_eq!(summary.total_output_tokens, 808);
    assert_eq!(summary.total_tokens, 231244);
    assert_eq!(summary.total_cost_usd, "0.353790");
    let logs =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(logs.data[0].input_tokens, 3);
    assert_eq!(logs.data[0].cache_creation_tokens, 7211);
    assert_eq!(logs.data[0].data_source, "session");
    assert_eq!(
        run_sync(&db, GatewayCliKey::Codex, root.path()).parsed_records,
        0
    );
}

#[test]
fn codex_cumulative_cache_writes_use_deltas_and_ignore_repeated_lanes() {
    let root = tempfile::tempdir().unwrap();
    let mut first = codex_cache_write_event(100, 20, 60, 10);
    first["payload"]["info"]
        .as_object_mut()
        .unwrap()
        .remove("last_token_usage");
    let mut duplicate = first.clone();
    duplicate["payload"]["rate_limits"]["limit_id"] = json!("review");
    let mut second = codex_cache_write_event(180, 40, 100, 30);
    second["timestamp"] = json!(THEN + 1);
    second["payload"]["info"]
        .as_object_mut()
        .unwrap()
        .remove("last_token_usage");
    write_codex_cache_write_session(
        &root.path().join(format!("rollout-{PARENT}.jsonl")),
        &[first, duplicate, second],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Codex, root.path()).inserted_records,
        2
    );
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_input_tokens, 40);
    assert_eq!(summary.total_cache_read_tokens, 40);
    assert_eq!(summary.total_cache_creation_tokens, 100);
    assert_eq!(summary.total_output_tokens, 30);
    assert_eq!(summary.total_tokens, 210);
}

#[test]
fn codex_cache_writes_deduplicate_in_either_arrival_order() {
    for gateway_first in [false, true] {
        let root = tempfile::tempdir().unwrap();
        write_codex_cache_write_session(
            &root.path().join(format!("rollout-{PARENT}.jsonl")),
            &[codex_cache_write_event(230436, 223222, 7211, 808)],
        );
        let db = SqliteDbState::in_memory_for_test().unwrap();
        if gateway_first {
            insert_codex_cache_write_proxy(&db);
        }
        assert_eq!(
            run_sync(&db, GatewayCliKey::Codex, root.path()).parsed_records,
            1
        );
        if !gateway_first {
            insert_codex_cache_write_proxy(&db);
        }
        run_sync(&db, GatewayCliKey::Codex, root.path());
        let logs =
            usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true)
                .unwrap();
        assert_eq!(logs.total, 1, "gateway_first={gateway_first}");
        assert_eq!(logs.data[0].data_source, "proxy");
        let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.total_tokens, 231244);
        assert_eq!(summary.total_cost_usd, "0.353790");
        assert_eq!(
            run_sync(&db, GatewayCliKey::Codex, root.path()).updated_records,
            0
        );
    }
}

#[test]
fn codex_cache_write_revision_repairs_unchanged_native_rows_and_keeps_the_ledger() {
    for with_proxy in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(format!("rollout-{PARENT}.jsonl"));
        let db_path = root.path().join("usage.db");
        let db = SqliteDbState::open(db_path.clone()).unwrap();
        insert_codex_cache_write_pricing(&db);
        let event = codex_cache_write_event(230436, 223222, 7211, 808);
        let mut legacy_event = event.clone();
        for counters in ["total_token_usage", "last_token_usage"] {
            legacy_event["payload"]["info"][counters]
                .as_object_mut()
                .unwrap()
                .remove("cache_write_input_tokens");
        }
        write_codex_cache_write_session(&file, &[legacy_event]);
        run_sync(&db, GatewayCliKey::Codex, root.path());
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, None, true)
                .unwrap()
                .total_cost_usd,
            "0.335762"
        );
        write_codex_cache_write_session(&file, &[event]);
        let source_before = fs::read(&file).unwrap();
        let mut states = load_states(&db).unwrap();
        let (source_id, state) = states
            .iter_mut()
            .find(|(_, state)| !state.records.is_empty())
            .unwrap();
        // Simulate revision 2 having scanned this exact file but dropped cache writes.
        let metadata = fs::metadata(&file).unwrap();
        state.parser_revision = 2;
        state.modified_nanos = modified_nanos(&metadata);
        state.size = metadata.len();
        state.pending = false;
        db.with_conn(|conn| save_state(conn, source_id, state))
            .unwrap();
        if with_proxy {
            insert_codex_cache_write_proxy(&db);
        }

        let result = run_sync(&db, GatewayCliKey::Codex, root.path());
        assert_eq!(result.failed_files, 0);
        assert_eq!(result.inserted_records, 0);
        assert_eq!(result.updated_records, 1);
        assert_eq!(fs::read(&file).unwrap(), source_before);
        let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.total_input_tokens, 3);
        assert_eq!(summary.total_cache_creation_tokens, 7211);
        assert_eq!(summary.total_cost_usd, "0.353790");
        let state = load_states(&db).unwrap().remove(source_id).unwrap();
        assert_eq!(
            state.parser_revision,
            parsers::revision(GatewayUsageTool::Codex)
        );
        assert_eq!(state.records.len(), 1);
        assert_eq!(
            state
                .records
                .values()
                .next()
                .unwrap()
                .matched_proxy_id
                .as_deref(),
            with_proxy.then_some("codex-cache-write-proxy")
        );
        drop(db);
        let db = SqliteDbState::open(db_path).unwrap();
        assert_eq!(
            run_sync(&db, GatewayCliKey::Codex, root.path()).parsed_records,
            0
        );
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, None, true).unwrap(),
            summary
        );
    }
}

#[test]
fn codex_known_cache_write_mismatch_does_not_merge_independent_calls() {
    let root = tempfile::tempdir().unwrap();
    write_codex_cache_write_session(
        &root.path().join(format!("rollout-{PARENT}.jsonl")),
        &[codex_cache_write_event(230436, 223222, 7210, 808)],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    insert_codex_cache_write_proxy(&db);
    run_sync(&db, GatewayCliKey::Codex, root.path());
    assert_eq!(count(&db), 2);
}

#[test]
fn codex_prefers_per_request_usage_and_ignores_repeated_snapshot_lanes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(format!("rollout-{PARENT}.jsonl"));
    let first = token_event(1000, 100, 10, THEN);
    let mut duplicate_lane = first.clone();
    duplicate_lane["payload"]["rate_limits"]["limit_id"] = json!("review");
    write_jsonl(
        &file,
        &[
            json!({"type":"session_meta","timestamp":THEN - 10,"payload":{"id":PARENT}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            first,
            duplicate_lane,
            token_event(1100, 50, 5, THEN + 1),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::Codex, root.path());
    assert_eq!(result.inserted_records, 2);
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_input_tokens, 75);
    assert_eq!(summary.total_cache_read_tokens, 75);
    assert_eq!(summary.total_output_tokens, 15);
}

#[test]
fn codex_child_does_not_rebill_the_parent_replay_prefix() {
    let root = tempfile::tempdir().unwrap();
    let parent_event = token_event(100, 100, 10, THEN);
    write_jsonl(
        &root.path().join(format!("rollout-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN - 10,"payload":{"id":PARENT}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            parent_event.clone(),
        ],
    );
    write_jsonl(
        &root.path().join(format!("rollout-{CHILD}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN + 1,"payload":{"id":CHILD,"forked_from_id":PARENT}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            parent_event,
            token_event(150, 50, 5, THEN + 2),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Codex, root.path()).inserted_records,
        2
    );
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None, true)
            .unwrap()
            .total_tokens,
        165
    );
}

#[test]
fn codex_archive_keeps_the_same_import_identity() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let archive = root.path().join("archived_sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::create_dir_all(&archive).unwrap();
    let filename = format!("rollout-{PARENT}.jsonl");
    write_jsonl(
        &sessions.join(&filename),
        &[
            json!({"type":"session_meta","payload":{"id":PARENT}}),
            token_event(100, 100, 10, THEN),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Codex, &sessions);
    fs::rename(sessions.join(&filename), archive.join(&filename)).unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Codex, &archive).inserted_records,
        0
    );
    assert_eq!(count(&db), 1);
}

fn gemini_message(id: &str, output: u64) -> Value {
    json!({"type":"gemini","id":id,"content":"answer","model":"gemini-2.5-pro","timestamp":THEN,
        "tokens":{"input":100,"output":output,"cached":80,"thoughts":5}})
}

#[test]
fn gemini_json_and_jsonl_use_fresh_input_and_keep_paid_rewound_messages() {
    let root = tempfile::tempdir().unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    fs::write(
        root.path().join("session-legacy.json"),
        json!({"sessionId":"old","messages":[gemini_message("a", 10)]}).to_string(),
    )
    .unwrap();
    write_jsonl(
        &root.path().join("session-stream.jsonl"),
        &[
            json!({"$set":{"sessionId":"stream"}}),
            gemini_message("b", 10),
            gemini_message("b", 20),
            json!({"$rewindTo":"b"}),
            gemini_message("c", 10),
        ],
    );
    assert_eq!(
        run_sync(&db, GatewayCliKey::Gemini, root.path()).inserted_records,
        3
    );
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_input_tokens, 60);
    assert_eq!(summary.total_cache_read_tokens, 240);
    assert_eq!(summary.total_output_tokens, 55);
}

#[test]
fn large_transcripts_and_partial_final_lines_do_not_lose_usage() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.jsonl");
    let mut file = fs::File::create(&path).unwrap();
    writeln!(file, "{}", "x".repeat(17 * 1024 * 1024)).unwrap();
    writeln!(file, "{}", claude_message("large", 10)).unwrap();
    write!(file, "{{\"message\":").unwrap();
    drop(file);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
    assert_eq!(count(&db), 1);
}

fn insert_proxy(db: &SqliteDbState, id: &str) {
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, created_at, status_code, data_source)
             VALUES (?1, 'provider', 'gemini', 'gemini-2.5-pro', 20, 15, 80, 0, ?2, 200, 'proxy')",
            params![id, THEN],
        ).map_err(|error| error.to_string())?;
        Ok(())
    }).unwrap();
}

#[test]
fn distinct_claude_envelopes_are_not_merged_when_usage_matches() {
    for gateway_first in [false, true] {
        for zero_usage in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut message = claude_message("native-independent", 10);
            let (input, output, read, creation) = if zero_usage {
                (0_i64, 0_i64, 0_i64, 0_i64)
            } else {
                (100, 10, 80, 20)
            };
            message["message"]["usage"] = json!({
                "input_tokens": input, "output_tokens": output,
                "cache_read_input_tokens": read, "cache_creation_input_tokens": creation,
            });
            write_jsonl(&root.path().join("independent.jsonl"), &[message]);
            let db = SqliteDbState::in_memory_for_test().unwrap();
            let insert_gateway = || {
                db.with_conn(|conn| {
                    conn.execute(
                        "INSERT INTO proxy_request_logs
                         (request_id, provider_id, app_type, model, input_tokens, output_tokens,
                          cache_read_tokens, cache_creation_tokens, created_at, status_code, data_source)
                         VALUES ('SESSION:proxy-independent', 'provider', 'claude', 'usage-test-model',
                                 ?1, ?2, ?3, ?4, ?5, 200, 'proxy')",
                        params![input, output, read, creation, THEN],
                    ).map_err(|error| error.to_string())?;
                    Ok(())
                }).unwrap();
            };
            if gateway_first {
                insert_gateway();
            }
            run_sync(&db, GatewayCliKey::Claude, root.path());
            if !gateway_first {
                insert_gateway();
            }
            run_sync(&db, GatewayCliKey::Claude, root.path());
            assert_eq!(
                count(&db),
                2,
                "gateway_first={gateway_first}, zero_usage={zero_usage}"
            );
            assert_eq!(
                usage_stats::usage_summary(&db, None, None, None, true)
                    .unwrap()
                    .total_tokens,
                (input + output + read + creation) as u64 * 2,
            );
        }
    }
}

#[test]
fn upgraded_ledger_restores_a_native_row_with_a_conflicting_proxy_identity() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("old-matched.jsonl");
    write_jsonl(&file, &[claude_message("native-recoverable", 10)]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Claude, root.path());
    let source_id = "claude:old-matched";
    let mut state = load_states(&db).unwrap().remove(source_id).unwrap();
    state.parser_revision -= 1;
    let record = state.records.get_mut("SESSION:native-recoverable").unwrap();
    record.envelope_id = None;
    record.matched_proxy_id = Some("SESSION:other-proxy".to_string());
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE proxy_request_logs SET request_id = 'SESSION:other-proxy',
             provider_id = 'provider', data_source = 'proxy'",
            [],
        )
        .map_err(|error| error.to_string())?;
        save_state(conn, source_id, &state)
    })
    .unwrap();

    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
    assert_eq!(count(&db), 2);
    let state = load_states(&db).unwrap().remove(source_id).unwrap();
    let record = state.records.get("SESSION:native-recoverable").unwrap();
    assert_eq!(record.envelope_id.as_deref(), Some("native-recoverable"));
    assert_eq!(record.matched_proxy_id, None);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).parsed_records,
        0
    );
}

#[test]
fn final_usage_rechecks_an_earlier_heuristic_proxy_match() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session-final.jsonl");
    write_jsonl(&file, &[gemini_message("native-final", 10)]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    insert_proxy(&db, "proxy-without-an-envelope");
    run_sync(&db, GatewayCliKey::Gemini, root.path());
    assert_eq!(count(&db), 1);

    write_jsonl(&file, &[gemini_message("native-final", 30)]);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Gemini, root.path()).inserted_records,
        1
    );
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 2);
    assert_eq!(summary.total_output_tokens, 50);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Gemini, root.path()).inserted_records,
        0
    );
}

#[test]
fn gateway_and_session_usage_converge_in_either_arrival_order() {
    for gateway_first in [false, true] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("session.json"), "{}").unwrap();
        write_jsonl(
            &root.path().join("session-test.jsonl"),
            &[
                json!({"$set":{"sessionId":"dedup"}}),
                gemini_message("a", 10),
            ],
        );
        let db = SqliteDbState::in_memory_for_test().unwrap();
        if gateway_first {
            insert_proxy(&db, "gateway-copy");
        }
        run_sync(&db, GatewayCliKey::Gemini, root.path());
        if !gateway_first {
            insert_proxy(&db, "gateway-copy");
        }
        run_sync(&db, GatewayCliKey::Gemini, root.path());
        assert_eq!(count(&db), 1, "gateway_first={gateway_first}");
        let logs =
            usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true)
                .unwrap();
        assert_eq!(logs.data[0].data_source, "proxy");
    }
}

#[test]
fn one_proxy_record_cannot_suppress_two_distinct_native_invocations() {
    let root = tempfile::tempdir().unwrap();
    write_jsonl(
        &root.path().join("session-test.jsonl"),
        &[
            json!({"$set":{"sessionId":"dedup"}}),
            gemini_message("a", 10),
            gemini_message("b", 10),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    insert_proxy(&db, "one-gateway-call");
    run_sync(&db, GatewayCliKey::Gemini, root.path());
    assert_eq!(count(&db), 2);
}

#[test]
fn proxy_execution_interval_and_delayed_session_writes_converge() {
    // Local Codex evidence: session timestamps precede gateway completion by
    // 109/119 seconds inside long requests. Issue #340 also shows a 21-second
    // delay after completion. Neither is a separate invocation.
    for (session_offset, duration_ms) in [(-109, 151_653), (-119, 239_252), (21, 9_800)] {
        for gateway_first in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut message = gemini_message("interval", 10);
            message["timestamp"] = json!(THEN + session_offset);
            write_jsonl(&root.path().join("session-interval.jsonl"), &[message]);
            let db = SqliteDbState::in_memory_for_test().unwrap();
            let insert_gateway = || {
                insert_proxy(&db, "gateway-interval");
                db.with_conn(|conn| {
                    conn.execute(
                        "UPDATE proxy_request_logs SET duration_ms = ?1 WHERE request_id = 'gateway-interval'",
                        [duration_ms],
                    ).map_err(|error| error.to_string())?;
                    Ok(())
                }).unwrap();
            };
            if gateway_first {
                insert_gateway();
            }
            run_sync(&db, GatewayCliKey::Gemini, root.path());
            if !gateway_first {
                insert_gateway();
            }
            // Revisit an unchanged source and a native row older than one hour.
            let result = sync_sources(
                &db,
                &[(GatewayCliKey::Gemini.into(), root.path().to_path_buf())],
                NOW + 7200,
            )
            .unwrap();
            assert_eq!(result.failed_files, 0);
            assert_eq!(
                count(&db),
                1,
                "offset={session_offset}, gateway_first={gateway_first}"
            );
            let logs =
                usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true)
                    .unwrap();
            assert_eq!(logs.data[0].data_source, "proxy");
            assert_eq!(
                run_sync(&db, GatewayCliKey::Gemini, root.path()).updated_records,
                0
            );
        }
    }
}

#[test]
fn interval_matching_rejects_outside_timestamps_and_ambiguous_candidates() {
    for (offset, second_proxy, expected) in [(-163, false, 2), (31, false, 2), (21, true, 3)] {
        let root = tempfile::tempdir().unwrap();
        let mut message = gemini_message("bounded", 10);
        message["timestamp"] = json!(THEN + offset);
        write_jsonl(&root.path().join("session-bounded.jsonl"), &[message]);
        let db = SqliteDbState::in_memory_for_test().unwrap();
        insert_proxy(&db, "gateway-one");
        if second_proxy {
            insert_proxy(&db, "gateway-two");
        }
        db.with_conn(|conn| {
            conn.execute("UPDATE proxy_request_logs SET duration_ms = 151653", [])
                .map_err(|error| error.to_string())?;
            Ok(())
        })
        .unwrap();
        run_sync(&db, GatewayCliKey::Gemini, root.path());
        assert_eq!(
            count(&db),
            expected,
            "offset={offset}, second_proxy={second_proxy}"
        );
    }
}

#[test]
fn final_native_usage_replaces_a_partial_row_when_the_proxy_arrives() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session-test.jsonl");
    write_jsonl(
        &file,
        &[
            json!({"$set":{"sessionId":"dedup"}}),
            gemini_message("a", 1),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Gemini, root.path()).inserted_records,
        1
    );
    insert_proxy(&db, "completed-gateway-call");
    db.with_conn(|conn| {
        conn.execute("UPDATE proxy_request_logs SET total_cost_usd = '0.012345' WHERE request_id = 'completed-gateway-call'", [])
            .map_err(|error| error.to_string())?;
        Ok(())
    }).unwrap();
    write_jsonl(
        &file,
        &[
            json!({"$set":{"sessionId":"dedup"}}),
            gemini_message("a", 10),
        ],
    );
    assert_eq!(
        run_sync(&db, GatewayCliKey::Gemini, root.path()).updated_records,
        1
    );
    assert_eq!(count(&db), 1);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Gemini, root.path()).updated_records,
        0
    );
    let logs =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(logs.data[0].data_source, "proxy");
    assert_eq!(logs.data[0].output_tokens, 15);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None, true)
            .unwrap()
            .total_cost_usd,
        "0.012345"
    );
}

#[test]
fn maintenance_failure_keeps_imported_usage_and_its_success_result() {
    let root = tempfile::tempdir().unwrap();
    let mut message = claude_message("maintenance-failure", 10);
    message["timestamp"] = json!((Utc::now() - chrono::Duration::days(400)).timestamp());
    write_jsonl(&root.path().join("session.jsonl"), &[message]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute_batch("ALTER TABLE usage_daily_rollups DROP COLUMN latency_sample_count")
            .map_err(|error| error.to_string())
    })
    .unwrap();
    let result = run_sync(&db, GatewayCliKey::Claude, root.path());
    assert_eq!(result.inserted_records, 1);
    assert_eq!(result.failed_files, 0);
    assert_eq!(count(&db), 1);
    assert!(!load_states(&db).unwrap().is_empty());
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        0
    );
}

#[test]
fn legacy_manual_import_is_adopted_without_duplicate_usage() {
    use std::hash::{Hash, Hasher};
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session-legacy.jsonl");
    let message = json!({"id":"response-one","model":"gemini-test","timestamp":THEN,
        "usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":10}});
    write_jsonl(&file, &[message]);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "gemini".hash(&mut hasher);
    file.to_string_lossy().hash(&mut hasher);
    0usize.hash(&mut hasher);
    let legacy_id = format!("SESSION:{:016x}", hasher.finish());
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model, input_tokens,
             output_tokens, created_at, status_code, data_source)
             VALUES (?1, 'session', 'gemini', 'gemini-test', 100, 10, ?2, 200, 'session')",
            params![legacy_id, THEN],
        ).map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Gemini, root.path()).inserted_records,
        0
    );
    assert_eq!(count(&db), 1);
    assert!(!db
        .with_conn(|conn| usage_stats::request_exists(conn, &legacy_id))
        .unwrap());
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None, true)
            .unwrap()
            .total_tokens,
        110
    );
}

#[test]
fn legacy_snapshot_rows_converge_when_the_canonical_record_already_exists() {
    use std::hash::{Hash, Hasher};
    for canonical_exists in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("session-legacy.jsonl");
        let messages = [10, 20].map(|output| {
            json!({
                "id": "response-one", "model": "gemini-test", "timestamp": THEN,
                "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": output}
            })
        });
        write_jsonl(&file, &messages);
        let db = SqliteDbState::in_memory_for_test().unwrap();
        if canonical_exists {
            run_sync(&db, GatewayCliKey::Gemini, root.path());
        }
        for (index, output) in [10, 20].into_iter().enumerate() {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            "gemini".hash(&mut hasher);
            file.to_string_lossy().hash(&mut hasher);
            index.hash(&mut hasher);
            let legacy_id = format!("SESSION:{:016x}", hasher.finish());
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                        input_tokens, output_tokens, created_at, status_code, data_source)
                     VALUES (?1, 'session', 'gemini', 'gemini-test', 100, ?2, ?3, 200, 'session')",
                    params![legacy_id, output, THEN],
                )
                .map_err(|error| error.to_string())?;
                Ok(())
            })
            .unwrap();
        }
        // An unchanged invocation can be revisited after another append or a
        // parser upgrade, while both legacy and canonical rows are present.
        writeln!(fs::OpenOptions::new().append(true).open(&file).unwrap()).unwrap();
        run_sync(&db, GatewayCliKey::Gemini, root.path());
        assert_eq!(count(&db), 1, "canonical_exists={canonical_exists}");
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, None, true)
                .unwrap()
                .total_tokens,
            120,
            "canonical_exists={canonical_exists}"
        );
        run_sync(&db, GatewayCliKey::Gemini, root.path());
        assert_eq!(count(&db), 1);
    }
}

#[test]
fn pending_usage_is_rechecked_without_another_file_write() {
    let root = tempfile::tempdir().unwrap();
    let mut message = claude_message("pending", 10);
    message["timestamp"] = json!(NOW);
    write_jsonl(&root.path().join("session.jsonl"), &[message]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        0
    );
    let result = sync_sources(
        &db,
        &[(GatewayCliKey::Claude.into(), root.path().to_path_buf())],
        NOW + 4,
    )
    .unwrap();
    assert_eq!(result.inserted_records, 1);
    assert_eq!(count(&db), 1);
}

#[test]
fn native_history_is_automatically_archived_without_gateway_and_not_reimported() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session.jsonl");
    let mut old_message = claude_message("archived-native", 10);
    old_message["timestamp"] = json!((Utc::now() - chrono::Duration::days(400)).timestamp());
    old_message["message"]["model"] = Value::Null;
    write_jsonl(&file, &[old_message.clone()]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
    assert_eq!(count(&db), 1);
    let logs =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert!(logs.data.is_empty());
    let models = usage_stats::model_stats(&db, None, None, None, true).unwrap();
    assert_eq!(models[0].model, "unknown");
    assert_eq!(models[0].avg_latency_ms, None);
    assert_eq!(models[0].total_tokens, 210);
    let providers = usage_stats::provider_stats(&db, None, None, None, true).unwrap();
    assert_eq!(providers[0].avg_latency_ms, None);
    assert_eq!(providers[0].cache_hit_rate, Some(0.4));

    old_message["message"]["usage"]["output_tokens"] = json!(20);
    write_jsonl(&file, &[old_message, claude_message("new-native", 30)]);
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).inserted_records,
        1
    );
    assert_eq!(count(&db), 2);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None, true)
            .unwrap()
            .total_output_tokens,
        40
    );
}

#[test]
fn opencode_reads_wal_updates_and_deduplicates_legacy_json_without_writing_native_db() {
    let root = tempfile::tempdir().unwrap();
    let native_path = root.path().join("opencode.db");
    let native = Connection::open(&native_path).unwrap();
    native.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE session (id TEXT PRIMARY KEY, time_updated INTEGER);
         CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
         INSERT INTO session VALUES ('ses-1', 1);",
    ).unwrap();
    let message = json!({"id":"msg-1","sessionID":"ses-1","role":"assistant","modelID":"native-model",
        "time":{"created":THEN * 1000, "completed":THEN * 1000},
        "tokens":{"input":100,"output":10,"reasoning":5,"cache":{"read":80,"write":20}},"cost":0.0012});
    native
        .execute(
            "INSERT INTO message VALUES ('msg-1', 'ses-1', ?1, 2, ?2)",
            params![THEN * 1000, message.to_string()],
        )
        .unwrap();
    let legacy = root.path().join("storage/message/ses-1");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("msg-1.json"), message.to_string()).unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::OpenCode, root.path());
    assert_eq!(count(&db), 1);
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_input_tokens, 100);
    assert_eq!(summary.total_output_tokens, 15);
    assert_eq!(summary.total_cost_usd, "0.001200");
    let mut updated = message.clone();
    updated["tokens"]["output"] = json!(20);
    native
        .execute(
            "UPDATE message SET data = ?1, time_updated = 3 WHERE id = 'msg-1'",
            [updated.to_string()],
        )
        .unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::OpenCode, root.path()).updated_records,
        1
    );
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None, true)
            .unwrap()
            .total_output_tokens,
        25
    );
    let native_count: i64 = native
        .query_row("SELECT COUNT(*) FROM message", [], |row| row.get(0))
        .unwrap();
    assert_eq!(native_count, 1);
}

/// OpenCode 2.0 stores assistant rows as `session_message` rows tagged with
/// `type`, with the native model nested under `model.id`.
fn opencode_v2_assistant(model: &str, input: u64, completed: i64) -> Value {
    json!({"type":"assistant","agent":"build",
        "model":{"providerID":"opencode","id":model},
        "time":{"created":completed * 1000,"completed":completed * 1000},
        "tokens":{"input":input,"output":10,"reasoning":5,"cache":{"read":80,"write":20}},
        "cost":0.0012})
}

#[test]
fn opencode_v2_reads_the_renamed_store_and_skips_inherited_fork_history() {
    let root = tempfile::tempdir().unwrap();
    let native_path = root.path().join("opencode.db");
    let native = Connection::open(&native_path).unwrap();
    native
        .execute_batch(&format!(
            "CREATE TABLE session_v2 (id TEXT PRIMARY KEY, parent_id TEXT, fork_session_id TEXT,
                time_created INTEGER, time_updated INTEGER);
             CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER,
                time_created INTEGER, time_updated INTEGER, data TEXT);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, data TEXT);
             INSERT INTO session_v2 VALUES ('ses-root', NULL, NULL, {root}, 1);
             INSERT INTO session_v2 VALUES ('ses-fork', NULL, 'ses-root', {fork_at}, 2);",
            root = THEN * 1000,
            fork_at = (THEN + 10) * 1000
        ))
        .unwrap();
    // The fork keeps the parent's message plus one of its own; OpenCode's own
    // stats only count what the fork created after branching.
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-own', 'ses-root', 'assistant', 0, ?1, 1, ?2)",
            params![
                THEN * 1000,
                opencode_v2_assistant("root-model", 100, THEN).to_string()
            ],
        )
        .unwrap();
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-inherited', 'ses-fork', 'assistant', 0, ?1, 2, ?2)",
            params![
                (THEN + 5) * 1000,
                opencode_v2_assistant("inherited-model", 7_000, THEN + 5).to_string()
            ],
        )
        .unwrap();
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-fork-own', 'ses-fork', 'assistant', 1, ?1, 3, ?2)",
            params![
                (THEN + 15) * 1000,
                opencode_v2_assistant("fork-model", 200, THEN + 15).to_string()
            ],
        )
        .unwrap();
    // User and unfinished rows carry no countable usage.
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-user', 'ses-root', 'user', 1, ?1, 1, '{\"type\":\"user\"}')",
            params![THEN * 1000],
        )
        .unwrap();
    let mut unfinished = opencode_v2_assistant("pending-model", 300, THEN);
    unfinished["time"] = json!({"created": THEN * 1000});
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-pending', 'ses-root', 'assistant', 2, ?1, 1, ?2)",
            params![(THEN + 1) * 1000, unfinished.to_string()],
        )
        .unwrap();

    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::OpenCode, root.path());

    let models: Vec<String> = db
        .with_conn(|conn| {
            let mut query = conn
                .prepare("SELECT model FROM proxy_request_logs ORDER BY model")
                .map_err(|error| error.to_string())?;
            let rows = query
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(models, vec!["fork-model", "root-model"]);
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 2);
    assert_eq!(summary.total_input_tokens, 300);
    assert_eq!(summary.total_output_tokens, 30);
}

/// Read-only audit against a store written by a real OpenCode 2.0 install.
/// Point `AI_TOOLBOX_OPENCODE_V2_DB` at its `opencode.db`; only a temp copy is
/// ever opened for writing.
#[test]
#[ignore = "Audit against a real OpenCode 2.0 store via AI_TOOLBOX_OPENCODE_V2_DB"]
fn local_opencode_v2_store_imports_per_request_usage() {
    let path = std::env::var("AI_TOOLBOX_OPENCODE_V2_DB")
        .expect("set AI_TOOLBOX_OPENCODE_V2_DB to a real OpenCode 2.0 opencode.db");
    let root = tempfile::tempdir().unwrap();
    // The live store is usually still in the write-ahead log, so the copy has to
    // carry it instead of relying on a checkpoint.
    for suffix in ["", "-wal", "-shm"] {
        let from = format!("{path}{suffix}");
        if std::path::Path::new(&from).is_file() {
            fs::copy(&from, root.path().join(format!("opencode.db{suffix}"))).unwrap();
        }
    }
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::OpenCode, root.path());
    let rows: Vec<(String, i64, i64, i64, i64)> = db
        .with_conn(|conn| {
            let mut query = conn
                .prepare(
                    "SELECT model, input_tokens, output_tokens, cache_read_tokens,
                            cache_creation_tokens
                     FROM proxy_request_logs ORDER BY created_at, model",
                )
                .map_err(|error| error.to_string())?;
            let rows = query
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok(rows)
        })
        .unwrap();
    println!(
        "imported={} inserted={} failed={} scanned={} rows={:?}",
        result.parsed_records,
        result.inserted_records,
        result.failed_files,
        result.scanned_files,
        rows
    );
    assert!(!rows.is_empty(), "a real 2.0 store must yield usage rows");
    assert!(
        rows.iter().all(|row| row.0 != "unknown"),
        "2.0 rows must resolve the nested model id: {rows:?}"
    );
}

#[test]
fn opencode_v2_store_wins_over_legacy_tables_kept_by_the_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let native_path = root.path().join("opencode.db");
    let native = Connection::open(&native_path).unwrap();
    // The 2.0 upgrade copies the V1 rows into the new tables and leaves the old
    // ones frozen, so both stores exist and only the renamed one is live.
    native
        .execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, time_updated INTEGER);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
             CREATE TABLE session_v2 (id TEXT PRIMARY KEY, parent_id TEXT, fork_session_id TEXT,
                time_created INTEGER, time_updated INTEGER, tokens_input INTEGER DEFAULT 0,
                tokens_output INTEGER DEFAULT 0, tokens_reasoning INTEGER DEFAULT 0,
                tokens_cache_read INTEGER DEFAULT 0, tokens_cache_write INTEGER DEFAULT 0,
                model TEXT);
             CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER,
                time_created INTEGER, time_updated INTEGER, data TEXT);",
        )
        .unwrap();
    let legacy = json!({"id":"msg-1","role":"assistant","modelID":"legacy-model",
        "time":{"created":THEN * 1000,"completed":THEN * 1000},
        "tokens":{"input":1,"output":1,"cache":{"read":0,"write":0}},"cost":0});
    native
        .execute_batch(&format!(
            "INSERT INTO session VALUES ('ses-1', 1);
             INSERT INTO message VALUES ('msg-1', 'ses-1', {created}, 1, '{legacy}');",
            created = THEN * 1000,
            legacy = legacy.to_string().replace('\'', "''")
        ))
        .unwrap();
    // The live aggregate matches its own rows, so the import adds no remainder.
    native
        .execute(
            "INSERT INTO session_v2 (id, fork_session_id, time_created, time_updated,
                tokens_input, tokens_output, tokens_reasoning, model)
             VALUES ('ses-1', NULL, ?1, 1, 400, 10, 0, ?2)",
            params![
                THEN * 1000,
                json!({"id": "live-model", "providerID": "mock"}).to_string()
            ],
        )
        .unwrap();
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-1', 'ses-1', 'assistant', 0, ?1, 1, ?2)",
            params![
                THEN * 1000,
                opencode_v2_assistant("live-model", 400, THEN).to_string()
            ],
        )
        .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::OpenCode, root.path());
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 1);
    assert_eq!(summary.total_input_tokens, 400);
}

/// Title generation and other auxiliary calls leave no message row; their usage
/// only exists in the session aggregate, and a fork's aggregate can carry the
/// parent's copied history, which must never be re-recorded.
#[test]
fn opencode_v2_records_only_the_settled_aggregate_remainder() {
    let root = tempfile::tempdir().unwrap();
    let native_path = root.path().join("opencode.db");
    let native = Connection::open(&native_path).unwrap();
    native
        .execute_batch(
            "CREATE TABLE session_v2 (id TEXT PRIMARY KEY, parent_id TEXT, fork_session_id TEXT,
                time_created INTEGER, time_updated INTEGER, tokens_input INTEGER DEFAULT 0,
                tokens_output INTEGER DEFAULT 0, tokens_reasoning INTEGER DEFAULT 0,
                tokens_cache_read INTEGER DEFAULT 0, tokens_cache_write INTEGER DEFAULT 0,
                model TEXT);
             CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER,
                time_created INTEGER, time_updated INTEGER, data TEXT);",
        )
        .unwrap();
    // Two aggregate calls against one row: the second is the auxiliary one.
    native
        .execute(
            "INSERT INTO session_v2 (id, fork_session_id, time_created, time_updated,
                tokens_input, tokens_output, tokens_reasoning, model)
             VALUES ('ses-1', NULL, ?1, ?3, 2468, 912, 0, ?2)",
            params![
                THEN * 1000,
                json!({"id": "mock-model", "providerID": "mock"}).to_string(),
                THEN * 1000
            ],
        )
        .unwrap();
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-1', 'ses-1', 'assistant', 0, ?1, ?2, ?3)",
            params![
                THEN * 1000,
                THEN * 1000,
                opencode_v2_assistant("mock-model", 1234, THEN).to_string()
            ],
        )
        .unwrap();
    // A fork whose aggregate still holds the copied parent total records nothing.
    native
        .execute(
            "INSERT INTO session_v2 (id, fork_session_id, time_created, time_updated,
                tokens_input, tokens_output, model)
             VALUES ('ses-fork', 'ses-1', ?1, ?3, 9999, 9999, ?2)",
            params![
                (THEN + 30) * 1000,
                json!({"id": "mock-model", "providerID": "mock"}).to_string(),
                THEN * 1000
            ],
        )
        .unwrap();
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-2', 'ses-fork', 'assistant', 0, ?1, ?2, ?3)",
            params![
                (THEN + 31) * 1000,
                (THEN + 31) * 1000,
                opencode_v2_assistant("fork-model", 5, THEN + 31).to_string()
            ],
        )
        .unwrap();
    // A settlement still in flight defers the remainder to a later sync.
    native
        .execute(
            "INSERT INTO session_v2 (id, fork_session_id, time_created, time_updated,
                tokens_input, tokens_output, model)
             VALUES ('ses-busy', NULL, ?1, ?3, 5000, 5000, ?2)",
            params![
                THEN * 1000,
                json!({"id": "mock-model", "providerID": "mock"}).to_string(),
                THEN * 1000
            ],
        )
        .unwrap();
    let mut unfinished = opencode_v2_assistant("mock-model", 1, THEN);
    unfinished["time"] = json!({"created": THEN * 1000});
    native
        .execute(
            "INSERT INTO session_message VALUES ('msg-3', 'ses-busy', 'assistant', 0, ?1, ?2, ?3)",
            params![THEN * 1000, THEN * 1000, unfinished.to_string()],
        )
        .unwrap();

    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::OpenCode, root.path());
    let rows: Vec<(String, String, i64, i64, String)> = db
        .with_conn(|conn| {
            let mut query = conn
                .prepare(
                    "SELECT request_id, model, input_tokens, output_tokens, data_source
                     FROM proxy_request_logs ORDER BY request_id",
                )
                .map_err(|error| error.to_string())?;
            let rows = query
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(
        rows,
        vec![
            // The aggregate minus the settled row: input 2468 - 1234, output
            // (912 + 0 reasoning) - (10 + 5 reasoning).
            (
                "SESSION:opencode:ses-1:aggregate".to_string(),
                "mock-model".to_string(),
                1234,
                897,
                "session".to_string(),
            ),
            (
                "SESSION:opencode:ses-1:msg-1".to_string(),
                "mock-model".to_string(),
                1234,
                15,
                "session".to_string(),
            ),
            (
                "SESSION:opencode:ses-fork:msg-2".to_string(),
                "fork-model".to_string(),
                5,
                15,
                "session".to_string(),
            ),
        ]
    );
}
