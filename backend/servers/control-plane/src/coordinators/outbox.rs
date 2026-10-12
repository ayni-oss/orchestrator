//! Independent, bounded delivery of already accepted facts.
use super::{AppState, ReconciliationError};
use crate::adapters::sqlite::verification_outbox::Publication;
use async_nats::{HeaderMap, jetstream};
use sha2::{Digest, Sha256};
use std::{env, time::Duration};
use tokio::sync::Mutex;

const SUBJECT: &str = "orchestrator.runtime.verification.accepted.v1";
const STREAM: &str = "ORCHESTRATOR_LIFECYCLE";
const TIMEOUT: Duration = Duration::from_secs(2);

/// Opt-in local transport; no connection is attempted during application startup.
pub struct OutboxPublisher {
    state: AppState,
    endpoint: String,
    connection: Mutex<Option<jetstream::Context>>,
}

impl OutboxPublisher {
    /// Read the explicitly trusted local profile without exposing endpoint contents.
    /// # Errors
    /// Rejects credentials in URLs and unsupported production configuration.
    pub fn from_environment(state: AppState) -> Result<Option<Self>, &'static str> {
        let endpoint = env::var("ORCHESTRATOR_OUTBOX_NATS_URL").ok();
        let local = env::var("ORCHESTRATOR_OUTBOX_LOCAL_DEVELOPMENT").ok();
        Self::from_values(state, endpoint.as_deref(), local.as_deref())
    }

    /// Construct the publisher with validated, credential-free local configuration.
    /// # Errors
    /// Rejects unsafe or unsupported endpoints.
    pub fn from_values(
        state: AppState,
        endpoint: Option<&str>,
        local: Option<&str>,
    ) -> Result<Option<Self>, &'static str> {
        let Some(endpoint) = endpoint.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let url = url::Url::parse(endpoint).map_err(|_| "invalid outbox endpoint")?;
        if endpoint.len() > 256
            || local != Some("true")
            || url.scheme() != "nats"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
            || url.port() == Some(0)
        {
            return Err(
                "outbox requires an explicit trusted local nats endpoint without credentials",
            );
        }
        Ok(Some(Self {
            state,
            endpoint: endpoint.into(),
            connection: Mutex::new(None),
        }))
    }

    /// Deliver at most eight due records, retaining immutable bytes on every error.
    /// # Errors
    /// Reports persistence failure; broker failure remains pending work.
    pub async fn publish_once(&self) -> Result<(), ReconciliationError> {
        let now =
            crate::adapters::sqlite::unix_seconds().map_err(|_| ReconciliationError::Store)?;
        let publications = self
            .state
            .database
            .execute(move |s| s.claim_publications(now))
            .await
            .map_err(|_| ReconciliationError::Store)?;
        self.state.refresh_outbox_metrics().await?;
        let mut connection = self.connection.lock().await;
        for publication in publications {
            let outcome =
                tokio::time::timeout(TIMEOUT, self.publish(&mut connection, &publication))
                    .await
                    .ok()
                    .flatten();
            let sequence = outcome.map(|(sequence, _)| sequence);
            if outcome.is_none() {
                *connection = None;
            }
            let metric = match outcome {
                Some((_, true)) => "duplicate",
                Some(_) => "stored",
                None => "failed",
            };
            crate::process::metrics::record_outbox_publication(metric);
            self.state
                .database
                .execute(move |s| s.record_publication(&publication, sequence))
                .await
                .map_err(|_| ReconciliationError::Store)?;
        }
        drop(connection);
        self.state.refresh_outbox_metrics().await
    }

    async fn publish(
        &self,
        connection: &mut Option<jetstream::Context>,
        publication: &Publication,
    ) -> Option<(u64, bool)> {
        // Validate the persisted envelope without serializing it again.
        let (identity, _) = ayni_orchestrator_events::decisions::decode(&publication.bytes).ok()?;
        if identity != publication.identity {
            return None;
        }
        if connection.is_none() {
            let client = async_nats::ConnectOptions::new()
                .connection_timeout(TIMEOUT)
                .max_reconnects(1)
                .client_capacity(16)
                .subscription_capacity(16)
                .connect(&self.endpoint)
                .await
                .ok()?;
            let mut context = jetstream::new(client);
            context.set_timeout(TIMEOUT);
            *connection = Some(context);
        }
        let mut headers = HeaderMap::new();
        headers.insert("Nats-Msg-Id", message_id(&identity));
        headers.insert("Nats-Expected-Stream", STREAM);
        let ack = connection
            .as_ref()?
            .publish_with_headers(SUBJECT, headers, publication.bytes.clone().into())
            .await
            .ok()?
            .await
            .ok()?;
        if ack.stream != STREAM || ack.sequence == 0 {
            return None;
        }
        Some((ack.sequence, ack.duplicate))
    }
}

// Length-prefixing prevents ambiguous concatenations of source and ID.
pub fn message_id(identity: &ayni_orchestrator_events::EventIdentity) -> String {
    let mut hash = Sha256::new();
    hash.update(
        u64::try_from(identity.source.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(identity.source.as_bytes());
    hash.update(identity.id.as_bytes());
    format!("{:x}", hash.finalize())
}

impl AppState {
    pub(crate) async fn refresh_outbox_metrics(&self) -> Result<(), ReconciliationError> {
        let stats = self
            .database
            .execute(|s| s.outbox_stats())
            .await
            .map_err(|_| ReconciliationError::Store)?;
        crate::process::metrics::set_outbox_stats(stats);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
