# GitHub security boundary

Ayni Orchestrator uses a customer-owned GitHub App installed only on enrolled
repositories. The controller obtains short-lived installation tokens; it does
not store those tokens in its database, evidence, logs, metrics, comments, or
workflow artifacts.

## Required GitHub App permissions

The App Manifest and existing-App setup require these repository permissions:

| Permission | Access | Purpose |
| --- | --- | --- |
| Metadata | Read | Identify an installed repository. |
| Contents | Read and write | Read the default branch and create the owned setup branch. |
| Pull requests | Read and write | Create and reconcile the owned setup PR. |
| Workflows | Read and write | Install and version the owned workflow files on the setup branch. |
| Actions | Read | Observe workflow runs and bounded artifacts. |
| Checks | Read | Observe required checks. |
| Issues | Read and write | Maintain the one marker-owned assessment-summary comment. |

Install the App only on repositories operated by this control plane. Missing
permissions must produce an actionable per-capability remediation; do not grant
organization-wide access or broaden another permission as a workaround.

Registry pull and build-push credentials are Kubernetes-scoped and are never
held by the controller or GitHub workflow. Pull-request workflows never receive
registry, controller, model, or App credentials.
Fork handling is fail-closed until a supported policy is implemented.

## Workflow rules

All repository workflows must use `pull_request`, never `pull_request_target`,
when they execute candidate-controlled code. They check out the event head with
persisted credentials disabled. Third-party Actions are pinned to a full commit
SHA with a human-readable release comment. Workflow templates use minimal job
permissions, bounded artifact retention, and concurrency that cancels stale
PR runs.

The setup PR is the only controller-initiated repository mutation. The
controller never merges it or pushes workflows directly to the default branch.
It verifies configured registry policy and immutable digests; a mutable tag,
comment, or check is not authoritative evidence.

## Webhooks and certified environments

GitHub App webhooks are optional event ingress, not a permission grant or a
candidate-code executor. When enabled, the receiver verifies the webhook
signature before parsing a bounded payload, records/deduplicates the delivery
identity, and reconciles provider state before acting. Delivery replay, delay,
or loss must converge through the periodic reconciler; webhook text and URLs are
not trusted evidence. Subscribe only to the events needed to accelerate
repository-installation, pull-request, and workflow-run observation.

Ayni environment certificates are a separate runtime boundary. A dedicated
build Job may create a certified image only with the dedicated signing seed and
key identifier supplied through its scoped Kubernetes Secret; a PR flow admits
an immutable image and runs its managed check. The controller retains the
independently observed OCI digest and source identity; Ayni verifies the
certificate, lock compatibility, and protected runtime content. Never duplicate
the certificate verifier in workflow glue or make the certificate replace
registry-policy checks.

Candidate lock changes require a separately specified ephemeral certification
policy. Until that policy exists, fail closed or report the change as unsupported:
a candidate-controlled `pull_request` workflow must never receive a reusable
certificate-signing secret, build credential, or weaken the configured
certificate trust policy.

## Credentials and hostile input

GitHub App, Actions, registry, model, and Kubernetes credentials are
short-lived where supported and are never passed on a command line. Task image
pull credentials are task-scoped and removed after admission or terminal
recovery. Workflow output, OCI metadata, comments, paths, symlinks, ANSI/control
characters, and credential-looking text are bounded and sanitized before they
are retained or displayed.

Build-time dependencies, candidate PR code, managed checks, registry pulls,
and model network access are distinct trust boundaries. A failure, drift, or
unknown result fails closed rather than reusing a credential, workflow result,
or registry tag.
