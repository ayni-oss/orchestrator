//! Bounded durable pull delivery; `SQLite` commit always precedes acknowledgement.
use super::{AppState, OutboxPublisher};
use async_nats::jetstream::{
    self,
    consumer::{AckPolicy, DeliverPolicy, PullConsumer, pull},
};
use std::{env, time::Duration};
use tokio::sync::Mutex;
use tokio_stream::StreamExt;

const STREAM: &str = "ORCHESTRATOR_LIFECYCLE";
const SUBJECT: &str = "orchestrator.runtime.verification.accepted.v1";
const DURABLE: &str = "verification-cleanup-v1";
const TIMEOUT: Duration = Duration::from_secs(2);

pub struct VerificationConsumer {
    state: AppState,
    endpoint: String,
    consumer: Mutex<Option<PullConsumer>>,
}

impl VerificationConsumer {
    /// Enable only the explicitly trusted local publisher endpoint.
    /// # Errors
    /// Rejects unsupported flags or missing trusted-local transport.
    pub fn from_environment(state: AppState) -> Result<Option<Self>, &'static str> {
        match env::var("ORCHESTRATOR_VERIFICATION_CONSUMER_ENABLED").as_deref() {
            Err(_) | Ok("false") => Ok(None),
            Ok("true") => {
                let publisher = OutboxPublisher::from_environment(state.clone())?
                    .ok_or("verification consumer requires local outbox transport")?;
                Ok(Some(Self {
                    state,
                    endpoint: publisher.endpoint,
                    consumer: Mutex::new(None),
                }))
            }
            _ => Err("invalid verification consumer flag"),
        }
    }

    /// One bounded pass. Failures preserve broker redelivery and application readiness.
    pub async fn consume_once(&self) {
        let mut consumer = self.consumer.lock().await;
        let result =
            tokio::time::timeout(Duration::from_secs(12), self.consume(&mut consumer)).await;
        if !matches!(result, Ok(Ok(()))) {
            *consumer = None;
            drop(consumer);
            crate::process::metrics::record_consumer_delivery("deferred");
            tracing::warn!(
                reason_code = "verification_delivery_deferred",
                "accepted-event delivery deferred"
            );
        }
    }

    async fn consume(&self, cached: &mut Option<PullConsumer>) -> Result<(), ()> {
        if cached.is_none() {
            let consumer = tokio::time::timeout(TIMEOUT, self.connect())
                .await
                .map_err(|_| ())??;
            *cached = Some(consumer);
        }
        let consumer = cached.as_mut().ok_or(())?;
        let info = consumer.info().await.map_err(|_| ())?;
        if !compatible(&info.config) {
            return Err(());
        }
        crate::process::metrics::set_consumer_lag(info.num_pending, info.num_ack_pending);
        let mut messages = consumer
            .fetch()
            .max_messages(8)
            .max_bytes(131_072)
            .expires(Duration::from_millis(500))
            .messages()
            .await
            .map_err(|_| ())?;
        while let Some(message) = messages.next().await {
            let message = message.map_err(|_| ())?;
            let info = message.info().map_err(|_| ())?;
            if info.stream != STREAM
                || info.consumer != DURABLE
                || message.subject.as_str() != SUBJECT
            {
                return Err(());
            }
            let bytes = message.payload.to_vec();
            let outcome = self
                .state
                .database
                .execute(move |store| store.consume_verification(&bytes))
                .await
                .map_err(|_| ())?;
            crate::process::metrics::record_consumer_delivery(outcome);
            tokio::time::timeout(TIMEOUT, message.double_ack())
                .await
                .map_err(|_| ())?
                .map_err(|_| ())?;
        }
        let info = consumer.info().await.map_err(|_| ())?;
        crate::process::metrics::set_consumer_lag(info.num_pending, info.num_ack_pending);
        Ok(())
    }

    async fn connect(&self) -> Result<PullConsumer, ()> {
        let client = async_nats::ConnectOptions::new()
            .connection_timeout(TIMEOUT)
            .max_reconnects(1)
            .client_capacity(16)
            .subscription_capacity(16)
            .connect(&self.endpoint)
            .await
            .map_err(|_| ())?;
        let mut context = jetstream::new(client);
        context.set_timeout(TIMEOUT);
        let stream = context.get_stream(STREAM).await.map_err(|_| ())?;
        let consumer = stream
            .get_or_create_consumer(DURABLE, configuration())
            .await
            .map_err(|_| ())?;
        if !compatible(&consumer.cached_info().config) {
            return Err(());
        }
        Ok(consumer)
    }
}

fn configuration() -> pull::Config {
    pull::Config {
        durable_name: Some(DURABLE.into()),
        deliver_policy: DeliverPolicy::All,
        ack_policy: AckPolicy::Explicit,
        ack_wait: Duration::from_secs(30),
        max_deliver: -1,
        filter_subject: SUBJECT.into(),
        max_ack_pending: 8,
        max_waiting: 1,
        max_batch: 8,
        max_bytes: 131_072,
        max_expires: TIMEOUT,
        num_replicas: 1,
        ..Default::default()
    }
}

fn compatible(actual: &jetstream::consumer::Config) -> bool {
    actual.durable_name.as_deref() == Some(DURABLE)
        && actual.deliver_subject.is_none()
        && actual.deliver_policy == DeliverPolicy::All
        && actual.ack_policy == AckPolicy::Explicit
        && actual.ack_wait == Duration::from_secs(30)
        && actual.max_deliver == -1
        && actual.filter_subject == SUBJECT
        && actual.max_ack_pending == 8
        && actual.max_waiting == 1
        && actual.max_batch == 8
        && actual.max_bytes == 131_072
        && actual.max_expires == TIMEOUT
        && actual.inactive_threshold.is_zero()
        && actual.num_replicas == 1
        && !actual.memory_storage
        && !actual.headers_only
        && actual.backoff.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_nats::jetstream::consumer::IntoConsumerConfig;
    #[test]
    fn refuses_an_existing_consumer_with_unsafe_delivery_settings() {
        let mut config: jetstream::consumer::Config = configuration().into_consumer_config();
        assert!(compatible(&config));
        config.ack_policy = AckPolicy::None;
        assert!(!compatible(&config));
        config = configuration().into_consumer_config();
        config.max_ack_pending = 1000;
        assert!(!compatible(&config));
    }
}
