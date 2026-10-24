#!/usr/bin/env bash
set -euo pipefail
# An owned fixture namespace, public fake credentials and a retained session PVC.
# Pass the locally built runtime image; never fall back to another Kubernetes context.
image=${1:?usage: dev/fixtures/mcp/acceptance.sh RUNTIME_IMAGE}
provider=${2:-openai}
[[ "$provider" == openai || "$provider" == anthropic ]] || { echo "provider must be openai or anthropic" >&2; exit 1; }
namespace="ayni-mcp-acceptance-$(date +%s)"
root=$(cd "$(dirname "$0")/../../.." && pwd)
manifest=$(mktemp)
forward_log=$(mktemp)
command_cache=$(mktemp -d)
worker_port=${MCP_WORKER_PORT:-18081}
fixture_port=${MCP_FIXTURE_PORT:-18080}
[[ "$worker_port" =~ ^[0-9]+$ && "$fixture_port" =~ ^[0-9]+$ ]] || { echo "fixture ports must be numeric" >&2; exit 1; }
forward_pid=''
cleanup() {
  if [[ -n "$forward_pid" ]]; then kill "$forward_pid" 2>/dev/null || true; fi
  kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || true
  rm -f "$manifest" "$forward_log"
  rm -rf "$command_cache"
}
trap cleanup EXIT
python3 - "$root" "$namespace" "$image" "$provider" > "$manifest" <<'PY'
import hashlib,json,pathlib,sys
root,namespace,image,provider=sys.argv[1:]
def id(n): return f'019a0000-0000-7000-8000-{n:012}'
schema={'type':'object','properties':{},'required':[],'additionalProperties':False}
tools=[]
for action in ['read','write']:
 tool={'alias':f'mcp_fixture_{action}','name':f'fixture_{action}','endpoint':'http://127.0.0.1:8080/mcp','description':f'Fixture {action}','input_schema':schema,'read_only':action=='read','max_output_bytes':4096,'timeout_seconds':5,'bearer_file':None}
 if action=='write': tool['consent']={'command_id':id(6),'arguments_digest':'sha256:'+hashlib.sha256(b'{}').hexdigest(),'acknowledge_external_effect':True}
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
kubectl --context docker-desktop apply -f "$manifest" >/dev/null
run_driver() {
  kubectl --context docker-desktop -n "$namespace" wait pod/session --for=condition=Ready --timeout=120s >/dev/null
  kubectl --context docker-desktop -n "$namespace" port-forward pod/session "$worker_port:8081" "$fixture_port:8080" >"$forward_log" 2>&1 &
  forward_pid=$!
  for _ in {1..60}; do if curl --fail --silent "http://127.0.0.1:$worker_port/health" >/dev/null; then break; fi; sleep 1; done
  local provider_flags=()
  if [[ "$provider" == anthropic ]]; then provider_flags+=(--anthropic); fi
  MCP_COMMAND_CACHE="$command_cache" MCP_WORKER_URL="http://127.0.0.1:$worker_port" MCP_FIXTURE_URL="http://127.0.0.1:$fixture_port" cargo run --quiet --manifest-path "$root/Cargo.toml" -p ayni-orchestrator-scripted-executor --bin ayni-mcp-acceptance -- "${provider_flags[@]}" "$@"
  kill "$forward_pid"; wait "$forward_pid" 2>/dev/null || true; forward_pid=''
}
run_driver
kubectl --context docker-desktop -n "$namespace" delete pod session --wait=true --timeout=60s >/dev/null
kubectl --context docker-desktop apply -f "$manifest" >/dev/null
run_driver --replay
