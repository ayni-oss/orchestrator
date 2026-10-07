# Operator onboarding

Configure the GitHub App connector under Sources → GitHub, connect authorized
repositories, and add repository membership to a project. Select a runtime
profile and build or discover its signed environment through the existing
runtime setup flow. Keep environment-building workflows and ordinary repository
CI when removing obsolete orchestrator-installed quality workflows.

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
