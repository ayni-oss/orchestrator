impl SourceClient {
    pub(crate) async fn github_installations(&self) -> Result<Value, &'static str> {
        let jwt = self.app_jwt()?;
        let app = bounded(self.client.get("https://api.github.com/app").bearer_auth(&jwt)).await?;
        let app_name = app["name"].as_str().ok_or("github_app_invalid")?;
        let mut items = Vec::new();
        for page in 1..=20 {
            let page = page.to_string();
            let installations = bounded(self.client.get("https://api.github.com/app/installations")
                .bearer_auth(&jwt).query(&[("per_page", "100"), ("page", page.as_str())])).await?;
            let batch = installations.as_array().ok_or("github_installations_invalid")?;
            let count = batch.len();
            items.extend(batch.iter().cloned());
            if count < 100 { break; }
        }
        Ok(json!({"app_name": app_name, "installations": items.iter().map(|item| json!({
            "installation_id": item["id"], "owner": item["account"]["login"]
        })).collect::<Vec<_>>()}))
    }

    pub(crate) async fn installation_repositories(&self, installation_id: u64) -> Result<Value, &'static str> {
        let jwt = self.app_jwt()?;
        let access = bounded(self.client.post(format!("https://api.github.com/app/installations/{installation_id}/access_tokens"))
            .bearer_auth(jwt)).await?;
        let token = access["token"].as_str().ok_or("github_authentication_failed")?;
        let mut repositories = Vec::new();
        for page in 1..=20 {
            let page = page.to_string();
            let result = bounded(self.client.get("https://api.github.com/installation/repositories")
                .bearer_auth(&token).query(&[("per_page", "100"), ("page", page.as_str())])).await?;
            let batch = result["repositories"].as_array().ok_or("github_repositories_invalid")?;
            let count = batch.len();
            repositories.extend(batch.iter().cloned());
            if count < 100 { break; }
        }
        Ok(json!({"repositories": repositories}))
    }

    pub(crate) fn setup_action_offset(&self, count: usize) -> usize {
        self.setup_cursor
            .fetch_add(8, std::sync::atomic::Ordering::Relaxed)
            % count.max(1)
    }

    pub(crate) async fn discover_repository(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Value, &'static str> {
        let installation = bounded(
            self.client
                .get(format!(
                    "https://api.github.com/repos/{owner}/{name}/installation"
                ))
                .bearer_auth(self.app_jwt()?),
        )
        .await?;
        let id = installation["id"]
            .as_u64()
            .ok_or("github_installation_invalid")?;
        let access = bounded(
            self.client
                .post(format!(
                    "https://api.github.com/app/installations/{id}/access_tokens"
                ))
                .bearer_auth(self.app_jwt()?)
                .json(&json!({"repositories":[name],"permissions":{"contents":"read"}})),
        )
        .await?;
        let token = access["token"]
            .as_str()
            .ok_or("github_authentication_failed")?;
        let mut repository = bounded(
            self.client
                .get(format!("https://api.github.com/repos/{owner}/{name}"))
                .bearer_auth(token),
        )
        .await?;
        repository["installation_id"] = json!(id);
        Ok(repository)
    }

    pub(crate) async fn setup_token(
        &self,
        github: &ayni_orchestrator_api::v1::GithubRepository,
    ) -> Result<String, &'static str> {
        self.scoped_token(
            github.installation_id,
            &github.repository_id,
            json!({"contents":"write","pull_requests":"write","workflows":"write"}),
        )
        .await
    }

    pub(crate) async fn setup_api(
        &self,
        github: &ayni_orchestrator_api::v1::GithubRepository,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<Value, &'static str> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/{path}",
            github.owner, github.name
        );
        bounded(if let Some(body) = body {
            self.client.post(url).bearer_auth(token).json(body)
        } else {
            self.client.get(url).bearer_auth(token)
        })
        .await
    }

    pub(crate) async fn setup_branch(
        &self,
        github: &ayni_orchestrator_api::v1::GithubRepository,
        request: &crate::github_setup::SetupPullRequest,
        token: &str,
    ) -> Result<(), &'static str> {
        match self
            .head(&github.owner, &github.name, &request.branch, token)
            .await
        {
            Ok(head) => {
                for (path, expected) in &request.files.files {
                    if self
                        .file(&github.owner, &github.name, &head, path, token)
                        .await?
                        != expected.as_bytes()
                    {
                        return Err("setup_branch_conflict");
                    }
                }
                return Ok(());
            }
            Err("github_not_found") => {}
            Err(reason) => return Err(reason),
        }
        let head = self
            .head(&github.owner, &github.name, &request.base_branch, token)
            .await?;
        let commit = self
            .setup_api(github, &format!("git/commits/{head}"), token, None)
            .await?;
        let tree = request.files.files.iter().map(|(path, contents)| json!({"path":path,"mode":"100644","type":"blob","content":contents})).collect::<Vec<_>>();
        let tree = self
            .setup_api(
                github,
                "git/trees",
                token,
                Some(&json!({"base_tree":commit["tree"]["sha"],"tree":tree})),
            )
            .await?;
        let commit = self
            .setup_api(
                github,
                "git/commits",
                token,
                Some(&json!({"message":request.title,"tree":tree["sha"],"parents":[head]})),
            )
            .await?;
        self.setup_api(
            github,
            "git/refs",
            token,
            Some(&json!({"ref":format!("refs/heads/{}",request.branch),"sha":commit["sha"]})),
        )
        .await?;
        Ok(())
    }
}
