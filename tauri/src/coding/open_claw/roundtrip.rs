//! OpenClaw 配置文件 JSON5 round-trip 读写引擎。
//!
//! 以 `json-five` 的 round-trip AST 为基础,保留文件中未改动部分的注释与格式
//! (整份 `serde_json` 重建会丢失这些内容)。设计对齐 cc-switch 的
//! `src-tauri/src/openclaw_config.rs`,但本模块是纯函数:显式接收
//! `path` / `backup_dir` / `retain_count`,不解析全局路径,便于单测。
//!
//! 关键约束:新增/替换的节值一律先经 `serde_json::to_string_pretty` 序列化,
//! 再重新解析进 round-trip AST —— 绝不手写 `RtJSONValue` 节点,
//! 否则 `json-five 0.3.1` 在空嵌套 map/array 上打印时会 panic。

use super::types::OpenClawHealthWarning;
use chrono::Local;
use json_five::rt::parser::{
    from_str as rt_from_str, JSONKeyValuePair as RtJSONKeyValuePair,
    JSONObjectContext as RtJSONObjectContext, JSONText as RtJSONText, JSONValue as RtJSONValue,
    KeyValuePairContext as RtKeyValuePairContext,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// OpenClaw 上游 `tools.profile` 合法枚举。
pub const OPENCLAW_TOOLS_PROFILES: &[&str] = &["minimal", "coding", "messaging", "full"];

/// 文件不存在时用于初始化 round-trip 文档的默认源码(JSON5)。
pub const OPENCLAW_DEFAULT_SOURCE: &str =
    "{\n  models: {\n    mode: 'merge',\n    providers: {},\n  },\n}\n";

/// 写路径全局排他锁(保证并发托盘/窗口保存串行化)。
pub fn openclaw_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// 一个待写回的 OpenClaw 配置文档。
///
/// `original_source` 记录构造时的磁盘原文,`save` 时用于检测外部并发修改。
pub struct OpenClawConfigDocument {
    path: PathBuf,
    pub(crate) original_source: Option<String>,
    text: RtJSONText,
}

impl OpenClawConfigDocument {
    /// 从 `path` 读取原文(文件不存在则用默认源)并解析为 round-trip AST。
    pub fn load(path: &Path) -> Result<Self, String> {
        let original_source =
            if path.exists() {
                Some(fs::read_to_string(path).map_err(|e| {
                    format!("Failed to read OpenClaw config '{}': {e}", path.display())
                })?)
            } else {
                None
            };

        let source = original_source
            .clone()
            .unwrap_or_else(|| OPENCLAW_DEFAULT_SOURCE.to_string());
        let text = rt_from_str(&source).map_err(|e| {
            format!(
                "Failed to parse OpenClaw config as JSON5: {} (line {})",
                e.message, e.lineno
            )
        })?;

        Ok(Self {
            path: path.to_path_buf(),
            original_source,
            text,
        })
    }

    /// 将根节点 `key` 的节值替换(或新增)为 `value`,保留其他节注释/格式。
    pub fn set_root_section(&mut self, key: &str, value: &Value) -> Result<(), String> {
        let RtJSONValue::JSONObject {
            key_value_pairs,
            context,
        } = &mut self.text.value
        else {
            return Err("OpenClaw config root must be a JSON5 object".to_string());
        };

        // 空对象时给出基础缩进,使新增节多行排布。
        if key_value_pairs.is_empty()
            && context
                .as_ref()
                .map(|ctx| ctx.wsc.0.is_empty())
                .unwrap_or(true)
        {
            *context = Some(RtJSONObjectContext {
                wsc: ("\n  ".to_string(),),
            });
        }

        let leading_ws = context
            .as_ref()
            .map(|ctx| ctx.wsc.0.clone())
            .unwrap_or_default();
        let entry_separator_ws = derive_entry_separator(&leading_ws);
        let child_indent = extract_trailing_indent(&leading_ws);
        let new_value = value_to_rt_value(value, &child_indent)?;

        // 已存在该节:仅替换值,保留原有 key/格式信息。
        if let Some(existing) = key_value_pairs
            .iter_mut()
            .find(|pair| json5_key_name(&pair.key) == Some(key))
        {
            existing.value = new_value;
            return Ok(());
        }

        // 新增节:挂在最后一个 pair 后面,吸收其尾随分隔符/结尾空白。
        let new_pair = if let Some(last_pair) = key_value_pairs.last_mut() {
            let last_ctx = ensure_kvp_context(last_pair);
            let closing_ws = if let Some(after_comma) = last_ctx.wsc.3.clone() {
                last_ctx.wsc.3 = Some(entry_separator_ws.clone());
                after_comma
            } else {
                let closing_ws = std::mem::take(&mut last_ctx.wsc.2);
                last_ctx.wsc.3 = Some(entry_separator_ws.clone());
                closing_ws
            };
            make_root_pair(key, new_value, closing_ws)
        } else {
            make_root_pair(
                key,
                new_value,
                derive_closing_ws_from_separator(&leading_ws),
            )
        };

        key_value_pairs.push(new_pair);
        Ok(())
    }

    /// 删除根节点 `key` 的节,合并分隔符空白以保持文档完整。
    ///
    /// 覆盖首/中/尾三种位置;若空白合并过于复杂可退回整块重渲,
    /// 代价是丢失被改区外的根级注释。
    pub fn remove_root_section(&mut self, key: &str) -> Result<bool, String> {
        let RtJSONValue::JSONObject {
            key_value_pairs, ..
        } = &mut self.text.value
        else {
            return Err("OpenClaw config root must be a JSON5 object".to_string());
        };

        let Some(index) = key_value_pairs
            .iter()
            .position(|pair| json5_key_name(&pair.key) == Some(key))
        else {
            return Ok(false);
        };

        let mut removed = key_value_pairs.remove(index);
        let was_last = index == key_value_pairs.len();

        // 拼接分隔符:让前一个 pair 的逗号/结尾空白衔接上被删节的位置。
        if let Some(prev) = key_value_pairs.get_mut(index.saturating_sub(1)) {
            let prev_ctx = ensure_kvp_context(prev);
            let removed_ctx = removed
                .context
                .get_or_insert_with(|| RtKeyValuePairContext {
                    wsc: (String::new(), String::new(), String::new(), None),
                });
            if was_last {
                match removed_ctx.wsc.3.clone() {
                    // 末尾带尾随逗号:保留,新的末位节直接沿用其结尾空白。
                    Some(trailing) => prev_ctx.wsc.3 = Some(trailing),
                    // 无尾随逗号:吸收被删节到 `}` 前的空白,并移除前一个逗号。
                    None => {
                        prev_ctx.wsc.2 = std::mem::take(&mut removed_ctx.wsc.2);
                        prev_ctx.wsc.3 = None;
                    }
                }
            } else {
                // 中间节:其逗号+分隔符继续充当前一个逗号到下一个节的空白。
                prev_ctx.wsc.3 = removed_ctx.wsc.3.clone();
            }
        }

        Ok(true)
    }

    /// 对比 `old_value` 与 `new_value` 两个根对象,仅对变化的顶层 key 执行
    /// 替换/删除,其余节原样保留。
    ///
    /// 变化判定比较**序列化文本**而非 `Value` 相等:本 crate 的 `serde_json`
    /// 开了 `preserve_order`,其 map 比较与 key 顺序无关,于是「只把
    /// `models.providers` 的两个键换了个位置」会被判成没变化——拖拽排序明明
    /// 落到了前端对象里,写盘却被跳过,重新加载后又变回原顺序。
    pub fn apply_root_section_diff(
        &mut self,
        old_value: &Value,
        new_value: &Value,
    ) -> Result<(), String> {
        let old_obj = old_value.as_object();
        let new_obj = new_value
            .as_object()
            .ok_or_else(|| "OpenClaw config root must serialize to a JSON5 object".to_string())?;

        for (key, new_section) in new_obj {
            let changed = match old_obj.and_then(|obj| obj.get(key)) {
                Some(old_section) => {
                    serde_json::to_string(old_section).map_err(|error| {
                        format!("Failed to serialize current '{key}' section: {error}")
                    })? != serde_json::to_string(new_section).map_err(|error| {
                        format!("Failed to serialize new '{key}' section: {error}")
                    })?
                }
                None => true,
            };
            if changed {
                self.set_root_section(key, new_section)?;
            }
        }

        if let Some(old_obj) = old_obj {
            for key in old_obj.keys() {
                if !new_obj.contains_key(key) {
                    self.remove_root_section(key)?;
                }
            }
        }

        Ok(())
    }

    /// 持久化:排他锁 → 冲突检测 → 无变化跳过 → 备份 + 原子写 → 健康扫描。
    pub fn save(
        self,
        backup_dir: &Path,
        retain_count: usize,
    ) -> Result<super::types::OpenClawWriteOutcome, String> {
        let _guard = openclaw_write_lock()
            .lock()
            .map_err(|e| format!("Failed to lock OpenClaw config write: {e}"))?;

        let current_source = if self.path.exists() {
            Some(fs::read_to_string(&self.path).map_err(|e| {
                format!(
                    "Failed to read OpenClaw config '{}': {e}",
                    self.path.display()
                )
            })?)
        } else {
            None
        };

        if current_source != self.original_source {
            return Err(
                "OpenClaw config changed on disk. Please reload and try again.".to_string(),
            );
        }

        let next_source = self.text.to_string();
        if current_source.as_deref() == Some(next_source.as_str()) {
            // 无变化:不写盘、不备份,仅返回健康扫描结果。
            return Ok(super::types::OpenClawWriteOutcome {
                backup_path: None,
                warnings: scan_openclaw_health_from_source(&next_source),
            });
        }

        let backup_path = current_source
            .as_ref()
            .map(|source| create_openclaw_backup(source, backup_dir, retain_count))
            .transpose()?
            .map(|path| path.display().to_string());

        atomic_write(&self.path, next_source.as_bytes())?;

        let warnings = scan_openclaw_health_from_source(&next_source);
        Ok(super::types::OpenClawWriteOutcome {
            backup_path,
            warnings,
        })
    }
}

// ============================================================================
// Backup / Atomic write
// ============================================================================

/// 将 `source` 快照写入 `backup_dir/openclaw_<时间戳>[_<N>].json5`,并按保留数清理。
pub fn create_openclaw_backup(
    source: &str,
    backup_dir: &Path,
    retain_count: usize,
) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_dir).map_err(|e| {
        format!(
            "Failed to create backup dir '{}': {e}",
            backup_dir.display()
        )
    })?;

    let base_id = format!("openclaw_{}", Local::now().format("%Y%m%d_%H%M%S"));
    let mut filename = format!("{base_id}.json5");
    let mut backup_path = backup_dir.join(&filename);
    let mut counter = 1;
    while backup_path.exists() {
        filename = format!("{base_id}_{counter}.json5");
        backup_path = backup_dir.join(&filename);
        counter += 1;
    }

    atomic_write(&backup_path, source.as_bytes())?;
    cleanup_openclaw_backups(backup_dir, retain_count)?;
    Ok(backup_path)
}

/// 保留最新 `retain_count` 个 `json5`/`json` 备份,删除更旧的。
pub fn cleanup_openclaw_backups(dir: &Path, retain_count: usize) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("Failed to list backup dir '{}': {e}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext == "json5" || ext == "json")
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    if entries.len() <= retain_count {
        return Ok(());
    }

    // 按文件名倒序(时间戳新者在前),保留前 retain_count 个。
    entries.sort_by(|a, b| b.path().file_name().cmp(&a.path().file_name()));
    for entry in entries.into_iter().skip(retain_count) {
        if let Err(err) = fs::remove_file(entry.path()) {
            log::warn!(
                "Failed to remove old OpenClaw config backup '{}': {err}",
                entry.path().display()
            );
        }
    }

    Ok(())
}

/// 原子写:写入同目录临时文件后 rename 覆盖目标。
fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create dir '{}': {e}", parent.display()))?;
        }
    }

    let tmp_path = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    fs::write(&tmp_path, content)
        .map_err(|e| format!("Failed to write temp file '{}': {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .map_err(|e| format!("Failed to move temp file onto '{}': {e}", path.display()))
}

// ============================================================================
// Health scan & legacy migration
// ============================================================================

fn warning(code: &str, message: impl Into<String>, path: Option<&str>) -> OpenClawHealthWarning {
    OpenClawHealthWarning {
        code: code.to_string(),
        message: message.into(),
        path: path.map(|value| value.to_string()),
    }
}

/// 对已解析的配置值执行健康扫描,返回警告列表。
pub fn scan_openclaw_health_from_value(config: &Value) -> Vec<OpenClawHealthWarning> {
    let mut warnings = Vec::new();

    if let Some(profile) = config
        .get("tools")
        .and_then(|tools| tools.get("profile"))
        .and_then(Value::as_str)
    {
        if !OPENCLAW_TOOLS_PROFILES.contains(&profile) {
            warnings.push(warning(
                "invalid_tools_profile",
                format!(
                    "tools.profile uses unsupported value '{profile}'; valid: {}.",
                    OPENCLAW_TOOLS_PROFILES.join(", ")
                ),
                Some("tools.profile"),
            ));
        }
    }

    if config
        .get("agents")
        .and_then(|agents| agents.get("defaults"))
        .and_then(|defaults| defaults.get("timeout"))
        .is_some()
    {
        warnings.push(warning(
            "legacy_agents_timeout",
            "agents.defaults.timeout is deprecated; use agents.defaults.timeoutSeconds instead.",
            Some("agents.defaults.timeout"),
        ));
    }

    if let Some(value) = config.get("env").and_then(|env| env.get("vars")) {
        if !value.is_object() {
            warnings.push(warning(
                "stringified_env_vars",
                "env.vars should be an object. The current value looks stringified or malformed.",
                Some("env.vars"),
            ));
        }
    }

    if let Some(value) = config.get("env").and_then(|env| env.get("shellEnv")) {
        if !value.is_object() {
            warnings.push(warning(
                "stringified_env_shell_env",
                "env.shellEnv should be an object. The current value looks stringified or malformed.",
                Some("env.shellEnv"),
            ));
        }
    }

    warnings
}

/// 对原始源码执行健康扫描;源码无法解析时返回单条 `config_parse_failed` 警告。
pub fn scan_openclaw_health_from_source(source: &str) -> Vec<OpenClawHealthWarning> {
    match json5::from_str::<Value>(source) {
        Ok(config) => scan_openclaw_health_from_value(&config),
        Err(err) => vec![warning(
            "config_parse_failed",
            format!("OpenClaw config could not be parsed as JSON5: {err}"),
            None,
        )],
    }
}

/// 将废弃的 `agents.defaults.timeout` 迁移为 `timeoutSeconds`(已存在则不覆盖)。
pub fn migrate_legacy_timeout(root: &mut Value) {
    let Some(defaults) = root
        .get_mut("agents")
        .and_then(|agents| agents.get_mut("defaults"))
        .and_then(|defaults| defaults.as_object_mut())
    else {
        return;
    };

    if let Some(timeout_value) = defaults.remove("timeout") {
        if !defaults.contains_key("timeoutSeconds") {
            defaults.insert("timeoutSeconds".to_string(), timeout_value);
        }
    }
}

// ============================================================================
// Round-trip AST helpers (port of cc-switch)
// ============================================================================

fn ensure_kvp_context(pair: &mut RtJSONKeyValuePair) -> &mut RtKeyValuePairContext {
    pair.context.get_or_insert_with(|| RtKeyValuePairContext {
        wsc: (String::new(), " ".to_string(), String::new(), None),
    })
}

fn extract_trailing_indent(separator_ws: &str) -> String {
    separator_ws
        .rsplit_once('\n')
        .map(|(_, tail)| tail.to_string())
        .unwrap_or_default()
}

fn derive_closing_ws_from_separator(separator_ws: &str) -> String {
    let Some((prefix, indent)) = separator_ws.rsplit_once('\n') else {
        return String::new();
    };

    let reduced_indent = if indent.ends_with('\t') {
        &indent[..indent.len().saturating_sub(1)]
    } else if indent.ends_with("  ") {
        &indent[..indent.len().saturating_sub(2)]
    } else if indent.ends_with(' ') {
        &indent[..indent.len().saturating_sub(1)]
    } else {
        indent
    };

    format!("{prefix}\n{reduced_indent}")
}

fn derive_entry_separator(leading_ws: &str) -> String {
    if leading_ws.is_empty() {
        return String::new();
    }

    if leading_ws.contains('\n') {
        return format!("\n{}", extract_trailing_indent(leading_ws));
    }

    String::new()
}

/// 节值从不直接参与 AST 构造,只经文本往返:
/// serde_json 序列化(规避空集合 panic)→ 归一化 → 重缩进 → 解析回 AST。
fn value_to_rt_value(value: &Value, parent_indent: &str) -> Result<RtJSONValue, String> {
    let source = serde_json::to_string_pretty(value)
        .map_err(|e| format!("Failed to serialize OpenClaw section: {e}"))?;

    let adjusted = reindent_json5_block(&source, parent_indent);
    let text = rt_from_str(&adjusted).map_err(|e| {
        format!(
            "Failed to parse generated JSON5 section: {} (line {})",
            e.message, e.lineno
        )
    })?;
    Ok(text.value)
}

fn reindent_json5_block(source: &str, parent_indent: &str) -> String {
    let normalized = normalize_json_five_output(source);
    if parent_indent.is_empty() || !normalized.contains('\n') {
        return normalized;
    }

    let mut lines = normalized.lines();
    let Some(first_line) = lines.next() else {
        return String::new();
    };

    let mut result = String::from(first_line);
    for line in lines {
        result.push('\n');
        result.push_str(parent_indent);
        result.push_str(line);
    }
    result
}

/// `serde_json` 会把 `/` 转义为 `\/`;JSON5 中无此需要,且会影响 round-trip 解析。
fn normalize_json_five_output(source: &str) -> String {
    source.replace("\\/", "/")
}

fn make_root_pair(key: &str, value: RtJSONValue, closing_ws: String) -> RtJSONKeyValuePair {
    RtJSONKeyValuePair {
        key: make_json5_key(key),
        value,
        context: Some(RtKeyValuePairContext {
            wsc: (String::new(), " ".to_string(), closing_ws, None),
        }),
    }
}

fn make_json5_key(key: &str) -> RtJSONValue {
    if is_identifier_key(key) {
        RtJSONValue::Identifier(key.to_string())
    } else {
        RtJSONValue::DoubleQuotedString(key.to_string())
    }
}

fn is_identifier_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    matches!(first, 'a'..='z' | 'A'..='Z' | '_' | '$')
        && chars.all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '$'))
}

fn json5_key_name(key: &RtJSONValue) -> Option<&str> {
    match key {
        RtJSONValue::Identifier(name)
        | RtJSONValue::DoubleQuotedString(name)
        | RtJSONValue::SingleQuotedString(name) => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // ------------------------------------------------------------------
    // Health scan & migration (pure, no fs)
    // ------------------------------------------------------------------

    #[test]
    fn scan_health_detects_known_openclaw_issues() {
        let config = json!({
            "tools": { "profile": "default" },
            "agents": { "defaults": { "timeout": 300 } },
            "env": { "vars": "[object Object]", "shellEnv": "oops" }
        });

        let warnings = scan_openclaw_health_from_value(&config);
        let codes = warnings.iter().map(|w| w.code.as_str()).collect::<Vec<_>>();

        assert!(codes.contains(&"invalid_tools_profile"));
        assert!(codes.contains(&"legacy_agents_timeout"));
        assert!(codes.contains(&"stringified_env_vars"));
        assert!(codes.contains(&"stringified_env_shell_env"));
    }

    #[test]
    fn scan_health_is_silent_on_valid_config() {
        let config = json!({
            "tools": { "profile": "coding" },
            "agents": { "defaults": { "timeoutSeconds": 600 } },
            "env": { "vars": { "A": "1" }, "shellEnv": { "B": "2" } }
        });
        assert!(scan_openclaw_health_from_value(&config).is_empty());
    }

    #[test]
    fn scan_health_from_source_reports_parse_failed() {
        let warnings = scan_openclaw_health_from_source("{ not valid json5 !!!");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "config_parse_failed");
    }

    #[test]
    fn migrate_legacy_timeout_moves_to_timeout_seconds() {
        let mut root = json!({ "agents": { "defaults": { "timeout": 300, "workspace": "~" } } });
        migrate_legacy_timeout(&mut root);
        let defaults = &root["agents"]["defaults"];
        assert!(defaults.get("timeout").is_none());
        assert_eq!(defaults["timeoutSeconds"], 300);
        assert_eq!(defaults["workspace"], "~");
    }

    #[test]
    fn migrate_legacy_timeout_keeps_existing_timeout_seconds() {
        let mut root =
            json!({ "agents": { "defaults": { "timeout": 300, "timeoutSeconds": 999 } } });
        migrate_legacy_timeout(&mut root);
        let defaults = &root["agents"]["defaults"];
        assert!(defaults.get("timeout").is_none());
        assert_eq!(defaults["timeoutSeconds"], 999);
    }

    #[test]
    fn migrate_legacy_timeout_is_noop_without_agents() {
        let mut root = json!({ "models": { "providers": {} } });
        migrate_legacy_timeout(&mut root);
        assert!(root.get("agents").is_none());
    }

    // ------------------------------------------------------------------
    // json-five panic regression
    // ------------------------------------------------------------------

    #[test]
    fn value_to_rt_value_serializes_empty_providers_without_panic() {
        let value = json!({
            "mode": "merge",
            "providers": {}
        });
        let rt = value_to_rt_value(&value, "").unwrap();
        assert_eq!(
            rt.to_string(),
            "{\n  \"mode\": \"merge\",\n  \"providers\": {}\n}"
        );
    }

    // ------------------------------------------------------------------
    // Round-trip document engine
    // ------------------------------------------------------------------

    #[test]
    fn roundtrip_preserves_top_level_comments_and_adds_section() {
        let _guard = test_guard();
        let dir = temp_dir();
        let path = dir.path().join("openclaw.json");
        let source =
            "{\n  // keep me\n  models: {\n    mode: 'merge',\n    providers: {},\n  },\n}\n";
        fs::write(&path, source).unwrap();
        let backup_dir = dir.path().join("backups");

        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        doc.set_root_section("env", &json!({ "TOKEN": "value" }))
            .unwrap();
        let outcome = doc.save(&backup_dir, 2).unwrap();

        assert!(outcome.backup_path.is_some());
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("// keep me"));
        assert!(written.contains("env"));
        assert!(written.contains("TOKEN"));
        assert!(fs::read_dir(&backup_dir).unwrap().count() >= 1);
    }

    #[test]
    fn save_detects_external_conflict() {
        let _guard = test_guard();
        let dir = temp_dir();
        let path = dir.path().join("openclaw.json");
        fs::write(&path, "{\n  models: { providers: {} },\n}\n").unwrap();

        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        doc.set_root_section("env", &json!({ "TOKEN": "value" }))
            .unwrap();

        fs::write(&path, "{ changedExternally: true }\n").unwrap();
        let err = doc.save(&dir.path().join("backups"), 2).unwrap_err();
        assert!(err.contains("changed on disk"));
    }

    #[test]
    fn noop_save_skips_backup() {
        let _guard = test_guard();
        let dir = temp_dir();
        let path = dir.path().join("openclaw.json");
        let source = r#"{
  models: {
    mode: 'merge',
    providers: {},
  },
}
"#;
        fs::write(&path, source).unwrap();
        let backup_dir = dir.path().join("backups");

        // 首次写入会产生备份。
        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        doc.set_root_section("env", &json!({ "TOKEN": "1" }))
            .unwrap();
        let first = doc.save(&backup_dir, 5).unwrap();
        assert!(first.backup_path.is_some());
        let first_written = fs::read_to_string(&path).unwrap();
        let backup_count = fs::read_dir(&backup_dir).unwrap().count();

        // 二次相同写入:不产生新备份、文件不变。
        let mut doc2 = OpenClawConfigDocument::load(&path).unwrap();
        doc2.set_root_section("env", &json!({ "TOKEN": "1" }))
            .unwrap();
        let second = doc2.save(&backup_dir, 5).unwrap();
        assert!(second.backup_path.is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), first_written);
        assert_eq!(fs::read_dir(&backup_dir).unwrap().count(), backup_count);
    }

    #[test]
    fn remove_root_section_middle_preserves_others() {
        let _guard = test_guard();
        let dir = temp_dir();
        let path = dir.path().join("openclaw.json");
        let source = "{\n  models: { providers: {} },\n  agents: { defaults: {} },\n  env: { A: '1' },\n  tools: { profile: 'coding' },\n}\n";
        fs::write(&path, source).unwrap();

        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        assert!(doc.remove_root_section("agents").unwrap());

        let written = doc.text.to_string();
        assert!(!written.contains("agents"));
        assert!(written.contains("models"));
        assert!(written.contains("env"));
        assert!(written.contains("tools"));
        // 仍是合法 JSON5。
        json5::from_str::<Value>(&written).expect("must stay valid JSON5");
    }

    #[test]
    fn remove_root_section_last_and_first() {
        let _guard = test_guard();
        let dir = temp_dir();
        let path = dir.path().join("openclaw.json");
        let source =
            "{\n  models: { providers: {} },\n  agents: { defaults: {} },\n  env: { A: '1' },\n}\n";
        fs::write(&path, source).unwrap();

        // 删除末尾 env。
        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        assert!(doc.remove_root_section("env").unwrap());
        let written = doc.text.to_string();
        assert!(!written.contains("env"));
        json5::from_str::<Value>(&written).expect("valid after removing last");

        // 删除首个 models。
        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        assert!(doc.remove_root_section("models").unwrap());
        let written = doc.text.to_string();
        assert!(!written.contains("models"));
        json5::from_str::<Value>(&written).expect("valid after removing first");
    }

    #[test]
    fn apply_root_section_diff_adds_removes_updates_only_changed() {
        let _guard = test_guard();
        let dir = temp_dir();
        let path = dir.path().join("openclaw.json");
        let source = "{\n  // comment stays\n  models: { mode: 'merge', providers: {} },\n  env: { A: '1' },\n}\n";
        fs::write(&path, source).unwrap();

        let old_value = json5::from_str::<Value>(source).unwrap();
        let new_value = json!({
            "models": { "mode": "merge", "providers": {} },
            "tools": { "profile": "coding" }
        });

        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        doc.apply_root_section_diff(&old_value, &new_value).unwrap();
        let written = doc.text.to_string();

        assert!(written.contains("// comment stays"));
        assert!(written.contains("tools"));
        assert!(!written.contains("env"));
        json5::from_str::<Value>(&written).expect("valid after diff");
    }

    /// Reordering providers changes only the key order of `models.providers`,
    /// and `serde_json`'s `preserve_order` makes `Value` equality
    /// order-insensitive. The diff must compare the serialized text instead —
    /// otherwise a drag is silently dropped as "nothing changed".
    #[test]
    fn apply_root_section_diff_writes_a_provider_reorder() {
        let _guard = test_guard();
        let dir = temp_dir();
        let path = dir.path().join("openclaw.json");
        let source = "{\n  models: { mode: 'merge', providers: { a: { models: [] }, b: { models: [] } } },\n}\n";
        fs::write(&path, source).unwrap();

        let old_value = json5::from_str::<Value>(source).unwrap();
        let new_value = json!({
            "models": { "mode": "merge", "providers": { "b": { "models": [] }, "a": { "models": [] } } }
        });
        assert_eq!(old_value.get("models"), new_value.get("models"), "equality is order-insensitive");

        let mut doc = OpenClawConfigDocument::load(&path).unwrap();
        doc.apply_root_section_diff(&old_value, &new_value).unwrap();
        let written = doc.text.to_string();

        let reparsed = json5::from_str::<Value>(&written).expect("valid after reorder");
        let provider_keys: Vec<&String> = reparsed
            .get("models")
            .and_then(|models| models.get("providers"))
            .and_then(Value::as_object)
            .expect("providers survive")
            .keys()
            .collect();
        assert_eq!(provider_keys, vec!["b", "a"], "written order: {written}");
    }
}
