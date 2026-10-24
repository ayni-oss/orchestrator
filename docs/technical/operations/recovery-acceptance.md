# Cumulative recovery acceptance

Run the conversation-to-continuation loop and its failure cases on explicit Docker Desktop Kubernetes.
The profile proves recovery for the current independent-task runtime and reports remaining M6 prerequisites.
The [M6 delivery plan](../research/project-workflow-m6-delivery-plan.md) owns milestone scope.

## Run the profile

Use a new evidence directory and an owned acceptance namespace:

```sh
export AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-m6
dev/ctl quality-runtime
AYNI_ORCHESTRATOR_TASK_FIXTURE=true dev/ctl deploy
pnpm exec playwright install chromium
dev/ctl acceptance /absolute/new/m6-evidence --m6
```

The profile first runs [cumulative quality and conversation recovery](task-acceptance.md), including browser consent, response loss and navigation.
It then creates three separate tasks with failed managed quality and accepted saved changes.
Their continuations use the same frozen source, runtime, configuration and parent patch, with separately approved time and token limits.

The explicit scripted fixture pauses each continued coding process after reconstructing saved work and making an unaccepted change.
The fixture never sends a paid provider request.
Fault checks distinguish files in that worker from accepted patch evidence in PostgreSQL.

## Failure matrix

| Gate | Intervention | Required observation |
| --- | --- | --- |
| Confirmation race | Confirm the same proposal concurrently with different keys. | Both callers receive the same receipt and child attempt; a changed digest is rejected. |
| Queued cancellation | Stop the supervisor, confirm recovery, restart the control plane and cancel the queued child. | Original confirmation and cancellation receipts replay without dispatching another attempt. |
| Supervisor adoption | Restart supervision while continued coding is paused. | The active worker keeps its Pod identity and saved source. |
| Active cancellation | Stop supervision, cancel the child and leave its old process alive. | The generation fence rejects its actual late report; unaccepted output never becomes a patch or quality result. |
| Continued Pod loss | Delete the continued worker while coding is paused. | The child becomes interrupted, preserves its frozen input patch and creates one open attention item. |
| Parent preservation | Reread the parent after every command and intervention. | Its complete attempt response, accepted patch and quality evidence remain byte-identical. |
| Attention | Read parent and child feedback after termination. | The parent decision stays resolved; interrupted continuation gains its own notice. Intentional cancellation alone creates none. |
| Owner restarts | Restart the control plane, supervisor, executor, JetStream and PostgreSQL in turn. | Every original proposal, receipt and terminal outcome replays exactly; the latest attempt stays selected. |
| Retained cleanup | Uninstall workloads, compare retained identities and reinstall the exact tested images. | PVC and immutable Secret identities survive, with unchanged recovery evidence and no new execution. |

The canonical client verifies evidence through gRPC.
It never repairs PostgreSQL or grants a new continuation while replaying a receipt.
A further recovery still requires a new reviewed proposal and explicit confirmation.

## Evidence and result

`quality/` retains the inherited source manifest, dirty-source diff, deployed image identities and browser screenshots.
Each fault retains the original parent, exact proposal and confirmation, cancellation when applicable, terminal response and feedback bytes.
Named replay files cover every owner restart and retained redeployment.
Only Secret names and UIDs enter inventories; Secret values remain unread.

`acceptance.json` appears only after all stages pass.
Its artifact hashes bind the retained evidence, and `outcome.json` records the run result even after failure.
The report integrity tests reject missing replay phases, replaced storage identities, successful stale-worker output and failed browser evidence.
Run those tests with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s dev/lib -p 'test_*.py'
```

A passing report contains:

```json
{
  "schema": "build.ayni.orchestrator.task-recovery-acceptance.v1",
  "result": "passed",
  "profile": "independent-task-recovery",
  "milestone_complete": false
}
```

`unmet_prerequisites` retains M5 dependency scheduling and frozen upstream selections, structured worker questions with answer delivery, and model-generated recovery proposals.
These remain unimplemented, so this report cannot establish full M6 closure.
Actual Ayni quality uses the managed worker runtime; coding and the browser scenario use deterministic fixtures.
This evidence establishes neither live-provider nor live-GitHub recovery.

## Recovery and cleanup

A failed run preserves its stores and evidence for inspection.
Restore a stopped supervisor before diagnosing the exact failed stage; never resend an uncertain external action or delete stores to obtain a passing run.
The script attempts to restore supervision automatically on failure.

After recording the evidence, purge only the owned test deployment and its ownership-labelled session namespace:

```sh
dev/ctl purge ayni-orchestrator-acceptance-m6
```

The command requires the configured namespace to match exactly.
Leave unrelated environments and retained user work intact.
