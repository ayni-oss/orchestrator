#!/usr/bin/env bash
set -euo pipefail

readonly task="${1:?usage: run-manual-math-arm.sh TASK RUN_ID WORKER_IMAGE}"
readonly run_id="${2:?usage: run-manual-math-arm.sh TASK RUN_ID WORKER_IMAGE}"
readonly worker_image="${3:?usage: run-manual-math-arm.sh TASK RUN_ID WORKER_IMAGE}"
readonly context="docker-desktop"
readonly namespace="ayni-orchestrator-dev"
readonly start_revision="916f0184bab0999c1a7172dce84898137923505c"
readonly script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly cohort="$script_dir/math-cohort.json"
readonly results_dir="${M5_RESULTS_DIR:-/tmp/m5-results}"
readonly harness_probe="${M5_HARNESS_PROBE:-0}"
if [[ "$harness_probe" == 1 ]]; then readonly deadline_seconds=3; else readonly deadline_seconds=300; fi

[[ "$run_id" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ ]] || {
  echo "run id must be a lowercase Kubernetes slug" >&2
  exit 2
}
python3 - "$cohort" "$task" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    cohort = json.load(handle)
if sys.argv[2] not in cohort:
    raise SystemExit(f"unsupported task: {sys.argv[2]}")
PY

readonly name="m5-${task//_/-}-manual-$run_id"
((${#name} <= 52)) || { echo "generated run name is too long" >&2; exit 2; }
readonly prep_job="$name-prepare"
readonly agent_job="$name-agent"
readonly run_path="m5/$name"

kubectl --context "$context" -n "$namespace" get configmap m5-sandbox-baseline >/dev/null
kubectl --context "$context" -n "$namespace" get secret openai-api >/dev/null
kubectl --context "$context" -n "$namespace" get pvc development-checkpoints >/dev/null

prep_json="$(mktemp)"
agent_json="$(mktemp)"
cleanup() { rm -f "$prep_json" "$agent_json"; }
trap cleanup EXIT

python3 - "$prep_json" "$prep_job" "$name" "$run_path" "$worker_image" "$start_revision" <<'PY'
import json, sys
path, job, run, run_path, image, revision = sys.argv[1:]
security = {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}, "readOnlyRootFilesystem": True, "runAsNonRoot": True, "runAsUser": 10001}
command = r'''set -eu
root="/manual/$RUN_PATH"
test ! -e "$root"
mkdir -p "$root/workspace/repository" "$root/checkpoint"
git clone /baseline/baseline.bundle "$root/workspace/repository"
git -C "$root/workspace/repository" checkout --detach "$START_REVISION"
test "$(git -C "$root/workspace/repository" rev-parse HEAD)" = "$START_REVISION"
git -C "$root/workspace/repository" remote set-url origin https://github.com/ayni-oss/sandbox.git
mkdir -p "$root/workspace/.cargo"
cp -R /usr/local/cargo/. "$root/workspace/.cargo/"
test -z "$(git -C "$root/workspace/repository" status --porcelain --untracked-files=all)"
'''
spec = {"apiVersion":"batch/v1","kind":"Job","metadata":{"name":job,"labels":{"orchestrator.ayni.dev/m5-run":run}},"spec":{"backoffLimit":0,"activeDeadlineSeconds":300,"ttlSecondsAfterFinished":3600,"template":{"metadata":{"labels":{"orchestrator.ayni.dev/m5-run":run}},"spec":{"restartPolicy":"Never","automountServiceAccountToken":False,"securityContext":{"seccompProfile":{"type":"RuntimeDefault"},"fsGroup":10001},"containers":[{"name":"prepare","image":image,"imagePullPolicy":"Never","command":["/bin/sh","-ec",command],"env":[{"name":"RUN_PATH","value":run_path},{"name":"START_REVISION","value":revision}],"securityContext":security,"resources":{"requests":{"cpu":"250m","memory":"256Mi"},"limits":{"cpu":"500m","memory":"512Mi"}},"volumeMounts":[{"name":"manual","mountPath":"/manual"},{"name":"baseline","mountPath":"/baseline","readOnly":True},{"name":"scratch","mountPath":"/tmp"}]}],"volumes":[{"name":"manual","persistentVolumeClaim":{"claimName":"development-checkpoints"}},{"name":"baseline","configMap":{"name":"m5-sandbox-baseline"}},{"name":"scratch","emptyDir":{}}]}}}}
with open(path,"w",encoding="utf-8") as handle: json.dump(spec,handle,separators=(",",":"))
PY

kubectl --context "$context" -n "$namespace" create -f "$prep_json" >/dev/null
if ! kubectl --context "$context" -n "$namespace" wait --for=condition=complete "job/$prep_job" --timeout=300s >/dev/null; then
  echo "manual checkout preparation failed: $prep_job" >&2
  kubectl --context "$context" -n "$namespace" logs "job/$prep_job" -c prepare --tail=50 >&2 || true
  exit 1
fi
kubectl --context "$context" -n "$namespace" delete "job/$prep_job" --wait=true >/dev/null

python3 - "$agent_json" "$agent_job" "$name" "$run_path" "$worker_image" "$start_revision" "$cohort" "$task" "$deadline_seconds" "$harness_probe" <<'PY'
import json, sys
path, job, run, run_path, image, revision, cohort_path, task, deadline, probe = sys.argv[1:]
deadline=int(deadline)
with open(cohort_path, encoding="utf-8") as handle: task_input=json.load(handle)[task]
criteria=json.dumps(task_input["acceptance_criteria"],separators=(",",":"))
security={"allowPrivilegeEscalation":False,"capabilities":{"drop":["ALL"]},"readOnlyRootFilesystem":True,"runAsNonRoot":True,"runAsUser":10001}
agent_command=["/bin/sh","-ec","sleep 30"] if probe == "1" else ["/usr/local/bin/development-agent"]
spec={"apiVersion":"batch/v1","kind":"Job","metadata":{"name":job,"labels":{"orchestrator.ayni.dev/m5-run":run}},"spec":{"backoffLimit":0,"activeDeadlineSeconds":deadline,"ttlSecondsAfterFinished":3600,"template":{"metadata":{"labels":{"orchestrator.ayni.dev/m5-run":run}},"spec":{"restartPolicy":"Never","automountServiceAccountToken":False,"terminationGracePeriodSeconds":10,"securityContext":{"seccompProfile":{"type":"RuntimeDefault"},"fsGroup":10001},"containers":[{"name":"coding-agent","image":image,"imagePullPolicy":"Never","command":agent_command,"env":[{"name":"DEVELOPMENT_WORKSPACE","value":"/workspace/repository"},{"name":"DEVELOPMENT_CHECKPOINT","value":"/checkpoint"},{"name":"DEVELOPMENT_START_REVISION","value":revision},{"name":"DEVELOPMENT_REQUEST","value":task_input["request"]},{"name":"DEVELOPMENT_ACCEPTANCE_CRITERIA","value":criteria},{"name":"AGENT_MODEL","value":"gpt-5"},{"name":"CARGO_HOME","value":"/workspace/.cargo"},{"name":"OPENAI_API_KEY","valueFrom":{"secretKeyRef":{"name":"openai-api","key":"api-key"}}}],"securityContext":security,"resources":{"requests":{"cpu":"500m","memory":"1Gi"},"limits":{"cpu":"2","memory":"3Gi"}},"volumeMounts":[{"name":"workspace","mountPath":"/workspace","subPath":f"{run_path}/workspace"},{"name":"checkpoint","mountPath":"/checkpoint","subPath":f"{run_path}/checkpoint"},{"name":"scratch","mountPath":"/tmp"}]}],"volumes":[{"name":"workspace","persistentVolumeClaim":{"claimName":"development-checkpoints"}},{"name":"checkpoint","persistentVolumeClaim":{"claimName":"development-checkpoints"}},{"name":"scratch","emptyDir":{}}]}}}}
with open(path,"w",encoding="utf-8") as handle: json.dump(spec,handle,separators=(",",":"))
PY

mkdir -p "$results_dir"
readonly started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
readonly start_epoch="$(date +%s)"
kubectl --context "$context" -n "$namespace" create -f "$agent_json" >/dev/null

outcome=""
reason=""
while :; do
  condition="$(kubectl --context "$context" -n "$namespace" get "job/$agent_job" -o jsonpath='{range .status.conditions[*]}{.type}={.status}:{.reason}{"\n"}{end}')"
  if grep -q '^Complete=True:' <<<"$condition"; then outcome="candidate_produced"; reason="complete"; break; fi
  if grep -q '^Failed=True:' <<<"$condition"; then
    reason="$(sed -n 's/^Failed=True://p' <<<"$condition")"
    if [[ "$reason" == DeadlineExceeded ]]; then outcome="timed_out"; else outcome="failed"; fi
    break
  fi
  sleep 2
done
readonly ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
readonly end_epoch="$(date +%s)"
readonly elapsed="$((end_epoch - start_epoch))"

if [[ "$outcome" != candidate_produced ]]; then
  selector="orchestrator.ayni.dev/m5-run=$name"
  kubectl --context "$context" -n "$namespace" delete "job/$agent_job" --cascade=foreground --wait=true --timeout=10s >/dev/null 2>&1 || true
  kubectl --context "$context" -n "$namespace" delete pods -l "$selector" --force --grace-period=0 --wait=true >/dev/null 2>&1 || true
  test -z "$(kubectl --context "$context" -n "$namespace" get jobs,pods -l "$selector" -o name)" || { echo "manual run resources remain after cutoff" >&2; exit 1; }
fi

readonly record="$results_dir/$name.json"
python3 - "$record" "$task" "$started" "$ended" "$elapsed" "$outcome" "$reason" "$agent_job" "$run_path/checkpoint" "$worker_image" <<'PY'
import json, sys
path,task,started,ended,elapsed,outcome,reason,job,checkpoint,image=sys.argv[1:]
with open(path,"w",encoding="utf-8") as handle:
    json.dump({"task":task,"arm":"manual_ayni","started":started,"ended":ended,"elapsed_seconds":int(elapsed),"outcome":outcome,"reason":reason,"job":job,"checkpoint_path":checkpoint,"worker_image":image},handle,indent=2,sort_keys=True)
    handle.write("\n")
PY
printf 'record=%s outcome=%s elapsed_seconds=%s job=%s\n' "$record" "$outcome" "$elapsed" "$agent_job"
