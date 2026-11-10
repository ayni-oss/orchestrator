//! Fixed-origin, bounded GitHub metadata access. No provider text or credentials leave this module.
use super::super::connections::github::{app_jwt, classify_status};
use ayni_orchestrator_workflow_postgres::store::repositories::{
    ObservedRepository, SourceCredential,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use sha1::{Digest as _, Sha1};
use std::collections::BTreeMap;
use std::time::Duration;

fn signing_pull_permissions(publish: bool) -> Value {
    if publish {
        json!({"contents":"write","pull_requests":"write","metadata":"read"})
    } else {
        json!({"pull_requests":"read","metadata":"read"})
    }
}

pub(super) struct Github {
    client: Client,
    origin: String,
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
            origin: "https://api.github.com".into(),
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
            .request(method, format!("{}{path}", self.origin))
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
                return Err(if path.contains("/git/") {
                    "source_bounds_exceeded"
                } else {
                    "github_response_invalid"
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| "github_response_invalid")
    }
    pub async fn branches(
        &self,
        installation: u64,
        repo: u64,
        page: u32,
    ) -> Result<(Vec<String>, bool), &'static str> {
        let token = self.installation_token(installation, Some(repo)).await?;
        let metadata = self
            .request(Method::GET, &format!("/repositories/{repo}"), &token, None)
            .await?;
        let metadata = parse_repository(&metadata)?;
        if metadata.provider_repository_id != repo || !safe_path(&metadata.full_name) {
            return Err("github_response_invalid");
        }
        let value = self
            .request(
                Method::GET,
                &format!(
                    "/repos/{}/branches?per_page=100&page={page}",
                    metadata.full_name
                ),
                &token,
                None,
            )
            .await?;
        let items = value
            .as_array()
            .filter(|items| items.len() <= 100)
            .ok_or("github_response_invalid")?;
        let branches = items
            .iter()
            .map(|item| text(item, "name", 4096))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((
            branches
                .into_iter()
                .filter(|name| name.len() <= 255 && safe_path(name))
                .collect(),
            items.len() == 100,
        ))
    }
    /// Resolve the selected branch once; subsequent inspection uses only its SHA.
    pub async fn default_commit(
        &self,
        installation: u64,
        repo: u64,
        branch: &str,
    ) -> Result<(String, String), &'static str> {
        let token = self.installation_token(installation, Some(repo)).await?;
        let metadata = self
            .request(Method::GET, &format!("/repositories/{repo}"), &token, None)
            .await?;
        let metadata = parse_repository(&metadata)?;
        if metadata.provider_repository_id != repo {
            return Err("github_response_invalid");
        }
        if !branch.is_empty() {
            if !safe_path(branch) || branch.len() > 255 {
                return Err("github_branch_invalid");
            }
            let reference = format!("refs/heads/{branch}");
            let mut url = reqwest::Url::parse("https://api.github.com")
                .map_err(|_| "github_response_invalid")?;
            url.path_segments_mut()
                .map_err(|()| "github_response_invalid")?
                .extend(["repositories", &repo.to_string(), "git", "ref", "heads"])
                .extend(branch.split('/'));
            let value = self.request(Method::GET, url.path(), &token, None).await?;
            if value["ref"].as_str() != Some(reference.as_str())
                || value["object"]["type"].as_str() != Some("commit")
            {
                return Err("github_response_invalid");
            }
            return Ok((branch.to_owned(), default_commit_sha(&value["object"])?));
        }
        let path = default_commit_path(repo, &metadata.default_branch)?;
        let value = self.request(Method::GET, &path, &token, None).await?;
        let sha = default_commit_sha(&value)?;
        Ok((metadata.default_branch, sha))
    }
    /// All subsequent reads are addressed by immutable tree/blob IDs, never a branch.
    pub async fn frozen_source(
        &self,
        installation: u64,
        repo: u64,
        commit: &str,
    ) -> Result<FrozenSource, &'static str> {
        if !object_id(commit) {
            return Err("github_response_invalid");
        }
        let token = self.installation_token(installation, Some(repo)).await?;
        let value = self
            .request(
                Method::GET,
                &format!("/repositories/{repo}/git/commits/{commit}"),
                &token,
                None,
            )
            .await
            .map_err(|reason| {
                if reason == "github_repository_unavailable" {
                    "github_commit_unavailable"
                } else {
                    reason
                }
            })?;
        if value["sha"].as_str() != Some(commit) {
            return Err("github_response_invalid");
        }
        let tree = value["tree"]["sha"]
            .as_str()
            .filter(|v| object_id(v))
            .ok_or("github_response_invalid")?;
        let value = self
            .request(
                Method::GET,
                &format!("/repositories/{repo}/git/trees/{tree}?recursive=1"),
                &token,
                None,
            )
            .await?;
        let entries = parse_tree(&value, tree)?;
        Ok(FrozenSource {
            repo,
            token,
            entries,
            tree_sha: tree.to_owned(),
        })
    }
    pub async fn frozen_blob(
        &self,
        source: &FrozenSource,
        path: &str,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        let Some(entry) = source.entries.get(path) else {
            return Ok(None);
        };
        if !safe_path(path)
            || entry.mode != "100644" && entry.mode != "100755"
            || entry.kind != "blob"
        {
            return Err("source_path_unsupported");
        }
        if entry.size > MAX_FILE_BYTES {
            return Err("source_bounds_exceeded");
        }
        let value = self
            .request(
                Method::GET,
                &format!("/repositories/{}/git/blobs/{}", source.repo, entry.sha),
                &source.token,
                None,
            )
            .await?;
        parse_blob(&value, entry).map(Some)
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
    async fn installation(&self, installation: u64) -> Result<Value, &'static str> {
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
        Ok(observed)
    }

    pub(super) async fn installation_token(
        &self,
        installation: u64,
        repo: Option<u64>,
    ) -> Result<String, &'static str> {
        self.installation(installation).await?;
        self.mint(
            installation,
            repo,
            json!({"contents":"read","metadata":"read"}),
        )
        .await
    }

    pub(super) async fn signing_pull_access(
        &self,
        installation: u64,
        repository: u64,
        publish: bool,
    ) -> Result<super::SigningPullAccess, &'static str> {
        let installed = self.installation(installation).await?;
        let permissions = &installed["permissions"];
        let can_publish =
            permissions["contents"] == "write" && permissions["pull_requests"] == "write";
        if publish && !can_publish {
            return Err("github_signing_pull_write_required");
        }
        let token = if matches!(
            permissions["pull_requests"].as_str(),
            Some("read" | "write")
        ) {
            Some(zeroize::Zeroizing::new(
                self.mint(
                    installation,
                    Some(repository),
                    signing_pull_permissions(publish),
                )
                .await?,
            ))
        } else {
            None
        };
        Ok(super::SigningPullAccess { token, can_publish })
    }

    async fn mint(
        &self,
        installation: u64,
        repo: Option<u64>,
        permissions: Value,
    ) -> Result<String, &'static str> {
        let mut body = json!({"permissions":permissions});
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
        if value["permissions"] != permissions {
            return Err("github_access_denied");
        }
        if let (Some(repo), Some(repositories)) = (repo, value.get("repositories"))
            && !matches!(repositories.as_array().map(Vec::as_slice), Some([repository]) if id(repository, "id") == Ok(repo))
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
pub(super) const MAX_FILE_BYTES: usize = 512 * 1024;
pub(super) struct FrozenSource {
    repo: u64,
    token: String,
    entries: BTreeMap<String, TreeEntry>,
    pub tree_sha: String,
}
struct TreeEntry {
    mode: String,
    kind: String,
    sha: String,
    size: usize,
}
fn default_commit_path(repo: u64, branch: &str) -> Result<String, &'static str> {
    let mut url =
        reqwest::Url::parse("https://api.github.com").map_err(|_| "github_response_invalid")?;
    url.path_segments_mut()
        .map_err(|()| "github_response_invalid")?
        .extend(["repositories", &repo.to_string(), "commits", branch]);
    Ok(url.path().to_owned())
}
fn default_commit_sha(value: &Value) -> Result<String, &'static str> {
    value["sha"]
        .as_str()
        .filter(|sha| object_id(sha))
        .map(str::to_owned)
        .ok_or("github_response_invalid")
}
fn object_id(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
pub(super) fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|v| !v.is_empty() && v != "." && v != ".." && !v.contains(':'))
}
fn parse_tree(value: &Value, expected: &str) -> Result<BTreeMap<String, TreeEntry>, &'static str> {
    if value["sha"].as_str() != Some(expected) {
        return Err("github_response_invalid");
    }
    if value["truncated"].as_bool() != Some(false) {
        return Err("source_bounds_exceeded");
    }
    let items = value["tree"].as_array().ok_or("github_response_invalid")?;
    if items.len() > 20_000 {
        return Err("source_bounds_exceeded");
    }
    let mut entries = BTreeMap::new();
    for item in items {
        let path = text(item, "path", 4096)?;
        let sha = text(item, "sha", 40)?;
        if !object_id(&sha) {
            return Err("github_response_invalid");
        }
        let entry = TreeEntry {
            mode: text(item, "mode", 6)?,
            kind: text(item, "type", 10)?,
            sha,
            size: item["size"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(usize::MAX),
        };
        if entries.insert(path, entry).is_some() {
            return Err("github_response_invalid");
        }
    }
    Ok(entries)
}
fn parse_blob(value: &Value, entry: &TreeEntry) -> Result<Vec<u8>, &'static str> {
    if value["sha"].as_str() != Some(&entry.sha)
        || value["encoding"] != "base64"
        || value["size"].as_u64() != Some(entry.size as u64)
    {
        return Err("github_response_invalid");
    }
    let encoded = value["content"].as_str().ok_or("github_response_invalid")?;
    let bytes = STANDARD
        .decode(
            encoded
                .bytes()
                .filter(|v| *v != b'\n' && *v != b'\r')
                .collect::<Vec<_>>(),
        )
        .map_err(|_| "github_response_invalid")?;
    if bytes.len() != entry.size || bytes.len() > MAX_FILE_BYTES {
        return Err("source_bounds_exceeded");
    }
    let mut hash = Sha1::new();
    hash.update(format!("blob {}\0", bytes.len()));
    hash.update(&bytes);
    if format!("{:x}", hash.finalize()) != entry.sha {
        return Err("github_response_invalid");
    }
    Ok(bytes)
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
    #[tokio::test]
    async fn signing_pull_tokens_are_scoped_to_the_operation_and_repository()
    -> Result<(), Box<dyn std::error::Error>> {
        use axum::{
            Json, Router,
            extract::{Path, State},
            http::StatusCode,
            routing::{get, post},
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/app/installations/{installation}",
                get(|Path(installation): Path<u64>| async move {
                    let contents = if installation == 21 { "write" } else { "read" };
                    Json(json!({"id":installation,"app_id":9,"suspended_at":null,
                    "permissions":{"contents":contents,"pull_requests":"write","metadata":"read"}}))
                }),
            )
            .route(
                "/app/installations/{installation}/access_tokens",
                post(
                    |State(requests): State<Arc<AtomicUsize>>, Json(body): Json<Value>| async move {
                        let call = requests.fetch_add(1, Ordering::Relaxed);
                        let expected = if call == 1 {
                            json!({"contents":"write","pull_requests":"write","metadata":"read"})
                        } else {
                            json!({"pull_requests":"read","metadata":"read"})
                        };
                        assert_eq!(body["repository_ids"], json!([42]));
                        assert_eq!(body["permissions"], expected);
                        let returned_repo = if call == 2 { 43 } else { 42 };
                        (
                            StatusCode::CREATED,
                            Json(json!({"token":"fixture",
                        "permissions":expected,"repositories":[{"id":returned_repo}]})),
                        )
                    },
                ),
            )
            .with_state(Arc::clone(&requests));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let github = Github {
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()?,
            origin: format!("http://{}", listener.local_addr()?),
            jwt: "fixture".into(),
            app_id: 9,
        };
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        assert_eq!(
            github.signing_pull_access(22, 42, true).await.err(),
            Some("github_signing_pull_write_required")
        );
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        let read = github.signing_pull_access(22, 42, false).await?;
        assert!(!read.can_publish && read.token.is_some());
        let write = github.signing_pull_access(21, 42, true).await?;
        assert!(write.can_publish && write.token.is_some());
        assert_eq!(
            github.signing_pull_access(21, 42, false).await.err(),
            Some("github_access_denied")
        );
        server.abort();
        Ok(())
    }

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

#[cfg(test)]
mod frozen_tests {
    use super::*;
    #[test]
    fn frozen_objects_reject_moved_identity_truncation_symlinks_and_bad_content() {
        let sha = "a".repeat(40);
        assert!(parse_tree(&json!({"sha":sha,"truncated":true,"tree":[]}), &sha).is_err());
        assert!(
            parse_tree(
                &json!({"sha":"b".repeat(40),"truncated":false,"tree":[]}),
                &sha
            )
            .is_err()
        );
        assert!(
            parse_tree(
                &json!({"sha":sha,"truncated":false,"tree":[
                    {"path":"a","sha":sha,"mode":"100644","type":"blob","size":0},
                    {"path":"a","sha":sha,"mode":"100644","type":"blob","size":0}
                ]}),
                &sha
            )
            .is_err()
        );
        let entry = TreeEntry {
            mode: "100644".into(),
            kind: "blob".into(),
            sha: "ce013625030ba8dba906f756967f9e9ca394464a".into(),
            size: 6,
        };
        let mut blob = json!({"sha":entry.sha,"size":6,"encoding":"base64","content":"aGVsbG8K"});
        assert_eq!(parse_blob(&blob, &entry), Ok(b"hello\n".to_vec()));
        blob["content"] = json!("aGVsbG8h");
        assert_eq!(parse_blob(&blob, &entry), Err("github_response_invalid"));
        blob["sha"] = json!("b".repeat(40));
        assert!(parse_blob(&blob, &entry).is_err());
        for path in [
            "../secret",
            "/etc/passwd",
            "foo/../bar",
            "./a",
            "a//b",
            "C:/file",
            "a\\b",
            "a\nb",
        ] {
            assert!(!safe_path(path), "{path:?}");
        }
        assert!(safe_path("nested/Cargo.lock"));
        assert!(!object_id(&"A".repeat(40)));
        assert!(!object_id("main"));
    }
}

#[cfg(test)]
mod default_commit_tests {
    use super::*;
    #[test]
    fn default_branch_refs_are_encoded_as_one_path_segment() {
        assert_eq!(
            default_commit_path(42, "release/current?#"),
            Ok("/repositories/42/commits/release%2Fcurrent%3F%23".into())
        );
    }
    #[test]
    fn only_exact_lowercase_commit_identities_can_be_frozen() {
        assert_eq!(
            default_commit_sha(&json!({"sha":"a".repeat(40)})),
            Ok("a".repeat(40))
        );
        for value in [
            json!({}),
            json!({"sha":"main"}),
            json!({"sha":"A".repeat(40)}),
            json!({"sha":"a".repeat(39)}),
        ] {
            assert_eq!(default_commit_sha(&value), Err("github_response_invalid"));
        }
    }
}
