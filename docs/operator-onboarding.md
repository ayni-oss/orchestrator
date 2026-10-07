# Operator onboarding

Start the control plane without a GitHub App, registry or model credentials to
configure connections through the trusted operator UI. Model credentials are
required when admitting model-backed work, not when opening onboarding.

Under **Source control → GitHub**, create a customer-owned App through the
manifest flow or import its App ID and PEM key. The server stores credentials in
a Kubernetes Secret and returns safe connection status. Install the App on the
repositories it may access in GitHub, then connect those repositories in Ayni.
App ownership and installation permissions remain under GitHub administration.

Under **Container registries → GitHub**, add a GHCR connection with an approved
image namespace, username, push token and a distinct pull-only token. Check the
independent authentication, push, pull and cleanup results. A cleanup failure is
reported separately and can be retried. Select the connection in the runtime’s **Edit** dialog on its repository overview. Repository
enrollment does not require a registry selection.

Administrator-managed connections show **Managed by your administrator** and
cannot be edited or deleted through the UI. Register them explicitly using the
Helm connection references described in the [deployment chart](../deploy/charts/ayni-orchestrator/README.md).
UI-managed connections support replacement, testing, disabling and deletion when
unreferenced. Credential inputs clear after submission and read APIs never return
them. Validated replacements activate automatically; no reload button, deployment
restart or Tilt restart is required. Connection pages refresh status periodically.

The repository overview shows connection status and branch once, followed by its
runtimes. Lock fingerprints and image destinations belong to each runtime.
Inspection notifications are transient; ongoing progress and actionable failures
remain visible on the affected runtime. The namespace is defined once on the registry connection; runtime dialogs show
the server-computed image destination without a namespace input. Registry changes invalidate that runtime’s
readiness until its new destination is verified; other runtimes are unaffected.

Add repository membership to a project. Select a runtime profile and build or
discover its signed environment through the existing runtime setup flow. Keep
environment-building workflows and ordinary repository CI when removing obsolete
orchestrator-installed quality workflows. Candidate workloads receive pull-only
registry access; builders and probes hold push access separately.

Submit a task against a ready runtime. Admission freezes source revision,
runtime digest, model, constraints, coding turns, quality repair limit and
deadline. The worker codes, commits, checks the full repository and makes bounded
repairs before creating a draft with the actual quality result.

Task history shows status and phase. Task details show Progress, Quality and
Pull request, including turns, round, frozen limits, completion counts, signal
details and actionable findings. Unavailable measurements remain unavailable.
A failed-quality draft is completed delivery. Publication failure can be retried
against retained final work without coding again. Ordinary retry creates a new
attempt.

Before a breaking deployment, follow the exact-resource inventory and state
reset procedure in [operations](operations.md). Existing source repositories,
runtime images and GitHub pull requests are retained.

Manage the GitHub App at `/source-control/github`. Use **Add GitHub App** to create an App in your organization or personal account, or import an existing App ID and private key. The compact connection card opens an **Edit** dialog for key rotation, testing, disablement, and deletion; administrator-managed Apps expose read-only **Details**. App installation stays in GitHub. Legacy `/setup/github` links redirect here, and existing Apps using `/setup/github/complete` retain callback compatibility.
