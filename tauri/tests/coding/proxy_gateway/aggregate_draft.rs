//! Aggregate draft persistence: the site selection the settings page shows while
//! aggregate mode is not engaged.

use ai_toolbox_lib::coding::proxy_gateway::{
    aggregate_draft::load_aggregate_draft,
    aggregate_naming::AggregateNamingMode,
    cli_proxy::save_aggregate_draft,
    paths::ProxyGatewayPaths,
    types::{GatewayAggregateConfig, GatewayCliKey},
};
use ai_toolbox_lib::db::{helpers::db_put, schema::DbTable, SqliteDbState};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn codex_provider_record(name: &str, sort_index: i64) -> Value {
    codex_provider_record_with_models(name, sort_index, &["fixture-model"])
}

fn codex_provider_record_with_models(name: &str, sort_index: i64, models: &[&str]) -> Value {
    let model_catalog = models
        .iter()
        .map(|model| json!({ "model": model }))
        .collect::<Vec<_>>();
    let default_model = models.first().copied().unwrap_or("fixture-model");
    json!({
        "name": name,
        "category": "custom",
        "settings_config": serde_json::to_string(&json!({
            "config": format!("model = \"{default_model}\"\nmodel_provider = \"fixture\"\n[model_providers.fixture]\nbase_url = \"https://api.example.com/v1\"\nwire_api = \"responses\"\n"),
            "auth": {"OPENAI_API_KEY": "fixture-key"},
            "modelCatalog": {"models": model_catalog}
        })).unwrap(),
        "meta": {"apiFormat": "openai_responses", "providerType": "custom"},
        "sort_index": sort_index,
        "is_applied": true,
        "is_disabled": false,
    })
}

fn draft_test_db(providers: &[(&str, &str)]) -> SqliteDbState {
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|connection| {
        db_put(
            connection,
            DbTable::Settings,
            "app",
            &json!({"proxy_mode": "direct"}),
        )?;
        for (index, (id, name)) in providers.iter().enumerate() {
            db_put(
                connection,
                DbTable::CodexProvider,
                id,
                &codex_provider_record(name, index as i64),
            )?;
        }
        Ok::<_, String>(())
    })
    .unwrap();
    db
}

fn draft_test_db_with_models(models: &[&str]) -> SqliteDbState {
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|connection| {
        db_put(
            connection,
            DbTable::Settings,
            "app",
            &json!({"proxy_mode": "direct"}),
        )?;
        db_put(
            connection,
            DbTable::CodexProvider,
            "site-a",
            &codex_provider_record_with_models("Site A", 0, models),
        )?;
        Ok::<_, String>(())
    })
    .unwrap();
    db
}

fn draft(
    provider_ids: &[&str],
    separator: &str,
    aliases: &[(&str, &str)],
) -> GatewayAggregateConfig {
    GatewayAggregateConfig {
        provider_ids: provider_ids.iter().map(|id| (*id).to_string()).collect(),
        separator: separator.to_string(),
        aliases: aliases
            .iter()
            .map(|(site_id, alias)| ((*site_id).to_string(), (*alias).to_string()))
            .collect::<BTreeMap<_, _>>(),
        naming: AggregateNamingMode::ModelAtSite,
        // Deliberately non-default so the round trip proves the new field is
        // persisted instead of silently collapsing back to `false`.
        cross_site_failover: true,
        // Deliberately non-default so the round trip proves the field is
        // persisted instead of silently collapsing to "expose everything".
        subagent_exposed_models: vec!["fixture-model".to_string()],
        subagent: None,
    }
}

#[tokio::test]
async fn draft_round_trips_and_rejects_an_empty_selection() {
    let directory = tempfile::tempdir().unwrap();
    let paths = ProxyGatewayPaths::new(directory.path());
    let db = draft_test_db(&[("site-a", "Site A"), ("site-b", "Site B")]);

    assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);

    let saved = save_aggregate_draft(
        &db,
        &paths,
        GatewayCliKey::Codex,
        draft(&[" site-a ", "site-b"], " | ", &[(" site-a ", " a ")]),
    )
    .await
    .unwrap();

    assert_eq!(
        saved,
        GatewayAggregateConfig {
            provider_ids: vec!["site-a".to_string(), "site-b".to_string()],
            separator: "|".to_string(),
            aliases: BTreeMap::from([("site-a".to_string(), "a".to_string())]),
            naming: AggregateNamingMode::ModelAtSite,
            cross_site_failover: true,
            subagent_exposed_models: vec!["fixture-model".to_string()],
            subagent: None,
        }
    );
    // What the settings page reads back is what the gateway accepted.
    assert_eq!(
        load_aggregate_draft(&paths, GatewayCliKey::Codex),
        Some(saved.clone())
    );

    // Aggregate mode cannot represent "no site", so an empty selection is refused
    // and the previously stored draft stays intact.
    let error = save_aggregate_draft(&db, &paths, GatewayCliKey::Codex, draft(&["   "], ".", &[]))
        .await
        .unwrap_err();
    assert!(error.contains("at least one site"), "{error}");
    assert_eq!(
        load_aggregate_draft(&paths, GatewayCliKey::Codex),
        Some(saved)
    );
}

#[tokio::test]
async fn draft_rejects_exposed_model_not_declared_by_selected_sites() {
    let directory = tempfile::tempdir().unwrap();
    let paths = ProxyGatewayPaths::new(directory.path());
    let db = draft_test_db(&[("site-a", "Site A")]);
    let mut config = draft(&["site-a"], ".", &[]);
    config.subagent_exposed_models = vec!["not-declared".to_string()];

    let error = save_aggregate_draft(&db, &paths, GatewayCliKey::Codex, config)
        .await
        .unwrap_err();

    assert!(error.contains("declared by the selected sites"), "{error}");
    assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);
}

#[tokio::test]
async fn draft_rejects_more_than_five_declared_exposed_models() {
    let directory = tempfile::tempdir().unwrap();
    let paths = ProxyGatewayPaths::new(directory.path());
    let models = [
        "model-1", "model-2", "model-3", "model-4", "model-5", "model-6",
    ];
    let db = draft_test_db_with_models(&models);
    let mut config = draft(&["site-a"], ".", &[]);
    config.subagent_exposed_models = models.iter().map(|model| (*model).to_string()).collect();

    let error = save_aggregate_draft(&db, &paths, GatewayCliKey::Codex, config)
        .await
        .unwrap_err();

    assert!(error.contains("At most 5"), "{error}");
    assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);
}

#[tokio::test]
async fn draft_rejects_mixed_valid_and_stale_models_even_when_valid_models_fit_the_limit() {
    let directory = tempfile::tempdir().unwrap();
    let paths = ProxyGatewayPaths::new(directory.path());
    let declared = ["model-1", "model-2", "model-3", "model-4", "model-5"];
    let db = draft_test_db_with_models(&declared);
    let mut config = draft(&["site-a"], ".", &[]);
    config.subagent_exposed_models = [
        "model-1",
        "stale-model",
        "model-2",
        "model-3",
        "model-4",
        "model-5",
    ]
    .iter()
    .map(|model| (*model).to_string())
    .collect();

    let error = save_aggregate_draft(&db, &paths, GatewayCliKey::Codex, config)
        .await
        .unwrap_err();

    assert!(error.contains("stale-model"), "{error}");
    assert!(
        error.contains("not declared by the selected sites"),
        "{error}"
    );
    assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);
}

#[tokio::test]
async fn draft_rejects_multiple_stale_models_mixed_with_valid_models() {
    let directory = tempfile::tempdir().unwrap();
    let paths = ProxyGatewayPaths::new(directory.path());
    let declared = ["model-1", "model-2", "model-3", "model-4"];
    let db = draft_test_db_with_models(&declared);
    let mut config = draft(&["site-a"], ".", &[]);
    config.subagent_exposed_models = [
        "stale-model-a",
        "model-1",
        "model-2",
        "stale-model-b",
        "model-3",
        "model-4",
    ]
    .iter()
    .map(|model| (*model).to_string())
    .collect();

    let error = save_aggregate_draft(&db, &paths, GatewayCliKey::Codex, config)
        .await
        .unwrap_err();

    assert!(
        error.contains("stale-model-a") || error.contains("stale-model-b"),
        "{error}"
    );
    assert!(
        error.contains("not declared by the selected sites"),
        "{error}"
    );
    assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);
}

#[tokio::test]
async fn draft_rejects_unavailable_sites_and_ambiguous_prefixes() {
    let directory = tempfile::tempdir().unwrap();
    let paths = ProxyGatewayPaths::new(directory.path());
    let db = draft_test_db(&[("site-a", "Site A"), ("site-b", "Site B")]);

    // A site the gateway cannot route right now must never be remembered.
    let error = save_aggregate_draft(
        &db,
        &paths,
        GatewayCliKey::Codex,
        draft(&["site-a", "missing"], ".", &[]),
    )
    .await
    .unwrap_err();
    assert!(error.contains("'missing' is not available"), "{error}");

    // Aliases only address selected sites.
    let error = save_aggregate_draft(
        &db,
        &paths,
        GatewayCliKey::Codex,
        draft(&["site-a"], ".", &[("site-b", "b")]),
    )
    .await
    .unwrap_err();
    assert!(error.contains("unselected site"), "{error}");

    // An alias may not shadow an unselected site's provider id: that site stays
    // addressable as a fallback, so the prefix would be ambiguous.
    let error = save_aggregate_draft(
        &db,
        &paths,
        GatewayCliKey::Codex,
        draft(&["site-a"], ".", &[("site-a", "site-b")]),
    )
    .await
    .unwrap_err();
    assert!(error.contains("addresses both"), "{error}");

    // The separator must stay splittable from the model name.
    let error = save_aggregate_draft(
        &db,
        &paths,
        GatewayCliKey::Codex,
        draft(&["site-a"], "a", &[]),
    )
    .await
    .unwrap_err();
    assert!(error.contains("separator"), "{error}");

    // Drafts exist for aggregate mode only, which is Codex-only today.
    let error = save_aggregate_draft(
        &db,
        &paths,
        GatewayCliKey::Claude,
        draft(&["site-a"], ".", &[]),
    )
    .await
    .unwrap_err();
    assert!(error.contains("Codex only"), "{error}");

    assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);
}
