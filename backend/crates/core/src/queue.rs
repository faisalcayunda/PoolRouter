//! Transport-independent queue ports.

use std::future::Future;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("queue transport: {0}")]
    Transport(String),
    #[error("handler: {0}")]
    Handler(String),
}

pub trait JobPublisher: Send + Sync {
    fn publish(&self, queue: &str, request_id: Uuid) -> impl Future<Output = Result<(), QueueError>> + Send;
}

pub trait JobConsumer: Send + Sync {
    fn run<H, F>(&self, queue: &str, handler: H) -> impl Future<Output = Result<(), QueueError>> + Send
    where
        H: Fn(Uuid) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), QueueError>> + Send + 'static;
}
