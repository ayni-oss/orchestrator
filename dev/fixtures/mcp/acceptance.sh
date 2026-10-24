#!/usr/bin/env bash
set -euo pipefail
# An owned fixture namespace, public fake credentials and a retained session PVC.
# Pass the locally built runtime image; never fall back to another Kubernetes context.
[[ $# -ge 1 && $# -le 3 ]] || { echo 'usage: dev/fixtures/mcp/acceptance.sh RUNTIME_IMAGE [openai|anthropic] [ABSOLUTE_NEW_EVIDENCE_DIRECTORY]' >&2; exit 2; }
image=$1
provider=${2:-openai}
[[ "$provider" == openai || "$provider" == anthropic ]] || { echo "provider must be openai or anthropic" >&2; exit 2; }
if [[ $# -eq 3 ]]; then
  [[ "$3" == /* && ! -e "$3" ]] || { echo 'new absolute evidence directory required' >&2; exit 2; }
  evidence=$3
  mkdir -p "$evidence"
else
  evidence=$(mktemp -d /tmp/ayni-mcp-evidence.XXXXXXXX)
fi
namespace="ayni-mcp-acceptance-$(date +%s)-$RANDOM"
root=$(cd "$(dirname "$0")/../../.." && pwd)
manifest="$evidence/manifest.json"
command_cache="$evidence/commands"
mkdir "$command_cache"
read -r default_worker_port default_fixture_port < <(python3 - <<'PORTS'
import socket
sockets=[socket.socket() for _ in range(2)]
for sock in sockets: sock.bind(('127.0.0.1',0))
print(*(sock.getsockname()[1] for sock in sockets))
for sock in sockets: sock.close()
PORTS
)
worker_port=${MCP_WORKER_PORT:-$default_worker_port}
fixture_port=${MCP_FIXTURE_PORT:-$default_fixture_port}
[[ "$worker_port" =~ ^[0-9]+$ && "$fixture_port" =~ ^[0-9]+$ && "$worker_port" != "$fixture_port" ]] || { echo 'fixture ports must be distinct numbers' >&2; exit 2; }
forward_pid=''
status=failed
cleanup() {
  if [[ -n "$forward_pid" ]]; then kill "$forward_pid" 2>/dev/null || true; fi
  # The Pod, PVC and public fixture data remain available for operator diagnosis.
  python3 - "$evidence" "$status" "$provider" "$image" "$namespace" <<'REPORT'
import hashlib,json,pathlib,sys
root,status,provider,runtime,namespace=sys.argv[1:]
evidence=pathlib.Path(root)
artifacts={str(path.relative_to(evidence)):hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(evidence.rglob("*")) if path.is_file() and path.name!="outcome.json"}
gates=['scoped-tools-and-exact-consent','retained-successful-tool-output','external-effect-response-loss','in-flight-external-effect-cancellation','pod-replacement-exact-replay'] if status=='passed' else []
if status=='passed' and provider=='anthropic': gates.append('anthropic-provider-uncertainty-and-cancellation')
pathlib.Path(root,'outcome.json').write_text(json.dumps({'schema':'build.ayni.orchestrator.mcp-acceptance.v1','result':status,'provider':provider,'runtime':runtime,'namespace':namespace,'gates':gates,'live_provider':False,'cleanup':'retained_authoritative_stores','artifacts_sha256':artifacts},indent=2)+'\n')
REPORT
}
trap cleanup EXIT
python3 - "$root" "$namespace" "$image" "$provider" > "$manifest" <<'PY'
import hashlib,json,pathlib,sys
root,namespace,image,provider=sys.argv[1:]
def id(n): return f'019a0000-0000-7000-8000-{n:012}'
schema={'type':'object','properties':{},'required':[],'additionalProperties':False}
tools=[]
for action in ['read','write','lost','cancel']:
 tool={'alias':f'mcp_fixture_{action}','name':f'fixture_{action}','endpoint':'http://127.0.0.1:8080/mcp','description':f'Fixture {action}','input_schema':schema,'read_only':action=='read','max_output_bytes':4096,'timeout_seconds':5,'bearer_file':None}
 if action!='read': tool['consent']={'command_id':id({'write':6,'lost':60,'cancel':80}[action]),'arguments_digest':'sha256:'+hashlib.sha256(b'{}').hexdigest(),'acknowledge_external_effect':True}
 tools.append(tool)
grant={'workspace_id':id(1),'project_id':id(2),'configuration_revision_id':id(5),'tools':tools}
env={'ORCHESTRATOR_SESSION_ID':id(4),'ORCHESTRATOR_SESSION_GENERATION':'1','ORCHESTRATOR_SESSION_CONVERSATION_ID':id(3),'ORCHESTRATOR_SESSION_WORKSPACE_ID':id(1),'ORCHESTRATOR_SESSION_PROJECT_ID':id(2),'ORCHESTRATOR_SESSION_IDLE_SECONDS':'3600','ORCHESTRATOR_SESSION_TOKEN_PATH':'/var/run/fixture/token','ORCHESTRATOR_SESSION_LEDGER_PATH':'/var/lib/session/results.sqlite','ORCHESTRATOR_OPENAI_ENABLED':'true','ORCHESTRATOR_OPENAI_API_KEY_PATH':'/var/run/fixture/api-key','OPENAI_BASE_URL':'http://127.0.0.1:8080/v1','ORCHESTRATOR_MCP_CONFIG_PATH':'/var/run/mcp-config/grants.json'}
env.update({'ORCHESTRATOR_SESSION_PROVIDER':provider,'ORCHESTRATOR_ANTHROPIC_ENABLED':'true','ORCHESTRATOR_ANTHROPIC_API_KEY_PATH':'/var/run/fixture/api-key','ANTHROPIC_BASE_URL':'http://127.0.0.1:8080/v1'})
node='node:24.14.0-alpine@sha256:7fddd9ddeae8196abf4a3ef2de34e11f7b1a722119f91f28ddf1e99dcafdf114'
objects=[{'apiVersion':'v1','kind':'Namespace','metadata':{'name':namespace}},
 {'apiVersion':'v1','kind':'ConfigMap','metadata':{'name':'fixture','namespace':namespace},'data':{'grants.json':json.dumps([grant]),'server.mjs':pathlib.Path(root,'dev/fixtures/provider/server.mjs').read_text(),'token':'public-mcp-acceptance-session-token-01234567890123456789','api-key':'provider-fixture-key'}},
 {'apiVersion':'v1','kind':'PersistentVolumeClaim','metadata':{'name':'retained','namespace':namespace},'spec':{'accessModes':['ReadWriteOnce'],'resources':{'requests':{'storage':'1Gi'}}}},
 {'apiVersion':'v1','kind':'Pod','metadata':{'name':'session','namespace':namespace},'spec':{'automountServiceAccountToken':False,'securityContext':{'runAsUser':10001,'runAsGroup':10001,'fsGroup':10001},'containers':[
  {'name':'session','image':image,'imagePullPolicy':'IfNotPresent','args':['session-worker'],'env':[{'name':k,'value':v} for k,v in env.items()],'ports':[{'containerPort':8081}],'readinessProbe':{'httpGet':{'path':'/health','port':8081}},'volumeMounts':[{'name':'retained','mountPath':'/var/lib/session'},{'name':'fixture','mountPath':'/var/run/fixture','readOnly':True},{'name':'fixture','mountPath':'/var/run/mcp-config','readOnly':True}]},
  {'name':'fixture','image':node,'command':['node','/var/run/fixture/server.mjs'],'env':[{'name':'MCP_FIXTURE_EFFECTS_PATH','value':'/var/lib/session/effects'}],'ports':[{'containerPort':8080}],'readinessProbe':{'httpGet':{'path':'/health','port':8080}},'volumeMounts':[{'name':'retained','mountPath':'/var/lib/session'},{'name':'fixture','mountPath':'/var/run/fixture','readOnly':True}]}
 ],'volumes':[{'name':'retained','persistentVolumeClaim':{'claimName':'retained'}},{'name':'fixture','configMap':{'name':'fixture'}}]}}]
print(json.dumps({'apiVersion':'v1','kind':'List','items':objects}))
PY
kubectl --context docker-desktop apply -f "$manifest" > "$evidence/apply.txt"
run_driver() {
  local stage=$1
  shift
  local forward_log="$evidence/$stage-port-forward.txt"
  kubectl --context docker-desktop -n "$namespace" wait pod/session --for=condition=Ready --timeout=120s > "$evidence/$stage-readiness.txt"
  kubectl --context docker-desktop -n "$namespace" get pod session -o json > "$evidence/$stage-pod.json"
  kubectl --context docker-desktop -n "$namespace" get pvc retained -o json > "$evidence/$stage-pvc.json"
  kubectl --context docker-desktop -n "$namespace" port-forward --address 127.0.0.1 pod/session "$worker_port:8081" "$fixture_port:8080" >"$forward_log" 2>&1 &
  forward_pid=$!
  local ready=false
  for _ in {1..60}; do
    if curl --fail --silent "http://127.0.0.1:$worker_port/health" >/dev/null && curl --fail --silent "http://127.0.0.1:$fixture_port/health" >/dev/null; then ready=true; break; fi
    kill -0 "$forward_pid" 2>/dev/null || { cat "$forward_log" >&2; return 1; }
    sleep 1
  done
  [[ "$ready" == true ]] || { echo 'fixture loopback forwards did not become ready' >&2; return 1; }
  local provider_flags=()
  if [[ "$provider" == anthropic ]]; then provider_flags+=(--anthropic); fi
  MCP_COMMAND_CACHE="$command_cache" MCP_WORKER_URL="http://127.0.0.1:$worker_port" MCP_FIXTURE_URL="http://127.0.0.1:$fixture_port" cargo run --quiet --manifest-path "$root/Cargo.toml" -p ayni-orchestrator-scripted-executor --bin ayni-mcp-acceptance -- "${provider_flags[@]}" "$@" > "$evidence/$stage.json" 2> "$evidence/$stage.stderr"
  kill "$forward_pid"; wait "$forward_pid" 2>/dev/null || true; forward_pid=''
}
run_driver initial
kubectl --context docker-desktop -n "$namespace" delete pod session --wait=true --timeout=60s > "$evidence/pod-replacement.txt"
kubectl --context docker-desktop apply -f "$manifest" >> "$evidence/pod-replacement.txt"
run_driver replay --replay
python3 - "$evidence" "$provider" <<'VERIFY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1])
initial,replay=[json.loads(root.joinpath(stage+'.json').read_text()) for stage in ('initial','replay')]
for report in (initial,replay):
    assert report['outcome']=='passed' and report['provider']==sys.argv[2]
    assert len(report['external_effect_failures'])==2
    assert all(item['external_effects']==1 for item in report['external_effect_failures'])
for field in ('success_result','external_effect_failures','provider_failures','command_id','command_digest'):
    assert initial[field]==replay[field], 'replacement changed retained '+field
assert replay['replay'] is True and initial['replay'] is False
assert replay['final_stats']['calls']==0 and replay['final_stats']['mcpReads']==0, 'replacement resent provider/MCP calls'
assert replay['final_stats']['mcpEffects']==replay['final_stats']['mcpLostEffects']==replay['final_stats']['mcpCancelledEffects']==1
pods=[json.loads(root.joinpath(stage+'-pod.json').read_text()) for stage in ('initial','replay')]
pvcs=[json.loads(root.joinpath(stage+'-pvc.json').read_text()) for stage in ('initial','replay')]
assert pods[0]['metadata']['uid']!=pods[1]['metadata']['uid'], 'session Pod was not replaced'
assert pvcs[0]['metadata']['uid']==pvcs[1]['metadata']['uid'], 'session authority PVC was replaced'
for pod in pods:
    assert all(container.get('imageID') for container in pod['status']['containerStatuses']), 'runtime image identity missing'
VERIFY
status=passed
printf 'Deployed %s MCP failure/replay profile passed; retained evidence=%s namespace=%s\n' "$provider" "$evidence" "$namespace"
