use rig_core::{client::ModelListingClient, providers::openai};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::Mutex;

// Keep one provider snapshot available for at least the minimum page-token lifetime.
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const FAILURE_BACKOFF: Duration = Duration::from_secs(30);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MODELS: usize = 2_048;
const MAX_MODEL_ID_BYTES: usize = 128;
const MAX_MODEL_NAME_BYTES: usize = 256;
const MAX_OWNER_BYTES: usize = 128;
const PROVIDER: &str = "openai";

/// Provider-reported model metadata safe to return to the browser.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AvailableModel {
    pub id: String,
    pub name: Option<String>,
    pub owned_by: Option<String>,
    pub created_at: Option<u64>,
    pub context_length: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AvailableModels {
    pub provider: &'static str,
    pub models: Vec<AvailableModel>,
    #[serde(skip_serializing)]
    pub observed_at: u64,
}

#[derive(Debug, Error)]
pub enum ModelCatalogError {
    #[error("could not initialize the OpenAI model catalog")]
    Initialization,
    #[error("OpenAI model discovery failed")]
    Discovery,
    #[error("provider returned an invalid model catalog")]
    InvalidCatalog,
}

#[derive(Clone)]
pub struct ModelCatalog {
    inner: Arc<CatalogInner>,
}

struct CatalogInner {
    source: CatalogSource,
    cache: Mutex<Option<CachedModels>>,
    verified: AtomicBool,
}

enum CatalogSource {
    OpenAi(openai::Client),
    #[cfg(test)]
    Unavailable,
    #[cfg(test)]
    Fixed(Vec<AvailableModel>),
}

struct CachedModels {
    refresh_after: Instant,
    models: Option<Vec<AvailableModel>>,
    observed_at: Option<u64>,
}

impl ModelCatalog {
    /// Creates a catalog backed by `OpenAI`'s authenticated model-list endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`ModelCatalogError::Initialization`] when the Rig client cannot
    /// be constructed from the supplied credential.
    pub fn openai(api_key: &str) -> Result<Self, ModelCatalogError> {
        let client = ayni_orchestrator_providers::openai_client("openai", api_key)
            .map_err(|_| ModelCatalogError::Initialization)?;
        Ok(Self {
            inner: Arc::new(CatalogInner {
                source: CatalogSource::OpenAi(client),
                cache: Mutex::new(None),
                verified: AtomicBool::new(false),
            }),
        })
    }

    #[cfg(test)]
    pub(crate) fn fixed(models: Vec<AvailableModel>) -> Self {
        Self {
            inner: Arc::new(CatalogInner {
                source: CatalogSource::Fixed(normalize(models).unwrap_or_default()),
                cache: Mutex::new(None),
                verified: AtomicBool::new(true),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn unavailable() -> Self {
        Self {
            inner: Arc::new(CatalogInner {
                source: CatalogSource::Unavailable,
                cache: Mutex::new(None),
                verified: AtomicBool::new(false),
            }),
        }
    }

    /// Whether this catalog has completed at least one successful provider fetch.
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.inner.verified.load(Ordering::Acquire)
    }

    /// Returns the provider-reported models, using a short-lived successful cache.
    ///
    /// # Errors
    ///
    /// Returns an error when provider discovery fails or its response violates
    /// the catalog bounds.
    #[allow(clippy::significant_drop_tightening)] // The mutex deliberately provides one refresh flight.
    pub async fn available(&self) -> Result<AvailableModels, ModelCatalogError> {
        let (models, observed_at) = match &self.inner.source {
            #[cfg(test)]
            CatalogSource::Unavailable => self
                .inner
                .cache
                .lock()
                .await
                .as_ref()
                .and_then(|cached| cached.models.clone().zip(cached.observed_at))
                .ok_or(ModelCatalogError::Discovery)?,
            #[cfg(test)]
            CatalogSource::Fixed(models) => (models.clone(), observed_now()?),
            CatalogSource::OpenAi(client) => {
                let mut cache = self.inner.cache.lock().await;
                let now = Instant::now();
                if let Some(cached) = cache.as_ref().filter(|cached| now < cached.refresh_after) {
                    cached
                        .models
                        .clone()
                        .zip(cached.observed_at)
                        .ok_or(ModelCatalogError::Discovery)?
                } else {
                    let stale = cache
                        .as_ref()
                        .and_then(|cached| cached.models.clone().zip(cached.observed_at));
                    let discovered = discover(client).await;
                    match discovered {
                        Ok((models, observed_at)) => {
                            self.inner.verified.store(true, Ordering::Release);
                            *cache = Some(CachedModels {
                                refresh_after: Instant::now() + CACHE_TTL,
                                models: Some(models.clone()),
                                observed_at: Some(observed_at),
                            });
                            (models, observed_at)
                        }
                        Err(error) => {
                            *cache = Some(CachedModels {
                                refresh_after: Instant::now() + FAILURE_BACKOFF,
                                models: stale.as_ref().map(|(models, _)| models.clone()),
                                observed_at: stale.as_ref().map(|(_, observed_at)| *observed_at),
                            });
                            stale.ok_or(error)?
                        }
                    }
                }
            }
        };
        Ok(AvailableModels {
            provider: PROVIDER,
            models,
            observed_at,
        })
    }

    /// Validate admission against a fresh provider response, without stale fallback.
    ///
    /// # Errors
    /// Returns an error when the bounded provider discovery fails or is invalid.
    pub async fn contains_current(&self, model: &str) -> Result<bool, ModelCatalogError> {
        let models = match &self.inner.source {
            #[cfg(test)]
            CatalogSource::Unavailable => return Err(ModelCatalogError::Discovery),
            #[cfg(test)]
            CatalogSource::Fixed(models) => models.clone(),
            CatalogSource::OpenAi(client) => {
                let (models, observed_at) = discover(client).await?;
                self.inner.verified.store(true, Ordering::Release);
                *self.inner.cache.lock().await = Some(CachedModels {
                    refresh_after: Instant::now() + CACHE_TTL,
                    models: Some(models.clone()),
                    observed_at: Some(observed_at),
                });
                models
            }
        };
        Ok(models
            .binary_search_by(|candidate| candidate.id.as_str().cmp(model))
            .is_ok())
    }

    /// Checks whether the provider's current catalog contains an exact model ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the catalog cannot be retrieved or validated.
    pub async fn contains(&self, model: &str) -> Result<bool, ModelCatalogError> {
        Ok(self
            .available()
            .await?
            .models
            .binary_search_by(|candidate| candidate.id.as_str().cmp(model))
            .is_ok())
    }
}

async fn discover(
    client: &openai::Client,
) -> Result<(Vec<AvailableModel>, u64), ModelCatalogError> {
    let listed = tokio::time::timeout(DISCOVERY_TIMEOUT, client.list_models())
        .await
        .map_err(|_| ModelCatalogError::Discovery)?
        .map_err(|_| ModelCatalogError::Discovery)?;
    let models = normalize(
        listed
            .into_iter()
            .map(|model| AvailableModel {
                id: model.id,
                name: model.name,
                owned_by: model.owned_by,
                created_at: model.created_at,
                context_length: model.context_length,
                max_output_tokens: model.max_output_tokens,
            })
            .collect(),
    )?;
    Ok((models, observed_now()?))
}

fn observed_now() -> Result<u64, ModelCatalogError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| ModelCatalogError::InvalidCatalog)
}

fn agent_compatible(id: &str) -> bool {
    matches!(id, "gpt-5.6-luna" | "gpt-5.6-terra" | "gpt-5.6-sol")
}

fn normalize(models: Vec<AvailableModel>) -> Result<Vec<AvailableModel>, ModelCatalogError> {
    if models.len() > MAX_MODELS {
        return Err(ModelCatalogError::InvalidCatalog);
    }
    let mut unique = BTreeMap::new();
    for mut model in models {
        if model.id.is_empty()
            || model.id.len() > MAX_MODEL_ID_BYTES
            || !model
                .id
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
        {
            return Err(ModelCatalogError::InvalidCatalog);
        }
        if model
            .created_at
            .is_some_and(|seconds| i64::try_from(seconds).is_err())
        {
            return Err(ModelCatalogError::InvalidCatalog);
        }
        if !agent_compatible(&model.id) {
            continue;
        }
        model.name = model.name.filter(|name| {
            name.len() <= MAX_MODEL_NAME_BYTES && !name.chars().any(char::is_control)
        });
        model.owned_by = model
            .owned_by
            .filter(|owner| owner.len() <= MAX_OWNER_BYTES && !owner.chars().any(char::is_control));
        unique.entry(model.id.clone()).or_insert(model);
    }
    Ok(unique.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::{AvailableModel, ModelCatalog};

    fn model(id: &str) -> AvailableModel {
        AvailableModel {
            id: id.into(),
            name: None,
            owned_by: Some("openai".into()),
            created_at: None,
            context_length: None,
            max_output_tokens: None,
        }
    }

    #[tokio::test]
    async fn admission_validation_never_uses_a_stale_catalog_after_provider_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let catalog = ModelCatalog::unavailable();
        *catalog.inner.cache.lock().await = Some(super::CachedModels {
            refresh_after: std::time::Instant::now(),
            models: Some(vec![model("gpt-5.6-sol")]),
            observed_at: Some(1),
        });
        assert!(catalog.contains("gpt-5.6-sol").await?);
        assert!(catalog.contains_current("gpt-5.6-sol").await.is_err());
        let fixed = ModelCatalog::fixed(vec![model("gpt-5.6-sol")]);
        assert!(fixed.contains_current("gpt-5.6-sol").await?);
        assert!(!fixed.contains_current("missing").await?);
        Ok(())
    }

    #[tokio::test]
    async fn fixed_catalog_is_sorted_deduplicated_and_searchable()
    -> Result<(), Box<dyn std::error::Error>> {
        let catalog = ModelCatalog::fixed(vec![
            model("gpt-5.6-sol"),
            model("text-embedding-3-large"),
            model("gpt-4o-realtime-preview"),
            model("gpt-5.6-terra"),
            model("gpt-5.6-luna"),
            model("gpt-5"),
            model("gpt-5.6-sol"),
        ]);
        let available = catalog.available().await?;
        assert_eq!(available.provider, "openai");
        assert_eq!(
            available
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["gpt-5.6-luna", "gpt-5.6-sol", "gpt-5.6-terra"]
        );
        assert!(catalog.contains("gpt-5.6-sol").await?);
        assert!(!catalog.contains("luna").await?);
        assert!(!catalog.contains("gpt-5").await?);
        assert!(!catalog.contains("gpt-6-sol").await?);
        Ok(())
    }
}
