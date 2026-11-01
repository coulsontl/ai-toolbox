//! 宿主 ↔ 插件进程的 RPC 帧协议。
//!
//! ── 形状 ──────────────────────────────────────────────────────
//! stdin/stdout 各一行一个 JSON，**双向对称**：每一方都能发
//! `{id, method, params}`，另一方回 `{id, result}` 或 `{id, error}`。
//!
//! 流式回复先发 `head`，再发若干 `chunk`（base64），最后以 `{id, result: null}`
//! 收尾：
//! ```text
//! {id, event:"head",  status, headers}
//! {id, event:"chunk", data:"<base64>"}
//! {id, result:null}
//! ```
//! 取消用独立的 `{method:"abort", params:{id}}`，背压额度用
//! `{method:"credit", params:{id,n}}`——两者都不占用被取消调用的 `id`。
//!
//! ── 为什么读循环里绝不能 await 业务 ────────────────────────────
//! 插件在处理一个请求时会反向回调宿主（`store.get` 等）。如果读循环正卡在
//! 等某个业务完成，那条回调的应答就永远读不到——自己把自己锁死。所以读循环
//! 只做「分类 → 投递给等待方 / 起独立 task」，所有业务 await 都在别处。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{mpsc, Mutex};

/// One line off the wire.
#[derive(Debug, Clone, Deserialize)]
pub struct IncomingFrame {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    /// `head` / `chunk` for a streaming reply.
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub headers: Option<Value>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

/// Everything a caller can observe for one request.
#[derive(Debug)]
pub enum Reply {
    /// A streaming call's response head (`fetch`).
    Head {
        status: u16,
        headers: Value,
    },
    /// A body chunk; only `fetch` produces these.
    Chunk(Vec<u8>),
    /// The call finished successfully.
    Done(Value),
    /// The call failed, or the host died before answering.
    Failed(String),
}

type PendingMap = HashMap<u64, mpsc::Sender<Reply>>;

/// The write half plus the in-flight table.
///
/// Writes are serialized through one mutex so two concurrent calls can never
/// interleave half a line — a corrupted frame breaks the whole channel, not
/// just the offending call.
pub struct HostLink {
    stdin: Mutex<Option<ChildStdin>>,
    pending: Mutex<PendingMap>,
    next_id: AtomicU64,
    /// Cleared when stdout reaches EOF: the host process is gone, even if the
    /// child handle still exists. Callers check this before trusting a link.
    alive: AtomicBool,
}

impl HostLink {
    pub fn new(stdin: ChildStdin) -> Arc<Self> {
        Arc::new(Self {
            stdin: Mutex::new(Some(stdin)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
        })
    }

    /// Whether the process on the other end is still answering.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn allocate_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn write_line(&self, value: &Value) -> Result<(), String> {
        let mut guard = self.stdin.lock().await;
        let Some(stdin) = guard.as_mut() else {
            return Err("The plugin host is not running".to_string());
        };
        if !self.is_alive() {
            return Err("The plugin host is not running".to_string());
        }
        let mut line = serde_json::to_vec(value)
            .map_err(|error| format!("Failed to encode a plugin host frame: {error}"))?;
        line.push(b'\n');
        stdin
            .write_all(&line)
            .await
            .map_err(|error| format!("Failed to write to the plugin host: {error}"))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("Failed to flush the plugin host stream: {error}"))
    }

    /// Send one request and return the receiver its replies arrive on.
    ///
    /// The caller drains until `Done`/`Failed`; dropping the receiver early
    /// abandons the call, and the host keeps running (it notices on the next
    /// `abort` or when its write fails).
    async fn start(&self, method: &str, params: Value) -> Result<(u64, mpsc::Receiver<Reply>), String> {
        let id = self.allocate_id();
        let (tx, rx) = mpsc::channel::<Reply>(16);
        self.pending.lock().await.insert(id, tx);
        if let Err(error) = self
            .write_line(&json!({ "id": id, "method": method, "params": params }))
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        Ok((id, rx))
    }

    /// A request whose reply is a single `result` (no streaming events).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let (id, mut rx) = self.start(method, params).await?;
        let outcome = drain_to_result(&mut rx).await;
        self.pending.lock().await.remove(&id);
        outcome
    }

    /// A streaming request (`fetch`): head, then chunks, then done.
    pub async fn request_stream(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(u64, mpsc::Receiver<Reply>), String> {
        self.start(method, params).await
    }

    /// Drop a pending entry — used when the caller abandons a stream.
    pub async fn forget(&self, id: u64) {
        self.pending.lock().await.remove(&id);
    }

    pub async fn abort(&self, id: u64) {
        let _ = self
            .write_line(&json!({ "method": "abort", "params": { "id": id } }))
            .await;
    }

    pub async fn credit(&self, id: u64, bytes: usize) {
        let _ = self
            .write_line(&json!({ "method": "credit", "params": { "id": id, "n": bytes } }))
            .await;
    }

    /// Answer a host → gateway callback (e.g. `store.get`).
    pub async fn reply(&self, id: u64, result: Value) -> Result<(), String> {
        self.write_line(&json!({ "id": id, "result": result })).await
    }

    pub async fn reply_error(&self, id: u64, message: impl Into<String>) -> Result<(), String> {
        let message = message.into();
        self.write_line(&json!({ "id": id, "error": { "message": message } }))
            .await
    }

    async fn deliver(&self, id: u64, event: Reply) {
        let guard = self.pending.lock().await;
        if let Some(sender) = guard.get(&id) {
            // A closed receiver means the caller gave up; the entry is dropped
            // by whoever notices first.
            let _ = sender.send(event).await;
        }
    }

    pub async fn close(&self) {
        // Dropping stdin is what makes a well-behaved host exit.
        self.stdin.lock().await.take();
        self.pending.lock().await.clear();
    }
}

async fn drain_to_result(rx: &mut mpsc::Receiver<Reply>) -> Result<Value, String> {
    let mut last = Value::Null;
    while let Some(reply) = rx.recv().await {
        match reply {
            Reply::Done(value) => return Ok(value),
            Reply::Failed(message) => return Err(message),
            // Head/chunk on a non-streaming call: ignore rather than fail, so a
            // host that upgrades a call to streaming still yields its result.
            Reply::Head { .. } | Reply::Chunk(_) => {}
        }
        last = Value::Null;
    }
    let _ = last;
    Err("The plugin host closed the connection before answering".to_string())
}

/// Read frames until stdout reaches EOF, then fail every waiting caller.
///
/// EOF is how a crashed or shut-down host surfaces: callers must see their
/// in-flight calls fail immediately rather than hang forever.
pub async fn read_loop(stdout: ChildStdout, link: Arc<HostLink>) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<IncomingFrame>(&line) else {
            // A malformed line cannot be attributed to a caller; the host is
            // responsible for its own output shape.
            continue;
        };
        dispatch(frame, &link).await;
    }

    link.alive.store(false, Ordering::SeqCst);
    let mut pending = link.pending.lock().await;
    for (_, sender) in pending.drain() {
        let _ = sender
            .send(Reply::Failed(
                "The plugin host stopped before answering".to_string(),
            ))
            .await;
    }
}

async fn dispatch(frame: IncomingFrame, link: &Arc<HostLink>) {
    let Some(id) = frame.id else {
        // Host-initiated, id-less events (`log`, `toast`, `auth`) are not part
        // of the request/response path.
        return;
    };

    if let Some(event) = frame.event.as_deref() {
        match event {
            "head" => {
                link.deliver(
                    id,
                    Reply::Head {
                        status: frame.status.unwrap_or(200),
                        headers: frame.headers.unwrap_or(Value::Null),
                    },
                )
                .await;
            }
            "chunk" => {
                let bytes = frame
                    .data
                    .as_deref()
                    .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
                    .unwrap_or_default();
                link.deliver(id, Reply::Chunk(bytes)).await;
            }
            _ => {}
        }
        return;
    }

    let terminal = match frame.error {
        Some(error) => Reply::Failed(describe_wire_error(&error)),
        None => Reply::Done(frame.result.unwrap_or(Value::Null)),
    };
    link.deliver(id, terminal).await;
    link.forget(id).await;
}

fn describe_wire_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(json: &str) -> IncomingFrame {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_result_frame_carries_its_value() {
        let parsed = frame(r#"{"id":7,"result":{"ok":true}}"#);
        assert_eq!(parsed.id, Some(7));
        assert_eq!(parsed.result.unwrap()["ok"], json!(true));
        assert!(parsed.error.is_none());
    }

    #[test]
    fn an_error_frame_exposes_its_message() {
        let parsed = frame(r#"{"id":3,"error":{"message":"no such provider"}}"#);
        let error = parsed.error.unwrap();
        assert_eq!(describe_wire_error(&error), "no such provider");
    }

    #[test]
    fn a_chunk_frame_decodes_base64() {
        let parsed = frame(r#"{"id":1,"event":"chunk","data":"aGVsbG8="}"#);
        assert_eq!(parsed.event.as_deref(), Some("chunk"));
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(parsed.data.unwrap())
            .unwrap();
        assert_eq!(decoded, b"hello");
    }

    #[test]
    fn a_head_frame_defaults_to_200_without_losing_headers() {
        let parsed = frame(r#"{"id":1,"event":"head","headers":{"x":"y"}}"#);
        assert_eq!(parsed.status.unwrap_or(200), 200);
        assert_eq!(parsed.headers.unwrap()["x"], json!("y"));
    }

    #[test]
    fn an_id_less_event_is_not_a_response() {
        // `{event:"log"}` and friends must not be mistaken for a call's reply.
        let parsed = frame(r#"{"event":"log","level":"info","message":"hi"}"#);
        assert!(parsed.id.is_none());
    }
}
