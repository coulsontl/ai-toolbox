use chrono::Local;
use serde_json::{json, Value};

use super::types::WorkbuddySettingsConfig;

pub fn settings_from_db_value(value: Value) -> WorkbuddySettingsConfig {
    WorkbuddySettingsConfig {
        config_dir: value
            .get("config_dir")
            .and_then(Value::as_str)
            .map(str::to_string),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| Local::now().to_rfc3339()),
    }
}

pub fn settings_to_db_value(config_dir: Option<&str>) -> Value {
    let now = Local::now().to_rfc3339();
    let mut value = json!({ "updated_at": now });
    if let Some(config_dir) = config_dir.filter(|dir| !dir.trim().is_empty()) {
        value["config_dir"] = json!(config_dir);
    }
    value
}
