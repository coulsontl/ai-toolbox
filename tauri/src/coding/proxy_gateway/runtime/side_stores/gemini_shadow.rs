use futures_util::{stream, Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::{append_side_store_sse_bytes, take_sse_block};

const MAX_GEMINI_SHADOW_SESSIONS: usize = 200;
const MAX_GEMINI_SHADOW_TURNS_PER_SESSION: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GeminiShadowSessionKey {
    provider_id: String,
    session_id: String,
}

impl GeminiShadowSessionKey {
    pub(crate) fn new(provider_id: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            session_id: session_id.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct GeminiShadowTurn {
    assistant_content: Value,
    function_call_names: Vec<String>,
}

#[derive(Debug, Default)]
struct GeminiShadowInner {
    sessions: HashMap<GeminiShadowSessionKey, VecDeque<GeminiShadowTurn>>,
    session_order: VecDeque<GeminiShadowSessionKey>,
}

#[derive(Debug, Default)]
pub(crate) struct GeminiShadowStore {
    inner: Mutex<GeminiShadowInner>,
}

impl GeminiShadowStore {
    pub(super) fn record_response(&self, key: GeminiShadowSessionKey, response: &Value) -> usize {
        let turns = response
            .get("candidates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|candidate| candidate.get("content"))
            .filter_map(gemini_shadow_turn_from_content)
            .collect::<Vec<_>>();
        if turns.is_empty() {
            return 0;
        }
        let Ok(mut inner) = self.inner.lock() else {
            return 0;
        };
        touch_session_order(&mut inner.session_order, &key);
        let session = inner.sessions.entry(key).or_default();
        let changed = turns.len();
        for turn in turns {
            session.push_back(turn);
            while session.len() > MAX_GEMINI_SHADOW_TURNS_PER_SESSION {
                session.pop_front();
            }
        }
        prune_sessions(&mut inner);
        changed
    }

    pub(super) fn enrich_request(&self, key: &GeminiShadowSessionKey, body: &mut Value) -> usize {
        let Some(contents) = body.get_mut("contents").and_then(Value::as_array_mut) else {
            return 0;
        };
        if request_needs_function_call_replay(contents) {
            let requested_names = function_response_names(contents);
            let Some(turn) = self.latest_matching_turn(key, &requested_names) else {
                return 0;
            };
            let insert_index = contents
                .iter()
                .position(content_has_function_response)
                .unwrap_or(contents.len());
            contents.insert(insert_index, turn.assistant_content);
            return 1;
        }
        // Long tool histories: when model functionCall parts are present but lost
        // their thoughtSignature on replay, backfill signatures from recorded turns
        // (cc-switch 8162adb3).
        self.backfill_missing_function_call_signatures(key, contents)
    }

    fn backfill_missing_function_call_signatures(
        &self,
        key: &GeminiShadowSessionKey,
        contents: &mut [Value],
    ) -> usize {
        let needs_backfill = contents.iter().any(|content| {
            content
                .get("parts")
                .and_then(Value::as_array)
                .is_some_and(|parts| {
                    let has_fc = parts.iter().any(|part| part.get("functionCall").is_some());
                    let has_signed_fc = parts
                        .iter()
                        .any(|part| part.get("functionCall").is_some() && part_signature(part).is_some());
                    has_fc && !has_signed_fc
                })
        });
        if !needs_backfill {
            return 0;
        }
        let Ok(mut inner) = self.inner.lock() else {
            return 0;
        };
        let Some(turns) = inner.sessions.get(key) else {
            return 0;
        };
        let mut sig_by_call_key: HashMap<(String, String), String> = HashMap::new();
        // Track unambiguous tool-name fallback: `Some(sig)` if every signed call
        // for `name` in this session carried the same signature, `None` if ambiguous.
        let mut sig_by_name: HashMap<String, Option<String>> = HashMap::new();
        for turn in turns {
            if let Some(parts) = turn
                .assistant_content
                .get("parts")
                .and_then(Value::as_array)
            {
                for part in parts {
                    let Some(sig) = part_signature(part) else {
                        continue;
                    };
                    if let Some(fc) = part.get("functionCall") {
                        if let Some(name) = fc.get("name").and_then(Value::as_str) {
                            let args = fc
                                .get("args")
                                .map(canonical_json_key)
                                .unwrap_or_default();
                            sig_by_call_key.insert((name.to_string(), args), sig.to_string());
                            sig_by_name
                                .entry(name.to_string())
                                .and_modify(|existing| {
                                    if existing.as_deref() != Some(sig) {
                                        *existing = None;
                                    }
                                })
                                .or_insert_with(|| Some(sig.to_string()));
                        }
                    }
                }
            }
        }
        if sig_by_call_key.is_empty() {
            return 0;
        }
        let mut restored = 0;
        for content in contents.iter_mut() {
            let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) else {
                continue;
            };
            // In Gemini parallel function calling, only the first `functionCall`
            // part of a model turn carries `thoughtSignature`. Never stamp a
            // signature onto trailing sibling calls when the turn is already signed.
            if parts
                .iter()
                .any(|part| part.get("functionCall").is_some() && part_signature(part).is_some())
            {
                continue;
            }
            let Some(first_fc_part) = parts
                .iter_mut()
                .find(|part| part.get("functionCall").is_some())
            else {
                continue;
            };
            let Some(name) = first_fc_part
                .pointer("/functionCall/name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
            else {
                continue;
            };
            let args = first_fc_part
                .pointer("/functionCall/args")
                .map(canonical_json_key)
                .unwrap_or_default();
            let sig = sig_by_call_key
                .get(&(name.clone(), args))
                .cloned()
                .or_else(|| sig_by_name.get(&name).and_then(|s| s.clone()));
            if let (Some(sig), Some(obj)) = (sig, first_fc_part.as_object_mut()) {
                obj.insert("thoughtSignature".to_string(), json!(sig));
                restored += 1;
            }
        }
        if restored > 0 {
            touch_session_order(&mut inner.session_order, key);
        }
        restored
    }

    fn latest_matching_turn(
        &self,
        key: &GeminiShadowSessionKey,
        requested_names: &[String],
    ) -> Option<GeminiShadowTurn> {
        let Ok(mut inner) = self.inner.lock() else {
            return None;
        };
        let turn = inner.sessions.get(key).and_then(|turns| {
            turns
                .iter()
                .rev()
                .find(|turn| {
                    requested_names.is_empty()
                        || requested_names
                            .iter()
                            .any(|name| turn.function_call_names.iter().any(|item| item == name))
                })
                .cloned()
        });
        if turn.is_some() {
            touch_session_order(&mut inner.session_order, key);
        }
        turn
    }
}

pub(crate) fn record_gemini_sse_stream(
    inner: Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>,
    store: Arc<GeminiShadowStore>,
    key: GeminiShadowSessionKey,
) -> Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send + 'static>> {
    struct State {
        inner: Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>,
        store: Arc<GeminiShadowStore>,
        key: GeminiShadowSessionKey,
        buffer: Vec<u8>,
        recording_enabled: bool,
    }

    Box::pin(stream::unfold(
        State {
            inner,
            store,
            key,
            buffer: Vec::new(),
            recording_enabled: true,
        },
        |mut state| async move {
            let item = state.inner.next().await?;
            if let Ok(bytes) = &item {
                if state.recording_enabled {
                    if !append_side_store_sse_bytes(&mut state.buffer, bytes) {
                        state.recording_enabled = false;
                        state.buffer.clear();
                    } else {
                        while let Some(block) = take_sse_block(&mut state.buffer) {
                            inspect_gemini_sse_block(&block, &state.store, &state.key);
                        }
                    }
                }
            }
            Some((item, state))
        },
    ))
}

fn inspect_gemini_sse_block(block: &[u8], store: &GeminiShadowStore, key: &GeminiShadowSessionKey) {
    let Ok(block) = std::str::from_utf8(block) else {
        return;
    };
    let mut data_parts = Vec::new();
    for line in block.lines() {
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        if field == "data" {
            data_parts.push(value.trim_start().to_string());
        }
    }
    let data = data_parts.join("\n");
    if data.trim().is_empty() || data.trim() == "[DONE]" {
        return;
    }
    if let Ok(value) = serde_json::from_str::<Value>(&data) {
        store.record_response(key.clone(), &value);
    }
}

fn gemini_shadow_turn_from_content(content: &Value) -> Option<GeminiShadowTurn> {
    let parts = content.get("parts").and_then(Value::as_array)?;
    let function_call_names = parts
        .iter()
        .filter(|part| part.get("functionCall").is_some() && part_signature(part).is_some())
        .filter_map(|part| part.pointer("/functionCall/name").and_then(Value::as_str))
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let has_part_signature = parts.iter().any(|part| part_signature(part).is_some());
    if function_call_names.is_empty() && !has_part_signature {
        return None;
    }
    Some(GeminiShadowTurn {
        assistant_content: normalize_assistant_content(content),
        function_call_names,
    })
}

fn normalize_assistant_content(content: &Value) -> Value {
    let mut content = content.clone();
    if content.get("role").and_then(Value::as_str).is_none() {
        if let Some(object) = content.as_object_mut() {
            object.insert("role".to_string(), json!("model"));
        }
    }
    content
}

fn request_needs_function_call_replay(contents: &[Value]) -> bool {
    let has_function_response = contents.iter().any(content_has_function_response);
    let has_function_call = contents.iter().any(content_has_function_call);
    has_function_response && !has_function_call
}

fn function_response_names(contents: &[Value]) -> Vec<String> {
    contents
        .iter()
        .flat_map(|content| {
            content
                .get("parts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|part| part.get("functionResponse"))
        .filter_map(|response| response.get("name").and_then(Value::as_str))
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn content_has_function_call(content: &Value) -> bool {
    content
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|part| part.get("functionCall").is_some())
}

fn content_has_function_response(content: &Value) -> bool {
    content
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|part| part.get("functionResponse").is_some())
}

fn part_signature(part: &Value) -> Option<&str> {
    part.get("thoughtSignature")
        .or_else(|| part.get("thought_signature"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn canonical_json_key(value: &Value) -> String {
    fn write_canonical(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                out.push('{');
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|(a, _), (b, _)| a.cmp(b));
                for (index, (k, v)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push(':');
                    write_canonical(v, out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_canonical(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn touch_session_order(order: &mut VecDeque<GeminiShadowSessionKey>, key: &GeminiShadowSessionKey) {
    if let Some(index) = order.iter().position(|existing| existing == key) {
        order.remove(index);
    }
    order.push_back(key.clone());
}

fn prune_sessions(inner: &mut GeminiShadowInner) {
    while inner.sessions.len() > MAX_GEMINI_SHADOW_SESSIONS {
        let Some(key) = inner.session_order.pop_front() else {
            break;
        };
        inner.sessions.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_latest_signed_function_call_before_function_response() {
        let store = GeminiShadowStore::default();
        let key = GeminiShadowSessionKey::new("provider-a", "session-a");
        store.record_response(
            key.clone(),
            &json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{
                            "functionCall": {"name": "read_file", "args": {"path": "README.md"}},
                            "thoughtSignature": "sig-1"
                        }]
                    }
                }]
            }),
        );

        let mut request = json!({
            "contents": [{
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": "read_file",
                        "response": {"content": "ok"}
                    }
                }]
            }]
        });

        assert_eq!(store.enrich_request(&key, &mut request), 1);
        let contents = request["contents"].as_array().unwrap();
        assert_eq!(contents[0]["role"], "model");
        assert_eq!(contents[0]["parts"][0]["thoughtSignature"], "sig-1");
        assert_eq!(
            contents[1]["parts"][0]["functionResponse"]["name"],
            "read_file"
        );
    }

    #[tokio::test]
    async fn records_signed_function_call_from_sse_stream_for_later_replay() {
        let store = Arc::new(GeminiShadowStore::default());
        let key = GeminiShadowSessionKey::new("provider-a", "session-stream");
        let sse = concat!(
            "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"name\":\"lookup\",\"args\":{\"query\":\"rust\"}},\"thoughtSignature\":\"sig-stream\"}]}}]}\n\n",
            "data: {\"usageMetadata\":{\"totalTokenCount\":8}}\n\n"
        );
        let inner = Box::pin(stream::iter(vec![Ok(sse.as_bytes().to_vec())]));
        let mut wrapped = record_gemini_sse_stream(inner, store.clone(), key.clone());

        let mut forwarded = Vec::new();
        while let Some(chunk) = wrapped.next().await {
            forwarded.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(forwarded, sse.as_bytes());

        let mut request = json!({
            "contents": [{
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": "lookup",
                        "response": {"content": "ok"}
                    }
                }]
            }]
        });

        assert_eq!(store.enrich_request(&key, &mut request), 1);
        let contents = request["contents"].as_array().unwrap();
        assert_eq!(contents[0]["role"], "model");
        assert_eq!(contents[0]["parts"][0]["thoughtSignature"], "sig-stream");
        assert_eq!(contents[0]["parts"][0]["functionCall"]["name"], "lookup");
        assert_eq!(
            contents[1]["parts"][0]["functionResponse"]["name"],
            "lookup"
        );
    }

    #[test]
    fn takes_physically_earliest_sse_delimiter() {
        let mut buffer = b"data: first\r\n\r\ndata: second\n\n".to_vec();

        assert_eq!(
            take_sse_block(&mut buffer).as_deref(),
            Some(b"data: first".as_slice())
        );
        assert_eq!(
            take_sse_block(&mut buffer).as_deref(),
            Some(b"data: second".as_slice())
        );
    }

    #[test]
    fn backfills_missing_thought_signatures_in_long_tool_history() {
        let store = GeminiShadowStore::default();
        let key = GeminiShadowSessionKey::new("provider-g", "session-long");
        store.record_response(
            key.clone(),
            &json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{
                            "functionCall": {"name": "read_file", "args": {"path": "src/lib.rs"}},
                            "thoughtSignature": "sig-lib"
                        }]
                    }
                }]
            }),
        );

        let mut request = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "inspect lib"}]},
                {
                    "role": "model",
                    "parts": [{
                        "functionCall": {"name": "read_file", "args": {"path": "src/lib.rs"}}
                    }]
                },
                {
                    "role": "user",
                    "parts": [{
                        "functionResponse": {"name": "read_file", "response": {"content": "ok"}}
                    }]
                }
            ]
        });

        assert_eq!(store.enrich_request(&key, &mut request), 1);
        assert_eq!(
            request["contents"][1]["parts"][0]["thoughtSignature"],
            "sig-lib"
        );
    }
}
