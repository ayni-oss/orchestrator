//! Provider publication recovery observes existing work before making new mutations.
use std::future::Future;
use thiserror::Error;

pub struct Delivery {
    pub branch: String,
    pub base_branch: String,
    pub base_revision: String,
    pub head_revision: String,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewState {
    Draft,
    Open,
    Closed,
    Merged,
}

pub struct Published {
    pub number: u64,
    pub url: String,
    pub state: ReviewState,
    pub head_revision: String,
    pub branch: String,
    pub base_branch: String,
}

#[derive(Debug, Error)]
pub enum DeliveryError<E> {
    #[error("publication provider operation failed")]
    Provider(#[source] E),
    #[error("provider publication does not match its frozen identity")]
    IdentityConflict,
}

/// A repository-scoped provider adapter; credentials and protocols remain outside the application.
pub trait Provider: Sync {
    type Error: std::error::Error;

    fn find(
        &self,
        delivery: &Delivery,
    ) -> impl Future<Output = Result<Option<Published>, Self::Error>> + Send;
    fn push(&self, delivery: &Delivery) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn create(
        &self,
        delivery: &Delivery,
    ) -> impl Future<Output = Result<Published, Self::Error>> + Send;
}

/// Recover existing publication, or push and create a draft only after confirmed absence.
///
/// # Errors
/// Stops on uncertain provider failures or a mismatched returned publication identity.
pub async fn deliver<P: Provider>(
    provider: &P,
    delivery: &Delivery,
) -> Result<Published, DeliveryError<P::Error>> {
    let published = if let Some(published) = provider
        .find(delivery)
        .await
        .map_err(DeliveryError::Provider)?
    {
        published
    } else {
        provider
            .push(delivery)
            .await
            .map_err(DeliveryError::Provider)?;
        provider
            .create(delivery)
            .await
            .map_err(DeliveryError::Provider)?
    };
    if published.head_revision != delivery.head_revision
        || published.branch != delivery.branch
        || published.base_branch != delivery.base_branch
    {
        return Err(DeliveryError::IdentityConflict);
    }
    Ok(published)
}

#[cfg(test)]
mod tests;
