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
    sync_sources(db, &[(cli.into(), root.to_path_buf())], NOW, false).unwrap()
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

/// A translating gateway opens every stream with `input: 1 / output: 1` (the
/// real counts only arrive with `message_delta`). When the CLI persists that
/// opening snapshot, it is not a billed call: importing it created thousands
/// of 2-token "requests" and a wrong total.
#[test]
fn claude_message_start_placeholders_are_not_imported() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("gateway-placeholder.jsonl");
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let mut opening = claude_message("gateway-message", 1);
    opening["message"]["usage"] = json!({
        "input_tokens": 1, "output_tokens": 1,
        "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0
    });
    // A settled reply with the same tiny counters is a real measurement the
    // gateway reported, so it must stay.
    let mut settled_small = opening.clone();
    settled_small["message"]["id"] = json!("settled-small");
    settled_small["message"]["stop_reason"] = json!("end_turn");
    settled_small["message"]["model"] = json!("glm-5.2");
    // The settled snapshot of the same response carries the real counts.
    let mut final_usage = claude_message("gateway-message", 120);
    final_usage["message"]["stop_reason"] = json!("tool_use");
    write_jsonl(&file, &[opening, settled_small, final_usage]);

    let result = run_sync(&db, GatewayCliKey::Claude, root.path());
    assert_eq!(result.inserted_records, 2);
    let logs =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    let ids = logs
        .data
        .iter()
        .map(|row| row.trace_id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"SESSION:gateway-message"));
    assert!(ids.contains(&"SESSION:settled-small"));
    let placeholder = logs
        .data
        .iter()
        .find(|row| row.trace_id == "SESSION:gateway-message")
        .unwrap();
    assert_eq!(placeholder.input_tokens, 100);
    assert_eq!(placeholder.output_tokens, 120);
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

/// A paginated parent writes a second rollout named
/// `rollout-<ts>-<thread>_<page>`. The trailing id is the page, so a child that
/// names its parent by *thread* id must still resolve it — and must see the
/// parent's records from **both** pages, or it re-bills the copied history.
#[test]
fn codex_child_sees_every_page_of_a_paginated_parent() {
    const PAGE: &str = "33333333-3333-4333-8333-333333333333";
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();

    let first_page = token_event(100, 100, 10, THEN - 20);
    let second_page = token_event(200, 100, 10, THEN - 10);
    write_jsonl(
        &sessions.join(format!("rollout-2026-09-02T05-00-00-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN - 30,"payload":{"id":PARENT}}),
            first_page.clone(),
        ],
    );
    write_jsonl(
        &sessions.join(format!("rollout-2026-09-02T05-50-45-{PARENT}_{PAGE}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN - 25,
                "payload":{"id":PARENT,"history_mode":"paginated"}}),
            second_page.clone(),
        ],
    );
    // The child copied both pages before doing its own work.
    write_jsonl(
        &sessions.join(format!("rollout-2026-09-02T05-55-00-{CHILD}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN,"payload":{"id":CHILD,"forked_from_id":PARENT}}),
            first_page,
            second_page,
            token_event(300, 100, 10, THEN + 1),
        ],
    );

    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::Codex, &sessions);
    assert_eq!(
        result.failed_files, 0,
        "a paginated parent must resolve, not defer its child"
    );
    // Two parent pages + the child's own event. The copied prefix is replay and
    // must not be billed again; missing the second page would let it through.
    assert_eq!(result.inserted_records, 3);
    assert_eq!(count(&db), 3);
}

/// Both pages carry the same thread id in `session_meta`, so a request id built
/// from the thread id alone collides and the newer page overwrites the older.
#[test]
fn codex_paginated_pages_keep_distinct_request_ids() {
    const PAGE: &str = "33333333-3333-4333-8333-333333333333";
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    write_jsonl(
        &sessions.join(format!("rollout-2026-09-02T05-00-00-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","payload":{"id":PARENT}}),
            token_event(100, 100, 10, THEN - 20),
        ],
    );
    write_jsonl(
        &sessions.join(format!("rollout-2026-09-02T05-50-45-{PARENT}_{PAGE}.jsonl")),
        &[
            json!({"type":"session_meta","payload":{"id":PARENT}}),
            token_event(200, 100, 10, THEN - 10),
        ],
    );

    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::Codex, &sessions);
    assert_eq!(
        result.inserted_records, 2,
        "neither page may overwrite the other"
    );
    let ids = db
        .with_conn(|conn| {
            let mut statement = conn
                .prepare("SELECT request_id FROM proxy_request_logs ORDER BY request_id")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert!(
        ids[0] != ids[1],
        "paginated pages must not share a request id: {ids:?}"
    );
    assert!(
        ids.iter().any(|id| id.contains(PAGE)),
        "the page's own rollout id must appear in its key: {ids:?}"
    );
}

/// A single-file rollout has rollout id == thread id, so its key must stay
/// byte-identical to the pre-pagination format. Otherwise every existing
/// imported row would be re-keyed on the next sync.
#[test]
fn codex_single_file_rollout_keeps_its_legacy_request_id() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    write_jsonl(
        &sessions.join(format!("rollout-2026-09-02T05-00-00-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","payload":{"id":PARENT}}),
            token_event(100, 100, 10, THEN),
        ],
    );

    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Codex, &sessions);
    let id = db
        .with_conn(|conn| {
            conn.query_row("SELECT request_id FROM proxy_request_logs", [], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| error.to_string())
        })
        .unwrap();
    assert_eq!(id, format!("SESSION:codex:{PARENT}:token:1"));
}

/// Codex compresses cold rollouts in place; the collector must still find them.
#[test]
fn codex_compressed_rollout_is_scanned() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let text = format!(
        "{}\n{}\n",
        json!({"type":"session_meta","payload":{"id":PARENT}}),
        token_event(100, 100, 10, THEN),
    );
    fs::write(
        sessions.join(format!("rollout-2026-09-02T05-00-00-{PARENT}.jsonl.zst")),
        zstd::stream::encode_all(text.as_bytes(), 0).unwrap(),
    )
    .unwrap();

    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::Codex, &sessions);
    assert_eq!(
        result.scanned_files, 1,
        "the compressed rollout must be walked"
    );
    assert_eq!(result.inserted_records, 1);
    assert_eq!(count(&db), 1);
}

fn codex_token_usage_record(response_id: &str, usage: Value, at: i64) -> Value {
    json!({"type":"token_usage_record","timestamp":at,"payload":{
        "thread_id":PARENT,"session_id":PARENT,"response_id":response_id,
        "usage":usage,"thread_token_usage":usage,"turn_token_usage":usage}})
}

#[test]
fn codex_token_usage_record_is_counted_once_and_deducts_its_legacy_mirror() {
    let root = tempfile::tempdir().unwrap();
    let usage = json!({"input_tokens":1000,"cached_input_tokens":400,
        "cache_write_input_tokens":100,"output_tokens":50,"reasoning_output_tokens":10,
        "total_tokens":1050});
    // The authoritative record arrives first, then the legacy notification
    // repeating the same numbers as `last_token_usage`.
    let mirror = json!({"type":"event_msg","timestamp":THEN + 1,"payload":{
        "type":"token_count","info":{
            "total_token_usage":usage,"last_token_usage":usage},
        "rate_limits":{"limit_id":"codex"}}});
    write_jsonl(
        &root.path().join(format!("rollout-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN - 10,"payload":{"id":PARENT}}),
            json!({"type":"event_msg","timestamp":THEN - 9,"payload":{"type":"task_started"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            codex_token_usage_record("resp-one", usage.clone(), THEN),
            mirror,
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::Codex, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.inserted_records, 1, "the mirror must not add a row");
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_input_tokens, 500);
    assert_eq!(summary.total_cache_read_tokens, 400);
    assert_eq!(summary.total_cache_creation_tokens, 100);
    assert_eq!(summary.total_output_tokens, 50);
    let logs =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(
        logs.data[0].trace_id,
        format!("SESSION:codex:{PARENT}:resp-one")
    );
}

#[test]
fn codex_legacy_first_keeps_one_call_and_adopts_the_response_identity() {
    let root = tempfile::tempdir().unwrap();
    let usage = json!({"input_tokens":1000,"cached_input_tokens":400,
        "cache_write_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,
        "total_tokens":1050});
    let legacy = json!({"type":"event_msg","timestamp":THEN,"payload":{
        "type":"token_count","info":{
            "total_token_usage":usage,"last_token_usage":usage},
        "rate_limits":{"limit_id":"codex"}}});
    write_jsonl(
        &root.path().join(format!("rollout-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN - 10,"payload":{"id":PARENT}}),
            json!({"type":"event_msg","timestamp":THEN - 9,"payload":{"type":"task_started"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            legacy,
            codex_token_usage_record("resp-late", usage, THEN + 1),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::Codex, root.path());
    assert_eq!(result.inserted_records, 1, "one call, not two");
    let ids = db
        .with_conn(|conn| {
            let mut statement = conn
                .prepare("SELECT request_id FROM proxy_request_logs ORDER BY request_id")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(ids, vec![format!("SESSION:codex:{PARENT}:resp-late")]);
    // The upgraded identity also carries the upstream response id, so a later
    // sync can find the proxy row by envelope even when the heuristic does not
    // apply.
    let state = load_states(&db)
        .unwrap()
        .remove(&format!("codex:{PARENT}"))
        .unwrap();
    assert_eq!(
        state
            .records
            .get(&format!("SESSION:codex:{PARENT}:resp-late"))
            .and_then(|record| record.envelope_id.as_deref()),
        Some("resp-late")
    );
}

/// Issue #432: a Codex call that the gateway recorded as `incomplete` (an
/// upstream `response.incomplete` still bills its tokens) is invisible to the
/// usage-equal heuristic, which only accepts `completed` proxy rows. The
/// rollout's `token_usage_record.response_id` is the upstream envelope id, so
/// the native row must converge on that identity without the heuristic.
#[test]
fn codex_response_identity_converges_with_an_incomplete_proxy_row() {
    let root = tempfile::tempdir().unwrap();
    let usage = json!({"input_tokens":1000,"cached_input_tokens":400,
        "cache_write_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,
        "total_tokens":1050});
    write_jsonl(
        &root.path().join(format!("rollout-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN - 10,"payload":{"id":PARENT}}),
            json!({"type":"event_msg","timestamp":THEN - 9,"payload":{"type":"task_started"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            codex_token_usage_record("resp-incomplete", usage, THEN),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,request_model,
                input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,created_at,
                duration_ms,status_code,stream_outcome,transport,request_kind,data_source)
             VALUES ('SESSION:codex:provider:resp-incomplete','provider','codex','gpt-5','gpt-5',
                500,50,400,0,?1,2000,0,'incomplete','websocket','request','proxy')",
            [THEN],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();

    let result = run_sync(&db, GatewayCliKey::Codex, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(
        result.inserted_records, 0,
        "the response identity must suppress the native row"
    );
    assert_eq!(count(&db), 1);
    let logs =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(logs.data[0].data_source, "proxy");
    let state = load_states(&db)
        .unwrap()
        .remove(&format!("codex:{PARENT}"))
        .unwrap();
    let record = state
        .records
        .get(&format!("SESSION:codex:{PARENT}:resp-incomplete"))
        .unwrap();
    assert_eq!(record.envelope_id.as_deref(), Some("resp-incomplete"));
    assert_eq!(
        record.matched_proxy_id.as_deref(),
        Some("SESSION:codex:provider:resp-incomplete")
    );
}

#[test]
fn codex_fork_records_never_bill_the_child() {
    let root = tempfile::tempdir().unwrap();
    let usage = json!({"input_tokens":1000,"cached_input_tokens":0,
        "cache_write_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,
        "total_tokens":1050});
    // A copied record still names its owner in `thread_id`; the child's own
    // record names the child.
    let mut copied = codex_token_usage_record("resp-parent", usage.clone(), THEN);
    copied["payload"]["thread_id"] = json!(PARENT);
    let mut own = codex_token_usage_record("resp-child", usage, THEN + 1);
    own["payload"]["thread_id"] = json!(CHILD);
    write_jsonl(
        &root.path().join(format!("rollout-{CHILD}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":THEN - 10,"payload":{"id":CHILD}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            copied,
            own,
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayCliKey::Codex, root.path());
    assert_eq!(result.inserted_records, 1);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None, true)
            .unwrap()
            .total_input_tokens,
        1000
    );
}

/// A rebuild drops only what the collector derived, and re-imports it from the
/// transcripts. Gateway-served requests have no on-disk source and must survive.
#[test]
fn rebuild_reimports_session_usage_and_keeps_proxy_rows() {
    // `rebuild_sources` writes the process-global rebuild panel
    // (`ScanningFiles` / `BackingUp` / `Clearing`), so every test that calls it
    // must hold the panel lock or a parallel test sees somebody else's phase.
    let _panel = rebuild_panel_test_lock();
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    write_jsonl(
        &sessions.join(format!("rollout-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","payload":{"id":PARENT}}),
            token_event(100, 100, 10, THEN),
        ],
    );

    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Codex, &sessions);
    let imported = count(&db);
    assert_eq!(imported, 1);

    // A gateway-served request, standing in for real proxy traffic.
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, latency_ms, duration_ms, status_code, created_at, data_source)
             VALUES ('gw-1','provider','codex','gpt-5','gpt-5',5,6,0,0,'0','0','0','0','0',0,0,200,?1,'proxy')",
            [THEN],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
    // A rollup for the imported session row, which the rebuild must also drop.
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO usage_daily_rollups
                (date, app_type, provider_id, model, request_count, success_count,
                 input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                 total_cost_usd, avg_latency_ms)
             VALUES ('2026-01-01','codex','session','gpt-5',1,1,1,1,0,0,'0',0)",
            [],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();

    let result = rebuild_sources(
        &db,
        &[(GatewayUsageTool::Codex, sessions.clone())],
        &[GatewayUsageTool::Codex],
        NOW,
    )
    .unwrap();
    assert_eq!(result.inserted_records, 1, "the transcript is re-imported");
    assert_eq!(count(&db), 2, "the proxy row survives the rebuild");
    db.with_conn(|conn| {
        let rollups: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM usage_daily_rollups WHERE provider_id = 'session'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(rollups, 0, "stale session rollups are dropped, not doubled");
        Ok(())
    })
    .unwrap();
}

/// A rebuild preserves the ledger's matched-proxy ids: after the proxy detail
/// rows are rolled up and pruned, `find_matching_proxy` has nothing left to
/// match against, so the only thing standing between the re-import and a
/// doubled daily rollup is the marker the clear writes.
#[test]
fn a_rebuild_keeps_matches_for_archived_proxy_rows() {
    // Same global-panel hazard as `rebuild_reimports_session_usage_...`.
    let _panel = rebuild_panel_test_lock();
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    write_codex_cache_write_session(
        &sessions.join(format!("rollout-{PARENT}.jsonl")),
        // The matching counters (7211 cache-write), not the mismatch case's
        // 7210: this session must be recognized as the proxy row's twin.
        &[codex_cache_write_event(230436, 223222, 7211, 808)],
    );

    let db = SqliteDbState::in_memory_for_test().unwrap();
    insert_codex_cache_write_pricing(&db);
    insert_codex_cache_write_proxy(&db);
    // The session import matches the proxy row and suppresses the native one.
    let result = run_sync(&db, GatewayCliKey::Codex, &sessions);
    assert_eq!(
        result.inserted_records, 0,
        "the proxy row represents the call"
    );
    assert_eq!(count(&db), 1);

    // Archive the proxy row. The production pruner keeps its tokens in the
    // daily rollup under the real provider and deletes the detail row; the
    // rollup cutoff is wall-clock based, so the test simulates the second half
    // directly — the point is that no live proxy row remains to match.
    let detail_rows = |db: &SqliteDbState| -> u64 {
        db.with_conn(|conn| {
            let total: i64 = conn
                .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })
                .map_err(|error| error.to_string())?;
            Ok(total.max(0) as u64)
        })
        .unwrap()
    };
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO usage_daily_rollups
                (date, app_type, provider_id, model, request_count, success_count,
                 input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                 total_cost_usd, avg_latency_ms)
             VALUES ('2027-01-15','codex','provider','gpt-6-astra',1,1,3,808,223222,7211,'0.353790',0)",
            [],
        )
        .map_err(|error| error.to_string())?;
        conn.execute(
            "DELETE FROM proxy_request_logs WHERE request_id = 'codex-cache-write-proxy'",
            [],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
    assert_eq!(detail_rows(&db), 0, "the proxy detail row is pruned");

    // Rebuild: the marker must carry the match across the clear, or the same
    // call is imported as a second (session-sourced) detail row.
    let rebuilt = rebuild_sources(
        &db,
        &[(GatewayUsageTool::Codex, sessions.clone())],
        &[GatewayUsageTool::Codex],
        NOW,
    )
    .unwrap();
    assert_eq!(rebuilt.inserted_records, 0, "the archived match survives");
    assert_eq!(detail_rows(&db), 0, "no second detail row appears");

    // And the marker's record re-lands in the real source state with the match.
    let states = load_states(&db).unwrap();
    let source_id = format!(
        "codex:{}",
        source_identity(
            GatewayUsageTool::Codex,
            &sessions.join(format!("rollout-{PARENT}.jsonl"))
        )
    );
    let state = states.get(&source_id).expect("state re-imported");
    assert_eq!(
        state
            .records
            .values()
            .find_map(|record| record.matched_proxy_id.clone()),
        Some("codex-cache-write-proxy".to_string()),
        "the re-imported record keeps the archived proxy match"
    );
}

/// A rebuild whose source resolution came back empty must not clear what the
/// previous import wrote: the CLI home may point at a directory that exists
/// but is unreachable (unmounted WSL, moved sessions root), and the delete
/// path keys on `cli_key`, not on what the scan found.
#[test]
fn a_rebuild_without_sources_refuses_to_clear_existing_usage() {
    let _panel = rebuild_panel_test_lock();
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
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
    assert_eq!(count(&db), 1);

    // The root exists but holds nothing the parser reads (or resolved to
    // nothing at all): the rebuild must refuse rather than wipe the import.
    let empty_root = root.path().join("unmounted");
    fs::create_dir_all(&empty_root).unwrap();
    let error = rebuild_sources(
        &db,
        &[(GatewayUsageTool::Codex, empty_root.clone())],
        &[GatewayUsageTool::Codex],
        NOW,
    )
    .unwrap_err();
    assert!(
        error.contains("Refusing to rebuild"),
        "the refusal names its reason: {error}"
    );
    assert_eq!(count(&db), 1, "the imported history survives the refusal");

    // A machine with no imports at all rebuilds as a no-op, not an error.
    let fresh = SqliteDbState::in_memory_for_test().unwrap();
    let result = rebuild_sources(
        &fresh,
        &[(GatewayUsageTool::Codex, empty_root.clone())],
        &[GatewayUsageTool::Codex],
        NOW,
    )
    .unwrap();
    assert_eq!(result.inserted_records, 0);
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

/// Codex compresses rollouts in place (`x.jsonl` -> `x.jsonl.zst`, then the
/// original is deleted). `Path::file_stem` only strips the last extension, so
/// the ledger key used to drift from the UUID to the whole stem across a
/// compression cycle — the old state row orphaned, the same transcript
/// re-scanned as a brand-new source.
#[test]
fn codex_zst_rollout_keeps_the_same_import_identity() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let plain = sessions.join(format!("rollout-{PARENT}.jsonl"));
    write_jsonl(
        &plain,
        &[
            json!({"type":"session_meta","payload":{"id":PARENT}}),
            token_event(100, 100, 10, THEN),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Codex, &sessions);

    // The parser only reads JSONL; the zst copy stands in for the compressed
    // file the CLI publishes. The identity is decided from the path, so a
    // rename is enough to reproduce the drift.
    let compressed = sessions.join(format!("rollout-{PARENT}.jsonl.zst"));
    fs::rename(&plain, &compressed).unwrap();
    assert_eq!(
        run_sync(&db, GatewayCliKey::Codex, &sessions).inserted_records,
        0,
        "the compressed rollout keeps its pre-compression identity"
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

/// Gemini CLI also writes a tree beside `~/.gemini/tmp`: the desktop/IDE
/// integration keeps its sessions under `.gemini/gemini-cli`. The session file
/// matcher keys on the `session-*` basename, so a root pointed there imports
/// exactly like `tmp/` does.
#[test]
fn gemini_sessions_under_the_gemini_cli_root_are_imported() {
    let root = tempfile::tempdir().unwrap();
    let chats = root.path().join("project").join("chats");
    fs::create_dir_all(&chats).unwrap();
    write_jsonl(
        &chats.join("session-2026-05-10T06-52-a00e324c.jsonl"),
        &[
            json!({"$set":{"sessionId":"cli-root"}}),
            gemini_message("cli", 7),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Gemini, root.path()).inserted_records,
        1
    );
    // A non-session file in the same tree stays ignored.
    fs::write(chats.join("conversation_summaries.json"), "{}").unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Gemini, root.path()).inserted_records,
        0
    );
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

/// The exotic-id lookup must find both spellings a proxy row can carry:
/// Claude writes a bare `SESSION:<envelope>`, every other CLI scopes it with
/// app and provider. Splitting the old `OR` of concatenated expressions into
/// two indexed lookups is what makes a rebuild finish in seconds, so this
/// guards the semantics the split has to preserve.
#[test]
fn exact_envelope_match_finds_bare_and_scoped_proxy_ids() {
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        for (id, app_type, provider_id) in [
            ("SESSION:claude-envelope", "claude", "provider"),
            ("SESSION:codex:provider:codex-envelope", "codex", "provider"),
        ] {
            conn.execute(
                "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, created_at, status_code, request_kind, data_source)
                 VALUES (?1, ?2, ?3, 'usage-test-model', 1, 1, ?4, 200, 'request', 'proxy')",
                params![id, provider_id, app_type, THEN],
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
    .unwrap();

    let record = |cli_key: GatewayUsageTool, envelope: &str| SessionUsageRecord {
        metadata: SessionUsageMetadata::default(),
        request_id: String::new(),
        legacy_request_ids: Vec::new(),
        cli_key,
        model: "usage-test-model".into(),
        usage: TokenUsage {
            envelope_id: Some(envelope.into()),
            ..Default::default()
        },
        created_at: THEN,
        session_id: String::new(),
        reported_cost_usd: None,
        project_path: None,
        credits: None,
        duration_ms: None,
    };
    let find = |cli_key, envelope| {
        db.with_conn(|conn| {
            find_matching_proxy(conn, &record(cli_key, envelope), None)
                .map_err(|error| error.to_string())
        })
        .unwrap()
    };

    assert_eq!(
        find(GatewayUsageTool::Claude, "claude-envelope").as_deref(),
        Some("SESSION:claude-envelope"),
    );
    assert_eq!(
        find(GatewayUsageTool::Codex, "codex-envelope").as_deref(),
        Some("SESSION:codex:provider:codex-envelope"),
    );
    // The scoped form is namespaced by CLI: a Claude record must not adopt a
    // Codex proxy row even though both carry the same envelope text.
    assert_eq!(find(GatewayUsageTool::Claude, "codex-envelope"), None);
    assert_eq!(find(GatewayUsageTool::Codex, "claude-envelope"), None);
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

/// Tests that drive the process-global rebuild panel serialize on this lock:
/// Rust runs the tests of one module on parallel threads by default, and two
/// tests claiming the single slot at once would fail spuriously.
static REBUILD_PANEL_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn rebuild_panel_test_lock() -> std::sync::MutexGuard<'static, ()> {
    REBUILD_PANEL_TEST_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The background rebuild exposes one task slot: a second start must be
/// rejected while the first is running so two rebuilds never clear the same
/// CLI concurrently. A finished (or failed) task releases the slot.
#[test]
fn rebuild_slot_rejects_overlapping_starts_until_the_first_finishes() {
    let mut task = RebuildTask::default();
    claim_rebuild_slot(&mut task).unwrap();
    assert!(task.progress.as_ref().is_some_and(|p| !p.is_finished()));
    assert!(
        claim_rebuild_slot(&mut task).is_err(),
        "a second rebuild must not start while one is running"
    );

    task.progress.as_mut().unwrap().phase = SessionRebuildPhase::Done;
    claim_rebuild_slot(&mut task).expect("a finished rebuild releases the slot");
    assert_eq!(
        task.progress.as_ref().unwrap().phase,
        SessionRebuildPhase::BackingUp,
        "the new task starts from its first phase"
    );

    task.progress.as_mut().unwrap().phase = SessionRebuildPhase::Failed;
    claim_rebuild_slot(&mut task).expect("a failed rebuild releases the slot too");
}

/// A rebuild task that dies from a panic must still write a terminal record:
/// `claim_rebuild_slot` only accepts finished progress, so without the panic
/// guard the slot would stay claimed until the app restarts.
#[test]
fn a_panicked_rebuild_writes_a_terminal_record_and_frees_the_slot() {
    let payload: Box<dyn std::any::Any + Send> = Box::new("scan exploded".to_string());
    let message = panic_payload_message(payload.as_ref());
    assert!(message.contains("scan exploded"));

    let progress = panicked_rebuild_progress(message);
    assert!(progress.is_finished());
    assert!(progress
        .error
        .as_deref()
        .is_some_and(|error| error.contains("panicked")));

    let mut task = RebuildTask::default();
    claim_rebuild_slot(&mut task).unwrap();
    *task.progress.as_mut().unwrap() = progress;
    claim_rebuild_slot(&mut task).expect("a panicked rebuild releases the slot");
}

/// The 60-second sync shares `sync_sources` with a rebuild. A sync pass must
/// not report through the rebuild progress panel: while a freshly claimed
/// rebuild still waits on the sync mutex, an in-flight sync's scan phase would
/// otherwise overwrite the new task's panel before it even starts.
#[test]
fn a_periodic_sync_does_not_touch_the_rebuild_progress_panel() {
    let _panel = rebuild_panel_test_lock();
    let root = tempfile::tempdir().unwrap();
    write_jsonl(
        &root.path().join("session.jsonl"),
        &[claude_message("one", 10)],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    {
        let guard = rebuild_task();
        let mut task = guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        claim_rebuild_slot(&mut task).unwrap();
    }

    let result = sync_sources(
        &db,
        &[(GatewayCliKey::Claude.into(), root.path().to_path_buf())],
        NOW,
        false,
    )
    .unwrap();
    assert_eq!(result.inserted_records, 1, "the sync itself must still run");

    assert_eq!(
        take_rebuild_progress().map(|progress| progress.phase),
        Some(SessionRebuildPhase::BackingUp),
        "the sync must not have written its own phase into the pending rebuild"
    );
    update_rebuild_progress(|progress| {
        progress.phase = SessionRebuildPhase::Failed;
    });
    take_rebuild_progress();
}

/// Switching tabs unmounts the requests view, so the progress it polls must
/// live in the backend. A running task stays readable for every poll; a
/// finished one is handed out once and then cleared, otherwise every later
/// visit would re-report "rebuild finished".
#[test]
fn a_running_rebuild_snapshot_persists_while_a_finished_one_is_reported_once() {
    let _panel = rebuild_panel_test_lock();
    {
        let guard = rebuild_task();
        let mut task = guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        claim_rebuild_slot(&mut task).unwrap();
        task.progress.as_mut().unwrap().phase = SessionRebuildPhase::ImportingFiles;
    }

    assert_eq!(
        take_rebuild_progress().map(|progress| progress.phase),
        Some(SessionRebuildPhase::ImportingFiles),
        "a running task must stay visible across remounts"
    );
    assert_eq!(
        take_rebuild_progress().map(|progress| progress.phase),
        Some(SessionRebuildPhase::ImportingFiles),
        "polling again while running must not consume it"
    );

    update_rebuild_progress(|progress| progress.phase = SessionRebuildPhase::Done);
    assert_eq!(
        take_rebuild_progress().map(|progress| progress.phase),
        Some(SessionRebuildPhase::Done),
    );
    assert!(
        take_rebuild_progress().is_none(),
        "the finished report is consumed exactly once"
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
                false,
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

/// A local row that converges onto a proxy row is deleted, not merged: the
/// gateway measured that request itself, so the session estimate must not
/// reappear in the table through a later re-import.
#[test]
fn a_proxy_match_removes_the_session_estimate_instead_of_merging_it() {
    let root = tempfile::tempdir().unwrap();
    let mut message = gemini_message("guessed-duration", 10);
    message["timestamp"] = json!(THEN);
    write_jsonl(&root.path().join("session-guess.jsonl"), &[message]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Gemini, root.path());
    let before =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(before.data[0].data_source, "session");

    insert_proxy(&db, "gateway-copy");
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE proxy_request_logs SET duration_ms = 4_321 WHERE request_id = 'gateway-copy'",
            [],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
    run_sync(&db, GatewayCliKey::Gemini, root.path());
    let after =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(after.data.len(), 1);
    assert_eq!(after.data[0].data_source, "proxy");
    assert_eq!(after.data[0].duration_ms, 4_321);
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
fn repeated_source_failures_stay_quiet_until_the_source_recovers() {
    let log = SourceFailureLog::default();
    let missing = "Skipping local session usage a.jsonl: No such file or directory (os error 2)";
    assert!(log.should_log("pi:file:one", missing));
    assert!(!log.should_log("pi:file:one", missing));
    // A different source, or a different cause for the same source, still shows.
    assert!(log.should_log("codex:file:two", missing));
    assert!(log.should_log("pi:file:one", "Invalid session record at line 4"));
    log.clear("pi:file:one");
    assert!(log.should_log("pi:file:one", missing));
    assert!(!log.should_log("codex:file:two", missing));
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
        false,
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

/// A transcript mirror carries no token counts: Antigravity usage comes from
/// `conversations/<uuid>.db`. The mirror must not add zero-token rows.
#[test]
fn antigravity_transcripts_do_not_import_zero_token_rows() {
    let root = tempfile::tempdir().unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let logs_dir = root
        .path()
        .join("11111111-2222-3333-4444-555555555555")
        .join(".system_generated")
        .join("logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let content = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-20T10:00:00Z","content":"hello"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-20T10:00:02Z","content":"Hi there!","thinking":"greeting"}
{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-20T10:00:05Z","content":"how are you"}
{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-20T10:00:08Z","content":"Doing great!"}
"#;
    fs::write(logs_dir.join("transcript_full.jsonl"), content).unwrap();

    let stats = run_sync(&db, GatewayUsageTool::Antigravity, root.path());
    assert_eq!(stats.failed_files, 0);
    assert_eq!(stats.inserted_records, 0);

    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 0);
}

/// Compaction rewrites the transcript, so a crash mid-flush can leave a broken
/// row anywhere in the file. The usage import must skip it like
/// `session_manager::antigravity` does instead of failing the whole session.
#[test]
fn antigravity_transcripts_skip_broken_rows_instead_of_failing() {
    let root = tempfile::tempdir().unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let logs_dir = root
        .path()
        .join("11111111-2222-3333-4444-555555555555")
        .join(".system_generated")
        .join("logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let content = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-20T10:00:00Z","content":"hello"}
{"step_index":1,"source":"MODEL","type"
{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-20T10:00:05Z","content":"how are you"}
{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-20T10:00:08Z","content":"Doing great!"}
"#;
    fs::write(logs_dir.join("transcript_full.jsonl"), content).unwrap();

    let stats = run_sync(&db, GatewayUsageTool::Antigravity, root.path());

    assert_eq!(stats.failed_files, 0);
    assert_eq!(stats.inserted_records, 0);
}

#[test]
fn prime_agent_reads_model_changes_and_deduplicates_shared_events() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let nested = root.path().join("session-artifacts/run-1");
    fs::create_dir_all(&nested).unwrap();
    let entry = json!({"type":"message","id":"reply-1","timestamp":THEN,
        "message":{"role":"assistant","model":"unknown","responseId":"resp-1",
            "usage":{"input":100,"output":10,"cacheRead":80,"cacheWrite":20}}});
    write_jsonl(
        &sessions.join("prime.jsonl"),
        &[
            json!({"type":"session","id":"prime-session","timestamp":THEN-10}),
            json!({"type":"model_change","timestamp":THEN-5,"provider":"prime","modelId":"prime-model"}),
            entry.clone(),
            // A sub-agent attribution event must not bill the child again.
            json!({"type":"child_usage_attributed","id":"child","timestamp":THEN,
                "usage":{"input":999,"output":999}}),
        ],
    );
    // The same logical session's artifact copy repeats the event, including
    // the session header that names it.
    write_jsonl(
        &nested.join("artifact.jsonl"),
        &[
            json!({"type":"session","id":"prime-session","timestamp":THEN-10}),
            entry,
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::PrimeAgent, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.inserted_records, 1);
    let logs = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::PrimeAgent),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(logs.data[0].upstream_model_id, "prime-model");
    assert_eq!(logs.data[0].input_tokens, 100);
    assert_eq!(logs.data[0].cache_read_tokens, 80);
    assert_eq!(logs.data[0].cache_creation_tokens, 20);
}

#[test]
fn musecode_reads_completed_model_events_and_folds_reasoning_into_output() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("sessions/2026/09/02/run-a");
    fs::create_dir_all(&session).unwrap();
    write_jsonl(
        &session.join("session.jsonl"),
        &[
            json!({"stream":{"id":"muse-session"},"payload_type":"run.model.configured",
                "payload":{"record":{"run_stream":{"id":"run-1"},"model_id":"muse-model"}}}),
            json!({"stream":{"id":"muse-session"},"payload_type":"runtime.session",
                "payload":{"kind":"run","run_id":"run-1","event":{"kind":"model_completed",
                    "usage":{"input_tokens":1000,"cached_tokens":400,"output_tokens":50,
                        "cache_write_tokens":10,"reasoning_tokens":20}}},
                "recorded_at": THEN * 1000, "source_run_record_id":"record-1"}),
            // The same record id is a duplicate, not a second call.
            json!({"stream":{"id":"muse-session"},"payload_type":"runtime.session",
                "payload":{"kind":"run","run_id":"run-1","event":{"kind":"model_completed",
                    "usage":{"input_tokens":1000,"cached_tokens":400,"output_tokens":50,
                        "cache_write_tokens":10,"reasoning_tokens":20}}},
                "recorded_at": THEN * 1000, "source_run_record_id":"record-1"}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::Musecode, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.inserted_records, 1);
    let logs = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Musecode),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(logs.data[0].input_tokens, 600);
    assert_eq!(logs.data[0].cache_read_tokens, 400);
    assert_eq!(logs.data[0].cache_creation_tokens, 10);
    assert_eq!(logs.data[0].output_tokens, 50);
    assert_eq!(logs.data[0].upstream_model_id, "muse-model");
}

#[test]
fn cmdcode_reads_both_usage_locations_and_skips_checkpoints() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("projects/example");
    fs::create_dir_all(&project).unwrap();
    let top_level = json!({"type":"message","id":"msg-1","timestamp":THEN,
        "role":"assistant","sessionId":"cmd-session","model":"cmd-model",
        "usage":{"inputTokens":100,"outputTokens":20,"cacheReadTokens":30,
            "cacheWriteTokens":5,"costUsd":0.0012}});
    let nested = json!({"type":"message","id":"msg-2","timestamp":THEN+1,
        "message":{"role":"assistant","usage":{"inputTokens":10,"outputTokens":2,
            "cacheReadTokens":0,"cacheWriteTokens":0}}});
    write_jsonl(
        &project.join("cmd-session.jsonl"),
        &[
            json!({"type":"session","id":"cmd-session","model":"cmd-model"}),
            top_level,
            nested,
        ],
    );
    // Audit and checkpoint files are not session transcripts.
    write_jsonl(
        &project.join("hooks-audit-1.jsonl"),
        &[json!({"role":"assistant","usage":{"inputTokens":999,"outputTokens":999}})],
    );
    write_jsonl(
        &project.join("cmd-session.1.jsonl"),
        &[json!({"role":"assistant","usage":{"inputTokens":888,"outputTokens":888}})],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::Cmdcode, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.inserted_records, 2);
    let logs = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Cmdcode),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    // 100 + 20 + 30 + 5 from the top-level row, 10 + 2 from the nested one.
    let total: u64 = logs.data.iter().map(|row| row.total_tokens).sum();
    assert_eq!(total, 167);
}

#[test]
fn workbuddy_selects_one_usage_source_and_respects_inclusive_totals() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("projects/example");
    fs::create_dir_all(&project).unwrap();
    // `total == input + output` marks the input as cache-inclusive.
    let inclusive = json!({"type":"message","id":"item-1","timestamp":THEN,
        "sessionId":"wb-session","cwd":"D:/work/example",
        "message":{"role":"assistant","model":"wb-model",
            "usage":{"input_tokens":1000,"output_tokens":100,"total_tokens":1100,
                "cache_read_input_tokens":400}}});
    // An exclusive total leaves the buckets untouched.
    let exclusive = json!({"type":"message","id":"item-2","timestamp":THEN+1,
        "sessionId":"wb-session",
        "providerData":{"messageId":"provider-msg","requestModelName":"wb-model",
            "usage":{"input_tokens":10,"output_tokens":2,"total_tokens":17,
                "cache_read_input_tokens":5}}});
    write_jsonl(&project.join("wb-session.jsonl"), &[inclusive, exclusive]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::Workbuddy, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.inserted_records, 2);
    let summary =
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Workbuddy), true)
            .unwrap();
    assert_eq!(summary.total_input_tokens, 600 + 10);
    assert_eq!(summary.total_output_tokens, 102);
    assert_eq!(summary.total_cache_read_tokens, 405);
}

/// The transcript's own timestamp wins over the file's mtime.
///
/// `parsers::timestamp` scans a *container* (it reads `pointer("/timestamp")`
/// and friends). The first version passed `item.get("timestamp")` — a bare
/// scalar — so the lookup always missed and every imported WorkBuddy row was
/// stamped with the file mtime instead. That silently broke proxy/native
/// de-duplication, whose grace window is measured in seconds: the two rows of
/// one call looked minutes apart and both survived.
#[test]
fn workbuddy_uses_the_transcript_timestamp_not_the_file_mtime() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("projects/example");
    fs::create_dir_all(&project).unwrap();
    write_jsonl(
        &project.join("wb-session.jsonl"),
        &[json!({"type":"message","id":"item-1","timestamp":THEN * 1000,
            "sessionId":"wb-session",
            "message":{"role":"assistant","model":"wb-model",
                "usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}})],
    );
    // The file was just written, so its mtime is "now" while the transcript
    // says `THEN` (2027) — the two are far enough apart to tell apart.
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::Workbuddy, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.inserted_records, 1);

    let logs = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Workbuddy),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(logs.data.len(), 1);
    assert_eq!(
        logs.data[0].created_at.timestamp(),
        THEN,
        "the row must carry the item's timestamp, not the file mtime"
    );
}

/// The same call must not be counted twice when the gateway served it.
///
/// Proxy rows are written with `GatewayCliKey::as_str()` (`workbuddy_desktop`)
/// and native transcripts with `GatewayUsageTool::as_str()` (`workbuddy`).
/// `find_matching_proxy` compared against the latter only, so every
/// gateway-served WorkBuddy call appeared once as a proxy row and once as an
/// imported session row — doubling every total on the statistics page.
#[test]
fn a_gateway_served_workbuddy_call_is_not_imported_a_second_time() {
    // Same lock as the other tests that touch the global rebuild panel; this
    // one only calls `sync_sources` (which reports nothing), but the helper
    // below is shared with tests that do.
    let _panel = rebuild_panel_test_lock();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("projects/example");
    fs::create_dir_all(&project).unwrap();
    write_jsonl(
        &project.join("wb-session.jsonl"),
        &[json!({"type":"message","id":"item-1","timestamp":THEN * 1000,
            "sessionId":"wb-session",
            "message":{"role":"assistant","model":"wb-model",
                // Exclusive total (total != input + output) so the buckets are
                // used verbatim: with an inclusive total the importer subtracts
                // the cached tokens and the counters would not match the proxy
                // row this test expects it to recognise.
                "usage":{"input_tokens":600,"output_tokens":100,"total_tokens":1100,
                    "cache_read_input_tokens":400}}})],
    );

    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,request_model,
                input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,created_at,
                duration_ms,status_code,stream_outcome,transport,request_kind,data_source)
             VALUES ('SESSION:workbuddy_desktop:provider:msg-1','provider','workbuddy_desktop',
                'wb-model','wb-model',600,100,400,0,?1,2000,200,'completed','http','request','proxy')",
            [THEN],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();

    let result = run_sync(&db, GatewayUsageTool::Workbuddy, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(
        result.inserted_records, 0,
        "the proxy row already represents this call; importing it again doubles every total"
    );

    let logs = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Workbuddy),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(logs.data.len(), 1, "one call, one row");
    assert_eq!(logs.data[0].data_source, "proxy");
}

#[test]
fn qwencode_merges_per_request_files_with_the_session_rollup_remainder() {
    let root = tempfile::tempdir().unwrap();
    let usage = root.path().join("usage");
    fs::create_dir_all(&usage).unwrap();
    write_jsonl(
        &usage.join("token-usage-2026-09-20.jsonl"),
        &[
            json!({"schemaVersion":1,"id":"req-1","sessionId":"qwen-session",
            "model":"qwen-model","timestamp":THEN,
            "inputTokens":1000,"cachedTokens":400,"outputTokens":50,"thoughtsTokens":10,
            "localDate":"2026-09-20"}),
        ],
    );
    // The rollup covers the whole session: 1500 prompt/60 out, of which the
    // request file already reported 1000 prompt/60 out.
    write_jsonl(
        &root.path().join("usage_record.jsonl"),
        &[
            json!({"version":1,"sessionId":"qwen-session","timestamp":THEN,
            "models":{"qwen-model":{"inputTokens":1500,"cachedTokens":400,"outputTokens":50,
                "thoughtsTokens":10}}}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::Qwencode, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.inserted_records, 2);
    let summary =
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Qwencode), true)
            .unwrap();
    // Requests: 600 fresh + 400 cached + 60 out; the rollup adds 500 more prompt.
    assert_eq!(summary.total_input_tokens, 600 + 500);
    assert_eq!(summary.total_cache_read_tokens, 400);
    assert_eq!(summary.total_output_tokens, 60);
}

/// The project dimension reads the cwd each CLI records and groups usage by it.
/// Claude repeats `cwd` on every row, Codex records it once in `session_meta`,
/// and the rollup keeps the split after the detail rows are pruned.
#[test]
fn project_stats_group_usage_by_the_recorded_working_directory() {
    let root = tempfile::tempdir().unwrap();
    let projects = root.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let mut repo_a = claude_message("msg-a", 10);
    repo_a["cwd"] = json!("D:/work/repo-a");
    let mut repo_b = claude_message("msg-b", 20);
    repo_b["cwd"] = json!("D:/work/repo-b");
    let mut repo_b_again = claude_message("msg-c", 5);
    repo_b_again["cwd"] = json!("D:/work/repo-b");
    write_jsonl(
        &projects.join("session.jsonl"),
        &[repo_a, repo_b, repo_b_again],
    );
    run_sync(&db, GatewayUsageTool::Claude, root.path());

    let stats = usage_stats::project_stats(&db, None, None, None, true).unwrap();
    let by_path = stats
        .iter()
        .map(|item| (item.project_path.as_str(), item))
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(by_path.len(), 2, "unknown rows must not be invented");
    let repo_b = by_path["D:/work/repo-b"];
    assert_eq!(repo_b.request_count, 2);
    assert_eq!(repo_b.session_count, 1);
    // Each row is 100 fresh input + 80 cache read + 20 cache creation + output.
    assert_eq!(repo_b.total_tokens, 425);
    assert_eq!(repo_b.top_model.as_deref(), Some("usage-test-model"));
    assert_eq!(repo_b.cli_keys, vec![GatewayUsageTool::Claude]);
    assert!(repo_b.last_active.is_some());

    // Proxy traffic has no project: a session-disabled read is empty, and the
    // project dimension never leaks the gateway's own requests.
    assert!(usage_stats::project_stats(&db, None, None, None, false)
        .unwrap()
        .is_empty());

    // The CLI filter narrows the same aggregation.
    assert!(
        usage_stats::project_stats(&db, None, None, Some(GatewayUsageTool::Codex), true)
            .unwrap()
            .is_empty()
    );
}

/// ISO timestamps relative to the real clock. Qoder fixtures must not pin
/// calendar dates: the sync's retention maintenance prunes detail rows older
/// than `log_retention_days` (default 7) measured from `Local::now()`, so a
/// hard-coded date turns the test into a time bomb that starts failing
/// exactly one week after the fixture was written (observed 2026-10-24).
fn relative_iso_timestamp(offset_secs: i64) -> String {
    use chrono::{Duration, SecondsFormat, Utc};
    (Utc::now() - Duration::seconds(offset_secs)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn qoder_cli_assistant(request: &str, model: &str, credits: f64, timestamp: &str) -> Value {
    json!({
        "type": "assistant",
        "sessionId": "qoder-session",
        "cwd": "D:/work/qoder-repo",
        "timestamp": timestamp,
        "message": {
            "id": format!("chatcmpl-{request}"),
            "model": model,
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
                "credits": credits,
                "request_id": request,
            },
        },
    })
}

#[test]
fn qoder_cli_keeps_zero_token_calls_and_reads_the_working_directory() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("D--work-qoder-repo");
    fs::create_dir_all(&project).unwrap();
    write_jsonl(
        &project.join("session.jsonl"),
        &[
            json!({"type":"runtime-config","sessionId":"qoder-session","model":"qfmodel"}),
            json!({"type":"user","sessionId":"qoder-session","cwd":"D:/work/qoder-repo",
                "timestamp":&relative_iso_timestamp(40),"message":{"role":"user","content":"hi"}}),
            // A row without usage is not a model call.
            json!({"type":"assistant","sessionId":"qoder-session","cwd":"D:/work/qoder-repo",
                "timestamp":&relative_iso_timestamp(30),"message":{"id":"chatcmpl-plain","model":"qfmodel"}}),
            qoder_cli_assistant("req-1", "qfmodel", 0.25, &relative_iso_timestamp(20)),
            // The same provider response repeated while streaming is one call.
            qoder_cli_assistant("req-1", "qfmodel", 0.25, &relative_iso_timestamp(10)),
            // The credit-limit notice is not a reply.
            json!({"type":"assistant","sessionId":"qoder-session","cwd":"D:/work/qoder-repo",
                "timestamp":&relative_iso_timestamp(5),
                "message":{"id":"chatcmpl-synthetic","model":"<synthetic>","usage":{}}}),
        ],
    );

    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::QoderCli, root.path());
    assert_eq!(
        result.parsed_records, 1,
        "one provider response is one call"
    );

    // Qoder's local routing writes zero token buckets; the calls are recorded
    // but no token count is invented for them.
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 1);
    assert_eq!(summary.total_tokens, 0);
    let project = usage_stats::project_stats(&db, None, None, None, true).unwrap();
    assert_eq!(project.len(), 1);
    assert_eq!(project[0].project_path, "D:/work/qoder-repo");
    assert_eq!(project[0].session_count, 1);
    assert_eq!(project[0].cli_keys, vec![GatewayUsageTool::QoderCli]);
}

#[test]
fn qoder_cli_reads_the_nested_cache_creation_pair_and_the_config_model() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("nested.jsonl");
    write_jsonl(
        &file,
        &[
            json!({"type":"runtime-config","sessionId":"qoder-session","model":"configured-model"}),
            json!({"type":"assistant","sessionId":"qoder-session","cwd":"D:/work/qoder-repo",
            "timestamp":&relative_iso_timestamp(20),
            "message":{"id":"chatcmpl-nested","usage":{
                "input_tokens":1000,
                "output_tokens":50,
                "cache_read_input_tokens":200,
                "cache_creation":{"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":20},
            }}}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::QoderCli, root.path());
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    // The transcript's `input_tokens` is the provider's prompt count and
    // already contains both cache buckets, so the stored fresh input is
    // 1000 - 200 - 50. Matching the reference collector here is what keeps the
    // same call from counting its cached tokens twice.
    assert_eq!(summary.total_input_tokens, 750);
    assert_eq!(summary.total_output_tokens, 50);
    assert_eq!(summary.total_cache_read_tokens, 200);
    assert_eq!(summary.total_cache_creation_tokens, 50);
    // The runtime-config header supplies the model when the call row omits it.
    let models = usage_stats::model_stats(&db, None, None, None, true).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model, "configured-model");
}

#[test]
fn qoder_cn_transcripts_stay_separate_from_the_international_cli() {
    let root = tempfile::tempdir().unwrap();
    write_jsonl(
        &root.path().join("cn.jsonl"),
        &[qoder_cli_assistant(
            "req-cn",
            "dfmodel",
            0.5,
            &relative_iso_timestamp(20),
        )],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::QoderCliCn, root.path());
    let summary =
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::QoderCli), true)
            .unwrap();
    assert_eq!(summary.total_requests, 0, "the two editions never merge");
    let summary =
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::QoderCliCn), true)
            .unwrap();
    assert_eq!(summary.total_requests, 1);
}

fn qoder_ide_fixture(path: &Path) {
    let native = Connection::open(path).unwrap();
    native
        .execute_batch(
            "CREATE TABLE chat_session (session_id TEXT PRIMARY KEY, cwd TEXT);
             CREATE TABLE chat_message (
                message_id TEXT PRIMARY KEY, session_id TEXT, request_id TEXT,
                token_info TEXT, model_info TEXT, gmt_create INTEGER);
             INSERT INTO chat_session VALUES ('ide-session', 'D:/work/ide-repo');
             INSERT INTO chat_message VALUES (
                'msg-1', 'ide-session', 'req-1',
                '{\"prompt_tokens\":1000,\"completion_tokens\":120,\"cached_tokens\":400}',
                '{\"model_key\":\"ide-model\"}', 1800000000000);",
        )
        .unwrap();
}

#[test]
fn qoder_ide_subtracts_the_cached_prompt_and_files_each_edition_apart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("local.db");
    qoder_ide_fixture(&path);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    // An IDE tool's root is the database file itself, not a directory.
    let result = run_sync(&db, GatewayUsageTool::QoderIde, &path);
    assert_eq!(
        result.failed_files, 0,
        "a database tool must never be read as JSON"
    );

    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 1);
    // `prompt_tokens` includes the cached part, so the fresh input is 600.
    assert_eq!(summary.total_input_tokens, 600);
    assert_eq!(summary.total_output_tokens, 120);
    assert_eq!(summary.total_cache_read_tokens, 400);
    assert_eq!(
        usage_stats::project_stats(&db, None, None, None, true).unwrap()[0].project_path,
        "D:/work/ide-repo"
    );

    // The CN edition is a different key, so an international read never sees it.
    let cn = usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::QoderIdeCn), true)
        .unwrap();
    assert_eq!(cn.total_requests, 0);
}

/// One request can be split across several `chat_message` rows. The reference
/// collector sums them and counts the distinct request id once; overwriting
/// would silently drop every earlier row's tokens.
#[test]
fn qoder_ide_sums_the_rows_of_one_request_instead_of_overwriting_them() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("local.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE chat_session (session_id TEXT PRIMARY KEY, cwd TEXT);
             CREATE TABLE chat_message (
                message_id TEXT PRIMARY KEY, session_id TEXT, request_id TEXT,
                token_info TEXT, model_info TEXT, gmt_create INTEGER);
             INSERT INTO chat_session VALUES ('ide-session', 'D:/work/ide-repo');
             INSERT INTO chat_message VALUES
                ('msg-a', 'ide-session', 'req-1',
                 '{\"prompt_tokens\":600,\"completion_tokens\":40,\"cached_tokens\":100}',
                 '{\"model_key\":\"ide-model\"}', 1800000000000),
                ('msg-b', 'ide-session', 'req-1',
                 '{\"prompt_tokens\":400,\"completion_tokens\":60,\"cached_tokens\":50}',
                 '{\"model_key\":\"ide-model\"}', 1800000001000);",
        )
        .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::QoderIde, &path);
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 1, "one request id is one call");
    assert_eq!(summary.total_input_tokens, 850, "600-100 plus 400-50");
    assert_eq!(summary.total_output_tokens, 100);
    assert_eq!(summary.total_cache_read_tokens, 150);
}

/// A trimmed database without `message_id` falls back to `rowid`, which is an
/// integer column. Reading it as TEXT fails the whole scan, so the query casts
/// it instead.
#[test]
fn qoder_ide_falls_back_to_the_integer_rowid_when_message_id_is_absent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("local.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE chat_message (
                token_info TEXT, gmt_create INTEGER);
             INSERT INTO chat_message VALUES
                ('{\"prompt_tokens\":100,\"completion_tokens\":10}', 1800000000000),
                ('{\"prompt_tokens\":200,\"completion_tokens\":20}', 1800000001000);",
        )
        .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::QoderIde, &path);
    assert_eq!(result.failed_files, 0);
    let summary = usage_stats::usage_summary(&db, None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 2, "each row is its own message");
    assert_eq!(summary.total_input_tokens, 300);
}

#[test]
fn qoder_ide_without_the_chat_message_table_is_a_no_op() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("local.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE chat_session_messages (session_id TEXT, payload_json TEXT);")
        .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::QoderIde, &path);
    assert_eq!(result.inserted_records, 0);
    assert_eq!(
        result.failed_files, 0,
        "a database tool must never be read as JSON"
    );
    assert_eq!(count(&db), 0);
}

#[test]
fn qoder_tool_keys_serialize_exactly_as_their_app_type() {
    // The frontend filters on `as_str()` and localizes with
    // `settings.gateway.cli.<key>`. `rename_all = "snake_case"` would derive
    // `qoder_cli`, which silently breaks both, so the serde name is pinned
    // here.
    for tool in [
        GatewayUsageTool::QoderIde,
        GatewayUsageTool::QoderIdeCn,
        GatewayUsageTool::QoderCli,
        GatewayUsageTool::QoderCliCn,
    ] {
        let json = serde_json::to_string(&tool).unwrap();
        assert_eq!(json, format!("\"{}\"", tool.as_str()));
    }
    assert_eq!(
        serde_json::from_str::<GatewayUsageTool>("\"qodercli\"").unwrap(),
        GatewayUsageTool::QoderCli
    );
    assert_eq!(
        serde_json::from_str::<GatewaySessionImportCli>("\"qodercli_cn\"").unwrap(),
        GatewaySessionImportCli::QoderCliCn
    );
}

#[test]
fn qoder_credits_are_stored_beside_zero_tokens() {
    let root = tempfile::tempdir().unwrap();
    write_jsonl(
        &root.path().join("qd.jsonl"),
        &[
            qoder_cli_assistant("req-a", "qfmodel", 0.25, &relative_iso_timestamp(20)),
            qoder_cli_assistant("req-b", "qfmodel", 0.75, &relative_iso_timestamp(10)),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::QoderCli, root.path());

    // Credits are the only usage measure Qoder records; they are persisted so
    // history survives until a consumer renders them.
    let credits = db
        .with_conn(|conn| {
            let mut query = conn
                .prepare("SELECT credits FROM proxy_request_logs ORDER BY request_id")
                .map_err(|error| error.to_string())?;
            let rows = query
                .query_map([], |row| row.get::<_, Option<f64>>(0))
                .map_err(|error| error.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())
        })
        .unwrap();
    assert_eq!(credits, vec![Some(0.25), Some(0.75)]);

    // Tools that never report credits leave the column NULL rather than 0, so
    // "not reported" stays distinguishable from "free".
    let root2 = tempfile::tempdir().unwrap();
    write_jsonl(
        &root2.path().join("claude.jsonl"),
        &[
            json!({"type":"assistant", "sessionId":"claude-session", "timestamp":THEN,
            "cwd": "D:/work/repo",
            "message":{"id":"msg-credits","model":"usage-test-model",
                "usage":{"input_tokens":100,"output_tokens":10}}}),
        ],
    );
    run_sync(&db, GatewayUsageTool::Claude, root2.path());
    let claude_credits: Option<f64> = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT credits FROM proxy_request_logs WHERE app_type = 'claude'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
        })
        .unwrap();
    assert_eq!(claude_credits, None);
}

/// Local transcripts only bracket a call with two events. The estimate must
/// land on the row as a real duration and must survive an unchanged rescan.
#[test]
fn session_rows_carry_the_duration_estimated_from_adjacent_events() {
    let root = tempfile::tempdir().unwrap();
    // Claude writes the prompt, then streamed snapshots of one reply.
    let prompt = json!({"type":"user","sessionId":"claude-session","timestamp":rfc3339(THEN),
        "message":{"role":"user","content":"go"}});
    let mut reply = claude_message("timed-message", 10);
    reply["timestamp"] = json!(rfc3339_millis(THEN * 1000 + 12_500));
    write_jsonl(&root.path().join("claude.jsonl"), &[prompt, reply]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Claude, root.path());
    let rows =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(rows.data.len(), 1);
    assert_eq!(rows.data[0].duration_ms, 12_500);
    // An unchanged transcript is not re-timed or re-billed.
    assert_eq!(
        run_sync(&db, GatewayCliKey::Claude, root.path()).parsed_records,
        0
    );
    let again =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(again.data[0].duration_ms, 12_500);
}

#[test]
fn session_timing_beyond_the_sanity_window_is_not_written() {
    let root = tempfile::tempdir().unwrap();
    let prompt = json!({"type":"user","sessionId":"claude-session",
        "timestamp":rfc3339(THEN - 45 * 60),
        "message":{"role":"user","content":"go"}});
    let mut reply = claude_message("slow-message", 10);
    // 45 minutes later is a resumed session, not one model call.
    reply["timestamp"] = json!(rfc3339(THEN));
    write_jsonl(&root.path().join("claude.jsonl"), &[prompt, reply]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Claude, root.path());
    let rows =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(rows.data[0].duration_ms, 0);
}

/// Codex records the tool result that sent the request and writes the
/// authoritative usage record when the response ends.
#[test]
fn codex_authoritative_records_use_the_sending_event_as_start() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    write_jsonl(
        &sessions.join(format!("rollout-{PARENT}.jsonl")),
        &[
            json!({"type":"session_meta","timestamp":rfc3339(THEN),"payload":{"id":PARENT}}),
            json!({"type":"event_msg","timestamp":rfc3339(THEN + 1),
                "payload":{"type":"task_started"}}),
            json!({"type":"response_item","timestamp":rfc3339(THEN + 5),
                "payload":{"type":"function_call_output"}}),
            json!({"type":"token_usage_record","timestamp":rfc3339_millis(THEN * 1000 + 9_250),
                "payload":{"response_id":"gen_timed","usage":{
                    "input_tokens":100,"cached_input_tokens":0,
                    "output_tokens":10,"reasoning_output_tokens":0}}}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayCliKey::Codex, &sessions);
    let rows =
        usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(rows.data.len(), 1);
    // The tool result at +5s is the start; task_started at +1s belongs to the
    // turn, not to this call.
    assert_eq!(rows.data[0].duration_ms, 4_250);
}

/// Every native adapter stores the call's own elapsed time when its source
/// records one; a source without timing leaves the field empty instead of
/// inventing a zero.
#[test]
fn native_sources_record_their_own_durations() {
    // ZCode: one row per request with both ends in milliseconds.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.sqlite");
    let native = Connection::open(&path).unwrap();
    native
        .execute_batch(
            "CREATE TABLE model_usage (
                id TEXT PRIMARY KEY, attempt_index INTEGER NOT NULL DEFAULT 0,
                session_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL,
                status TEXT NOT NULL, started_at INTEGER NOT NULL,
                completed_at INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0, raw_usage_json TEXT
            );
            INSERT INTO model_usage VALUES
              ('u1', 0, 'sess_a', 'account:zai', 'GLM-5.3', 'completed', 1, 2, 100, 20, 0, 0, NULL);",
        )
        .unwrap();
    native
        .execute(
            "UPDATE model_usage SET started_at = ?1, completed_at = ?2",
            [(THEN - 4) * 1000, (THEN - 4) * 1000 + 1_854],
        )
        .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Zcode, &path);
    let rows = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Zcode),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(rows.data[0].duration_ms, 1_854);

    // Grok: each inference reports how long the model itself ran.
    let root = tempfile::tempdir().unwrap();
    let logs = root.path().join("logs");
    fs::create_dir_all(&logs).unwrap();
    write_jsonl(
        &logs.join("unified.jsonl"),
        &[
            json!({"ts":THEN,"sid":"grok-session","msg":"shell.turn.inference_done",
            "ctx":{"loop_index":1,"attempts":1,"prompt_tokens":100,
                "completion_tokens":10,"model_elapsed_ms":5_466}}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Grok, root.path());
    let rows = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Grok),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(rows.data[0].duration_ms, 5_466);

    // Muse Code: the completed model event carries its own duration.
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("sessions/2026/09/02/run-a");
    fs::create_dir_all(&session).unwrap();
    write_jsonl(
        &session.join("session.jsonl"),
        &[
            json!({"stream":{"id":"muse-session"},"payload_type":"runtime.session",
            "payload":{"kind":"run","run_id":"run-1","event":{"kind":"model_completed",
                "duration_ms":3_250,
                "usage":{"input_tokens":100,"output_tokens":10}}},
            "recorded_at":THEN * 1000,"source_run_record_id":"record-1"}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Musecode, root.path());
    let rows = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Musecode),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(rows.data[0].duration_ms, 3_250);

    // Qwen Code: the per-request record times the API call.
    let root = tempfile::tempdir().unwrap();
    let usage = root.path().join("usage");
    fs::create_dir_all(&usage).unwrap();
    write_jsonl(
        &usage.join("token-usage-2026-09-20.jsonl"),
        &[
            json!({"schemaVersion":1,"id":"req-1","sessionId":"qwen-session",
            "model":"qwen-model","timestamp":THEN,"inputTokens":100,"outputTokens":10,
            "apiDurationMs":7_500,"localDate":"2026-09-20"}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Qwencode, root.path());
    let rows = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::Qwencode),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(rows.data[0].duration_ms, 7_500);
}

fn rfc3339(seconds: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn rfc3339_millis(millis: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[test]
fn scoped_envelope_lookup_stays_bounded_with_large_mixed_history() {
    use rusqlite::StatementStatus;
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute_batch(
            "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<12000)
             INSERT INTO proxy_request_logs
                (request_id, provider_id, app_type, model, created_at, request_kind, data_source)
             SELECT 'SESSION:codex:provider:proxy-' || n, 'provider', 'codex', 'gpt-5', 1800000000, 'request', 'proxy' FROM numbers;
             WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<12000)
             INSERT INTO proxy_request_logs
                (request_id, provider_id, app_type, model, created_at, request_kind, data_source)
             SELECT 'SESSION:codex:session:native-' || n, 'session', 'codex', 'gpt-5', 1800000000, 'request', 'session' FROM numbers;
             INSERT INTO proxy_request_logs
                (request_id, provider_id, app_type, model, created_at, request_kind, data_source)
             VALUES ('SESSION:codex:provider:with:colon:请求:one', 'provider:with:colon', 'codex', 'gpt-5', 1800000000, 'request', 'proxy'),
                    ('SESSION:codex:provider:warmup', 'provider', 'codex', 'gpt-5', 1800000000, 'prewarm', 'proxy'),
                    ('BROKEN!:codex:provider:forged', 'provider', 'codex', 'gpt-5', 1800000000, 'request', 'proxy');"
        ).map_err(|error| error.to_string())?;
        let plan = conn.prepare(&format!("EXPLAIN QUERY PLAN {SCOPED_PROXY_ENVELOPE_LOOKUP}"))
            .unwrap().query_map(params!["codex", "missing", Option::<&str>::None], |row| row.get::<_, String>(3))
            .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert!(plan.iter().any(|line| line.contains("idx_request_logs_proxy_envelope") && line.contains("<expr>=?")), "{plan:?}");
        let mut statement = conn.prepare_cached(SCOPED_PROXY_ENVELOPE_LOOKUP).unwrap();
        for (cli, envelope, only, expected) in [
            ("codex", "proxy-12000", None, Some("SESSION:codex:provider:proxy-12000")),
            ("codex", "请求:one", None, Some("SESSION:codex:provider:with:colon:请求:one")),
            ("codex", "proxy-12000", Some("another-id"), None),
            ("claude", "proxy-12000", None, None),
            ("codex", "native-12000", None, None),
            ("codex", "warmup", None, None),
            ("codex", "forged", None, None),
            ("codex", "missing", None, None),
        ] {
            let before = statement.get_status(StatementStatus::VmStep);
            let result = statement.query_row(params![cli, envelope, only], |row| row.get::<_, String>(0)).optional().unwrap();
            assert_eq!(result.as_deref(), expected);
            let steps = statement.get_status(StatementStatus::VmStep) - before;
            assert!(steps < 300, "{cli}/{envelope}: {steps} VM steps; history must not be scanned");
        }
        Ok(())
    }).unwrap();
}
