//! Idempotent comment delivery, including uncertain provider responses.
use super::{Context, api};
use serde_json::{Value, json};
use std::io;

pub(super) struct Github<'a> {
    pub context: &'a Context,
    pub endpoint: String,
}
pub(super) trait Provider: Sync {
    fn list(&self, page: u32) -> impl Future<Output = io::Result<Vec<Value>>> + Send;
    fn post(&self, body: &str) -> impl Future<Output = io::Result<()>> + Send;
}
impl Provider for Github<'_> {
    async fn list(&self, page: u32) -> io::Result<Vec<Value>> {
        let response = api(
            self.context,
            reqwest::Method::GET,
            &self.endpoint,
            &[("per_page", "100".into()), ("page", page.to_string())],
            None,
        )
        .await?
        .ok_or_else(|| io::Error::other("PR comments unavailable"))?;
        response
            .as_array()
            .cloned()
            .ok_or_else(|| io::Error::other("invalid comments response"))
    }
    async fn post(&self, body: &str) -> io::Result<()> {
        let comment = api(
            self.context,
            reqwest::Method::POST,
            &self.endpoint,
            &[],
            Some(json!({"body":body})),
        )
        .await?
        .ok_or_else(|| io::Error::other("quality comment response uncertain"))?;
        if comment["id"].as_u64().is_none_or(|id| id == 0) || comment["body"] != body {
            return Err(io::Error::other("quality comment receipt invalid"));
        }
        Ok(())
    }
}
pub(super) async fn publish(provider: &impl Provider, body: &str) -> io::Result<()> {
    // Exact report matching binds attempt, evidence digest and commit, and also
    // recovers a POST which GitHub accepted before the response was lost.
    for page in 1..=20 {
        let comments = provider.list(page).await?;
        if comments
            .iter()
            .any(|comment| comment["body"].as_str() == Some(body))
        {
            return Ok(());
        }
        if comments.len() < 100 {
            return provider.post(body).await;
        }
    }
    Err(io::Error::other("PR comment recovery exceeds page bound"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct UncertainProvider {
        comments: Mutex<Vec<Value>>,
        uncertain: AtomicBool,
        posts: AtomicUsize,
    }
    impl Provider for UncertainProvider {
        async fn list(&self, page: u32) -> io::Result<Vec<Value>> {
            Ok(self
                .comments
                .lock()
                .map_err(|_| io::Error::other("poisoned fixture"))?
                .iter()
                .skip((page as usize - 1) * 100)
                .take(100)
                .cloned()
                .collect())
        }
        async fn post(&self, body: &str) -> io::Result<()> {
            self.comments
                .lock()
                .map_err(|_| io::Error::other("poisoned fixture"))?
                .push(json!({"body":body}));
            self.posts.fetch_add(1, Ordering::SeqCst);
            if self.uncertain.swap(false, Ordering::SeqCst) {
                Err(io::Error::other("provider response uncertain"))
            } else {
                Ok(())
            }
        }
    }
    #[tokio::test]
    async fn lost_post_response_is_recovered_without_a_duplicate_comment() -> io::Result<()> {
        let provider = UncertainProvider {
            comments: Mutex::new(vec![json!({"body":"older report"}); 100]),
            uncertain: AtomicBool::new(true),
            posts: AtomicUsize::new(0),
        };
        let body = "<!-- ayni-quality attempt:1 digest:sha256:a -->\n# ayni check";
        assert!(publish(&provider, body).await.is_err());
        publish(&provider, body).await?;
        publish(&provider, body).await?;
        assert_eq!(provider.posts.load(Ordering::SeqCst), 1);
        // A later attempt has distinct evidence and deserves its own comment.
        publish(
            &provider,
            "<!-- ayni-quality attempt:2 digest:sha256:b -->\n# ayni check",
        )
        .await?;
        assert_eq!(provider.posts.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
