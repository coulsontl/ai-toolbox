use super::middleware::{ErrorDecision, Middleware, PipelineContext};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) struct ExecutorRequest {
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) struct ExecutorResponse {
    pub body: Vec<u8>,
}

#[allow(dead_code)]
pub(super) type ExecutorFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ExecutorResponse, String>> + Send + 'a>>;

#[allow(dead_code)]
pub(super) trait PipelineExecutor: Send + Sync {
    fn execute<'a>(&'a self, request: ExecutorRequest) -> ExecutorFuture<'a>;
}

#[allow(dead_code)]
pub(super) trait ChannelCustomizedExecutor: Send + Sync {
    fn customize_executor(&self, executor: Arc<dyn PipelineExecutor>) -> Arc<dyn PipelineExecutor>;
}

#[derive(Default, Clone)]
pub(super) struct Pipeline {
    middleware: Vec<Arc<dyn Middleware>>,
    executor_customizer: Option<Arc<dyn ChannelCustomizedExecutor>>,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field("middleware_len", &self.middleware.len())
            .field(
                "has_executor_customizer",
                &self.executor_customizer.is_some(),
            )
            .finish()
    }
}

impl Pipeline {
    #[allow(dead_code)]
    pub(super) fn new(middleware: Vec<Arc<dyn Middleware>>) -> Self {
        Self {
            middleware,
            executor_customizer: None,
        }
    }

    #[allow(dead_code)]
    pub(super) fn with_executor_customizer(
        middleware: Vec<Arc<dyn Middleware>>,
        executor_customizer: Arc<dyn ChannelCustomizedExecutor>,
    ) -> Self {
        Self {
            middleware,
            executor_customizer: Some(executor_customizer),
        }
    }

    #[allow(dead_code)]
    pub(super) fn run_inbound_request(
        &self,
        body: &mut Value,
        ctx: &mut PipelineContext,
    ) -> Result<(), String> {
        for middleware in &self.middleware {
            middleware.on_inbound_request(body, ctx)?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(super) fn run_outbound_body(
        &self,
        body: &mut Value,
        ctx: &PipelineContext,
    ) -> Result<(), String> {
        for middleware in &self.middleware {
            middleware.on_outbound_body(body, ctx)?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(super) fn run_stream_chunk(
        &self,
        chunk: &mut Value,
        ctx: &mut PipelineContext,
    ) -> Result<(), String> {
        for middleware in &self.middleware {
            middleware.on_stream_chunk(chunk, ctx)?;
        }
        Ok(())
    }

    /// Client-facing response path: reverse order (M_n → … → M_1), AxonHub-style.
    pub(super) fn run_outbound_response(
        &self,
        body: &mut Value,
        ctx: &PipelineContext,
    ) -> Result<(), String> {
        for middleware in self.middleware.iter().rev() {
            middleware.on_outbound_response(body, ctx)?;
        }
        Ok(())
    }

    /// Client-facing stream path: reverse order (M_n → … → M_1).
    pub(super) fn run_outbound_stream(
        &self,
        chunk: &mut Value,
        ctx: &mut PipelineContext,
    ) -> Result<(), String> {
        for middleware in self.middleware.iter().rev() {
            middleware.on_outbound_stream(chunk, ctx)?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(super) fn decide_error(&self, message: &str, ctx: &PipelineContext) -> ErrorDecision {
        for middleware in &self.middleware {
            if middleware.on_error(message, ctx) == ErrorDecision::Retry {
                return ErrorDecision::Retry;
            }
        }
        ErrorDecision::Propagate
    }

    #[allow(dead_code)]
    pub(super) async fn execute(
        &self,
        executor: Arc<dyn PipelineExecutor>,
        request: ExecutorRequest,
    ) -> Result<ExecutorResponse, String> {
        let executor = self
            .executor_customizer
            .as_ref()
            .map(|customizer| customizer.customize_executor(executor.clone()))
            .unwrap_or(executor);
        executor.execute(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct EchoExecutor;

    impl PipelineExecutor for EchoExecutor {
        fn execute<'a>(&'a self, request: ExecutorRequest) -> ExecutorFuture<'a> {
            Box::pin(async move { Ok(ExecutorResponse { body: request.body }) })
        }
    }

    struct PrefixExecutor {
        prefix: &'static [u8],
    }

    impl PipelineExecutor for PrefixExecutor {
        fn execute<'a>(&'a self, request: ExecutorRequest) -> ExecutorFuture<'a> {
            Box::pin(async move {
                let mut body = self.prefix.to_vec();
                body.extend(request.body);
                Ok(ExecutorResponse { body })
            })
        }
    }

    struct FailingExecutor;

    impl PipelineExecutor for FailingExecutor {
        fn execute<'a>(&'a self, _request: ExecutorRequest) -> ExecutorFuture<'a> {
            Box::pin(async move { Err("custom executor failed".to_string()) })
        }
    }

    struct TestCustomizer {
        called: AtomicBool,
        executor: Arc<dyn PipelineExecutor>,
    }

    impl ChannelCustomizedExecutor for TestCustomizer {
        fn customize_executor(
            &self,
            _executor: Arc<dyn PipelineExecutor>,
        ) -> Arc<dyn PipelineExecutor> {
            self.called.store(true, Ordering::SeqCst);
            self.executor.clone()
        }
    }

    #[tokio::test]
    async fn pipeline_uses_channel_customized_executor() {
        let customizer = Arc::new(TestCustomizer {
            called: AtomicBool::new(false),
            executor: Arc::new(PrefixExecutor { prefix: b"custom:" }),
        });
        let pipeline = Pipeline::with_executor_customizer(Vec::new(), customizer.clone());
        let response = pipeline
            .execute(
                Arc::new(EchoExecutor),
                ExecutorRequest {
                    body: b"body".to_vec(),
                },
            )
            .await
            .unwrap();

        assert!(customizer.called.load(Ordering::SeqCst));
        assert_eq!(response.body, b"custom:body");
    }

    #[tokio::test]
    async fn pipeline_propagates_channel_customized_executor_error() {
        let pipeline = Pipeline::with_executor_customizer(
            Vec::new(),
            Arc::new(TestCustomizer {
                called: AtomicBool::new(false),
                executor: Arc::new(FailingExecutor),
            }),
        );
        let error = pipeline
            .execute(
                Arc::new(EchoExecutor),
                ExecutorRequest {
                    body: b"body".to_vec(),
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error, "custom executor failed");
    }
}
