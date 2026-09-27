use crate::{
    development::AgentProfile,
    orchestration::{Store, StoreError},
};
use rig_core::{client::ModelListingClient, providers::openai};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::Mutex;

const CACHE_TTL: Duration = Duration::from_secs(300);
const FAILURE_BACKOFF: Duration = Duration::from_secs(30);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MODELS: usize = 2_048;
const MAX_MODEL_ID_BYTES: usize = 128;
const MAX_MODEL_NAME_BYTES: usize = 256;
const MAX_OWNER_BYTES: usize = 128;
const PROVIDER: &str = "openai";
const SECRET_REF: &str = "openai-api";

/// Provider-reported model metadata safe to return to the browser.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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
    Fixed(Vec<AvailableModel>),
}

struct CachedModels {
    refresh_after: Instant,
    models: Option<Vec<AvailableModel>>,
}

impl ModelCatalog {
    /// Creates a catalog backed by `OpenAI`'s authenticated model-list endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`ModelCatalogError::Initialization`] when the Rig client cannot
    /// be constructed from the supplied credential.
    pub fn openai(api_key: &str) -> Result<Self, ModelCatalogError> {
        let client = openai::Client::new(api_key).map_err(|_| ModelCatalogError::Initialization)?;
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
        let models = match &self.inner.source {
            #[cfg(test)]
            CatalogSource::Fixed(models) => models.clone(),
            CatalogSource::OpenAi(client) => {
                let mut cache = self.inner.cache.lock().await;
                let now = Instant::now();
                if let Some(cached) = cache.as_ref().filter(|cached| now < cached.refresh_after) {
                    cached.models.clone().ok_or(ModelCatalogError::Discovery)?
                } else {
                    let stale = cache.as_ref().and_then(|cached| cached.models.clone());
                    let discovered = async {
                        let listed = tokio::time::timeout(DISCOVERY_TIMEOUT, client.list_models())
                            .await
                            .map_err(|_| ModelCatalogError::Discovery)?
                            .map_err(|_| ModelCatalogError::Discovery)?;
                        normalize(
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
                        )
                    }
                    .await;
                    match discovered {
                        Ok(models) => {
                            self.inner.verified.store(true, Ordering::Release);
                            *cache = Some(CachedModels {
                                refresh_after: Instant::now() + CACHE_TTL,
                                models: Some(models.clone()),
                            });
                            models
                        }
                        Err(error) => {
                            *cache = Some(CachedModels {
                                refresh_after: Instant::now() + FAILURE_BACKOFF,
                                models: stale.clone(),
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
        })
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

impl Store {
    pub(crate) fn discovered_model_profile_id(model: &str) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let digest = Sha256::digest(model.as_bytes());
        let mut suffix = String::with_capacity(32);
        for byte in &digest[..16] {
            suffix.push(char::from(HEX[usize::from(byte >> 4)]));
            suffix.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        format!("openai-{suffix}")
    }

    pub(crate) fn put_discovered_model(&self, model: &str) -> Result<String, StoreError> {
        let profile = AgentProfile {
            id: Self::discovered_model_profile_id(model),
            provider: PROVIDER.into(),
            model: model.into(),
            secret_ref: SECRET_REF.into(),
        };
        self.put_agent_profile(&profile)?;
        Ok(profile.id)
    }
}

fn agent_compatible(id: &str) -> bool {
    let supported_family = ["gpt-5", "gpt-4.1", "gpt-4o", "o1", "o3", "o4"]
        .iter()
        .any(|prefix| id.starts_with(prefix));
    let unsupported_variant = [
        "audio",
        "realtime",
        "transcribe",
        "tts",
        "image",
        "search",
        "instruct",
    ]
    .iter()
    .any(|marker| id.contains(marker));
    supported_family && !unsupported_variant
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
    async fn fixed_catalog_is_sorted_deduplicated_and_searchable()
    -> Result<(), Box<dyn std::error::Error>> {
        let catalog = ModelCatalog::fixed(vec![
            model("gpt-5"),
            model("text-embedding-3-large"),
            model("gpt-4o-realtime-preview"),
            model("gpt-4.1"),
            model("gpt-5"),
        ]);
        let available = catalog.available().await?;
        assert_eq!(available.provider, "openai");
        assert_eq!(
            available
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["gpt-4.1", "gpt-5"]
        );
        assert!(catalog.contains("gpt-5").await?);
        assert!(!catalog.contains("luna").await?);
        Ok(())
    }
}
