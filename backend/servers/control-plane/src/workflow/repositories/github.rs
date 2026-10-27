//! Fixed-origin, bounded GitHub metadata access. No provider text or credentials leave this module.
use super::super::connections::github::{app_jwt, classify_status};
use ayni_orchestrator_workflow_postgres::store::repositories::{
    ObservedRepository, SourceCredential,
};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) struct Github {
    client: Client,
    jwt: String,
    app_id: u64,
}
pub(super) struct Installation {
    pub id: u64,
    pub login: String,
    pub suspended: bool,
}
impl Github {
    pub fn new(source: &SourceCredential) -> Result<Self, &'static str> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "github_unavailable")?,
            jwt: app_jwt(&source.app_id, &source.private_key_pem)?,
            app_id: source
                .app_id
                .parse()
                .map_err(|_| "github_response_invalid")?,
        })
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> Result<Value, &'static str> {
        let expected_status = if method == Method::POST {
            reqwest::StatusCode::CREATED
        } else {
            reqwest::StatusCode::OK
        };
        let mut request = self
            .client
            .request(method, format!("https://api.github.com{path}"))
            .header("User-Agent", "ayni-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| "github_unavailable")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("github_repository_unavailable");
        }
        let limited = response
            .headers()
            .get("x-ratelimit-remaining")
            .is_some_and(|v| v == "0")
            || response.headers().contains_key("retry-after");
        if response.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return Err("github_access_denied");
        }
        if response.status() != expected_status {
            classify_status(response.status().as_u16(), limited)?;
            return Err("github_response_invalid");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "github_unavailable")? {
            if chunk.len() > 1_048_576_usize.saturating_sub(bytes.len()) {
                return Err("github_response_invalid");
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| "github_response_invalid")
    }
    pub async fn installations(&self, page: u32) -> Result<Vec<Installation>, &'static str> {
        let value = self
            .request(
                Method::GET,
                &format!("/app/installations?per_page=100&page={page}"),
                &self.jwt,
                None,
            )
            .await?;
        let items = value
            .as_array()
            .filter(|v| v.len() <= 100)
            .ok_or("github_response_invalid")?;
        items
            .iter()
            .map(|v| {
                Ok(Installation {
                    id: id(v, "id")?,
                    login: text(&v["account"], "login", 100)?,
                    suspended: !v["suspended_at"].is_null(),
                })
            })
            .collect()
    }
    async fn installation_token(
        &self,
        installation: u64,
        repo: Option<u64>,
    ) -> Result<String, &'static str> {
        let observed = self
            .request(
                Method::GET,
                &format!("/app/installations/{installation}"),
                &self.jwt,
                None,
            )
            .await?;
        if id(&observed, "id")? != installation
            || id(&observed, "app_id")? != self.app_id
            || !observed["suspended_at"].is_null()
        {
            return Err("github_access_denied");
        }
        let mut body = json!({"permissions":{"contents":"read","metadata":"read"}});
        if let Some(repo) = repo {
            body["repository_ids"] = json!([repo]);
        }
        let value = self
            .request(
                Method::POST,
                &format!("/app/installations/{installation}/access_tokens"),
                &self.jwt,
                Some(body),
            )
            .await?;
        if value["permissions"]["contents"] != "read" || value["permissions"]["metadata"] != "read"
        {
            return Err("github_access_denied");
        }
        text(&value, "token", 4096)
    }
    pub async fn repositories(
        &self,
        installation: u64,
        page: u32,
    ) -> Result<Vec<ObservedRepository>, &'static str> {
        let token = self.installation_token(installation, None).await?;
        let value = self
            .request(
                Method::GET,
                &format!("/installation/repositories?per_page=100&page={page}"),
                &token,
                None,
            )
            .await?;
        let items = value["repositories"]
            .as_array()
            .filter(|v| v.len() <= 100)
            .ok_or("github_response_invalid")?;
        items.iter().map(parse_repository).collect()
    }
    pub async fn repository(
        &self,
        installation: u64,
        repo: u64,
    ) -> Result<ObservedRepository, &'static str> {
        let token = self.installation_token(installation, Some(repo)).await?;
        let value = self
            .request(Method::GET, &format!("/repositories/{repo}"), &token, None)
            .await?;
        let result = parse_repository(&value)?;
        if result.provider_repository_id != repo {
            return Err("github_response_invalid");
        }
        Ok(result)
    }
}
fn id(value: &Value, field: &str) -> Result<u64, &'static str> {
    value[field]
        .as_u64()
        .filter(|v| *v > 0 && i64::try_from(*v).is_ok())
        .ok_or("github_response_invalid")
}
fn text(value: &Value, field: &str, max: usize) -> Result<String, &'static str> {
    value[field]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= max && !v.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or("github_response_invalid")
}
fn parse_repository(value: &Value) -> Result<ObservedRepository, &'static str> {
    let name = text(value, "full_name", 256)?;
    if name.split('/').count() != 2
        || name.split('/').any(str::is_empty)
        || !name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"-._/".contains(&v))
    {
        return Err("github_response_invalid");
    }
    Ok(ObservedRepository {
        provider_repository_id: id(value, "id")?,
        full_name: name,
        default_branch: text(value, "default_branch", 256)?,
        private: value["private"]
            .as_bool()
            .ok_or("github_response_invalid")?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_identity_shapes_and_unbounded_provider_text() {
        for value in [json!({"id":0}), json!({"id":u64::MAX})] {
            assert!(id(&value, "id").is_err());
        }
        let mut repo =
            json!({"id":42,"full_name":"owner/repo","default_branch":"main","private":true});
        assert!(parse_repository(&repo).is_ok_and(|v| v.provider_repository_id == 42));
        repo["full_name"] = json!("https://evil.test/repo");
        assert!(parse_repository(&repo).is_err());
        repo["full_name"] = json!("owner/repo");
        repo["default_branch"] = json!("x".repeat(257));
        assert!(parse_repository(&repo).is_err());
        assert_eq!(
            Github::new(&SourceCredential {
                app_id: "1".into(),
                private_key_pem: "fixture".into()
            })
            .err(),
            Some("github_key_invalid")
        );
    }
}
