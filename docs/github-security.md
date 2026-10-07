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

## Connection credentials

The trusted operator API stores GitHub App private keys and GHCR tokens in
namespace-scoped Kubernetes Secrets. SQLite stores safe connection metadata,
ownership, desired/observed revisions and bounded test results. Read responses
never return credentials; the manifest callback converts and stores credentials
server-side and redirects to connection status. Credential writes require a
same-origin browser request and all connection pages use `Cache-Control: no-store`.
The custom-client listener does not expose credential management.

An explicit deployment reference registers an administrator-managed App. Missing
labels never authorize adoption. UI changes verify ownership and Secret UID.
The connection Role is namespaced and remains enabled without runtime execution:
GitHub authentication requires Secret reads, and registry test Jobs require Job
creation. Standard Kubernetes RBAC cannot enforce label ownership or grant only
metadata reads; a Secret get permission permits reading its contents. Use a
dedicated namespace and trusted operator access.

GHCR test/build Jobs receive push credentials. Candidate workloads receive a
separate pull-only Secret. Neither receives the control-plane service-account token
through connection testing. Rotation validates a candidate revision before
activation; each process reconciles changes without self-restart.
