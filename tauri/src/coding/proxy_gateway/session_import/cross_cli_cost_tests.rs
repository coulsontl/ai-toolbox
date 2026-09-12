use super::*;

fn summary_for(db: &SqliteDbState, tool: GatewayUsageTool) -> GatewayUsageSummary {
    usage_stats::usage_summary(db, None, None, Some(tool), true).unwrap()
}

fn unpriced_pi_message(id: &str, model: &str, time: i64) -> Value {
    let mut message = pi_message(id, 1_000_000, 100_000);
    message["timestamp"] = json!(time);
    message["message"]["model"] = json!(model);
    message["message"]["usage"] = json!({"input":1_000_000,"output":100_000,
        "cacheRead":2_000_000,"cacheWrite":0,"cost":{"total":0}});
    message
}

#[test]
fn pi_and_omp_late_prices_repair_live_and_archived_costs_but_keep_reported_amounts() {
    for tool in [GatewayUsageTool::Pi, GatewayUsageTool::OhMyPi] {
        for archived in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let time = if archived {
                (Utc::now() - chrono::Duration::days(400)).timestamp()
            } else {
                THEN
            };
            let unpriced = unpriced_pi_message("unpriced", "late-price-model", time);
            let mut paid = unpriced_pi_message("reported", "late-price-model", time);
            paid["message"]["usage"]["cost"]["total"] = json!(0.123456);
            write_jsonl(
                &root.path().join("session.jsonl"),
                &[
                    json!({"type":"session","id":"cost-session"}),
                    unpriced,
                    paid,
                ],
            );
            let db = SqliteDbState::in_memory_for_test().unwrap();
            run_sync(&db, tool, root.path());
            assert_eq!(summary_for(&db, tool).total_cost_usd, "0.123456");
            cost_pricing(&db, "late-price-model", "2", "4", "0.2");
            let result = run_sync(&db, tool, root.path());
            assert_eq!(result.failed_files, 0);
            assert_eq!(result.inserted_records, 0);
            assert_eq!(
                result.updated_records, 1,
                "tool={tool:?}, archived={archived}"
            );
            let summary = summary_for(&db, tool);
            assert_eq!(summary.total_requests, 2);
            assert_eq!(summary.total_tokens, 6_200_000);
            assert_eq!(summary.total_cost_usd, "2.923456");
            cost_pricing(&db, "late-price-model", "20", "40", "2");
            assert_eq!(run_sync(&db, tool, root.path()).updated_records, 0);
            assert_eq!(summary_for(&db, tool), summary);
        }
    }
}

fn unpriced_opencode_message(model: &str, time: i64) -> Value {
    json!({"id":"msg-cost","sessionID":"ses-cost","role":"assistant","modelID":model,
        "time":{"created":time*1000,"completed":time*1000},"cost":0,
        "tokens":{"input":1_000_000,"output":60_000,"reasoning":40_000,
            "cache":{"read":2_000_000,"write":0}}})
}

#[test]
fn opencode_sqlite_late_prices_repair_without_a_new_native_watermark() {
    for archived in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let time = if archived {
            (Utc::now() - chrono::Duration::days(400)).timestamp()
        } else {
            THEN
        };
        let source = Connection::open(root.path().join("opencode.db")).unwrap();
        source.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            CREATE TABLE session(id TEXT PRIMARY KEY,time_updated INTEGER);
            CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);
            INSERT INTO session VALUES('ses-cost',1);").unwrap();
        let message = unpriced_opencode_message("sqlite-price-model", time);
        source
            .execute(
                "INSERT INTO message VALUES('msg-cost','ses-cost',?1,2,?2)",
                params![time * 1000, message.to_string()],
            )
            .unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        run_sync(&db, GatewayUsageTool::OpenCode, root.path());
        let before = summary_for(&db, GatewayUsageTool::OpenCode);
        assert_eq!(before.total_cost_usd, "0.000000");
        cost_pricing(&db, "sqlite-price-model", "2", "4", "0.2");
        let result = run_sync(&db, GatewayUsageTool::OpenCode, root.path());
        assert_eq!(result.failed_files, 0);
        assert_eq!(result.updated_records, 1);
        let after = summary_for(&db, GatewayUsageTool::OpenCode);
        assert_eq!(after.total_cost_usd, "2.800000");
        assert_eq!(after.total_requests, before.total_requests);
        assert_eq!(after.total_tokens, before.total_tokens);
        assert_eq!(
            run_sync(&db, GatewayUsageTool::OpenCode, root.path()).updated_records,
            0
        );
        assert_eq!(
            source
                .query_row("SELECT data FROM message", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            message.to_string()
        );
        assert_eq!(
            source
                .query_row("SELECT time_updated FROM message", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

#[test]
fn opencode_legacy_thinking_alias_costs_repair_only_the_proven_archive() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("storage/message/ses-cost");
    fs::create_dir_all(&directory).unwrap();
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    fs::write(
        directory.join("msg-cost.json"),
        unpriced_opencode_message("gemini-claude-opus-4-5-thinking", time).to_string(),
    )
    .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        conn.execute("DELETE FROM model_pricing", [])
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
    .unwrap();
    run_sync(&db, GatewayUsageTool::OpenCode, root.path());
    assert_eq!(
        summary_for(&db, GatewayUsageTool::OpenCode).total_cost_usd,
        "0.000000"
    );
    remove_old_cost_provenance(&db);
    cost_pricing(&db, "claude-opus-4-5-20251101", "2", "4", "0.2");
    let result = run_sync(&db, GatewayUsageTool::OpenCode, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.updated_records, 1);
    let summary = summary_for(&db, GatewayUsageTool::OpenCode);
    assert_eq!(summary.total_cost_usd, "2.800000");
    assert_eq!(summary.total_tokens, 3_100_000);
    assert_eq!(summary.total_requests, 1);
}

#[test]
fn openclaw_sqlite_archives_recover_missing_prices_without_reimporting_tokens() {
    let root = tempfile::tempdir().unwrap();
    let agent = root.path().join("main/agent");
    fs::create_dir_all(&agent).unwrap();
    let source = Connection::open(agent.join("openclaw-agent.sqlite")).unwrap();
    source.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
        CREATE TABLE transcript_events(session_id TEXT,seq INTEGER,event_json TEXT,created_at INTEGER);
        CREATE TABLE session_transcript_archives(session_id TEXT,generation TEXT,encoding TEXT,archive_blob BLOB,created_at INTEGER);").unwrap();
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    let live = unpriced_pi_message("live-cost", "openclaw-cost-model", time);
    let archived = unpriced_pi_message("archived-cost", "openclaw-cost-model", time);
    source
        .execute(
            "INSERT INTO transcript_events VALUES('live-session',1,?1,?2)",
            params![live.to_string(), time * 1000],
        )
        .unwrap();
    let blob = zstd::stream::encode_all(format!("{archived}\n").as_bytes(), 0).unwrap();
    source
        .execute(
            "INSERT INTO session_transcript_archives VALUES('archived-session','g1','zstd',?1,?2)",
            params![blob, time * 1000],
        )
        .unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::OpenClaw, root.path());
    let before = summary_for(&db, GatewayUsageTool::OpenClaw);
    cost_pricing(&db, "openclaw-cost-model", "2", "4", "0.2");
    let result = run_sync(&db, GatewayUsageTool::OpenClaw, root.path());
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.updated_records, 2);
    let after = summary_for(&db, GatewayUsageTool::OpenClaw);
    assert_eq!(after.total_cost_usd, "5.600000");
    assert_eq!(after.total_tokens, before.total_tokens);
    assert_eq!(after.total_requests, before.total_requests);
    assert_eq!(
        run_sync(&db, GatewayUsageTool::OpenClaw, root.path()).updated_records,
        0
    );
}

#[test]
fn hermes_unknown_costs_gain_estimates_and_later_native_totals_replace_them() {
    let root = tempfile::tempdir().unwrap();
    let source = Connection::open(root.path().join("state.db")).unwrap();
    source.execute_batch("CREATE TABLE session_model_usage(session_id TEXT,model TEXT,input_tokens INTEGER,output_tokens INTEGER,
        cache_read_tokens INTEGER,api_call_count INTEGER,estimated_cost_usd REAL,actual_cost_usd REAL,cost_status TEXT,last_seen INTEGER);").unwrap();
    source.execute("INSERT INTO session_model_usage VALUES('cost-session','hermes-cost-model',1000000,100000,2000000,1,NULL,NULL,'unknown',?1)", [THEN]).unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    run_sync(&db, GatewayUsageTool::Hermes, root.path());
    assert_eq!(
        summary_for(&db, GatewayUsageTool::Hermes).total_cost_usd,
        "0.000000"
    );
    cost_pricing(&db, "hermes-cost-model", "2", "4", "0.2");
    run_sync(&db, GatewayUsageTool::Hermes, root.path());
    let priced = summary_for(&db, GatewayUsageTool::Hermes);
    assert_eq!(priced.total_cost_usd, "2.800000");
    assert_eq!(priced.total_tokens, 3_100_000);
    assert_eq!(priced.total_requests, 1);
    cost_pricing(&db, "hermes-cost-model", "20", "40", "2");
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        0
    );
    assert_eq!(summary_for(&db, GatewayUsageTool::Hermes), priced);
    source
        .execute(
            "UPDATE session_model_usage SET actual_cost_usd=1.25,cost_status='actual'",
            [],
        )
        .unwrap();
    run_sync(&db, GatewayUsageTool::Hermes, root.path());
    let actual = summary_for(&db, GatewayUsageTool::Hermes);
    assert_eq!(actual.total_cost_usd, "1.250000");
    assert_eq!(actual.total_tokens, priced.total_tokens);
    assert_eq!(actual.total_requests, priced.total_requests);
    source.execute("UPDATE session_model_usage SET actual_cost_usd=0,estimated_cost_usd=0,cost_status='included'", []).unwrap();
    run_sync(&db, GatewayUsageTool::Hermes, root.path());
    assert_eq!(
        summary_for(&db, GatewayUsageTool::Hermes).total_cost_usd,
        "0.000000"
    );
    assert_eq!(
        run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
        0
    );
}

#[test]
fn other_file_adapters_backfill_prices_without_changing_native_granularity() {
    let time = (Utc::now() - chrono::Duration::days(400)).timestamp();
    let model = "late-file-model";
    let cases = [
        (
            GatewayUsageTool::Codex,
            "rollout-cost.jsonl",
            vec![
                json!({"type":"session_meta","payload":{"id":"cost-session"}}),
                json!({"type":"turn_context","payload":{"model":model}}),
                json!({"type":"event_msg","timestamp":time,"payload":{"type":"token_count","info":{
                "last_token_usage":{"input_tokens":3_000_000,"output_tokens":100_000,"cached_input_tokens":2_000_000}}}}),
            ],
        ),
        (
            GatewayUsageTool::Gemini,
            "session-cost.jsonl",
            vec![
                json!({"type":"gemini","id":"cost-message","model":model,"timestamp":time,
                "tokens":{"input":3_000_000,"output":60_000,"thoughts":40_000,"cached":2_000_000}}),
            ],
        ),
        (
            GatewayUsageTool::Grok,
            "updates.jsonl",
            vec![
                json!({"method":"_x.ai/session/update","timestamp":time,"params":{"sessionId":"cost-session",
                "update":{"sessionUpdate":"turn_completed","prompt_id":"cost-prompt","usage":{
                    "modelUsage":{(model):{"inputTokens":3_000_000,"outputTokens":100_000,"cachedReadTokens":2_000_000,"modelCalls":3}}}}}}),
            ],
        ),
        (
            GatewayUsageTool::Kimi,
            "wire.jsonl",
            vec![
                json!({"type":"usage.record","agentId":"main","model":model,"time":time*1000,
                "usage":{"inputOther":1_000_000,"output":100_000,"inputCacheRead":2_000_000,"inputCacheCreation":0}}),
            ],
        ),
        (
            GatewayUsageTool::KimiCli,
            "wire.jsonl",
            vec![
                json!({"timestamp":time,"message":{"type":"StatusUpdate","payload":{"message_id":"cost-message","model":model,
                "token_usage":{"input_other":1_000_000,"output":100_000,"input_cache_read":2_000_000,"input_cache_creation":0}}}}),
            ],
        ),
        (
            GatewayUsageTool::OpenClaw,
            "main/sessions/session-cost.jsonl",
            vec![
                json!({"type":"session","id":"cost-session"}),
                unpriced_pi_message("cost-message", model, time),
            ],
        ),
    ];
    for (tool, filename, messages) in cases {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(filename);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_jsonl(&path, &messages);
        let db = SqliteDbState::in_memory_for_test().unwrap();
        assert_eq!(run_sync(&db, tool, root.path()).failed_files, 0);
        let before = summary_for(&db, tool);
        assert_eq!(before.total_cost_usd, "0.000000");
        assert_eq!(before.total_tokens, 3_100_000, "{tool:?}");
        cost_pricing(&db, model, "2", "4", "0.2");
        let result = run_sync(&db, tool, root.path());
        assert_eq!(result.failed_files, 0);
        assert_eq!(result.inserted_records, 0);
        assert_eq!(result.updated_records, 1, "{tool:?}");
        let after = summary_for(&db, tool);
        assert_eq!(after.total_requests, before.total_requests);
        assert_eq!(after.total_tokens, before.total_tokens);
        assert_eq!(after.total_cost_usd, "2.800000");
        assert_eq!(run_sync(&db, tool, root.path()).updated_records, 0);
    }
}

#[test]
fn failed_native_sources_are_not_counted_twice_and_do_not_block_other_tools() {
    let pi_root = tempfile::tempdir().unwrap();
    let omp_root = tempfile::tempdir().unwrap();
    let message = unpriced_pi_message("pending-cost", "mixed-source-model", THEN);
    for root in [pi_root.path(), omp_root.path()] {
        write_jsonl(&root.join("session.jsonl"), &[message.clone()]);
    }
    let sources = [
        (GatewayUsageTool::Pi, pi_root.path().to_path_buf()),
        (GatewayUsageTool::OhMyPi, omp_root.path().to_path_buf()),
    ];
    let db = SqliteDbState::in_memory_for_test().unwrap();
    assert_eq!(sync_sources(&db, &sources, NOW).unwrap().failed_files, 0);
    fs::write(pi_root.path().join("broken.jsonl"), "invalid session\n").unwrap();
    cost_pricing(&db, "mixed-source-model", "2", "4", "0.2");
    let partial = sync_sources(&db, &sources, NOW).unwrap();
    assert_eq!(partial.failed_files, 1);
    assert_eq!(partial.updated_records, 1);
    assert_eq!(
        summary_for(&db, GatewayUsageTool::Pi).total_cost_usd,
        "0.000000"
    );
    assert_eq!(
        summary_for(&db, GatewayUsageTool::OhMyPi).total_cost_usd,
        "2.800000"
    );
    fs::write(
        pi_root.path().join("broken.jsonl"),
        "{\"type\":\"session\",\"id\":\"empty\"}\n",
    )
    .unwrap();
    let retried = sync_sources(&db, &sources, NOW).unwrap();
    assert_eq!(retried.failed_files, 0);
    assert_eq!(retried.updated_records, 1);
    assert_eq!(
        summary_for(&db, GatewayUsageTool::Pi).total_cost_usd,
        "2.800000"
    );
    assert_eq!(summary_for(&db, GatewayUsageTool::Pi).total_requests, 1);
}

#[test]
fn hermes_resolved_zero_prices_and_old_estimates_survive_later_price_changes() {
    for status in ["unknown", "actual", "estimated", "included"] {
        let root = tempfile::tempdir().unwrap();
        let source = Connection::open(root.path().join("state.db")).unwrap();
        source.execute_batch("CREATE TABLE session_model_usage(session_id TEXT,model TEXT,input_tokens INTEGER,output_tokens INTEGER,
            api_call_count INTEGER,estimated_cost_usd REAL,actual_cost_usd REAL,cost_status TEXT,last_seen INTEGER);").unwrap();
        source.execute("INSERT INTO session_model_usage VALUES('cost-session','hermes-zero-model',1000000,0,1,0,0,?1,?2)", params![status,THEN]).unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        run_sync(&db, GatewayUsageTool::Hermes, root.path());
        if status != "unknown" {
            let logs =
                usage_stats::request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10, true)
                    .unwrap();
            assert_eq!(
                logs.data[0]
                    .usage_metadata
                    .as_ref()
                    .unwrap()
                    .cost_source
                    .as_deref(),
                Some(if status == "estimated" {
                    "native_estimate"
                } else {
                    "reported"
                })
            );
        }
        cost_pricing(&db, "hermes-zero-model", "0", "0", "0");
        run_sync(&db, GatewayUsageTool::Hermes, root.path());
        cost_pricing(&db, "hermes-zero-model", "2", "4", "0.2");
        assert_eq!(
            run_sync(&db, GatewayUsageTool::Hermes, root.path()).inserted_records,
            0
        );
        assert_eq!(
            summary_for(&db, GatewayUsageTool::Hermes).total_cost_usd,
            "0.000000",
            "{status}"
        );
        if status == "unknown" {
            source
                .execute(
                    "UPDATE session_model_usage SET input_tokens=2000000,api_call_count=2",
                    [],
                )
                .unwrap();
            run_sync(&db, GatewayUsageTool::Hermes, root.path());
            assert_eq!(
                summary_for(&db, GatewayUsageTool::Hermes).total_cost_usd,
                "2.000000"
            );
            cost_pricing(&db, "hermes-zero-model", "20", "40", "2");
            run_sync(&db, GatewayUsageTool::Hermes, root.path());
            assert_eq!(
                summary_for(&db, GatewayUsageTool::Hermes).total_cost_usd,
                "2.000000"
            );
        }
    }
}

#[test]
#[ignore = "Read-only cost audit for other installed CLIs; writes only database snapshots and in-memory fresh imports"]
fn local_other_cli_costs_on_a_snapshot() {
    let path = dirs::data_dir()
        .unwrap()
        .join("com.ai-toolbox/ai-toolbox.db");
    let source =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let copy = temporary.path().join("other-cli-costs.db");
    source.backup(rusqlite::MAIN_DB, &copy, None).unwrap();
    drop(source);
    let db = SqliteDbState::open(copy).unwrap();
    let mut states = load_states(&db).unwrap();
    for tool in [
        GatewayUsageTool::OpenCode,
        GatewayUsageTool::Pi,
        GatewayUsageTool::OhMyPi,
        GatewayUsageTool::Grok,
        GatewayUsageTool::Gemini,
        GatewayUsageTool::Codex,
    ] {
        let sources = default_session_roots(&db, tool)
            .into_iter()
            .map(|path| (tool, path))
            .collect::<Vec<_>>();
        let before = summary_for(&db, tool);
        let started = std::time::Instant::now();
        let repaired =
            cost_reconciliation::reconcile_session_costs(&db, tool, &sources, &mut states).unwrap();
        let after = summary_for(&db, tool);
        println!(
            "{} cost audit: before={} after={} repaired={} calls={} tokens={} elapsed_ms={}",
            tool.as_str(),
            before.total_cost_usd,
            after.total_cost_usd,
            repaired,
            after.total_requests,
            after.total_tokens,
            started.elapsed().as_millis()
        );
        assert_eq!(after.total_requests, before.total_requests);
        assert_eq!(after.total_tokens, before.total_tokens);
        assert_eq!(
            cost_reconciliation::reconcile_session_costs(&db, tool, &sources, &mut states).unwrap(),
            0
        );
        if matches!(
            tool,
            GatewayUsageTool::OpenCode | GatewayUsageTool::Pi | GatewayUsageTool::OhMyPi
        ) {
            let fresh = SqliteDbState::in_memory_for_test().unwrap();
            fresh
                .with_conn(|conn| {
                    conn.execute("DELETE FROM model_pricing", [])
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                })
                .unwrap();
            for model in pricing::get_model_pricing_list(&db).unwrap() {
                pricing::upsert_model_pricing(&fresh, model).unwrap();
            }
            assert_eq!(
                sync_sources(&fresh, &sources, Utc::now().timestamp())
                    .unwrap()
                    .failed_files,
                0
            );
            let expected = summary_for(&fresh, tool);
            println!(
                "{} fresh import: cost={} calls={} tokens={}",
                tool.as_str(),
                expected.total_cost_usd,
                expected.total_requests,
                expected.total_tokens
            );
            assert_eq!(after.total_requests, expected.total_requests);
            assert_eq!(after.total_tokens, expected.total_tokens);
            assert_eq!(after.total_cost_usd, expected.total_cost_usd);
        }
    }
}
