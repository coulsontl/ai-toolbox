use std::fs;
use std::path::{Path, PathBuf};

use ai_toolbox_lib::coding::{
    deeplink, dsh, hermes, oh_my_pi, open_claw, open_code, pi, runtime_location,
};
use ai_toolbox_lib::db::helpers::{db_get, db_list, db_put};
use ai_toolbox_lib::db::schema::DbTable;
use ai_toolbox_lib::db::SqliteDbState;
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::Manager;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn app() -> tauri::App<MockRuntime> {
    mock_builder()
        .manage(SqliteDbState::in_memory_for_test().unwrap())
        .build(mock_context(noop_assets()))
        .unwrap()
}

fn request(target: &str) -> deeplink::DeepLinkImportRequest {
    serde_json::from_value(json!({
        "resource": "provider", "app": target, "sourceApp": "opencode",
        "name": "Shared Relay", "category": "custom", "apiKey": "test-shared-key",
        "baseUrl": "https://relay.test/v1", "apiFormat": "openai_chat",
        "model": "vendor/model-b", "sourceProviderId": "share:opencode:relay",
        "models": [
            { "id": "vendor/model-a", "name": "Model A", "contextWindow": 64000, "maxTokens": 8000, "input": ["text", "image"], "reasoning": true },
            { "id": "vendor/model-b", "name": "Model B", "contextWindow": 128000 }
        ],
        "rawUrl": ""
    })).unwrap()
}

async fn import(
    app: &tauri::App<MockRuntime>,
    request: deeplink::DeepLinkImportRequest,
    policy: deeplink::ImportConflictPolicy,
) -> deeplink::DeepLinkImportResult {
    deeplink::import_from_deeplink_unified(app.state(), app.handle().clone(), request, Some(policy))
        .await
        .unwrap()
}

fn row(app: &tauri::App<MockRuntime>, table: DbTable, id: &str) -> Value {
    app.state::<SqliteDbState>()
        .with_conn(|conn| db_get(conn, table, id))
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn database_targets_import_read_back_skip_and_copy_without_activating() {
    let _guard = TEST_LOCK.lock().await;
    let app = app();
    for (target, table) in [
        ("claude", DbTable::ClaudeProvider),
        ("claudedesktop", DbTable::ClaudeDesktopProvider),
        ("codex", DbTable::CodexProvider),
        ("grok", DbTable::GrokProvider),
        ("kimi", DbTable::KimiProvider),
        ("gemini", DbTable::GeminiCliProvider),
    ] {
        let imported = import(&app, request(target), deeplink::ImportConflictPolicy::Skip).await;
        assert_eq!(imported.status, "created", "{target}");
        assert_eq!(
            imported.requires_gateway,
            !matches!(target, "grok" | "kimi"),
            "{target}"
        );
        let saved = row(&app, table, &imported.id);
        assert_eq!(saved["is_applied"], false, "{target}");
        assert_eq!(
            saved["source_provider_id"], "share:opencode:relay",
            "{target}"
        );
        assert_eq!(saved["meta"]["apiFormat"], "openai_chat", "{target}");
        let settings: Value =
            serde_json::from_str(saved["settings_config"].as_str().unwrap()).unwrap();
        match target {
            "claude" | "claudedesktop" => {
                assert_eq!(settings["env"]["ANTHROPIC_AUTH_TOKEN"], "test-shared-key");
                assert_eq!(
                    settings["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
                    "vendor/model-b"
                );
                if target == "claudedesktop" {
                    assert_eq!(
                        saved["meta"]["claudeDesktopModelRoutes"]
                            .as_object()
                            .unwrap()
                            .len(),
                        2
                    );
                    assert!(
                        ai_toolbox_lib::coding::claude_desktop::config_writer::has_routing_models(
                            Some(&saved["meta"]),
                            Some(&settings)
                        )
                    );
                }
            }
            "codex" => {
                let config: toml::Value =
                    toml::from_str(settings["config"].as_str().unwrap()).unwrap();
                assert_eq!(config["model"].as_str(), Some("vendor/model-b"));
                assert_eq!(
                    settings["modelCatalog"]["models"].as_array().unwrap().len(),
                    2
                );
            }
            "grok" | "kimi" => {
                assert_eq!(settings["auth"]["API_KEY"], "test-shared-key");
                assert_eq!(settings["defaultModelKey"], "model-2");
                assert_eq!(
                    settings["modelCatalog"]["models"][1]["model"],
                    "vendor/model-b"
                );
                if target == "kimi" {
                    assert_eq!(
                        settings["providerConfigs"]["custom"]["type"],
                        "openai_legacy"
                    );
                    assert_eq!(
                        settings["modelCatalog"]["models"][0]["capabilities"],
                        json!(["image_in", "thinking"])
                    );
                }
            }
            "gemini" => assert_eq!(settings["env"]["GEMINI_MODEL"], "vendor/model-b"),
            _ => unreachable!(),
        }
        let skipped = import(&app, request(target), deeplink::ImportConflictPolicy::Skip).await;
        assert_eq!(skipped.status, "skipped", "{target}");
        assert_eq!(skipped.id, imported.id, "{target}");
        assert_eq!(
            row(&app, table, &imported.id),
            saved,
            "skip must not update {target}"
        );
        let copied = import(&app, request(target), deeplink::ImportConflictPolicy::Copy).await;
        assert_eq!(copied.name, "Shared Relay (2)", "{target}");
        assert_ne!(copied.id, imported.id, "{target}");
        assert_eq!(
            app.state::<SqliteDbState>()
                .with_conn(|conn| db_list(conn, table, None))
                .unwrap()
                .len(),
            2,
            "{target}"
        );
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn write_yaml(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_yaml::to_string(value).unwrap()).unwrap();
}

fn read_config(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    if matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("yaml" | "yml")
    ) {
        serde_yaml::from_str(&text).unwrap()
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

async fn configure_native(app: &tauri::App<MockRuntime>, target: &str, root: &Path) -> PathBuf {
    let (table, field, config_name) = match target {
        "opencode" => (
            DbTable::OpenCodeCommonConfig,
            "config_path",
            "opencode.json",
        ),
        "openclaw" => (
            DbTable::OpenClawCommonConfig,
            "config_path",
            "openclaw.json",
        ),
        "pi" => (DbTable::PiSettingsConfig, "root_dir", "models.json"),
        "omp" => (DbTable::OhMyPiSettingsConfig, "root_dir", "models.yml"),
        "hermes" => (DbTable::HermesSettingsConfig, "config_dir", "config.yaml"),
        "dsh" => (DbTable::DshSettingsConfig, "config_dir", "settings.yaml"),
        _ => unreachable!(),
    };
    let config_path = root.join(config_name);
    let location: &Path = if field == "config_path" {
        &config_path
    } else {
        root
    };
    let mut settings = json!({});
    settings[field] = json!(location.to_string_lossy());
    let state = app.state::<SqliteDbState>();
    state
        .with_conn(|conn| db_put(conn, table, "common", &settings))
        .unwrap();
    runtime_location::refresh_runtime_location_cache_for_module_async(
        &state,
        if target == "omp" { "oh_my_pi" } else { target },
    )
    .await
    .unwrap();
    let resolved = match target {
        "opencode" => PathBuf::from(
            open_code::get_opencode_config_path(app.state())
                .await
                .unwrap(),
        ),
        "openclaw" => PathBuf::from(
            open_claw::get_openclaw_config_path(app.state())
                .await
                .unwrap(),
        ),
        "pi" => pi::get_pi_models_path_async(&state).await.unwrap(),
        "omp" => oh_my_pi::get_omp_models_path_async(&state).await.unwrap(),
        "hermes" => hermes::get_hermes_config_path_async(&state).await.unwrap(),
        "dsh" => dsh::get_dsh_config_path_async(&state).await.unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(
        resolved, config_path,
        "all writes must stay in the test directory"
    );
    config_path
}

fn imported_native_provider(target: &str, value: &Value, id: &str) -> Value {
    match target {
        "opencode" => value["provider"][id].clone(),
        "openclaw" => value["models"]["providers"][id].clone(),
        "pi" | "omp" => value["providers"][id].clone(),
        "hermes" => value["custom_providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["name"] == id)
            .unwrap()
            .clone(),
        "dsh" => value["llm-pi-ai"]["providers"][id].clone(),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn runtime_targets_preserve_existing_providers_defaults_and_unknown_fields() {
    let _guard = TEST_LOCK.lock().await;
    let app = app();
    let temp = tempfile::tempdir().unwrap();
    for target in ["opencode", "openclaw", "pi", "omp", "hermes", "dsh"] {
        let root = temp.path().join(target);
        let config_path = configure_native(&app, target, &root).await;
        let keep = json!({ "api": "openai-completions", "baseUrl": "https://keep.test/v1", "models": [{ "id": "old-model" }], "future": { "enabled": true } });
        let mut before = match target {
            "opencode" => {
                json!({ "model": "keep/old-model", "small_model": "keep/old-model", "agent": { "build": { "model": "keep/old-model", "variant": "high" } }, "provider": { "keep": { "name": "keep", "npm": "@ai-sdk/openai", "models": { "old-model": {} } } } })
            }
            "openclaw" => {
                json!({ "agents": { "defaults": { "model": { "primary": "keep/old-model" } } }, "models": { "providers": { "keep": keep } } })
            }
            "pi" | "omp" => json!({ "providers": { "keep": keep } }),
            "hermes" => {
                json!({ "model": { "provider": "keep", "default": "old-model" }, "custom_providers": [{ "name": "keep", "api_mode": "openai", "base_url": "https://keep.test/v1", "models": { "old-model": {} } }] })
            }
            "dsh" => {
                json!({ "agent-default-model": { "provider": "keep", "model": "old-model" }, "llm-pi-ai": { "providers": { "keep": { "api": "openai-completions", "apiKeyEnv": "OLD_KEY", "models": [{ "id": "old-model" }] } } } })
            }
            _ => unreachable!(),
        };
        before["unknownSection"] = json!({ "preserve": [1, 2, 3] });
        if matches!(target, "omp" | "hermes" | "dsh") {
            write_yaml(&config_path, &before);
        } else {
            write_json(&config_path, &before);
        }
        let sidecar = match target {
            "pi" => Some(root.join("settings.json")),
            "omp" => Some(root.join("config.yml")),
            _ => None,
        };
        if let Some(path) = &sidecar {
            let config = json!({ "defaultProvider": "keep", "defaultModel": "old-model", "model": "keep/old-model", "theme": "dark" });
            if target == "pi" {
                write_json(path, &config);
            } else {
                write_yaml(path, &config);
            }
        }
        let sidecar_before = sidecar.as_ref().map(|path| fs::read(path).unwrap());
        if target == "dsh" {
            write_yaml(
                &root.join(".credentials.yaml"),
                &json!({ "version": 1, "refs": { "OLD_KEY": "keep-key" }, "records": { "llm-pi-ai/keep": { "kind": "grant", "payload": { "access_token": "keep-oauth" } } } }),
            );
        }

        let imported = import(&app, request(target), deeplink::ImportConflictPolicy::Skip).await;
        assert_eq!(imported.status, "created", "{target}");
        assert!(!imported.requires_gateway, "{target}");
        let after = read_config(&config_path);
        assert_eq!(
            after["unknownSection"], before["unknownSection"],
            "{target}"
        );
        assert_eq!(
            imported_native_provider(target, &after, "keep"),
            imported_native_provider(target, &before, "keep"),
            "{target}"
        );
        for field in [
            "model",
            "small_model",
            "agent",
            "agents",
            "agent-default-model",
        ] {
            assert_eq!(after[field], before[field], "{target}: {field}");
        }
        if let Some(path) = &sidecar {
            assert_eq!(Some(fs::read(path).unwrap()), sidecar_before, "{target}");
        }
        let added = imported_native_provider(target, &after, &imported.id);
        assert!(added.is_object(), "{target}");
        match target {
            "opencode" => {
                assert_eq!(added["npm"], "@ai-sdk/openai-compatible");
                assert_eq!(added["options"]["apiKey"], "test-shared-key");
                assert_eq!(added["models"]["vendor/model-a"]["limit"]["output"], 8000);
            }
            "hermes" => {
                assert_eq!(added["api_key"], "test-shared-key");
                assert!(added["models"].get("vendor/model-a").is_some());
                assert!(hermes::read_hermes_runtime_config(app.state())
                    .await
                    .unwrap()
                    .providers
                    .iter()
                    .any(|provider| provider.provider_key == imported.id));
            }
            "dsh" => {
                assert!(added.get("apiKey").is_none());
                let credentials = read_config(&root.join(".credentials.yaml"));
                assert_eq!(
                    credentials["refs"][added["apiKeyEnv"].as_str().unwrap()],
                    "test-shared-key"
                );
                assert_eq!(credentials["refs"]["OLD_KEY"], "keep-key");
                assert_eq!(
                    credentials["records"]["llm-pi-ai/keep"]["payload"]["access_token"],
                    "keep-oauth"
                );
                let runtime = dsh::read_dsh_runtime_config(app.state()).await.unwrap();
                assert_eq!(
                    runtime
                        .providers
                        .iter()
                        .find(|provider| provider.provider_key == imported.id)
                        .unwrap()
                        .api_key,
                    "test-shared-key"
                );
            }
            _ => assert_eq!(added["apiKey"], "test-shared-key"),
        }
        if target == "pi" {
            assert!(pi::read_pi_runtime_config(app.state())
                .await
                .unwrap()
                .providers
                .iter()
                .any(|provider| provider.provider_key == imported.id));
        }
        if target == "omp" {
            assert!(oh_my_pi::read_omp_runtime_config(app.state())
                .await
                .unwrap()
                .providers
                .iter()
                .any(|provider| provider.provider_key == imported.id));
        }
        let bytes = fs::read(&config_path).unwrap();
        let skipped = import(&app, request(target), deeplink::ImportConflictPolicy::Skip).await;
        assert_eq!(skipped.status, "skipped", "{target}");
        assert_eq!(
            fs::read(&config_path).unwrap(),
            bytes,
            "{target}: skip does not write"
        );
        let copied = import(&app, request(target), deeplink::ImportConflictPolicy::Copy).await;
        assert_ne!(copied.id, imported.id, "{target}");
        assert_eq!(
            imported_native_provider(target, &read_config(&config_path), &imported.id),
            added,
            "{target}: copy does not overwrite"
        );
    }
}

#[tokio::test]
async fn malformed_runtime_files_are_rejected_without_overwriting_them() {
    let _guard = TEST_LOCK.lock().await;
    let app = app();
    let temp = tempfile::tempdir().unwrap();
    for target in ["opencode", "openclaw", "pi", "omp", "hermes", "dsh"] {
        let path = configure_native(&app, target, &temp.path().join(target)).await;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let malformed = b"{ definitely: [invalid";
        fs::write(&path, malformed).unwrap();
        assert!(
            deeplink::import_from_deeplink_unified(
                app.state(),
                app.handle().clone(),
                request(target),
                None
            )
            .await
            .is_err(),
            "{target}"
        );
        assert_eq!(fs::read(path).unwrap(), malformed, "{target}");
    }
}

#[tokio::test]
async fn sharing_an_unconfigured_builtin_does_not_replace_the_active_builtin() {
    let _guard = TEST_LOCK.lock().await;
    let app = app();
    let temp = tempfile::tempdir().unwrap();
    let models_path = configure_native(&app, "pi", temp.path()).await;
    let settings_path = temp.path().join("settings.json");
    write_json(
        &settings_path,
        &json!({ "defaultProvider": "openai", "defaultModel": "gpt-old" }),
    );
    let before = fs::read(&settings_path).unwrap();
    let mut input = request("pi");
    input.name = "OpenAI".to_string();
    let result = import(&app, input, deeplink::ImportConflictPolicy::Skip).await;
    assert_eq!(result.status, "created");
    assert_eq!(result.id, "openai-shared");
    assert!(read_config(&models_path)["providers"]
        .get("openai")
        .is_none());
    assert_eq!(fs::read(settings_path).unwrap(), before);
}
