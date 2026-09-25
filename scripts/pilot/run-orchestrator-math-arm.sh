#!/usr/bin/env bash
set -euo pipefail

readonly task="${1:?usage: run-orchestrator-math-arm.sh TASK RUN_ID API_URL}"
readonly run_id="${2:?usage: run-orchestrator-math-arm.sh TASK RUN_ID API_URL}"
readonly api="${3:?usage: run-orchestrator-math-arm.sh TASK RUN_ID API_URL}"
readonly context="docker-desktop"
readonly namespace="ayni-orchestrator-dev"
readonly start_revision="916f0184bab0999c1a7172dce84898137923505c"
readonly script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly cohort="$script_dir/math-cohort.json"
readonly results_dir="${PILOT_RESULTS_DIR:-/tmp/pilot-results}"
readonly harness_probe="${PILOT_HARNESS_PROBE:-0}"

[[ "$run_id" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ ]] || { echo "run id must be a lowercase slug" >&2; exit 2; }
[[ "$api" =~ ^http://(127\.0\.0\.1|localhost):[0-9]+$ ]] || { echo "API must be a loopback HTTP URL" >&2; exit 2; }
curl --fail --silent --show-error "$api/readyz" >/dev/null

payload="$(mktemp)"
response="$(mktemp)"
status_json="$(mktemp)"
cleanup() { rm -f "$payload" "$response" "$status_json"; }
trap cleanup EXIT

python3 - "$payload" "$cohort" "$task" "$run_id" "$start_revision" "$harness_probe" <<'PY'
import json, sys
path, cohort_path, task, run_id, revision, probe = sys.argv[1:]
with open(cohort_path, encoding="utf-8") as handle: cohort=json.load(handle)
if task not in cohort: raise SystemExit(f"unsupported task: {task}")
value=cohort[task]
if probe == "1":
    value={"request":"Deadline harness probe. Make no publication; the one-second task deadline must expire before coding starts.","acceptance_criteria":["the durable task deadline stops all owned execution"]}
payload={"idempotency_key":f"pilot-{task}-orchestrator-{run_id}","repository_id":"sandbox-main","profile_id":"openai-gpt5","start_revision":revision,"request":value["request"],"acceptance_criteria":value["acceptance_criteria"],"deadline_seconds":1 if probe == "1" else 300}
with open(path,"w",encoding="utf-8") as handle: json.dump(payload,handle,separators=(",",":"))
PY

curl --fail --silent --show-error -H 'content-type: application/json' --data-binary "@$payload" "$api/v1/development-tasks" >"$response"
read -r task_id created_at deadline_at phase < <(python3 - "$response" <<'PY'
import json, sys
with open(sys.argv[1],encoding="utf-8") as handle: task=json.load(handle)
print(task["id"],task["created_at"],task["deadline_at"],task["phase"])
PY
)
readonly task_id created_at deadline_at
started="$(date -u -r "$created_at" +%Y-%m-%dT%H:%M:%SZ)"
outcome=""
reason=""

while :; do
  case "$phase" in
    completed) outcome="candidate_produced"; reason="completed"; break ;;
    failed) outcome="failed"; reason="$(python3 - "$status_json" <<'PY'
import json, sys
with open(sys.argv[1],encoding="utf-8") as handle: value=json.load(handle)
print(value.get("failure") or "failed")
PY
)"; [[ "$reason" == "task deadline exceeded" ]] && outcome="timed_out"; break ;;
    cancelled) outcome="cancelled"; reason="cancelled"; break ;;
    unsupported) outcome="unsupported"; reason="unsupported"; break ;;
    waiting_for_input) outcome="input_required"; reason="input_required"; break ;;
  esac
  now="$(date +%s)"
  if ((now > deadline_at + 10)); then
    cancel_payload="$(printf '{"idempotency_key":"pilot-%s-cutoff-cancel"}' "$task_id")"
    curl --fail --silent --show-error -H 'content-type: application/json' --data-binary "$cancel_payload" "$api/v1/development-tasks/$task_id/cancel" >/dev/null || true
    selector="orchestrator.ayni.dev/development-task-id=$task_id"
    kubectl --context "$context" -n "$namespace" delete jobs -l "$selector" --cascade=foreground --wait=true --timeout=10s >/dev/null 2>&1 || true
    kubectl --context "$context" -n "$namespace" delete pods -l "$selector" --force --grace-period=0 --wait=true >/dev/null 2>&1 || true
    test -z "$(kubectl --context "$context" -n "$namespace" get jobs,pods -l "$selector" -o name)" || { echo "orchestrator resources remain after cutoff" >&2; exit 1; }
    outcome="timed_out"
    reason="cutoff watchdog fallback"
    break
  fi
  sleep 2
  if curl --fail --silent --show-error "$api/v1/development-tasks/$task_id" >"$status_json"; then
    phase="$(python3 - "$status_json" <<'PY'
import json, sys
with open(sys.argv[1],encoding="utf-8") as handle: print(json.load(handle)["phase"])
PY
)"
  fi
done

if [[ "$outcome" == timed_out ]]; then
  selector="orchestrator.ayni.dev/development-task-id=$task_id"
  for _ in $(seq 1 10); do
    resources="$(kubectl --context "$context" -n "$namespace" get jobs,pods -l "$selector" -o name)"
    [[ -z "$resources" ]] && break
    sleep 1
  done
  if [[ -n "$resources" ]]; then
    kubectl --context "$context" -n "$namespace" delete jobs -l "$selector" --cascade=foreground --wait=true --timeout=10s >/dev/null 2>&1 || true
    kubectl --context "$context" -n "$namespace" delete pods -l "$selector" --force --grace-period=0 --wait=true >/dev/null 2>&1 || true
  fi
  test -z "$(kubectl --context "$context" -n "$namespace" get jobs,pods -l "$selector" -o name)" || { echo "orchestrator resources remain after cutoff" >&2; exit 1; }
fi

readonly ended_epoch="$(date +%s)"
readonly ended="$(date -u -r "$ended_epoch" +%Y-%m-%dT%H:%M:%SZ)"
readonly elapsed="$((ended_epoch - created_at))"
mkdir -p "$results_dir"
readonly record="$results_dir/pilot-${task//_/-}-orchestrator-$run_id.json"
python3 - "$record" "$task" "$task_id" "$started" "$ended" "$elapsed" "$outcome" "$reason" "$created_at" "$deadline_at" <<'PY'
import json, sys
path,task,task_id,started,ended,elapsed,outcome,reason,created_at,deadline_at=sys.argv[1:]
with open(path,"w",encoding="utf-8") as handle:
    json.dump({"task":task,"arm":"orchestrator","task_id":task_id,"started":started,"ended":ended,"elapsed_seconds":int(elapsed),"outcome":outcome,"reason":reason,"created_at":int(created_at),"deadline_at":int(deadline_at)},handle,indent=2,sort_keys=True)
    handle.write("\n")
PY
printf 'record=%s outcome=%s elapsed_seconds=%s task_id=%s\n' "$record" "$outcome" "$elapsed" "$task_id"
