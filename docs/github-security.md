# GitHub security boundary

The GitHub App is installed only on enrolled repositories. Reusable App
credentials remain in the connector boundary. The worker receives short-lived,
repository-scoped credentials for checkout, push and draft creation. Tokens are
mounted separately, refreshed while active and never retained in output,
evidence, logs, metrics or pull request content.

## Required permissions

| Permission | Access | Purpose |
| --- | --- | --- |
| Metadata | Read | Identify an authorized repository |
| Contents | Read and write | Checkout source and push owned task/setup branches |
| Pull requests | Read and write | Create and adopt task drafts and runtime setup PRs |
| Workflows | Read and write | Maintain environment-building workflow through reviewed setup PRs |

Task delivery does not require Actions, Checks or Issues permission. Preserve
ordinary repository CI and useful environment-building workflows. Remove
obsolete orchestrator-installed quality workflows through reviewed cleanup PRs.
Never merge setup or cleanup PRs automatically.

Publication verifies repository, deterministic branch, base, head and task
identity. Intent is durable before pushing. After an uncertain response, adopt
only a matching pull request. Conflicting remote heads stop delivery without
rewriting human changes. Final quality and evidence identity are disclosed in
the draft, including failed and unavailable outcomes.

Managed runtime admission retains independently observed image digest and
source identity. Ayni validates certificate, lock compatibility and protected
runtime content. Registry pull, environment build/signing, model access and
repository delivery remain distinct credential boundaries. Never grant build
or signing authority to task source.

Treat repository content, findings, model output and provider metadata as
untrusted. Bound and sanitize retained text, validate URLs, escape Markdown
content and preserve output isolation. No credential values belong in inventory.
