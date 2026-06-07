use super::SessionMessageBlock;

pub(super) fn normalize_tool_name(raw_tool_name: &str) -> String {
    let normalized = raw_tool_name
        .trim()
        .trim_start_matches("functions.")
        .to_ascii_lowercase()
        .replace(['-', ' ', '.'], "_");

    if normalized.is_empty() {
        return "unknown".to_string();
    }

    if normalized.starts_with("mcp__")
        || normalized.starts_with("mcp_")
        || normalized.contains("__mcp__")
        || normalized.contains("server_tool")
    {
        return "mcp".to_string();
    }

    if matches!(
        normalized.as_str(),
        "bash" | "shell" | "terminal" | "command" | "execute" | "execute_command" | "run_command"
    ) || normalized.contains("shell")
        || normalized.contains("terminal")
        || normalized.contains("execute_command")
    {
        return "bash".to_string();
    }

    if matches!(
        normalized.as_str(),
        "read" | "read_file" | "file_read" | "view_file" | "open_file"
    ) {
        return "read".to_string();
    }

    if matches!(
        normalized.as_str(),
        "write" | "write_file" | "create_file" | "file_write"
    ) {
        return "write".to_string();
    }

    if matches!(
        normalized.as_str(),
        "multiedit" | "multi_edit" | "batch_edit" | "multi_file_edit"
    ) {
        return "multi_edit".to_string();
    }

    if matches!(
        normalized.as_str(),
        "edit" | "edit_file" | "file_edit" | "replace_in_file"
    ) {
        return "edit".to_string();
    }

    if matches!(
        normalized.as_str(),
        "apply_patch" | "patch" | "file_patch" | "applypatch"
    ) {
        return "apply_patch".to_string();
    }

    if matches!(
        normalized.as_str(),
        "notebookedit" | "notebook_edit" | "edit_notebook"
    ) {
        return "notebook_edit".to_string();
    }

    if matches!(
        normalized.as_str(),
        "grep" | "rg" | "search_text" | "text_search"
    ) {
        return "grep".to_string();
    }

    if matches!(
        normalized.as_str(),
        "glob" | "file_glob" | "find_files" | "folder_search"
    ) {
        return "glob".to_string();
    }

    if matches!(
        normalized.as_str(),
        "webfetch" | "web_fetch" | "fetch_url" | "browser_fetch"
    ) {
        return "web_fetch".to_string();
    }

    if matches!(
        normalized.as_str(),
        "websearch" | "web_search" | "search_web" | "browser_search"
    ) {
        return "web_search".to_string();
    }

    if matches!(
        normalized.as_str(),
        "todowrite" | "todo_write" | "todo" | "write_todos"
    ) {
        return "todo_write".to_string();
    }

    if matches!(normalized.as_str(), "update_plan" | "updateplan" | "plan") {
        return "update_plan".to_string();
    }

    if matches!(
        normalized.as_str(),
        "task" | "subagent_task" | "task_create" | "task_update" | "task_output"
    ) {
        return "task".to_string();
    }

    if matches!(
        normalized.as_str(),
        "agent" | "subagent" | "delegate" | "create_agent"
    ) {
        return "agent".to_string();
    }

    "unknown".to_string()
}

pub(super) fn infer_tool_variant(normalized_tool_name: &str, raw_tool_name: &str) -> String {
    match normalized_tool_name {
        "bash" => "terminal",
        "read" | "write" | "edit" | "multi_edit" | "apply_patch" | "notebook_edit" => "code",
        "grep" => "search",
        "glob" => "file",
        "web_fetch" | "web_search" => "web",
        "todo_write" | "update_plan" | "task" | "agent" => "task",
        "mcp" => "mcp",
        _ if raw_tool_name.to_ascii_lowercase().contains("mcp") => "mcp",
        _ => "neutral",
    }
    .to_string()
}

pub(super) fn infer_tool_status(block: &SessionMessageBlock) -> String {
    if block.is_error == Some(true) {
        return "error".to_string();
    }

    if let Some(status) = block.status.as_deref() {
        let normalized_status = status.to_ascii_lowercase();
        if matches!(
            normalized_status.as_str(),
            "error" | "failed" | "failure" | "interrupted"
        ) {
            return "error".to_string();
        }
        if matches!(normalized_status.as_str(), "warning" | "warn") {
            return "warning".to_string();
        }
        if matches!(normalized_status.as_str(), "pending" | "running") {
            return "pending".to_string();
        }
        if matches!(normalized_status.as_str(), "success" | "ok" | "completed") {
            return "success".to_string();
        }
    }

    if block.output.is_none() {
        return "pending".to_string();
    }

    "success".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_tool_name_maps_common_aliases() {
        assert_eq!(normalize_tool_name("Bash"), "bash");
        assert_eq!(normalize_tool_name("execute_command"), "bash");
        assert_eq!(normalize_tool_name("Read"), "read");
        assert_eq!(normalize_tool_name("read_file"), "read");
        assert_eq!(normalize_tool_name("MultiEdit"), "multi_edit");
        assert_eq!(normalize_tool_name("web_search"), "web_search");
        assert_eq!(normalize_tool_name("mcp__server__tool"), "mcp");
    }

    #[test]
    fn infer_tool_variant_uses_normalized_name() {
        assert_eq!(infer_tool_variant("bash", "Bash"), "terminal");
        assert_eq!(infer_tool_variant("grep", "Grep"), "search");
        assert_eq!(infer_tool_variant("todo_write", "TodoWrite"), "task");
        assert_eq!(infer_tool_variant("unknown", "custom"), "neutral");
    }
}
