#!/usr/bin/env bash
# Sourced by publication-acceptance.sh: all commands retain the same canonical
# scope, source/quality evidence, fixture transport and exact retry files.
m4_wait_pause() {
  local pause="$1" output="$2"
  for _ in $(seq 1 100); do
    fixture /fixture/state > "$output"
    if python3 - "$output" "$pause" <<'PY'
import json,sys
raise SystemExit(0 if json.load(open(sys.argv[1]))['waiting'][sys.argv[2]] else 1)
PY
    then return; fi
    sleep .1
  done
  echo "publication fixture did not reach $pause" >&2; return 1
}

m4_restart_publisher() {
  local name="$1" pod
  pod="$(kubectl --context "$context" --namespace "$namespace" get pod \
    -l app.kubernetes.io/name=ayni-task-publisher -o json | python3 -c '
import json,sys
pods=[p for p in json.load(sys.stdin)["items"] if not p["metadata"].get("deletionTimestamp")]
assert len(pods)==1, "exactly one live publisher required"
print(pods[0]["metadata"]["name"])
')"
  # Freeze the process before deletion: a graceful shutdown can finish the
  # in-flight HTTP request and reconcile under the old generation.
  kubectl --context "$context" --namespace "$namespace" exec "$pod" -- /bin/sh -c 'kill -STOP 1' \
    > "$evidence/$name-publisher-loss.txt"
  kubectl --context "$context" --namespace "$namespace" delete pod "$pod" --grace-period=1 --wait=true --timeout=90s \
    >> "$evidence/$name-publisher-loss.txt"
  kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-publisher --timeout=2m \
    >> "$evidence/$name-publisher-loss.txt"
}

m4_publication_faults() {
  local name pause before after
  for name in before-create after-create; do
    pause=pause_list
    [[ "$name" != after-create ]] || pause=pause_create_response
    prepare_task --prepare-publication-task > "$evidence/$name-task.json"
    bind_scope "$evidence/$name-task.json"
    fixture /fixture/control "{\"$pause\":true}" > "$evidence/$name-control.json"
    command_stdin --start-publication-pending-stdin "$evidence/$name-task.json" > "$evidence/$name-pending.json"
    m4_wait_pause "$pause" "$evidence/$name-paused-remote.json"
    command_stdin --inspect-publication-stdin "$evidence/$name-pending.json" > "$evidence/$name-prepared-identity.json"
    if [[ "$name" == after-create ]]; then
      command_stdin --reject-publication-cancel-stdin "$evidence/$name-pending.json" > "$evidence/$name-cancel-rejected.txt"
    fi
    m4_restart_publisher "$name"
    fixture /fixture/control "{\"$pause\":false}" > "$evidence/$name-release.json"
    command_stdin --capture-publication-stdin "$evidence/$name-pending.json" > "$evidence/$name-publication.json"
    command_stdin --inspect-publication-stdin "$evidence/$name-publication.json" > "$evidence/$name-recovered-identity.json"
    python3 - "$evidence" "$name" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]);name=sys.argv[2]
a=json.loads((root/f'{name}-prepared-identity.json').read_text());b=json.loads((root/f'{name}-recovered-identity.json').read_text())
assert a['state']==('prepared' if name=='before-create' else 'creating')
assert b['state']=='draft' and b['generation']>a['generation']
for key in ['publication_id','branch','base_branch','commit_sha','tree_sha']:
    assert a[key]==b[key] and a[key]
remote=json.loads((root/f'{name}-paused-remote.json').read_text())
matching=[p for p in remote['pulls'] if p['head']['ref']==a['branch']]
assert len(matching)==(0 if name=='before-create' else 1)
assert f"refs/heads/{a['branch']} {a['commit_sha']}" in remote['references']
PY
  done
  # A consumed permit without positive remote evidence stays uncertain even
  # after the fixture becomes healthy. Neither cancellation nor a second POST
  # can make the ambiguous action safe to forget.
  prepare_task --prepare-publication-task > "$evidence/uncertain-task.json"
  bind_scope "$evidence/uncertain-task.json"
  fixture /fixture/control '{"hold_create":true}' > "$evidence/uncertain-control.json"
  command_stdin --start-publication-pending-stdin "$evidence/uncertain-task.json" > "$evidence/uncertain-pending.json"
  command_stdin --capture-publication-uncertain-stdin "$evidence/uncertain-pending.json" > "$evidence/uncertain-publication.json"
  command_stdin --reject-publication-cancel-stdin "$evidence/uncertain-publication.json" > "$evidence/uncertain-cancel-rejected.txt"
  fixture /fixture/control '{"hold_create":false}' > "$evidence/uncertain-release.json"
  fixture /fixture/state > "$evidence/uncertain-before-restart.json"
  m4_restart_publisher uncertain
  command_stdin --verify-publication-uncertain-stdin "$evidence/uncertain-publication.json" > "$evidence/uncertain-recovered.txt"
  expected_creates=4
  expected_pulls=3
}

m4_verify_publications() {
  local suffix="$1" name
  for name in before-create after-create; do
    command_stdin --verify-publication-stdin "$evidence/$name-publication.json" > "$evidence/$name-replay-$suffix.txt"
  done
  command_stdin --verify-publication-uncertain-stdin "$evidence/uncertain-publication.json" > "$evidence/uncertain-replay-$suffix.txt"
}

m4_retained_inventory() {
  local output="$1"
  kubectl --context "$context" --namespace "$namespace" get pvc -o json > "$output-parent-pvc.json"
  kubectl --context "$context" --namespace "${namespace}-sessions" get pvc -o json > "$output-worker-pvc.json"
  # Print metadata only, never Secret values. Immutable identities must survive.
  kubectl --context "$context" --namespace "$namespace" get secret -l owner!=helm \
    -o jsonpath='{range .items[*]}{.metadata.name}{" "}{.metadata.uid}{"\n"}{end}' > "$output-secret-identities.txt"
  kubectl --context "$context" --namespace "${namespace}-sessions" get secret -l owner!=helm \
    -o jsonpath='{range .items[*]}{.metadata.name}{" "}{.metadata.uid}{"\n"}{end}' > "$output-worker-secret-identities.txt"
}

m4_retained_cleanup() {
  python3 "$DEV_REPOSITORY_ROOT/scripts/ci/check-task-topology.py" --namespace "$namespace" > "$evidence/task-topology.json"
  m4_retained_inventory "$evidence/cleanup-before"
  "$dev_directory/ctl" cleanup > "$evidence/cleanup.txt" 2>&1
  kubectl --context "$context" --namespace "$namespace" wait --for=delete pod --all --timeout=2m >> "$evidence/cleanup.txt"
  kubectl --context "$context" --namespace "$namespace" get pod -o json > "$evidence/cleanup-parent-pods.json"
  kubectl --context "$context" --namespace "${namespace}-sessions" get pod -o json > "$evidence/cleanup-worker-pods.json"
  m4_retained_inventory "$evidence/cleanup-stopped"
  # Restore the exact tested images and publisher binding, without rebuilding,
  # recreating credentials, deleting retained state or dispatching new coding.
  bind_scope "$evidence/uncertain-task.json"
  "$dev_directory/scripts/check.sh" > "$evidence/cleanup-redeployed.txt" 2>&1
  m4_retained_inventory "$evidence/cleanup-after"
  python3 - "$evidence" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1])
for kind in ['parent','worker']:
    assert json.loads((root/f'cleanup-{kind}-pods.json').read_text())['items']==[]
    def claims(stage):
        return sorted((p['metadata']['name'],p['metadata']['uid'],p['spec'].get('volumeName',''))
                      for p in json.loads((root/f'cleanup-{stage}-{kind}-pvc.json').read_text())['items'])
    assert claims('before')==claims('stopped')==claims('after')
for kind in ['','worker-']:
    values=[sorted((root/f'cleanup-{stage}-{kind}secret-identities.txt').read_text().splitlines()) for stage in ['before','stopped','after']]
    assert values[0]==values[1]==values[2]
PY
  command_stdin --verify-publication-stdin "$evidence/publication.json" > "$evidence/cleanup-original-draft.txt"
  command_stdin --verify-publication-cancelled-stdin "$evidence/cancel-publication.json" > "$evidence/cleanup-cancelled.txt"
  m4_verify_publications cleanup
  for mode in task-run task-quality-run task-quality-failed-run; do
    command_stdin "--verify-$mode-stdin" "$evidence/quality/$mode.json" > "$evidence/cleanup-$mode.txt"
  done
  command_stdin --verify-plans-stdin "$evidence/quality/plans-before-restart.json" > "$evidence/cleanup-plans.txt"
  command_stdin --verify-execution-stdin "$evidence/quality/execution-before-restart.json" > "$evidence/cleanup-preparation.txt"
  forward_fixture
  fixture /fixture/state > "$evidence/cleanup-remote.json"
}
