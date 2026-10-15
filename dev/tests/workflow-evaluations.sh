#!/usr/bin/env bash
# Called inside the disposable sessions gate; all model calls target its local stub.
set -euo pipefail
[[ $# == 2 ]] || { echo 'usage: workflow-evaluations.sh NAMESPACE REPORT_DIRECTORY' >&2; exit 2; }
namespace=$1
report_directory=$2
[[ "$namespace" == ayni-sessions-* ]] || { echo 'disposable namespace required' >&2; exit 2; }
k() { kubectl --context docker-desktop -n "$namespace" "$@"; }
umask 077
mkdir -p "$report_directory"
pod=$(k get pods -l app.kubernetes.io/component=project-session -o json | jq -er '.items|map(select(.metadata.deletionTimestamp==null))|first|.metadata.name')
k exec "$pod" -c research -- /usr/local/bin/ayni-orchestrator-worker workflow-eval fixture-model > "$report_directory/provider-stub.json"
# This stub supplies text, never tools. The grader must detect missing activation/quality.
jq -e '.mode=="model" and .status=="completed" and (.cases|length)==6 and (.cases|any(.status=="failed")) and (.cases|all(.model_usage|length>0)) and (.cases|all(.request.profile.revision|startswith("sha256:")))' "$report_directory/provider-stub.json" >/dev/null
jq -e '[.cases[].model_usage[]]|all(.tokens.input_tokens==16 and .tokens.output_tokens==8 and .tokens.total_tokens==24)' "$report_directory/provider-stub.json" >/dev/null
k logs "$pod" -c research > "$report_directory/research.jsonl"
k logs deployment/control-plane > "$report_directory/control-plane.jsonl"
python3 - "$report_directory" <<'PY'
import json
from pathlib import Path
import sys
root = Path(sys.argv[1])
research = [json.loads(line) for line in (root / 'research.jsonl').read_text().splitlines() if line.startswith('{')]
control = [json.loads(line) for line in (root / 'control-plane.jsonl').read_text().splitlines() if line.startswith('{')]
def spans(rows):
    return [row['span'] for row in rows if row.get('span', {}).get('name') == 'workflow']
assert any(s.get('stage') == 'planner.request' and s.get('run_id') for s in spans(research))
assert any(s.get('stage') == 'provider.completion' and s.get('input_tokens') == 16 for s in spans(research))
assert any(s.get('stage') == 'research.state' and s.get('conversation_id') and s.get('profile_revision') for s in spans(control))
for path in root.iterdir():
    text = path.read_text()
    assert 'fixture-session-key' not in text
    assert 'api_key' not in text and 'source_tokens' not in text
print('Workflow evaluations: retained model/profile inputs and real stub usage; missing tool activation fails; correlated deployed traces exclude credentials.')
PY
