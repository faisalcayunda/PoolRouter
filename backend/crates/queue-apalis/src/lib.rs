//! Queue adapter (ADR-0001 section 6, ADR-0004 section 4.3).
//!
//! RabbitMQ adapter: lapin confirms publishes; apalis-amqp owns consumption.

// Keep the pinned dependencies linked so the dependency tree is checked in CI.
use apalis as _;
use apalis_amqp as _;
use lapin as _;
use poolrouter_core as _;

use lapin::{options::{BasicPublishOptions, ConfirmSelectOptions, ExchangeDeclareOptions, QueueDeclareOptions}, types::FieldTable, BasicProperties, Channel, Connection, ConnectionProperties};
use poolrouter_core::queue::{JobPublisher, JobConsumer, QueueError};
use uuid::Uuid;
use std::future::Future;

pub struct ApalisQueue {
    channel: Channel,
}

impl ApalisQueue {
    pub async fn connect(uri: &str) -> Result<Self, QueueError> {
        let connection = Connection::connect(uri, ConnectionProperties::default()).await
            .map_err(|e| QueueError::Transport(e.to_string()))?;
        let channel = connection.create_channel().await.map_err(|e| QueueError::Transport(e.to_string()))?;
        channel.confirm_select(ConfirmSelectOptions::default()).await.map_err(|e| QueueError::Transport(e.to_string()))?;
        Ok(Self { channel })
    }

    pub async fn declare(&self, queue: &str) -> Result<(), QueueError> {
        self.channel.exchange_declare(queue.into(), lapin::ExchangeKind::Direct, ExchangeDeclareOptions::default(), FieldTable::default()).await.map_err(|e| QueueError::Transport(e.to_string()))?;
        self.channel.queue_declare(queue.into(), QueueDeclareOptions::default(), FieldTable::default()).await.map_err(|e| QueueError::Transport(e.to_string()))?;
        Ok(())
    }
}

impl JobPublisher for ApalisQueue {
    async fn publish(&self, queue: &str, request_id: Uuid) -> Result<(), QueueError> {
        let confirm = self.channel.basic_publish(queue.into(), queue.into(), BasicPublishOptions { mandatory: true, ..Default::default() }, request_id.to_string().as_bytes(), BasicProperties::default().with_delivery_mode(2).with_message_id(request_id.to_string().into())).await.map_err(|e| QueueError::Transport(e.to_string()))?;
        confirm.await.map_err(|e| QueueError::Transport(e.to_string())).map(|_| ())
    }
}

impl JobConsumer for ApalisQueue {
    async fn run<H, F>(&self, _queue: &str, _handler: H) -> Result<(), QueueError>
    where H: Fn(Uuid) -> F + Send + Sync + 'static, F: Future<Output = Result<(), QueueError>> + Send + 'static {
        // Consumption is deliberately wired through apalis-amqp in the worker composition root.
        Err(QueueError::Transport("consumer requires apalis worker composition".into()))
    }
}
