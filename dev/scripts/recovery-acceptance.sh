#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
readonly context="$(dev_context)" namespace="$(dev_namespace)"
[[ $# -eq 1 && "$1" == /* && ! -e "$1" && "$namespace" == ayni-orchestrator-acceptance-* ]] || {
  echo 'usage: dev/ctl acceptance ABSOLUTE_NEW_EVIDENCE_DIRECTORY --m6 (owned acceptance namespace required)' >&2; exit 2;
}
readonly evidence="$1"
mkdir -p "$evidence"
status=failed
supervisor_stopped=false
cleanup() {
  if [[ "$supervisor_stopped" == true ]]; then
    kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=1 >/dev/null 2>&1 || true
  fi
  printf '{"result":"%s","profile":"m6-independent-tasks","milestone_complete":false,"cleanup":"retained_authoritative_stores"}\n' "$status" > "$evidence/outcome.json"
}
trap cleanup EXIT
cd "$DEV_REPOSITORY_ROOT"
"$dev_directory/scripts/acceptance.sh" "$evidence/quality" --quality-only
runtime="${AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE:-$(python3 -c 'import json; print(json.load(open(".ayni/task-quality-runtime.json"))["runtime_image"])')}"
control_pod() {
  kubectl --context "$context" --namespace "$namespace" get pods \
    -l app.kubernetes.io/name=ayni-orchestrator-control-plane --sort-by=.metadata.creationTimestamp -o jsonpath='{.items[-1:].metadata.name}'
}
command_stdin() {
  kubectl --context "$context" --namespace "$namespace" exec -i "$(control_pod)" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 "$1" < "$2"
}
stop_supervisor() {
  kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=0 > "$1"
  supervisor_stopped=true
  kubectl --context "$context" --namespace "$namespace" wait --for=delete pod \
    -l app.kubernetes.io/name=ayni-task-supervisor --timeout=2m >> "$1"
}
start_supervisor() {
  kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=1 > "$1"
  kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-supervisor --timeout=2m >> "$1"
  supervisor_stopped=false
}
wait_saved_work() {
  local worker="$1" output="$2"
  [[ "$worker" =~ ^task-[a-f0-9]{32}$ ]] || { echo 'invalid retained child identity' >&2; exit 1; }
  for _ in $(seq 1 120); do
    if kubectl --context "$context" --namespace "${namespace}-sessions" exec "$worker" --container coding -- \
      test -f /workspace/task-recovery-result.txt >/dev/null 2>&1; then
      kubectl --context "$context" --namespace "${namespace}-sessions" exec "$worker" --container coding -- \
        cat /workspace/src/lib.rs > "$output"
      return
    fi
    sleep .5
  done
  echo 'continued worker did not reconstruct saved work' >&2; exit 1
}
for scenario in queued-cancel active-cancel pod-loss; do
  kubectl --context "$context" --namespace "$namespace" exec "$(control_pod)" --container control-plane -- \
    env "ORCHESTRATOR_TASK_QUALITY_RUNTIME_IMAGE=$runtime" \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-recovery-fault > "$evidence/$scenario-parent.json"
  if [[ "$scenario" == queued-cancel ]]; then stop_supervisor "$evidence/$scenario-stop.txt"; fi
  command_stdin --confirm-recovery-fault-stdin "$evidence/$scenario-parent.json" > "$evidence/$scenario-confirmed.json"
  worker="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["worker"])' "$evidence/$scenario-confirmed.json")"
  if [[ "$scenario" == queued-cancel ]]; then
    # The accepted confirmation survives authority loss while dispatch remains stopped.
    kubectl --context "$context" --namespace "$namespace" rollout restart deployment/control-plane > "$evidence/confirmation-restart.txt"
    kubectl --context "$context" --namespace "$namespace" rollout status deployment/control-plane --timeout=3m >> "$evidence/confirmation-restart.txt"
    command_stdin --cancel-recovery-fault-stdin "$evidence/$scenario-confirmed.json" > "$evidence/$scenario-cancelled.json"
    command_stdin --capture-recovery-fault-cancelled-stdin "$evidence/$scenario-cancelled.json" > "$evidence/$scenario-terminal.json"
    start_supervisor "$evidence/$scenario-start.txt"
  else
    wait_saved_work "$worker" "$evidence/$scenario-saved-source.txt"
    grep -q 'fn fails_quality()' "$evidence/$scenario-saved-source.txt"
    if [[ "$scenario" == active-cancel ]]; then
      before_uid="$(kubectl --context "$context" --namespace "${namespace}-sessions" get pod "$worker" -o jsonpath='{.metadata.uid}')"
      kubectl --context "$context" --namespace "$namespace" rollout restart deployment/task-supervisor > "$evidence/supervisor-adoption.txt"
      kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-supervisor --timeout=2m >> "$evidence/supervisor-adoption.txt"
      after_uid="$(kubectl --context "$context" --namespace "${namespace}-sessions" get pod "$worker" -o jsonpath='{.metadata.uid}')"
      [[ "$before_uid" == "$after_uid" ]] || { echo 'supervisor replaced active recovery' >&2; exit 1; }
      printf '%s\n' "$before_uid" >> "$evidence/supervisor-adoption.txt"
      stop_supervisor "$evidence/$scenario-stop.txt"
      command_stdin --cancel-recovery-fault-stdin "$evidence/$scenario-confirmed.json" > "$evidence/$scenario-cancelled.json"
      # Leave the old coding process alive past the fence so its real late report is rejected.
      exit_code=''
      for _ in $(seq 1 360); do
        exit_code="$(kubectl --context "$context" --namespace "${namespace}-sessions" get pod "$worker" -o jsonpath='{.status.initContainerStatuses[?(@.name=="coding")].lastState.terminated.exitCode}')"
        [[ -z "$exit_code" ]] || break
        sleep .5
      done
      [[ "$exit_code" == 1 ]] || { echo 'late continued-worker output was not fenced' >&2; exit 1; }
      printf '%s\n' "$exit_code" > "$evidence/late-worker-exit.txt"
      command_stdin --verify-recovery-fault-fenced-stdin "$evidence/$scenario-cancelled.json" > "$evidence/late-worker-fence.txt"
      start_supervisor "$evidence/$scenario-start.txt"
      command_stdin --capture-recovery-fault-cancelled-stdin "$evidence/$scenario-cancelled.json" > "$evidence/$scenario-terminal.json"
    else
      kubectl --context "$context" --namespace "${namespace}-sessions" delete pod "$worker" --wait=true --timeout=60s > "$evidence/continued-pod-loss.txt"
      command_stdin --capture-recovery-fault-interrupted-stdin "$evidence/$scenario-confirmed.json" > "$evidence/$scenario-terminal.json"
    fi
  fi
done
verify_faults() {
  local suffix="$1" scenario
  for scenario in queued-cancel active-cancel pod-loss; do
    command_stdin --verify-recovery-fault-stdin "$evidence/$scenario-terminal.json" > "$evidence/$scenario-replay-$suffix.txt"
  done
}
for workload in deployment/control-plane deployment/task-supervisor statefulset/scripted-executor statefulset/workflow-jetstream statefulset/workflow-postgres; do
  name="${workload#*/}"
  kubectl --context "$context" --namespace "$namespace" rollout restart "$workload" > "$evidence/restart-$name.txt"
  kubectl --context "$context" --namespace "$namespace" rollout status "$workload" --timeout=3m >> "$evidence/restart-$name.txt"
  "$dev_directory/scripts/check.sh" >> "$evidence/restart-$name.txt" 2>&1
  verify_faults "$name"
done
inventory() {
  local scope
  for scope in "$namespace" "${namespace}-sessions"; do
    kubectl --context "$context" --namespace "$scope" get pvc -o json | python3 -c 'import json,sys; print(json.dumps(sorted((p["metadata"]["name"],p["metadata"]["uid"],p["spec"].get("volumeName","")) for p in json.load(sys.stdin)["items"])))'
    kubectl --context "$context" --namespace "$scope" get secret -l owner!=helm -o jsonpath='{range .items[*]}{.metadata.name}{" "}{.metadata.uid}{"\n"}{end}' | sort
  done
}
control_image="$(kubectl --context "$context" --namespace "$namespace" get deployment control-plane -o jsonpath='{.spec.template.spec.containers[0].image}')"
executor_image="$(kubectl --context "$context" --namespace "$namespace" get statefulset scripted-executor -o jsonpath='{.spec.template.spec.containers[0].image}')"
inventory > "$evidence/retained-before.txt"
"$dev_directory/ctl" cleanup > "$evidence/cleanup.txt" 2>&1
kubectl --context "$context" --namespace "$namespace" wait --for=delete pod --all --timeout=2m >> "$evidence/cleanup.txt"
inventory > "$evidence/retained-stopped.txt"
cmp "$evidence/retained-before.txt" "$evidence/retained-stopped.txt"
# Reinstall the exact tested images; retained claims and immutable credentials remain in place.
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --reset-values --kube-context "$context" --namespace "$namespace" \
  --set ayni-orchestrator.taskExecution.fixtureEnabled=true \
  --set-string "ayni-orchestrator.images.controlPlane=$control_image" \
  --set-string "ayni-orchestrator.images.scriptedExecutor=$executor_image" --wait --timeout=5m > "$evidence/redeploy.txt"
"$dev_directory/scripts/check.sh" >> "$evidence/redeploy.txt" 2>&1
inventory > "$evidence/retained-after.txt"
cmp "$evidence/retained-before.txt" "$evidence/retained-after.txt"
verify_faults redeploy
python3 "$dev_directory/lib/recovery-report.py" "$evidence"
status=passed
printf 'Cumulative independent-task recovery passed; M6 closure prerequisites remain. evidence=%s\n' "$evidence"
