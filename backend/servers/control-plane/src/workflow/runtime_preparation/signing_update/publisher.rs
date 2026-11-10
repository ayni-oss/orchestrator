use super::v1;
use ayni_core::sha256_fingerprint;
use ayni_orchestrator_events::task_publication::github_repository;
use reqwest::{Client, Method, Url};
use serde_json::{Value, json};
use std::time::Duration;
use tonic::Status;
use zeroize::Zeroizing;

static PUBLICATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const TITLE: &str = "fix(runtime): update the trusted signing key";

pub(super) struct Change<'a> {
    pub repository: &'a str,
    pub repository_id: u64,
    pub base: &'a str,
    pub commit: &'a str,
    pub tree: &'a str,
    pub policy: &'a str,
    pub lock: &'a str,
}

struct Publisher {
    client: Client,
    origin: Url,
    repository: String,
    token: Zeroizing<String>,
}

impl Publisher {
    fn new(repository: &str, token: Zeroizing<String>) -> Result<Self, Status> {
        let canonical = format!("https://github.com/{repository}.git");
        if github_repository(&canonical).as_deref() != Some(canonical.as_str()) {
            return Err(Status::failed_precondition(
                "Invalid publication repository.",
            ));
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| unavailable())?;
        Ok(Self {
            client,
            origin: Url::parse("https://api.github.com").map_err(|_| unavailable())?,
            repository: repository.into(),
            token,
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Option<Value>, Status> {
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|()| unavailable())?
            .extend(["repos"])
            .extend(self.repository.split('/'))
            .extend(path.iter().copied());
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let read = method == Method::GET;
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(self.token.trim())
            .header("User-Agent", "ayni-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| unavailable())?;
        match response.status().as_u16() {
            200 if read => {}
            201 if !read => {}
            404 if read => return Ok(None),
            401 | 403 | 404 => {
                return Err(Status::permission_denied(
                    "The installed GitHub App lacks permission for this repository action.",
                ));
            }
            409 | 422 => {
                return Err(Status::aborted(
                    "GitHub rejected the signing update. Refresh and retry to reconcile the branch and pull request.",
                ));
            }
            _ => return Err(unavailable()),
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if chunk.len() > 1_048_576_usize.saturating_sub(bytes.len()) {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| unavailable())
    }

    async fn get(&self, path: &[&str]) -> Result<Option<Value>, Status> {
        self.request(Method::GET, path, &[], None).await
    }

    async fn post(&self, path: &[&str], body: Value) -> Result<Value, Status> {
        self.request(Method::POST, path, &[], Some(body))
            .await?
            .ok_or_else(unavailable)
    }

    async fn head(&self, branch: &str) -> Result<Option<String>, Status> {
        let mut path = vec!["git", "ref", "heads"];
        path.extend(branch.split('/'));
        self.get(&path)
            .await?
            .map(|value| {
                if value["ref"].as_str() != Some(format!("refs/heads/{branch}").as_str())
                    || value["object"]["type"] != "commit"
                {
                    return Err(unavailable());
                }
                sha(&value["object"]["sha"]).map(str::to_owned)
            })
            .transpose()
    }

    async fn verify_repository(&self, request: &Change<'_>) -> Result<(), Status> {
        let repository = self.get(&[]).await?.ok_or_else(unavailable)?;
        if repository["id"].as_u64() != Some(request.repository_id)
            || repository["full_name"].as_str() != Some(request.repository)
        {
            return Err(Status::aborted("The publication repository changed."));
        }
        Ok(())
    }

    async fn pull(
        &self,
        request: &Change<'_>,
        branch: &str,
    ) -> Result<Option<v1::RuntimeSigningPullRequest>, Status> {
        let owner = self.repository.split('/').next().ok_or_else(unavailable)?;
        let head_query = format!("{owner}:{branch}");
        let reply = self
            .request(
                Method::GET,
                &["pulls"],
                &[
                    ("head", &head_query),
                    ("base", request.base),
                    ("state", "all"),
                    ("per_page", "100"),
                ],
                None,
            )
            .await?
            .ok_or_else(unavailable)?;
        match reply
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(unavailable)?
        {
            [] => Ok(None),
            [pull] => {
                let number = pull_number(pull, request, branch)?;
                let detail = self
                    .get(&["pulls", &number.to_string()])
                    .await?
                    .ok_or_else(unavailable)?;
                if pull_number(&detail, request, branch)? != number {
                    return Err(unavailable());
                }
                Ok(Some(public_pull(&detail, request, branch, number)?))
            }
            _ => Err(Status::aborted(
                "Multiple pull requests use this signing update branch.",
            )),
        }
    }
}

fn sha(value: &Value) -> Result<&str, Status> {
    value
        .as_str()
        .filter(|s| {
            s.len() == 40
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(unavailable)
}

fn pull_number(value: &Value, request: &Change<'_>, branch: &str) -> Result<u64, Status> {
    if value["title"].as_str() != Some(TITLE)
        || value["head"]["ref"].as_str() != Some(branch)
        || value["head"]["repo"]["id"].as_u64() != Some(request.repository_id)
        || value["base"]["repo"]["id"].as_u64() != Some(request.repository_id)
        || value["base"]["ref"].as_str() != Some(request.base)
    {
        return Err(Status::aborted(
            "The existing pull request does not match the reviewed signing update.",
        ));
    }
    value["number"]
        .as_u64()
        .filter(|number| *number > 0)
        .ok_or_else(unavailable)
}

fn public_pull(
    value: &Value,
    request: &Change<'_>,
    branch: &str,
    number: u64,
) -> Result<v1::RuntimeSigningPullRequest, Status> {
    use v1::RuntimeSigningPullRequestState as State;
    let state = match value["state"].as_str() {
        Some("open") if value["draft"] == true => State::Draft,
        Some("open") => State::Open,
        Some("closed") if value["merged"] == true => State::Merged,
        Some("closed") => State::Closed,
        _ => return Err(unavailable()),
    };
    Ok(v1::RuntimeSigningPullRequest {
        url: format!("https://github.com/{}/pull/{number}", request.repository),
        branch: branch.into(),
        state: state as i32,
    })
}

fn unavailable() -> Status {
    Status::unavailable(
        "The signing update could not be confirmed. Select Open PR again to recover its result.",
    )
}

fn change_branch(request: &Change<'_>) -> String {
    // Include the reviewed commit so an updated target requires its own proposal.
    // The regenerated lock already binds the policy digest and trust fingerprint.
    let input = json!([
        request.repository_id,
        request.base,
        request.commit,
        request.lock
    ]);
    let fingerprint = sha256_fingerprint(input.to_string());
    format!(
        "fix/runtime-signing-{}",
        &fingerprint["sha256:".len().."sha256:".len() + 24]
    )
}

pub(super) async fn lookup(
    request: Change<'_>,
    token: Zeroizing<String>,
) -> Result<Option<v1::RuntimeSigningPullRequest>, Status> {
    let publisher = Publisher::new(request.repository, token)?;
    publisher.verify_repository(&request).await?;
    publisher.pull(&request, &change_branch(&request)).await
}

pub(super) async fn open(
    request: Change<'_>,
    token: Zeroizing<String>,
) -> Result<v1::RuntimeSigningPullRequest, Status> {
    // Keep simultaneous requests in this process from racing between PR lookup and creation.
    let _guard = PUBLICATION.lock().await;
    let publisher = Publisher::new(request.repository, token)?;
    publish(&publisher, &request).await
}

async fn publish(
    publisher: &Publisher,
    request: &Change<'_>,
) -> Result<v1::RuntimeSigningPullRequest, Status> {
    let branch = change_branch(request);
    publisher.verify_repository(request).await?;
    if let Some(pull) = publisher.pull(request, &branch).await? {
        return Ok(pull);
    }
    let existing = publisher.head(&branch).await?;
    if existing.is_none() {
        confirm_base(publisher, request).await?;
    }
    let head = create_commit(publisher, request).await?;
    if let Some(existing) = existing {
        if existing != head {
            return Err(Status::aborted(
                "The signing update branch has different changes.",
            ));
        }
    } else {
        confirm_base(publisher, request).await?;
        let creation = publisher
            .post(
                &["git", "refs"],
                json!({"ref":format!("refs/heads/{branch}"),"sha":head}),
            )
            .await;
        if publisher.head(&branch).await?.as_deref() != Some(head.as_str()) {
            return Err(creation.err().unwrap_or_else(|| {
                Status::aborted("The signing update branch has different changes.")
            }));
        }
    }
    confirm_base(publisher, request).await?;
    let result = publisher.post(&["pulls"], json!({
        "title":TITLE,"base":request.base,"head":branch,"draft":true,"maintainer_can_modify":false,
        "body":"Updates the selected public signing key in .ayni.toml and refreshes .ayni.lock while preserving the other trusted keys."
    })).await;
    match result {
        Ok(pull) if sha(&pull["head"]["sha"])? == head => {
            let number = pull_number(&pull, request, &branch)?;
            public_pull(&pull, request, &branch, number)
        }
        Ok(_) => Err(unavailable()),
        Err(error) => publisher.pull(request, &branch).await?.ok_or(error),
    }
}

async fn create_commit(publisher: &Publisher, request: &Change<'_>) -> Result<String, Status> {
    let base_commit = publisher
        .get(&["git", "commits", request.commit])
        .await?
        .ok_or_else(unavailable)?;
    if sha(&base_commit["sha"])? != request.commit
        || sha(&base_commit["tree"]["sha"])? != request.tree
    {
        return Err(Status::aborted(
            "The inspected commit did not match the publication repository.",
        ));
    }
    let date = base_commit["committer"]["date"]
        .as_str()
        .filter(|date| date.len() <= 40)
        .ok_or_else(unavailable)?;
    let tree = publisher
        .post(
            &["git", "trees"],
            json!({"base_tree":request.tree,"tree":[
                {"path":".ayni.toml","mode":"100644","type":"blob","content":request.policy},
                {"path":".ayni.lock","mode":"100644","type":"blob","content":request.lock}
            ]}),
        )
        .await?;
    let tree = sha(&tree["sha"])?;
    if tree == request.tree {
        return Err(Status::failed_precondition(
            "The signing update did not change the tree.",
        ));
    }
    let identity =
        json!({"name":"Ayni Orchestrator","email":"orchestrator@ayni.build","date":date});
    let commit = publisher.post(&["git", "commits"], json!({
        "message":format!("{TITLE}\n\nSigned-off-by: Ayni Orchestrator <orchestrator@ayni.build>\nai-generated: true"),
        "tree":tree,"parents":[request.commit],"author":identity,"committer":identity
    })).await?;
    let head = sha(&commit["sha"])?;
    if sha(&commit["tree"]["sha"])? != tree
        || !matches!(commit["parents"].as_array().map(Vec::as_slice), Some([parent]) if parent["sha"] == request.commit)
    {
        return Err(unavailable());
    }
    Ok(head.to_owned())
}

async fn confirm_base(publisher: &Publisher, request: &Change<'_>) -> Result<(), Status> {
    if publisher.head(request.base).await?.as_deref() == Some(request.commit) {
        return Ok(());
    }
    Err(Status::aborted(
        "The target branch moved. Check ayni.lock again before publishing.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Request, State},
        http::StatusCode,
        response::{IntoResponse, Response},
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    const REPO: &str = "fixture/runtime";
    const BASE: &str = "release/main";
    const COMMIT: &str = "1111111111111111111111111111111111111111";
    const TREE: &str = "2222222222222222222222222222222222222222";
    const UPDATED_TREE: &str = "3333333333333333333333333333333333333333";
    const HEAD: &str = "4444444444444444444444444444444444444444";

    #[derive(Default)]
    struct Fixture {
        branch: String,
        pull: Option<Value>,
        moved: bool,
        ref_writes: u32,
        pull_writes: u32,
    }

    async fn github(State(state): State<Arc<Mutex<Fixture>>>, request: Request) -> Response {
        let (parts, body) = request.into_parts();
        let path = parts
            .uri
            .path()
            .trim_start_matches("/repos/fixture/runtime");
        let body = axum::body::to_bytes(body, 8192).await.unwrap_or_default();
        let body: Value = serde_json::from_slice(&body).unwrap_or_default();
        let mut fixture = state.lock().await;
        if parts.method == Method::GET {
            let value = match path {
                "" => json!({"id":42,"full_name":REPO}),
                "/git/ref/heads/release/main" => json!({
                    "ref":format!("refs/heads/{BASE}"),
                    "object":{"type":"commit","sha":if fixture.moved { HEAD } else { COMMIT }},
                }),
                "/pulls" => json!(fixture.pull.iter().collect::<Vec<_>>()),
                "/pulls/7" => fixture.pull.clone().unwrap_or_default(),
                value if value == format!("/git/commits/{COMMIT}") => json!({
                    "sha":COMMIT,"tree":{"sha":TREE},"committer":{"date":"2026-01-01T00:00:00Z"}
                }),
                value
                    if !fixture.branch.is_empty()
                        && value == format!("/git/ref/heads/{}", fixture.branch) =>
                {
                    json!({
                        "ref":format!("refs/heads/{}",fixture.branch),"object":{"type":"commit","sha":HEAD}
                    })
                }
                _ => return StatusCode::NOT_FOUND.into_response(),
            };
            return Json(value).into_response();
        }
        let value = match path {
            "/git/trees" => {
                assert_eq!(body["base_tree"], TREE);
                assert_eq!(
                    body["tree"],
                    json!([
                        {"path":".ayni.toml","mode":"100644","type":"blob","content":"policy"},
                        {"path":".ayni.lock","mode":"100644","type":"blob","content":"lock"},
                    ])
                );
                json!({"sha":UPDATED_TREE})
            }
            "/git/commits" => {
                assert_eq!(body["tree"], UPDATED_TREE);
                assert_eq!(body["parents"], json!([COMMIT]));
                json!({"sha":HEAD,"tree":{"sha":UPDATED_TREE},"parents":[{"sha":COMMIT}]})
            }
            "/git/refs" => {
                fixture.ref_writes += 1;
                fixture.branch = body["ref"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches("refs/heads/")
                    .into();
                assert!(fixture.branch.starts_with("fix/runtime-signing-"));
                assert_eq!(body["sha"], HEAD);
                json!({"ref":body["ref"]})
            }
            "/pulls" => {
                fixture.pull_writes += 1;
                assert_eq!(body["title"], TITLE);
                assert_eq!(body["base"], BASE);
                assert_eq!(body["head"], fixture.branch);
                assert_eq!(body["draft"], true);
                fixture.pull = Some(json!({"number":7,"title":TITLE,"state":"open","draft":true,
                    "head":{"ref":fixture.branch,"sha":HEAD,"repo":{"id":42}},
                    "base":{"ref":BASE,"repo":{"id":42}}
                }));
                // GitHub accepted the PR but the publisher did not receive confirmation.
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            _ => return StatusCode::NOT_FOUND.into_response(),
        };
        (StatusCode::CREATED, Json(value)).into_response()
    }

    #[tokio::test]
    async fn publishes_two_files_and_recovers_a_lost_pull_request_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = Arc::new(Mutex::new(Fixture {
            moved: true,
            ..Fixture::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = Url::parse(&format!("http://{}", listener.local_addr()?))?;
        let router = Router::new()
            .fallback(github)
            .with_state(Arc::clone(&state));
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let publisher = Publisher {
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()?,
            origin,
            repository: REPO.into(),
            token: Zeroizing::new("test-token".into()),
        };
        let change = Change {
            repository: REPO,
            repository_id: 42,
            base: BASE,
            commit: COMMIT,
            tree: TREE,
            policy: "policy",
            lock: "lock",
        };
        assert_eq!(
            publish(&publisher, &change)
                .await
                .err()
                .map(|error| error.code()),
            Some(tonic::Code::Aborted)
        );
        {
            let mut fixture = state.lock().await;
            assert_eq!((fixture.ref_writes, fixture.pull_writes), (0, 0));
            fixture.moved = false;
        }
        let result = publish(&publisher, &change).await?;
        assert_eq!(result.url, "https://github.com/fixture/runtime/pull/7");
        assert_eq!(publish(&publisher, &change).await?, result);
        assert_eq!(
            result.state,
            v1::RuntimeSigningPullRequestState::Draft as i32
        );
        {
            let mut fixture = state.lock().await;
            fixture.branch.clear();
            let pull = fixture.pull.as_mut().ok_or("missing fixture pull")?;
            pull["state"] = json!("closed");
            pull["merged"] = json!(true);
            drop(fixture);
        }
        let merged = publisher
            .pull(&change, &change_branch(&change))
            .await?
            .ok_or("missing merged pull")?;
        assert_eq!(
            merged.state,
            v1::RuntimeSigningPullRequestState::Merged as i32
        );
        let fixture = state.lock().await;
        assert_eq!((fixture.ref_writes, fixture.pull_writes), (1, 1));
        drop(fixture);
        server.abort();
        Ok(())
    }
}
