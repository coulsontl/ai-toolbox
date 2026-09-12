use super::*;
use crate::coding::proxy_gateway::pricing;
use crate::coding::proxy_gateway::types::{GatewayUsageSummary, ModelPricing};

const COST_MODEL: &str = "cost-test-model-0731";

#[path = "cross_cli_cost_tests.rs"]
mod cross_cli;

fn cost_pricing(db: &SqliteDbState, model: &str, input: &str, output: &str, cache: &str) {
    pricing::upsert_model_pricing(
        db,
        ModelPricing {
            model_id: model.into(),
            display_name: model.into(),
            input_cost_per_million: input.into(),
            output_cost_per_million: output.into(),
            cache_read_cost_per_million: cache.into(),
            cache_creation_cost_per_million: "0".into(),
        },
    )
    .unwrap();
}

fn cost_message(step: u64, created_at: i64) -> Value {
    let (input, output, cache) = if step == 1 {
        (1_000_000, 100_000, 2_000_000)
    } else {
        (2_000_000, 200_000, 1_000_000)
    };
    let mut message = dsh_message(1, step, input, output);
    message["time"] = json!(created_at * 1000);
    message["data"]["message"]["source"]["model"] = json!(COST_MODEL);
    message["data"]["usage"]["cacheReadTokens"] = json!(cache);
    message["data"]["usage"]["cacheWriteTokens"] = json!(0);
    message
}

fn write_cost_session(path: &Path, messages: &[Value]) {
    let mut values = vec![json!({"type":"session","version":0,"id":"cost-session"})];
    values.extend_from_slice(messages);
    write_jsonl(path, &values);
}

fn cost_summary(db: &SqliteDbState) -> GatewayUsageSummary {
    usage_stats::usage_summary(db, None, None, Some(GatewayUsageTool::Dsh)).unwrap()
}

fn remove_old_cost_provenance(db: &SqliteDbState) {
    let mut states = load_states(db).unwrap();
    for (id, state) in &mut states {
        for record in state.records.values_mut() {
            record.recorded_cost = None;
            record.envelope_id = None;
        }
        db.with_conn(|conn| save_state(conn, id, state)).unwrap();
    }
}

#[test]
fn dsh_missing_costs_backfill_live_and_legacy_archived_usage_without_recounting() {
    for archived in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let time = if archived {
            (Utc::now() - chrono::Duration::days(400)).timestamp()
        } else {
            THEN
        };
        write_cost_session(
            &root.path().join("session.jsonl"),
            &[cost_message(1, time), cost_message(2, time)],
        );
        let db = SqliteDbState::in_memory_for_test().unwrap();
        run_sync(&db, GatewayUsageTool::Dsh, root.path());
        assert_eq!(cost_summary(&db).total_cost_usd, "0.000000");
        remove_old_cost_provenance(&db);
        cost_pricing(&db, "cost-test-model", "2", "4", "0.2");
        let result = run_sync(&db, GatewayUsageTool::Dsh, root.path());
        assert_eq!(result.inserted_records, 0);
        assert_eq!(result.updated_records, 2);
        assert_eq!(result.failed_files, 0);
        let summary = cost_summary(&db);
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.total_tokens, 6_300_000);
        assert_eq!(summary.total_cost_usd, "7.800000");
        assert_eq!(
            run_sync(&db, GatewayUsageTool::Dsh, root.path()).updated_records,
            0
        );
        assert_eq!(cost_summary(&db), summary);
        let states = load_states(&db).unwrap();
        let repaired = states
            .values()
            .flat_map(|state| state.records.values())
            .filter(|row| row.cost_repair.is_some())
            .count();
        assert_eq!(repaired, 2);
        if !archived {
            let logs = usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10)
                .unwrap();
            assert!(logs.data.iter().all(|row| row
                .usage_metadata
                .as_ref()
                .unwrap()
                .cost_source
                .as_deref()
                == Some("model_pricing")));
        }
    }
}

#[test]
fn dsh_known_zero_or_nonzero_prices_are_not_repriced_after_price_changes() {
    for original_input in ["0", "2"] {
        let root = tempfile::tempdir().unwrap();
        let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
        write_cost_session(&root.path().join("session.jsonl"), &[cost_message(1, time)]);
        let db = SqliteDbState::in_memory_for_test().unwrap();
        cost_pricing(&db, COST_MODEL, original_input, "0", "0");
        run_sync(&db, GatewayUsageTool::Dsh, root.path());
        let original = cost_summary(&db);
        cost_pricing(&db, COST_MODEL, "20", "40", "2");
        assert_eq!(
            run_sync(&db, GatewayUsageTool::Dsh, root.path()).updated_records,
            0
        );
        assert_eq!(cost_summary(&db), original);
    }
}

#[test]
fn dsh_archived_cost_repair_requires_a_complete_native_bucket() {
    let root = tempfile::tempdir().unwrap();
    let first_dir = root.path().join("first");
    let second_dir = root.path().join("second");
    fs::create_dir_all(&first_dir).unwrap();
    fs::create_dir_all(&second_dir).unwrap();
    let first = first_dir.join("session.jsonl");
    let second = second_dir.join("session.jsonl");
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    write_cost_session(&first, &[cost_message(1, time)]);
    write_cost_session(&second, &[cost_message(2, time)]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    remove_old_cost_provenance(&db);
    fs::remove_file(&second).unwrap();
    cost_pricing(&db, "cost-test-model", "2", "4", "0.2");
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    assert_eq!(cost_summary(&db).total_cost_usd, "0.000000");
    assert_eq!(cost_summary(&db).total_requests, 2);
    write_cost_session(&second, &[cost_message(2, time)]);
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    assert_eq!(cost_summary(&db).total_cost_usd, "7.800000");
    assert_eq!(cost_summary(&db).total_requests, 2);
}

#[test]
fn dsh_mixed_archives_preserve_known_costs_and_fill_only_unpriced_records() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session.jsonl");
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    write_cost_session(&file, &[cost_message(1, time)]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    cost_pricing(&db, COST_MODEL, "2", "4", "0.2");
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    pricing::delete_model_pricing(&db, COST_MODEL.into()).unwrap();
    write_cost_session(&file, &[cost_message(1, time), cost_message(2, time)]);
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    assert_eq!(cost_summary(&db).total_cost_usd, "2.800000");
    cost_pricing(&db, COST_MODEL, "3", "6", "0.3");
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Dsh, root.path()).updated_records,
        1
    );
    assert_eq!(cost_summary(&db).total_cost_usd, "10.300000");
    assert_eq!(cost_summary(&db).total_requests, 2);
}

#[test]
fn dsh_cost_repair_and_audit_ledger_roll_back_together_and_retry() {
    let root = tempfile::tempdir().unwrap();
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    write_cost_session(&root.path().join("session.jsonl"), &[cost_message(1, time)]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    cost_pricing(&db, "cost-test-model", "2", "4", "0.2");
    db.with_conn(|conn| conn.execute_batch("CREATE TRIGGER reject_cost_audit BEFORE UPDATE ON gateway_session_usage_state BEGIN SELECT RAISE(ABORT,'blocked cost audit'); END;").map_err(|error| error.to_string())).unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Dsh, root.path()).failed_files,
        1
    );
    assert_eq!(cost_summary(&db).total_cost_usd, "0.000000");
    assert!(load_states(&db)
        .unwrap()
        .values()
        .flat_map(|state| state.records.values())
        .all(|record| record.cost_repair.is_none()));
    db.with_conn(|conn| {
        conn.execute_batch("DROP TRIGGER reject_cost_audit")
            .map_err(|error| error.to_string())
    })
    .unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Dsh, root.path()).updated_records,
        1
    );
    assert_eq!(cost_summary(&db).total_cost_usd, "2.800000");
}

fn priced_claude_message(id: &str, session: &str, time: i64) -> Value {
    json!({"type":"assistant","sessionId":session,"timestamp":time,
        "message":{"id":id,"role":"assistant","model":COST_MODEL,
            "usage":{"input_tokens":1_000_000,"output_tokens":100_000,"cache_read_input_tokens":2_000_000,"cache_creation_input_tokens":0}}})
}

#[test]
fn claude_dated_model_costs_repair_archives_without_recounting() {
    let root = tempfile::tempdir().unwrap();
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    write_jsonl(
        &root.path().join("session.jsonl"),
        &[priced_claude_message(
            "cost-claude",
            "cost-claude-session",
            time,
        )],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Claude, root.path());
    remove_old_cost_provenance(&db);
    cost_pricing(&db, "cost-test-model", "2", "4", "0.2");
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Claude, root.path()).updated_records,
        1
    );
    let summary =
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Claude)).unwrap();
    assert_eq!(summary.total_requests, 1);
    assert_eq!(summary.total_tokens, 3_100_000);
    assert_eq!(summary.total_cost_usd, "2.800000");
}

#[test]
fn session_cost_cache_ignores_changes_in_fully_priced_sources() {
    let root = tempfile::tempdir().unwrap();
    let pending_dir = root.path().join("pending");
    let paid_dir = root.path().join("paid");
    fs::create_dir_all(&pending_dir).unwrap();
    fs::create_dir_all(&paid_dir).unwrap();
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    write_cost_session(&pending_dir.join("session.jsonl"), &[cost_message(1, time)]);
    let mut paid = cost_message(2, time);
    paid["data"]["message"]["source"]["model"] = json!("paid-model");
    let paid_path = paid_dir.join("session.jsonl");
    write_cost_session(&paid_path, &[paid]);
    let db = SqliteDbState::in_memory_for_test().unwrap();
    cost_pricing(&db, "paid-model", "2", "4", "0.2");
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    // A missing zero-token historical call prevents proving the whole bucket.
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE usage_daily_rollups SET request_count=request_count+1 WHERE model=?1",
            [COST_MODEL],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    })
    .unwrap();
    cost_pricing(&db, "cost-test-model", "2", "4", "0.2");
    run_sync(&db, GatewayUsageTool::Dsh, root.path());
    fs::write(&paid_path, "invalid unrelated transcript\n").unwrap();
    let mut states = load_states(&db).unwrap();
    assert_eq!(
        cost_reconciliation::reconcile_session_costs(
            &db,
            GatewayUsageTool::Dsh,
            &[(GatewayUsageTool::Dsh, root.path().to_path_buf())],
            &mut states
        )
        .unwrap(),
        0
    );
}

#[test]
fn legacy_live_costs_without_metadata_can_recover() {
    let root = tempfile::tempdir().unwrap();
    write_jsonl(
        &root.path().join("session.jsonl"),
        &[priced_claude_message("legacy-cost", "legacy-session", THEN)],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Claude, root.path());
    remove_old_cost_provenance(&db);
    db.with_conn(|conn| {
        conn.execute("UPDATE proxy_request_logs SET usage_metadata=NULL", [])
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
    .unwrap();
    cost_pricing(&db, "cost-test-model", "2", "4", "0.2");
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Claude, root.path()).updated_records,
        1
    );
    let logs = usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).unwrap();
    assert_eq!(logs.total, 1);
    assert_eq!(logs.data[0].total_tokens, 3_100_000);
    assert_eq!(logs.data[0].total_cost_usd, "2.800000");
    assert_eq!(
        logs.data[0]
            .usage_metadata
            .as_ref()
            .unwrap()
            .cost_source
            .as_deref(),
        Some("model_pricing")
    );
}

#[test]
fn desktop_pricing_repair_updates_the_snapshot_used_to_retire_audit_totals() {
    let root = tempfile::tempdir().unwrap();
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    let message = priced_claude_message("cost-desktop", "cost-desktop-session", time);
    write_jsonl(
        &root.path().join("audit.jsonl"),
        &[json!({"type":"result","uuid":"audit-cost",
        "session_id":"cost-desktop-session","timestamp":time,"modelCalls":1,
        "usage":message["message"]["usage"],"modelUsage":{(COST_MODEL):{}}})],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path());
    cost_pricing(&db, "cost-test-model", "2", "4", "0.2");
    assert_eq!(
        run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path()).updated_records,
        1
    );
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::ClaudeDesktop))
            .unwrap()
            .total_cost_usd,
        "2.800000"
    );
    let transcript_dir = root.path().join(".claude/projects/project");
    fs::create_dir_all(&transcript_dir).unwrap();
    write_jsonl(
        &transcript_dir.join("cost-desktop-session.jsonl"),
        &[message],
    );
    run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path());
    let summary =
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::ClaudeDesktop)).unwrap();
    assert_eq!(summary.total_cost_usd, "2.800000");
    assert_eq!(summary.total_tokens, 3_100_000);
    assert_eq!(summary.total_requests, 1);
}

#[test]
#[ignore = "Read-only native cost review across Claude Code, Claude Desktop and DSH; writes only a temporary database copy"]
fn local_cost_reconciliation_all_sources_on_a_snapshot() {
    let source_path = dirs::data_dir()
        .unwrap()
        .join("com.ai-toolbox/ai-toolbox.db");
    let source =
        Connection::open_with_flags(source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let copy = temporary.path().join("native-cost-audit.db");
    source.backup(rusqlite::MAIN_DB, &copy, None).unwrap();
    drop(source);
    let db = SqliteDbState::open(copy).unwrap();
    let mut states = load_states(&db).unwrap();
    for tool in [
        GatewayUsageTool::Claude,
        GatewayUsageTool::ClaudeDesktop,
        GatewayUsageTool::Dsh,
    ] {
        let sources = default_session_roots(&db, tool)
            .into_iter()
            .map(|path| (tool, path))
            .collect::<Vec<_>>();
        let before = usage_stats::usage_summary(&db, None, None, Some(tool)).unwrap();
        let started = std::time::Instant::now();
        let repaired =
            cost_reconciliation::reconcile_session_costs(&db, tool, &sources, &mut states).unwrap();
        let after = usage_stats::usage_summary(&db, None, None, Some(tool)).unwrap();
        println!(
            "{}: before={} after={} repaired={} elapsed_ms={}",
            tool.as_str(),
            before.total_cost_usd,
            after.total_cost_usd,
            repaired,
            started.elapsed().as_millis()
        );
        assert_eq!(after.total_requests, before.total_requests);
        assert_eq!(after.total_tokens, before.total_tokens);
        assert_eq!(
            cost_reconciliation::reconcile_session_costs(&db, tool, &sources, &mut states).unwrap(),
            0
        );
    }
}

#[test]
#[ignore = "Read-only local DSH cost audit: repairs a database copy and compares with freshly imported transcripts using the same prices"]
fn local_dsh_cost_reconciliation_on_a_snapshot() {
    let source_path = dirs::data_dir()
        .unwrap()
        .join("com.ai-toolbox/ai-toolbox.db");
    let source =
        Connection::open_with_flags(source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let copy = temporary.path().join("dsh-cost-audit.db");
    source.backup(rusqlite::MAIN_DB, &copy, None).unwrap();
    drop(source);
    let db = SqliteDbState::open(copy).unwrap();
    let sources = default_session_roots(&db, GatewayUsageTool::Dsh)
        .into_iter()
        .map(|path| (GatewayUsageTool::Dsh, path))
        .collect::<Vec<_>>();
    let before = cost_summary(&db);
    let fresh = SqliteDbState::in_memory_for_test().unwrap();
    for model in pricing::get_model_pricing_list(&db).unwrap() {
        pricing::upsert_model_pricing(&fresh, model).unwrap();
    }
    let now = Utc::now().timestamp();
    assert_eq!(sync_sources(&fresh, &sources, now).unwrap().failed_files, 0);
    let expected = cost_summary(&fresh);
    let result = sync_sources(&db, &sources, now).unwrap();
    assert_eq!(result.failed_files, 0);
    let after = cost_summary(&db);
    println!(
        "DSH cost audit: before={} after={} fresh={} calls={} tokens={} repaired={}",
        before.total_cost_usd,
        after.total_cost_usd,
        expected.total_cost_usd,
        after.total_requests,
        after.total_tokens,
        result.updated_records
    );
    assert_eq!(after.total_requests, before.total_requests);
    assert_eq!(after.total_tokens, before.total_tokens);
    assert_eq!(after.total_cost_usd, expected.total_cost_usd);
    assert_ne!(after.total_cost_usd, "0.000000");
    assert_eq!(sync_sources(&db, &sources, now).unwrap().updated_records, 0);
    assert_eq!(cost_summary(&db), after);
}
