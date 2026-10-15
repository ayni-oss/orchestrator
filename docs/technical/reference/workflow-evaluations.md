# Workflow traces and evaluations

Run `dev/ctl eval` from the repository root to retain a repeatable workflow report
under `.local/evals/<UTC timestamp>/`. Install the pinned Rust toolchain, locked
Node dependencies (`pnpm install --frozen-lockfile`) and Chromium
(`pnpm --dir apps/web exec playwright install chromium`) first.

## Read the results

| Evidence | What it establishes |
| --- | --- |
| `report.json` deterministic cases | Store approval, atomic plans, duplicate prevention, session expiry/recreation, read-only guards, frozen profiles and returned outcomes satisfy their assertions |
| `scripted.json` | Six synthetic scenarios exercise the real planner, tool handlers, budget and usage adapters with a scripted completion model |
| Browser case | Six existing project-conversation flows pass in Chromium against a fixture server |
| `report.json` provider cases | An explicitly selected worker ran the same synthetic scenarios through its configured provider |

Reports retain source revision, exact fixture sources and hashes, case selectors,
inputs, frozen model/profile/skill revisions, outputs, tool success and latency,
provider-reported model identifiers, token usage and wall time. Optional usage
fields remain unknown when a provider omits them. Fixture counts and stub tokens
do not measure model quality. Every selected Rust test must actually execute;
zero matches, ignored cases and changed selectors fail the suite.

`--skip-browser` explicitly records omitted browser coverage. Provider evaluation
is `unavailable` by default, rather than a passing score. A selected provider
failure also fails the suite. Use `--require-provider` when availability is a gate.

## Evaluate a model

Choose an idle resident planner Pod in your local deployment and a model available
through its existing connection. This makes up to eight calls per scenario, with
six scenarios and a 90-second deadline per scenario, and may incur provider charges.
The command always uses the explicit `docker-desktop` context.

```sh
dev/ctl eval --provider-pod PLANNER_POD --namespace LOCAL_NAMESPACE \
  --model MODEL_ID --require-provider
```

The worker reads its normal mounted model credential internally. The harness
neither retrieves Secrets nor supplies keys. Evaluation uses temporary synthetic
README content and a synthetic task outcome, with no GitHub checkout, task
admission, write tool or publication. It does not modify the resident conversation.
The profiles in the report describe the evaluation's fixed synthetic capability
selection and exact compiled instructions, rather than the conversation's settings.

The rubric checks a clarifying question before proposals, a complete README-only
plan, no mutation activation in research, positive and negative pinned-skill
activation, successful context retrieval and independent outcome labels.
These are observable checks, not a general semantic judge. Inspect retained outputs
and run multiple samples when comparing models; latency and wording can vary.
Approval, expiry, recreation and duplicate prevention are deterministic store/browser
cases because those decisions belong to the application.

Compare two retained reports without conflating fixture and provider cases:

```sh
python3 evals/compare.py BEFORE/report.json AFTER/report.json
```

The comparison shows changed fixture sources, inputs, models, profile hashes,
outcomes, latency and usage for each case. Reports are private local artifacts;
commit reviewed fixtures and rubrics, rather than generated reports.

## Follow a workflow

The shared observability subscriber emits `workflow` spans to JSON stdout and,
when `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` is configured, the existing bounded OTLP
exporter. Resident planner processes emit the same JSON spans; export from those
processes requires the endpoint in the worker image environment. The launcher keeps
its existing immutable Pod specification. Exec protocol clients continue to emit
only protocol responses.

Filter by `conversation_id` or `run_id` to connect turns, research and model/tool
activity. `plan_id`, `plan_version` and `proposal_id` connect the exact reviewed
version to `task_id` and `attempt_id`; `session_id` and `generation` distinguish
resident recreation. Model and profile revision identify the frozen configuration.
Returned-outcome spans carry numeric execution, quality and publication values
from the public protobuf enums, independently.

Tool spans record only compiled tool names, classified success/failure and elapsed
time. Model spans record safe outcome codes and optional usage counters. Prompts,
arguments, source, output bodies, error bodies, skills and credential/Secret
bindings never enter these spans. Reports contain only the synthetic evaluation
inputs and their model outputs. Unknown tool names become `unknown_tool`.

Spans from different processes are joined by durable workflow identifiers; they
do not require a single trace ID spanning a human approval delay. Database spans
observe state and may precede a surrounding transaction commit, repeat on a later
save, or be lost. Accepted database state and receipts remain authoritative.
Telemetry failures never change admission, deduplication or execution decisions.

## Exercise the deployed boundary

```sh
dev/ctl test workflow-evals CONTROL_PLANE_IMAGE WEB_IMAGE WORKER_IMAGE
```

This uses the local Helm chart in a disposable namespace. It runs real resident
workers with a synthetic provider stub, checks session reuse and fencing, frozen
profiles across restart, deployed correlated logs, model-usage reports and grader
failure when the stub omits required tool activation. Reports remain under
`.local/evals/kubernetes-<namespace>/` after namespace cleanup. Its control-plane
phase uses valid pre-frozen empty source scope; it does not establish live GitHub
admission or paid-provider quality. Feature-specific regression tests remain with
their owning features.
