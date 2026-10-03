impl SourceClient {
    pub(crate) async fn review_get(
        &self,
        github: &ayni_orchestrator_api::v1::GithubRepository,
        suffix: &str,
        token: &str,
    ) -> Result<Value, &'static str> {
        bounded(
            self.client
                .get(format!(
                    "https://api.github.com/repos/{}/{}/{suffix}",
                    github.owner, github.name
                ))
                .bearer_auth(token),
        )
        .await
    }

    pub(crate) async fn mark_ready(
        &self,
        node: &str,
        action: &str,
        token: &str,
    ) -> Result<(), &'static str> {
        let response = bounded(self.client.post("https://api.github.com/graphql").bearer_auth(token).json(&json!({
            "query":"mutation($id:ID!,$key:String!){markPullRequestReadyForReview(input:{pullRequestId:$id,clientMutationId:$key}){pullRequest{id isDraft}}}",
            "variables":{"id":node,"key":action}
        }))).await?;
        if response.get("errors").is_some()
            || response["data"]["markPullRequestReadyForReview"]["pullRequest"]["isDraft"] != false
        {
            return Err("readiness_response_uncertain");
        }
        Ok(())
    }

    pub(crate) async fn review_archive(
        &self,
        github: &ayni_orchestrator_api::v1::GithubRepository,
        id: u64,
        token: &str,
    ) -> Result<Vec<u8>, &'static str> {
        let response = self
            .client
            .get(format!(
                "https://api.github.com/repos/{}/{}/actions/artifacts/{id}/zip",
                github.owner, github.name
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| "artifact_unavailable")?;
        if response.status().as_u16() != 302 {
            return Err("artifact_unavailable");
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .ok_or("artifact_location_invalid")?;
        let url = reqwest::Url::parse(location).map_err(|_| "artifact_location_invalid")?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || !url.host_str().is_some_and(|h| {
                h.ends_with(".blob.core.windows.net")
                    || h.ends_with(".actions.githubusercontent.com")
            })
        {
            return Err("artifact_location_invalid");
        }
        // The signed artifact URL receives no App token and is never retained.
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| "artifact_unavailable")?;
        if !response.status().is_success() {
            return Err("artifact_unavailable");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "artifact_unavailable")? {
            if bytes.len() + chunk.len() > 12 * 1024 * 1024 {
                return Err("artifact_oversized");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}
