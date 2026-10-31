#!/usr/bin/env python3
"""Render signer namespace retention and explicit API authority without a cluster."""
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile

os.environ['HELM_NO_PLUGINS'] = '1'
ROOT = Path(__file__).resolve().parents[2]
CHART = ROOT / 'deploy/charts/ayni-orchestrator'


def documents(render):
    return render.split('\n---')


def document(render, kind, name):
    return next(doc for doc in documents(render) if f'kind: {kind}\n' in doc and re.search(r'^  name: ' + re.escape(name) + r'$', doc, re.M))


def check(namespace, runtime):
    digest = 'sha256:' + 'a' * 64
    command = ['helm', 'template', 'independent-release', str(CHART), '--namespace', namespace,
        '--set', 'signerProvisioning.enabled=true', '--set', 'runtimePreparation.enabled=' + str(runtime).lower()]
    if runtime:
        command += ['--set', 'runtimePreparation.profile.qualified=true']
        for name in ('worker_image', 'executor_image', 'verifier_image'):
            command += ['--set-string', f'runtimePreparation.profile.{name}=local/fixture@{digest}']
    render = subprocess.check_output(command, text=True)
    expected = namespace + '-runtime'
    if len(expected) > 63:
        expected = namespace[:45].rstrip('-') + '-' + hashlib.sha256(namespace.encode()).hexdigest()[:8] + '-runtime'
    names = [doc for doc in documents(render) if 'kind: Namespace\n' in doc and f'  name: {expected}\n' in doc]
    assert len(names) == 1, 'shared runtime namespace must render once'
    assert 'helm.sh/resource-policy: keep' in names[0], 'disable/uninstall must retain signer namespace'
    config = document(render, 'ConfigMap', 'control-plane-config')
    assert f'ORCHESTRATOR_SIGNER_PROVISIONING_NAMESPACE: "{expected}"' in config
    role = document(render, 'Role', 'runtime-signer-provisioner')
    assert f'  namespace: {expected}\n' in role
    assert 'verbs: [create, get]' in role
    assert not any(verb in role for verb in ('list', 'delete', 'patch', 'update', 'watch'))
    cp = document(render, 'Deployment', 'control-plane')
    assert 'automountServiceAccountToken: false' in cp
    assert 'expirationSeconds: 600' in cp and 'defaultMode: 0600' in cp
    assert cp.count('mountPath: /var/run/secrets/ayni/signer-provisioning') == 1
    init = cp.split('      initContainers:', 1)[1].split('      containers:', 1)[0]
    assert '/var/run/secrets/ayni/signer-provisioning' not in init, 'migrator must not mount API authority'
    assert 'independent-release-runtime' not in render, 'namespace cannot derive from release name'


for namespace in ('separate-signing-environment', 'a' * 54):
    for runtime in (False, True):
        check(namespace, runtime)
for options in ([], ['--set', 'signerProvisioning=null']):
    render = subprocess.check_output(['helm', 'template', 'disabled', str(CHART), *options], text=True)
    assert 'serviceAccountName: runtime-signer-provisioner' not in render
    assert 'mountPath: /var/run/secrets/ayni/signer-provisioning' not in render
    assert 'name: runtime-signer-provisioner' not in render
print('Signer provisioning chart gates passed: isolated namespaces, retention and scoped API mounts.')

# The full application currently reserves -sessions and caps namespaces at54.
# Exercise the shared runtime helper's defensive hash branch independently.
with tempfile.TemporaryDirectory() as directory:
    chart = Path(directory)
    (chart / 'templates').mkdir()
    (chart / 'Chart.yaml').write_text('apiVersion: v2\nname: runtime-namespace-fixture\nversion: 0.1.0\n')
    (chart / 'templates/_helpers.tpl').write_text((CHART / 'templates/_helpers.tpl').read_text())
    (chart / 'templates/value.yaml').write_text('apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {{ include "ayni-orchestrator.runtimeNamespace" . }}\n')
    namespace = 'a' * 63
    expected = namespace[:45] + '-' + hashlib.sha256(namespace.encode()).hexdigest()[:8] + '-runtime'
    render = subprocess.check_output(['helm', 'template', 'independent-release', str(chart), '--namespace', namespace], text=True)
    assert f'  name: {expected}' in render and len(expected) <= 63
