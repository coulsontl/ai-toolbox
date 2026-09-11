use super::*;
use crate::coding::proxy_gateway::types::{GatewayUsageTool, SessionUsageGranularity};

#[path = "cost_tests.rs"]
mod cost_tests;

fn pi_message(id: &str, input: u64, output: u64) -> Value {
    json!({"type":"message","id":id,"timestamp":THEN,
        "message":{"role":"assistant","model":"native-test-model","provider":"historical-provider",
            "usage":{"input":input,"output":output,"cacheRead":80,"cacheWrite":20,
                "reasoningTokens":7,"totalTokens":input+output+100,"cost":{"total":0.0123}}}})
}

#[test]
fn pi_and_omp_usage_round_trip_preserves_native_tokens_cost_and_tool_identity() {
    for tool in [GatewayUsageTool::Pi, GatewayUsageTool::OhMyPi] {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let mut assistant = pi_message("response", 100, 10);
        assistant["message"]["usage"]["totalTokens"] = json!(240);
        assistant["message"]["usage"]["orchestration"] = json!({"input":30});
        let auxiliary = json!({"type":"model_usage","id":"auxiliary","timestamp":THEN,"model":"native-test-model",
            "usage":{"input":5,"output":2,"cacheRead":0,"cacheWrite":0}});
        let summary = json!({"type":"compaction","id":"summary","timestamp":THEN,
            "usage":{"input":4,"output":1,"cacheRead":0,"cacheWrite":0}});
        let subagent_total = json!({"type":"message","id":"task-summary","timestamp":THEN,
            "message":{"role":"toolResult","toolName":"task","usage":{"input":999,"output":999}}});
        write_jsonl(
            &root.path().join("session.jsonl"),
            &[
                json!({"type":"session","id":"native-session"}),
                assistant,
                auxiliary,
                summary,
                subagent_total,
            ],
        );
        assert_eq!(run_sync(&db, tool, root.path()).inserted_records, 3);
        let logs = usage_stats::request_logs(
            &db,
            &GatewayRequestLogFilters {
                cli_key: Some(tool),
                ..Default::default()
            },
            0,
            10,
        )
        .unwrap();
        assert_eq!(logs.total, 3);
        let row = logs
            .data
            .iter()
            .find(|row| row.trace_id.ends_with(":response"))
            .unwrap();
        assert_eq!(row.output_tokens, 10); // reasoning is already included
        assert_eq!(row.extra_tokens, 30);
        assert_eq!(row.total_tokens, 240);
        assert_eq!(row.total_cost_usd, "0.0123");
        assert_eq!(
            row.usage_metadata
                .as_ref()
                .unwrap()
                .native_provider
                .as_deref(),
            Some("historical-provider")
        );
        let detail = usage_stats::request_log_detail_from_summary(&db, &row.trace_id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.summary.cli_key, Some(tool));
        assert_eq!(detail.summary.total_tokens, Some(240));
        assert_eq!(detail.summary.status_code, None);
        let summary = usage_stats::usage_summary(&db, None, None, Some(tool)).unwrap();
        assert_eq!(summary.total_tokens, 252);
        assert_eq!(summary.total_requests, 3);
        assert!(usage_stats::usage_summary_by_cli(&db, None, None)
            .unwrap()
            .iter()
            .any(|entry| entry.cli_key == tool));
        assert_eq!(run_sync(&db, tool, root.path()).inserted_records, 0);
        let old = (Utc::now() - chrono::Duration::days(400)).timestamp();
        db.with_conn(|conn| {
            conn.execute("UPDATE proxy_request_logs SET created_at = ?1", [old])
                .map_err(|error| error.to_string())?;
            usage_stats::rollup_and_prune(conn, 7)
        })
        .unwrap();
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, Some(tool))
                .unwrap()
                .total_tokens,
            252
        );
        assert_eq!(
            usage_stats::model_stats(&db, None, None, Some(tool))
                .unwrap()
                .iter()
                .map(|row| row.total_tokens)
                .sum::<u64>(),
            252
        );
        assert_eq!(run_sync(&db, tool, root.path()).inserted_records, 0);
    }
}

#[test]
fn pi_fork_does_not_reimport_archived_parent_usage_or_zeroed_omp_cost() {
    for tool in [GatewayUsageTool::Pi, GatewayUsageTool::OhMyPi] {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("parent.jsonl");
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let mut old = pi_message("copied-id", 100, 10);
        old["timestamp"] = json!((Utc::now() - chrono::Duration::days(400)).timestamp());
        write_jsonl(
            &parent,
            &[json!({"type":"session","id":"parent"}), old.clone()],
        );
        run_sync(&db, tool, root.path());
        assert_eq!(count(&db), 1);
        old["message"]["usage"]["cost"]["total"] = json!(0);
        write_jsonl(
            &root.path().join("child.jsonl"),
            &[
                json!({"type":"session","id":"child","parentSession":parent}),
                old,
                pi_message("new-id", 3, 2),
            ],
        );
        run_sync(&db, tool, root.path());
        assert_eq!(count(&db), 2);
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, Some(tool))
                .unwrap()
                .total_tokens,
            315
        );
        assert_eq!(run_sync(&db, tool, root.path()).inserted_records, 0);
    }
}

fn dsh_message(turn: u64, step: u64, input: u64, output: u64) -> Value {
    json!({"type":"assistant/message","seq":turn*100+step,"time":THEN*1000,"surfaceOp":"append",
        "data":{"turn":turn,"step":step,"message":{"id":format!("message-{turn}-{step}"),"source":{"provider":"dsh-provider","model":"dsh-model"}},
            "usage":{"inputTokens":input,"outputTokens":output,"cacheReadTokens":80,"cacheWriteTokens":20}}})
}

fn write_zstd(path: &Path, values: &[Value]) {
    let text = values
        .iter()
        .map(|value| format!("{value}\n"))
        .collect::<String>();
    fs::write(path, zstd::stream::encode_all(text.as_bytes(), 0).unwrap()).unwrap();
}

#[test]
fn dsh_v0_and_v3_generations_keep_retry_and_summary_usage_once() {
    let root = tempfile::tempdir().unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let file = root.path().join("session.jsonl.zstd");
    let header = json!({"type":"session","id":"dsh-session","version":0});
    let seed =
        json!({"type":"session/end-seed","seq":1,"time":THEN*1000,"data":{"inherited":true}});
    let first = dsh_message(1, 1, 100, 10);
    write_zstd(
        &file,
        &[
            header.clone(),
            dsh_message(9, 9, 999, 999),
            seed.clone(),
            first.clone(),
            json!({"type":"assistant/chunk","data":{"usage":{"inputTokens":100,"outputTokens":10}}}),
        ],
    );
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Dsh, root.path()).inserted_records,
        1
    );
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None)
            .unwrap()
            .total_tokens,
        210
    );
    let mut attempt = first.clone();
    attempt["type"] = json!("assistant/attempt");
    attempt["data"].as_object_mut().unwrap().remove("usage");
    attempt["data"]["stream"] = json!([{"type":"usage","usage":{"inputTokens":100,"outputTokens":10,"cacheReadTokens":80,"cacheWriteTokens":20}}]);
    let mut retried = dsh_message(1, 1, 5, 2);
    retried["data"]["message"]["id"] = json!("retried-message");
    write_zstd(
        &root.path().join("session.v3.jsonl.zstd"),
        &[
            json!({"type":"session","id":"dsh-session","version":3}),
            seed,
            attempt,
            json!({"type":"llm/retry-started","data":{"turn":1,"step":1}}),
            retried,
            json!({"type":"compaction/summary","seq":50,"time":THEN*1000,"data":{"usage":{"inputTokens":10,"outputTokens":5}}}),
        ],
    );
    let result = run_sync(&db, GatewayUsageTool::Dsh, root.path());
    assert_eq!(result.scanned_files, 1);
    assert_eq!(result.failed_files, 0);
    assert_eq!(count(&db), 3);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None)
            .unwrap()
            .total_tokens,
        332
    );
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Dsh, root.path()).inserted_records,
        0
    );
}

#[test]
fn dsh_truncated_compressed_tail_keeps_committed_prefix_pending() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("session.jsonl.zstd");
    write_zstd(
        &file,
        &[
            json!({"type":"session","id":"tail"}),
            dsh_message(1, 1, 100, 10),
        ],
    );
    let mut tail =
        zstd::stream::encode_all(format!("{}\n", dsh_message(1, 2, 3, 2)).as_bytes(), 0).unwrap();
    tail.truncate(tail.len() / 2);
    fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(&tail)
        .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::Dsh, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(count(&db), 1);
    assert!(load_states(&db)
        .unwrap()
        .values()
        .any(|state| state.pending));
}

#[test]
fn dsh_resume_markers_preserve_paid_history_while_tagged_seed_excludes_inheritance() {
    for inherited in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut before = dsh_message(1, 1, 100, 10);
        before["data"]["message"]["id"] = json!("original-call");
        let mut after = dsh_message(1, 1, 3, 2);
        after["data"]["message"]["id"] = json!("new-call");
        write_zstd(
            &root.path().join("session.jsonl.zstd"),
            &[
                json!({"type":"session","version":0,"id":"resume"}),
                before,
                json!({"type":"session/end-seed","data":if inherited { json!({"inherited":true}) } else { json!({}) }}),
                after,
                json!({"type":"session/end-seed","data":{}}),
            ],
        );
        let db = SqliteDbState::in_memory_for_test().unwrap();
        run_sync(&db, GatewayUsageTool::Dsh, root.path());
        assert_eq!(count(&db), if inherited { 1 } else { 2 });
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, None)
                .unwrap()
                .total_tokens,
            if inherited { 105 } else { 315 }
        );
    }
}

#[test]
fn dsh_parser_upgrade_backfills_resume_history_without_rebilling_old_identities() {
    for archived in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("session.jsonl.zstd");
        let time = if archived {
            (Utc::now() - chrono::Duration::days(400)).timestamp()
        } else {
            THEN
        };
        let mut before = dsh_message(1, 1, 100, 10);
        before["time"] = json!(time * 1000);
        let mut after = dsh_message(1, 2, 3, 2);
        after["time"] = json!(time * 1000);
        let header = json!({"type":"session","version":0,"id":"migrated"});
        let marker = json!({"type":"session/end-seed","data":{}});
        write_zstd(
            &file,
            &[
                header.clone(),
                before.clone(),
                marker.clone(),
                after.clone(),
            ],
        );
        let parsed = dsh::parse(&file, time).unwrap();
        let mut old = parsed
            .records
            .into_iter()
            .find(|record| record.usage.input_tokens == Some(3))
            .unwrap();
        old.request_id = "SESSION:dsh:migrated:turn-1-step-2-attempt-0".into();
        old.legacy_request_ids.clear();
        let source_id = format!("dsh:{}", source_identity(GatewayUsageTool::Dsh, &file));
        let metadata = fs::metadata(&file).unwrap();
        let mut state = SourceState {
            parser_revision: 2,
            modified_nanos: modified_nanos(&metadata),
            size: metadata.len(),
            ..Default::default()
        };
        state.records.insert(
            old.request_id.clone(),
            ImportedRecord {
                fingerprint: old.fingerprint(),
                envelope_id: old.usage.envelope_id.clone(),
                ..Default::default()
            },
        );
        let db = SqliteDbState::in_memory_for_test().unwrap();
        db.with_conn(|conn| {
            write_record(conn, &old)?;
            save_state(conn, &source_id, &state)?;
            usage_stats::rollup_and_prune(conn, 7)
        })
        .unwrap();
        run_sync(&db, GatewayUsageTool::Dsh, root.path());
        assert_eq!(count(&db), 2, "archived={archived}");
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, None)
                .unwrap()
                .total_tokens,
            315
        );
        assert!(load_states(&db).unwrap()[&source_id].records[&old.request_id].retired);
        assert_eq!(
            run_sync(&db, GatewayUsageTool::Dsh, root.path()).inserted_records,
            0
        );
        // One old alias must not suppress a newly appended invocation even
        // when an older CLI reuses the same turn/step and usage snapshot.
        let mut new_call = after.clone();
        new_call["data"]["message"]["id"] = json!("later-independent-call");
        write_zstd(&file, &[header, before, marker, after, new_call]);
        run_sync(&db, GatewayUsageTool::Dsh, root.path());
        assert_eq!(count(&db), 3, "archived={archived}");
    }
}

#[test]
fn grok_turns_preserve_model_call_counts_and_ignore_total_and_subagent_mirrors() {
    let root = tempfile::tempdir().unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let event = json!({"method":"_x.ai/session/update","timestamp":THEN,"params":{"sessionId":"grok-session","_meta":{"totalTokens":999999},
        "update":{"sessionUpdate":"turn_completed","prompt_id":"prompt-one","usage":{"inputTokens":999,"outputTokens":999,"modelCalls":99,"usageIsIncomplete":true,
            "modelUsage":{"grok-model":{"inputTokens":100,"outputTokens":20,"cachedReadTokens":80,"modelCalls":3,"reasoningTokens":10,"totalTokens":120},
                "aux-model":{"inputTokens":10,"outputTokens":5,"cacheCreationInputTokens":5,"modelCalls":2,"totalTokens":15}}}}}});
    write_jsonl(
        &root.path().join("updates.jsonl"),
        &[event.clone(), event.clone()],
    );
    fs::create_dir_all(root.path().join("subagents/child")).unwrap();
    write_jsonl(&root.path().join("subagents/child/updates.jsonl"), &[event]);
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Grok, root.path()).inserted_records,
        2
    );
    let summary =
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Grok)).unwrap();
    assert_eq!(summary.total_requests, 5);
    assert_eq!(summary.total_tokens, 135);
    assert_eq!(summary.total_output_tokens, 25);
    let logs = usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).unwrap();
    assert_eq!(logs.total, 2);
    assert!(logs
        .data
        .iter()
        .all(|row| row.usage_metadata.as_ref().unwrap().incomplete));
    assert!(logs.data.iter().all(
        |row| row.usage_metadata.as_ref().unwrap().granularity == SessionUsageGranularity::Turn
    ));
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Grok, root.path()).inserted_records,
        0
    );
}

fn desktop_fixture(root: &Path, include_transcript: bool, timestamp: i64) -> (PathBuf, Value) {
    let audit = root.join("local-session/audit.jsonl");
    fs::create_dir_all(audit.parent().unwrap()).unwrap();
    let mut message = claude_message("desktop-message", 10);
    message["timestamp"] = json!(timestamp);
    message["sessionId"] = json!("desktop-session");
    let result = json!({"type":"result","uuid":"desktop-result","session_id":"desktop-session","timestamp":timestamp,
        "usage":message["message"]["usage"],"modelUsage":{"usage-test-model":{"inputTokens":100,"outputTokens":10,"cacheReadInputTokens":80,"cacheCreationInputTokens":20}},"total_cost_usd":0.002});
    let mut partial = message.clone();
    partial["message"]["usage"] = json!({"input_tokens":1,"output_tokens":1});
    write_jsonl(&audit, &[partial, result.clone()]);
    if include_transcript {
        let projects = audit.parent().unwrap().join(".claude/projects/project");
        fs::create_dir_all(&projects).unwrap();
        write_jsonl(&projects.join("desktop-session.jsonl"), &[message]);
    }
    (audit, result)
}

#[test]
fn desktop_transcript_replaces_audit_fallback_before_and_after_archival() {
    for archive_first in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let time = if archive_first {
            (Utc::now() - chrono::Duration::days(400)).timestamp()
        } else {
            THEN
        };
        desktop_fixture(root.path(), false, time);
        run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path());
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, None)
                .unwrap()
                .total_tokens,
            210
        );
        desktop_fixture(root.path(), true, time);
        run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path());
        assert_eq!(
            usage_stats::usage_summary(&db, None, None, None)
                .unwrap()
                .total_tokens,
            210,
            "archive_first={archive_first}"
        );
        assert_eq!(count(&db), 1);
        assert!(load_states(&db)
            .unwrap()
            .values()
            .flat_map(|state| state.records.values())
            .any(|record| record.retired));
        run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path());
        assert_eq!(count(&db), 1);
    }
}

#[test]
fn desktop_legacy_unknown_rollup_is_repaired_only_when_the_bucket_reconciles() {
    for unrelated_unknown in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
        let (_, result) = desktop_fixture(root.path(), true, time);
        let record = parsers::parse_value(
            GatewayUsageTool::ClaudeDesktop,
            &result,
            "desktop-session",
            1,
            time,
        )
        .unwrap();
        let mut state = SourceState::default();
        state.records.insert(
            record.request_id.clone(),
            ImportedRecord {
                fingerprint: record.legacy_fingerprint(),
                ..Default::default()
            },
        );
        db.with_conn(|conn| {
            save_state(conn, "claude_desktop:audit", &state)?;
            conn.execute("INSERT INTO usage_daily_rollups (date,app_type,provider_id,model,request_count,success_count,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,total_cost_usd,avg_latency_ms,latency_sample_count)
                VALUES (date(?1,'unixepoch','localtime'),'claude_desktop','session','unknown',?2,?2,?3,10,80,20,'0',0,0)", params![time, if unrelated_unknown {2} else {1}, if unrelated_unknown {107} else {100}]).map_err(|error| error.to_string())?;
            Ok(())
        }).unwrap();
        run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path());
        let states = load_states(&db).unwrap();
        assert_eq!(
            states["claude_desktop:audit"].records[&record.request_id].retired,
            !unrelated_unknown
        );
        assert_eq!(count(&db), if unrelated_unknown { 3 } else { 1 });
        run_sync(&db, GatewayUsageTool::ClaudeDesktop, root.path());
        assert_eq!(count(&db), if unrelated_unknown { 3 } else { 1 });
    }
}

#[test]
fn kimi_wire_and_store_mirrors_use_the_same_native_event_identity() {
    let root = tempfile::tempdir().unwrap();
    let wire_dir = root.path().join("sessions/kimi-session/agents/main");
    let tree_dir = root.path().join("store/trees/kimi-session");
    fs::create_dir_all(&wire_dir).unwrap();
    fs::create_dir_all(&tree_dir).unwrap();
    let usage = json!({"type":"usage.record","agentId":"main","model":"kimi-test","time":THEN*1000,
        "usageScope":"turn","usage":{"inputOther":20,"output":10,"inputCacheRead":80,"inputCacheCreation":5}});
    write_jsonl(
        &wire_dir.join("wire.jsonl"),
        &[
            json!({"type":"metadata","protocol_version":"1.0"}),
            usage.clone(),
            json!({"type":"agent.status.updated","usage":{"total":{"inputOther":999999,"output":999999}}}),
        ],
    );
    write_jsonl(
        &tree_dir.join("main.jsonl"),
        &[
            json!({"version":1,"tree":"kimi-session","branch":"main","createdAt":THEN*1000}),
            json!({"kind":"entry","seq":1,"ts":THEN*1000,"type":"usage.record","payload":{"kind":"event","data":usage}}),
        ],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::Kimi, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(count(&db), 1);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None)
            .unwrap()
            .total_tokens,
        115
    );
    let logs = usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).unwrap();
    assert_eq!(logs.data[0].upstream_model_id, "kimi-test");
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Kimi, root.path()).inserted_records,
        0
    );
}

#[test]
fn python_kimi_status_updates_and_nested_subagents_preserve_usage_without_guessing_models() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    fs::create_dir_all(session.join("subagents/child")).unwrap();
    let status = json!({"type":"StatusUpdate","payload":{"message_id":"kimi-response","context_tokens":99999,
        "token_usage":{"input_other":10,"output":5,"input_cache_read":80,"input_cache_creation":20}}});
    write_jsonl(
        &session.join("wire.jsonl"),
        &[
            json!({"timestamp":THEN as f64+0.25,"message":status}),
            json!({"timestamp":THEN as f64+0.5,"message":{"type":"SubagentEvent","payload":{"agent_id":"child","event":status}}}),
            json!({"timestamp":THEN,"message":{"type":"StatusUpdate","payload":{"context_usage":0.9,"context_tokens":999999}}}),
        ],
    );
    write_jsonl(
        &session.join("subagents/child/wire.jsonl"),
        &[json!({"timestamp":THEN,"message":status})],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::KimiCli, root.path()).failed_files,
        0
    );
    assert_eq!(count(&db), 1);
    let logs = usage_stats::request_logs(
        &db,
        &GatewayRequestLogFilters {
            cli_key: Some(GatewayUsageTool::KimiCli),
            ..Default::default()
        },
        0,
        10,
    )
    .unwrap();
    assert_eq!(logs.data[0].total_tokens, 115);
    assert_eq!(logs.data[0].upstream_model_id, "unknown");
    assert_eq!(logs.data[0].created_at.timestamp(), THEN);
}

#[test]
fn hermes_cumulative_wal_updates_charge_only_deltas_and_reconcile_actual_cost() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.db");
    let source = Connection::open(&path).unwrap();
    source.execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;
        CREATE TABLE sessions(id TEXT PRIMARY KEY, input_tokens INTEGER, output_tokens INTEGER, api_call_count INTEGER);
        CREATE TABLE session_model_usage(session_id TEXT, model TEXT, billing_provider TEXT, billing_base_url TEXT, billing_mode TEXT, task TEXT,
            api_call_count INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER,
            estimated_cost_usd REAL, actual_cost_usd REAL, first_seen REAL, last_seen REAL, cost_status TEXT);
        INSERT INTO sessions VALUES ('hermes-session',999999,999999,999);").unwrap();
    source.execute("INSERT INTO session_model_usage VALUES ('hermes-session','hermes-model','historical','https://example.test/v1','api','main',3,100,20,80,10,0.03,0,?1,?2,'estimated')", params![THEN-86400, THEN]).unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        1
    );
    let first = usage_stats::usage_summary(&db, None, None, None).unwrap();
    assert_eq!(first.total_tokens, 210);
    assert_eq!(first.total_requests, 3);
    assert_eq!(first.total_cost_usd, "0.030000");
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        0
    );
    source.execute("UPDATE session_model_usage SET input_tokens = 120, output_tokens = 25, api_call_count = 4, estimated_cost_usd = 0.04, last_seen = ?1", [THEN+10]).unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        1
    );
    let summary = usage_stats::usage_summary(&db, None, None, None).unwrap();
    assert_eq!(summary.total_tokens, 235);
    assert_eq!(summary.total_requests, 4);
    assert_eq!(summary.total_cost_usd, "0.040000");
    // Final actual billing is smaller than the original estimate.
    source.execute("UPDATE session_model_usage SET actual_cost_usd = 0.035, cost_status = 'actual', last_seen = ?1", [THEN+20]).unwrap();
    run_sync(&db, GatewayUsageTool::Hermes, root.path());
    let corrected = usage_stats::usage_summary(&db, None, None, None).unwrap();
    assert_eq!(corrected.total_requests, 4);
    assert_eq!(corrected.total_tokens, 235);
    assert_eq!(corrected.total_cost_usd, "0.035000");
    let correction_only =
        usage_stats::usage_summary_by_cli(&db, Some(THEN + 20), Some(THEN + 20)).unwrap();
    assert_eq!(correction_only.len(), 1);
    assert_eq!(correction_only[0].cli_key, GatewayUsageTool::Hermes);
    assert_eq!(correction_only[0].summary.total_requests, 0);
    assert_eq!(correction_only[0].summary.total_tokens, 0);
    assert_eq!(correction_only[0].summary.total_cost_usd, "-0.005000");
    let logs = usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).unwrap();
    assert!(logs
        .data
        .iter()
        .all(|row| row.usage_metadata.as_ref().unwrap().granularity
            == SessionUsageGranularity::Session));
    assert!(logs
        .data
        .iter()
        .any(|row| row.total_cost_usd.starts_with('-')));
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        0
    );
    assert_eq!(
        source
            .query_row("SELECT input_tokens FROM sessions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        999999
    );
}

#[test]
fn hermes_equal_model_counters_and_repeated_cost_corrections_stay_independent() {
    let root = tempfile::tempdir().unwrap();
    let source = Connection::open(root.path().join("state.db")).unwrap();
    source.execute_batch("CREATE TABLE session_model_usage(session_id TEXT, model TEXT, input_tokens INTEGER, output_tokens INTEGER,
        api_call_count INTEGER, estimated_cost_usd REAL, actual_cost_usd REAL, cost_status TEXT, last_seen INTEGER);").unwrap();
    for model in ["model-a", "model-b"] {
        source.execute("INSERT INTO session_model_usage VALUES ('shared-session',?1,100,20,1,0.04,0,'estimated',?2)", params![model, THEN]).unwrap();
    }
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        2
    );
    for (cost, total) in [(0.03, "0.070000"), (0.04, "0.080000"), (0.03, "0.070000")] {
        source.execute("UPDATE session_model_usage SET actual_cost_usd=?1,cost_status='actual' WHERE model='model-a'", [cost]).unwrap();
        assert_eq!(
            run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
            1
        );
        let summary = usage_stats::usage_summary(&db, None, None, None).unwrap();
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.total_tokens, 240);
        assert_eq!(summary.total_cost_usd, total);
        assert_eq!(
            run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
            0
        );
    }
    let models = usage_stats::model_stats(&db, None, None, None).unwrap();
    assert_eq!(models.len(), 2);
    assert!(models.iter().all(|model| model.total_tokens == 120));
}

#[test]
fn hermes_richer_schema_does_not_rebill_an_imported_legacy_total() {
    let root = tempfile::tempdir().unwrap();
    let source = Connection::open(root.path().join("state.db")).unwrap();
    source.execute_batch("CREATE TABLE sessions(id TEXT,model TEXT,input_tokens INTEGER,output_tokens INTEGER,api_call_count INTEGER);
        INSERT INTO sessions VALUES ('old-session','latest-model',100,10,2)").unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Hermes, root.path());
    assert_eq!(count(&db), 2);
    source.execute_batch("CREATE TABLE session_model_usage(session_id TEXT,model TEXT,input_tokens INTEGER,output_tokens INTEGER,api_call_count INTEGER);
        INSERT INTO session_model_usage VALUES ('old-session','model-a',60,6,1),('old-session','model-b',40,4,1);").unwrap();
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        0
    );
    assert_eq!(count(&db), 2);
    source.execute("UPDATE session_model_usage SET input_tokens = 70, output_tokens = 8, api_call_count = 2 WHERE model = 'model-a'", []).unwrap();
    run_sync(&db, GatewayUsageTool::Hermes, root.path());
    assert_eq!(count(&db), 3);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None)
            .unwrap()
            .total_tokens,
        122
    );
}

#[test]
fn openclaw_prefers_sqlite_and_deduplicates_file_and_database_archives() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("main/agent");
    let sessions = root.path().join("main/sessions");
    fs::create_dir_all(&agent).unwrap();
    fs::create_dir_all(&sessions).unwrap();
    let source = Connection::open(agent.join("openclaw-agent.sqlite")).unwrap();
    source.execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;
        CREATE TABLE transcript_events(session_id TEXT, seq INTEGER, event_json TEXT, created_at INTEGER);
        CREATE TABLE session_transcript_archives(session_id TEXT,generation TEXT,encoding TEXT,archive_blob BLOB,created_at INTEGER);").unwrap();
    let mut message = pi_message("openclaw-message", 100, 10);
    message["message"]["usage"] = json!({"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":80},"reasoning_tokens":10});
    source
        .execute(
            "INSERT INTO transcript_events VALUES ('openclaw-session',1,?1,?2)",
            params![message.to_string(), THEN * 1000],
        )
        .unwrap();
    let mut stale_file = message.clone();
    stale_file["message"]["usage"]["output_tokens"] = json!(1);
    write_jsonl(
        &sessions.join("openclaw-session.jsonl"),
        &[
            json!({"type":"session","id":"openclaw-session"}),
            stale_file,
        ],
    );
    let archived = format!(
        "{}\n{}\n",
        json!({"type":"session","id":"archived-session"}),
        pi_message("archive-message", 5, 2)
    );
    let compressed = zstd::stream::encode_all(archived.as_bytes(), 0).unwrap();
    source
        .execute(
            "INSERT INTO session_transcript_archives VALUES ('archived-session','g1','zstd',?1,?2)",
            params![compressed, THEN * 1000],
        )
        .unwrap();
    fs::write(
        sessions.join("archived-session.jsonl.reset.2026-09-10T00-00-00Z.zst"),
        compressed,
    )
    .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let result = run_sync(&db, GatewayUsageTool::OpenClaw, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(count(&db), 2);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None)
            .unwrap()
            .total_tokens,
        227
    );
    source.execute("UPDATE transcript_events SET event_json = json_set(event_json,'$.message.usage.output_tokens',30)", []).unwrap();
    run_sync(&db, GatewayUsageTool::OpenClaw, root.path());
    assert_eq!(count(&db), 2);
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, None)
            .unwrap()
            .total_tokens,
        237
    );
    assert_eq!(
        run_sync(&db, GatewayUsageTool::OpenClaw, root.path()).inserted_records,
        0
    );
}

#[test]
fn usage_source_and_historical_provider_filters_round_trip() {
    let root = tempfile::tempdir().unwrap();
    write_jsonl(
        &root.path().join("session.jsonl"),
        &[pi_message("filter", 10, 2)],
    );
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Pi, root.path());
    insert_proxy(&db, "proxy-filter");
    let filters = GatewayRequestLogFilters {
        data_source: Some("session".into()),
        provider_name: Some("historical-provider".into()),
        ..Default::default()
    };
    let logs = usage_stats::request_logs(&db, &filters, 0, 10).unwrap();
    assert_eq!(logs.total, 1);
    assert_eq!(logs.data[0].cli_key, GatewayUsageTool::Pi);
    assert_eq!(
        usage_stats::request_logs(
            &db,
            &GatewayRequestLogFilters {
                data_source: Some("proxy".into()),
                ..Default::default()
            },
            0,
            10
        )
        .unwrap()
        .total,
        1
    );
}

#[test]
#[ignore = "Read-only developer session verification; writes usage to a temporary database only"]
fn local_native_sources_read_only_smoke() {
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let sources = [
        GatewayUsageTool::Pi,
        GatewayUsageTool::OhMyPi,
        GatewayUsageTool::Dsh,
        GatewayUsageTool::Grok,
        GatewayUsageTool::ClaudeDesktop,
    ]
    .into_iter()
    .flat_map(|tool| {
        default_session_roots(&db, tool)
            .into_iter()
            .map(move |root| (tool, root))
    })
    .collect::<Vec<_>>();
    let result = sync_sources(&db, &sources, Utc::now().timestamp()).unwrap();
    assert_eq!(result.failed_files, 0);
    let before = usage_stats::usage_summary_by_cli(&db, None, None).unwrap();
    for item in &before {
        println!(
            "{}: calls={} tokens={}",
            item.cli_key.as_str(),
            item.summary.total_requests,
            item.summary.total_tokens
        );
    }
    let repeated = sync_sources(&db, &sources, Utc::now().timestamp()).unwrap();
    assert_eq!(repeated.inserted_records, 0);
    assert_eq!(
        usage_stats::usage_summary_by_cli(&db, None, None).unwrap(),
        before
    );
    println!(
        "scanned={} parsed={} inserted={} updated={} repeated_inserted={}",
        result.scanned_files,
        result.parsed_records,
        result.inserted_records,
        result.updated_records,
        repeated.inserted_records
    );
}

#[test]
#[ignore = "Copies the local application database through a read-only connection; reconciles only the temporary copy"]
fn local_database_reconciliation_on_a_snapshot() {
    let source_path = dirs::data_dir()
        .unwrap()
        .join("com.ai-toolbox/ai-toolbox.db");
    let source =
        Connection::open_with_flags(&source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let copy = temporary.path().join("usage-reconciliation.db");
    source.backup(rusqlite::MAIN_DB, &copy, None).unwrap();
    drop(source);
    let db = SqliteDbState::open(copy).unwrap();
    let before = usage_stats::usage_summary(&db, None, None, None).unwrap();
    let proxy_count = |db: &SqliteDbState| {
        db.with_conn(|conn| {
        conn.query_row("SELECT COUNT(*) FROM proxy_request_logs WHERE COALESCE(data_source,'proxy')='proxy'",[],|row| row.get::<_,u32>(0)).map_err(|error|error.to_string())
    }).unwrap()
    };
    let before_proxies = proxy_count(&db);
    let mut states = load_states(&db).unwrap();
    let mut claims = states
        .values()
        .flat_map(|state| state.records.iter())
        .filter_map(|(id, state)| {
            state
                .matched_proxy_id
                .as_ref()
                .map(|proxy| (proxy.clone(), id.clone()))
        })
        .collect();
    let reconciled = reconcile_late_proxy_rows(&db, &mut states, &mut claims).unwrap();
    let after = usage_stats::usage_summary(&db, None, None, None).unwrap();
    assert_eq!(proxy_count(&db), before_proxies);
    assert_eq!(before.total_requests - after.total_requests, reconciled);
    assert_eq!(
        reconcile_late_proxy_rows(&db, &mut states, &mut claims).unwrap(),
        0
    );
    println!("snapshot_reconciled={reconciled} token_contribution_removed={} gateway_rows_preserved={before_proxies}",before.total_tokens-after.total_tokens);
}

#[test]
#[ignore = "Verifies the local DSH parser upgrade on a temporary database copy; requires all original DSH transcripts"]
fn local_dsh_parser_upgrade_on_a_snapshot() {
    let source_path = dirs::data_dir()
        .unwrap()
        .join("com.ai-toolbox/ai-toolbox.db");
    let source =
        Connection::open_with_flags(&source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let copy = temporary.path().join("dsh-parser-upgrade.db");
    source.backup(rusqlite::MAIN_DB, &copy, None).unwrap();
    drop(source);
    let db = SqliteDbState::open(copy).unwrap();
    let sources = default_session_roots(&db, GatewayUsageTool::Dsh)
        .into_iter()
        .map(|root| (GatewayUsageTool::Dsh, root))
        .collect::<Vec<_>>();
    let before = usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Dsh)).unwrap();
    let fresh = SqliteDbState::in_memory_for_test().unwrap();
    let now = Utc::now().timestamp();
    assert_eq!(sync_sources(&fresh, &sources, now).unwrap().failed_files, 0);
    let expected =
        usage_stats::usage_summary(&fresh, None, None, Some(GatewayUsageTool::Dsh)).unwrap();
    let upgraded = sync_sources(&db, &sources, now).unwrap();
    assert_eq!(upgraded.failed_files, 0);
    let after = usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Dsh)).unwrap();
    println!("dsh_snapshot_before_calls={} before_tokens={} after_calls={} after_tokens={} fresh_calls={} fresh_tokens={} inserted={}", before.total_requests, before.total_tokens, after.total_requests, after.total_tokens, expected.total_requests, expected.total_tokens, upgraded.inserted_records);
    assert_eq!(after.total_requests, expected.total_requests);
    assert_eq!(after.total_tokens, expected.total_tokens);
    assert_eq!(
        sync_sources(&db, &sources, now).unwrap().inserted_records,
        0
    );
    assert_eq!(
        usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Dsh)).unwrap(),
        after
    );
}
