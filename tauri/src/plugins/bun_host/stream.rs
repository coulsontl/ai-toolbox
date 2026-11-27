//! A fetch reply body delivered by the host, with credit-based backpressure.
//!
//! The host may have at most [`STREAM_WINDOW`] decoded bytes outstanding for
//! one fetch. Every chunk we hand to the consumer is credited back (its
//! payload plus [`FRAME_OVERHEAD`]), so a slow consumer pauses the vendor
//! stream instead of buffering without bound. Dropping the body before the
//! host finished aborts the fetch, which cancels the upstream request.
//!
//! A cancelled fetch gets no answer from the host (it suppresses output for
//! a request nobody waits on), so the abort path also removes the pending
//! entry itself; otherwise every abandoned body would leak a slot.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use super::protocol::{encode_abort, encode_credit, FRAME_OVERHEAD, STREAM_BUDGET};
use super::HostInner;

pub type PluginFetchBody = Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>;

/// Head of a fetch reply plus the stream of its body.
pub struct PluginFetchResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: PluginFetchBody,
}

/// The one-shot abort a fetch owes the host if nobody consumes its reply.
pub(super) struct FetchAbort {
    fetch_id: u64,
    stdin_tx: mpsc::UnboundedSender<String>,
    finished: Arc<AtomicBool>,
    state: Arc<AsyncMutex<HostInner>>,
    armed: bool,
}

impl FetchAbort {
    pub(super) fn new(
        fetch_id: u64,
        stdin_tx: mpsc::UnboundedSender<String>,
        finished: Arc<AtomicBool>,
        state: Arc<AsyncMutex<HostInner>>,
    ) -> Self {
        Self {
            fetch_id,
            stdin_tx,
            finished,
            state,
            armed: true,
        }
    }

    pub(super) fn disarm(mut self) {
        self.armed = false;
    }

    fn fire(&mut self) {
        if !self.armed || self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.stdin_tx.send(encode_abort(self.fetch_id));
        // The host will not answer an aborted fetch. Forgetting its slot now
        // keeps `pending_requests` honest; if the lock is busy, a short task
        // removes it instead.
        if let Ok(mut inner) = self.state.try_lock() {
            inner.pending.remove(&self.fetch_id);
        } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let state = self.state.clone();
            let fetch_id = self.fetch_id;
            handle.spawn(async move {
                state.lock().await.pending.remove(&fetch_id);
            });
        }
    }
}

impl Drop for FetchAbort {
    fn drop(&mut self) {
        self.fire();
    }
}

pub(super) struct FetchBodyStream {
    fetch_id: u64,
    receiver: mpsc::Receiver<Result<Bytes, String>>,
    stdin_tx: mpsc::UnboundedSender<String>,
    received: usize,
    /// Holds the reader's `finished` flag: a completed fetch is not aborted.
    abort: FetchAbort,
}

impl FetchBodyStream {
    pub(super) fn new(
        fetch_id: u64,
        receiver: mpsc::Receiver<Result<Bytes, String>>,
        stdin_tx: mpsc::UnboundedSender<String>,
        finished: Arc<AtomicBool>,
        state: Arc<AsyncMutex<HostInner>>,
    ) -> Self {
        let abort = FetchAbort::new(fetch_id, stdin_tx.clone(), finished, state);
        Self {
            fetch_id,
            receiver,
            stdin_tx,
            received: 0,
            abort,
        }
    }
}

impl Stream for FetchBodyStream {
    type Item = Result<Bytes, String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        match Pin::new(&mut this.receiver).poll_recv(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                this.received += bytes.len();
                if this.received > STREAM_BUDGET {
                    this.abort.fire();
                    return Poll::Ready(Some(Err(format!(
                        "plugin fetch exceeded the {STREAM_BUDGET}-byte reply budget"
                    ))));
                }
                let credit = bytes.len() + FRAME_OVERHEAD;
                let _ = this.stdin_tx.send(encode_credit(this.fetch_id, credit));
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(message))) => Poll::Ready(Some(Err(message))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for FetchBodyStream {
    fn drop(&mut self) {
        self.abort.fire();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn empty_state() -> Arc<AsyncMutex<HostInner>> {
        Arc::new(AsyncMutex::new(HostInner::new()))
    }

    #[test]
    fn a_consumed_chunk_is_credited_back() {
        let (tx, rx) = mpsc::channel(4);
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel();
        let finished = Arc::new(AtomicBool::new(true));
        let mut body = Box::pin(FetchBodyStream::new(
            5,
            rx,
            stdin_tx,
            finished,
            empty_state(),
        ));
        tx.try_send(Ok(Bytes::from_static(b"hello"))).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let chunk = runtime.block_on(async {
            use futures_util::StreamExt;
            body.next().await.unwrap().unwrap()
        });
        assert_eq!(&chunk[..], b"hello");
        let credit: Value = serde_json::from_str(stdin_rx.try_recv().unwrap().trim()).unwrap();
        assert_eq!(credit["method"], "credit");
        assert_eq!(credit["params"]["n"], 5 + FRAME_OVERHEAD);
    }

    #[tokio::test]
    async fn dropping_an_unfinished_body_aborts_and_forgets_the_slot() {
        let (_tx, rx) = mpsc::channel(4);
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel();
        let finished = Arc::new(AtomicBool::new(false));
        let state = empty_state();
        {
            let (head_tx, _head_rx) = tokio::sync::oneshot::channel();
            let (chunk_tx, _chunk_rx) = mpsc::channel(4);
            state.lock().await.pending.insert(
                6,
                super::super::Pending::Head {
                    method: "fetch".to_string(),
                    tx: head_tx,
                    chunk_tx,
                    finished: finished.clone(),
                },
            );
        }
        let body = FetchBodyStream::new(6, rx, stdin_tx, finished.clone(), state.clone());
        drop(body);

        let abort: Value = serde_json::from_str(stdin_rx.try_recv().unwrap().trim()).unwrap();
        assert_eq!(abort["method"], "abort");
        assert_eq!(abort["params"]["id"], 6);
        assert!(finished.load(Ordering::SeqCst));
        assert!(state.lock().await.pending.is_empty());
    }
}
