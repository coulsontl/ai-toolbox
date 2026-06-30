use serde_json::Value;

pub(crate) fn extract_error_message(error: &Value) -> Option<String> {
    if let Some(message) = error.as_str().filter(|message| !message.trim().is_empty()) {
        return Some(message.to_string());
    }

    for pointer in [
        "/error/message",
        "/message",
        "/detail",
        "/msg",
        "/status_msg",
        "/base_resp/status_msg",
    ] {
        if let Some(message) = error
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
        {
            return Some(message.to_string());
        }
    }

    if let Some(message) = error
        .get("error")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
    {
        return Some(message.to_string());
    }

    if !error.is_null() {
        return Some(error.to_string());
    }

    None
}
