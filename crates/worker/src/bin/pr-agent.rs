//! M3 PR author. Its only external action is a structured request to the
//! control plane, which owns idempotency and GitHub App credentials.

use reqwest::Client;
use rig_agent::{
    client::AgentClientExt,
    completion::Prompt,
    tool::{Tool, ToolContext},
};
use rig_core::{message::ToolChoice, providers::openai};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    env, fs, io,
    process::ExitCode,
    sync::{Arc, Mutex},
};

#[derive(Deserialize, Serialize)]
struct PublishArgs {
    title: String,
    body: String,
}

#[derive(Clone)]
struct PublishDraftPr {
    client: Client,
    endpoint: String,
    token: String,
    result: Arc<Mutex<Option<String>>>,
}

impl Tool for PublishDraftPr {
    const NAME: &'static str = "publish_draft_pr";
    type Args = PublishArgs;
    type Output = String;
    type Error = io::Error;

    fn description(&self) -> String {
        "Ask the control plane to publish exactly one draft PR for the validated candidate. The title must use Conventional Commits with a scope, for example: fix(retry): correct backoff formula.".into()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{
            "title":{"type":"string","maxLength":120,
                "pattern":"^(feat|fix|docs|test|chore|refactor|ci|perf)\\([^)]+\\): .+"},
            "body":{"type":"string","minLength":1,"maxLength":16384}},
            "required":["title","body"]})
    }

    async fn call(
        &self,
        _: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        if !conventional_title(&args.title)
            || args.title.len() > 120
            || args.body.trim().is_empty()
            || args.body.len() > 16_384
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "title must be a scoped Conventional Commit and body must be non-empty",
            ));
        }
        let existing = self
            .result
            .lock()
            .map_err(|_| io::Error::other("publication state unavailable"))?
            .clone();
        if let Some(existing) = existing {
            return Ok(existing);
        }
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .json(&args)
            .send()
            .await
            .map_err(io::Error::other)?;
        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "publish request returned {}",
                response.status()
            )));
        }
        let value: serde_json::Value = response.json().await.map_err(io::Error::other)?;
        let url = value
            .get("draft_pr_url")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| io::Error::other("publish response had no PR URL"))?
            .to_owned();
        *self
            .result
            .lock()
            .map_err(|_| io::Error::other("publication state unavailable"))? = Some(url.clone());
        Ok(url)
    }
}

fn conventional_title(value: &str) -> bool {
    [
        "feat", "fix", "docs", "test", "chore", "refactor", "ci", "perf",
    ]
    .iter()
    .any(|kind| value.starts_with(&format!("{kind}(")) && value.contains("): "))
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let model = env::var("AGENT_MODEL")?;
    let key = env::var("OPENAI_API_KEY")?;
    let context = match env::var("DEVELOPMENT_PR_CONTEXT") {
        Ok(context) => context,
        Err(_) => fs::read_to_string(env::var("DEVELOPMENT_PR_CONTEXT_PATH")?)?,
    };
    let publisher = PublishDraftPr {
        client: Client::new(),
        endpoint: env::var("DEVELOPMENT_PUBLISH_ENDPOINT")?,
        token: env::var("DEVELOPMENT_PUBLISH_TOKEN")?,
        result: Arc::new(Mutex::new(None)),
    };
    let client = openai::Client::new(&key)?;
    let agent = client.agent(&model)
        .preamble("You author a factual draft PR for one independently validated candidate. Summarize the change, exact candidate commit, validation result, limitations, and any policy or test edits. Invoke publish_draft_pr exactly once. You have no other tools and cannot change source or evidence.")
        .tool(publisher.clone())
        .tool_choice(ToolChoice::Required)
        .build();
    let prompt_result = agent.prompt(context).max_turns(4).await;
    let did_publish = publisher
        .result
        .lock()
        .map_err(|_| io::Error::other("publication state unavailable"))?
        .is_some();
    if did_publish {
        return Ok(());
    }
    prompt_result?;
    Err("PR agent did not invoke publish_draft_pr".into())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("PR agent failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::conventional_title;

    #[test]
    fn accepts_only_scoped_conventional_titles() {
        assert!(conventional_title("fix(retry): correct backoff formula"));
        assert!(!conventional_title("Fix retry backoff formula"));
        assert!(!conventional_title("fix: correct backoff formula"));
    }
}
