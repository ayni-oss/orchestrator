#!/usr/bin/env python3
"""Qualify the real supervisor's cleanup and restart on disposable Desktop Kubernetes.

The memory-only HTTP authority is a protocol fixture, not control-plane admission.
Namespace-only RBAC and synthetic credentials isolate all effects from live releases.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_IMAGE = 'local/ayni-runtime-worker@sha256:714c825045ea20996c8aefb330f3a29642b647bb6a7b53f96698af4e9c1bb7db'


def authority_module():
    spec = importlib.util.spec_from_file_location('supervisor_authority', ROOT / 'dev/fixtures/runtime-preparation/supervisor-authority.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def seed_claim(image, operation):
    now = int(time.time() * 1000)
    sha = 'sha256:' + 'a' * 64
    ids = {key: str(uuid.uuid4()) for key in ('repository_id', 'inspection_id', 'registry_connection_id', 'signer_id')}
    frozen = {key: '' for key in authority_module().FROZEN_FIELDS}
    frozen.update(ids, source_commit='a' * 40, source_tree='b' * 40, source_connection_id=str(uuid.uuid4()),
        source_connection_revision=1, source_credential_revision=str(uuid.uuid4()), installation_id=1,
        provider_repository_id=1, repository_full_name='ayni-oss/synthetic-cleanup-fixture',
        registry_connection_revision=1, registry_credential_revision=str(uuid.uuid4()), registry_destination='ghcr.io/ayni-oss/synthetic-cleanup-fixture',
        signer_revision=1, signer_active_revision=str(uuid.uuid4()), sources=[], policy_digest=sha, lock_digest=sha, lock_fingerprint=sha)
    profile = dict(profile_id='rootless-podman-vfs-userns-v1', qualified=True, platform='linux/arm64',
        worker_image=image, executor_image=image, verifier_image=image, trusted_cli_sha256=sha,
        max_cpu_millis=500, max_memory_mib=256, max_storage_mib=64, max_seconds=300, max_builds=1)
    frozen['profile_json'] = json.dumps(profile, separators=(',', ':'))
    evidence = {key: {} if key == 'details' else False if key in ('cleanup_complete', 'content_verified', 'managed_compatible') else '' for key in authority_module().EVIDENCE_FIELDS}
    op = dict(id=operation, idempotency_key='synthetic-supervisor-cleanup', **ids, platform='linux/arm64',
        state='cleanup_pending', stage='cleanup', safe_reason='cleanup_pending', generation=7,
        allow_build=False, allow_replace=False, max_builds=1, builds_started=0, deadline_unix_ms=now + 300000,
        accepted_at_unix_ms=now, updated_at_unix_ms=now, frozen_input_digest=sha, frozen=frozen, evidence=evidence)
    attempt = dict(id=str(uuid.uuid4()), operation_id=operation, stage='cleanup', generation=7,
        command_digest=sha, pod_uid='', state='claimed', deadline_unix_ms=now + 300000,
        accepted_at_unix_ms=now, report_digest='')
    return authority_module().validate_claim(dict(operation=op, attempt=attempt))


class Qualification:
    def __init__(self, args):
        self.args = args
        self.namespace = 'ayni-supervisor-qual-' + uuid.uuid4().hex[:10]
        self.labels = {'ayni.build/qualification': self.namespace}
        self.operation = str(uuid.uuid4())
        self.token = os.urandom(32).hex()
        self.forward = None
        self.created = False
        self.evidence = {'schema_version': 1, 'qualification': 'real-supervisor-cleanup-restart',
            'context': args.context, 'namespace': self.namespace, 'worker_image': args.worker_image,
            'authority': 'synthetic-memory-only', 'control_plane_admission_qualified': False,
            'chart_deployment_qualified': False, 'namespace_cleaned': False}

    def kube(self, *args, value=None):
        result = subprocess.run(['kubectl', '--context', self.args.context, '-n', self.namespace, *args],
            input=json.dumps(value) if value is not None else None, text=True, capture_output=True, timeout=90)
        if result.returncode:
            raise RuntimeError('kubectl_' + args[0] + '_failed')
        return result.stdout

    def apply(self, value):
        value.setdefault('metadata', {}).setdefault('labels', {}).update(self.labels)
        self.kube('apply', '-f', '-', value=value)

    def get(self, kind, name):
        value = self.kube('get', kind, name, '--ignore-not-found', '-o', 'json')
        return json.loads(value) if value.strip() else None

    def until(self, predicate, reason, seconds=100):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(1)
        raise RuntimeError(reason)

    def request(self, path, value=None):
        request = urllib.request.Request('http://127.0.0.1:' + str(self.port) + path,
            data=json.dumps(value or {}).encode(), headers={'Authorization': 'Bearer ' + self.token,
                'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=5) as response:
            return json.load(response)

    def start_forward(self):
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        self.forward = subprocess.Popen(['kubectl', '--context', self.args.context, '-n', self.namespace,
            'port-forward', 'pod/authority', str(self.port) + ':8080'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(40):
            try:
                self.request('/result')
                return
            except OSError:
                time.sleep(0.25)
        raise RuntimeError('authority_forward_unavailable')

    def pod(self, name, command, labels=None):
        return {'apiVersion': 'v1', 'kind': 'Pod', 'metadata': {'name': name, 'labels': labels or {}},
            'spec': {'hostUsers': False, 'automountServiceAccountToken': False, 'restartPolicy': 'Never',
                'terminationGracePeriodSeconds': 5,
                'securityContext': {'runAsUser': 1000, 'runAsGroup': 1000, 'fsGroup': 1000},
                'containers': [{'name': 'main', 'image': self.args.worker_image, 'imagePullPolicy': 'Never', 'command': command,
                    'securityContext': {'allowPrivilegeEscalation': False, 'readOnlyRootFilesystem': True,
                        'capabilities': {'drop': ['ALL']}, 'seccompProfile': {'type': 'RuntimeDefault'}},
                    'resources': {'limits': {'cpu': '500m', 'memory': '256Mi'}}}]}}

    def setup(self):
        self.kube('create', '-f', '-', value={'apiVersion': 'v1', 'kind': 'Namespace',
            'metadata': {'name': self.namespace, 'labels': self.labels}})
        self.created = True
        self.apply({'apiVersion': 'v1', 'kind': 'Secret', 'metadata': {'name': 'authority'},
            'immutable': True, 'stringData': {'token': self.token}})
        self.apply({'apiVersion': 'v1', 'kind': 'ConfigMap', 'metadata': {'name': 'authority-code'},
            'data': {'authority.py': (ROOT / 'dev/fixtures/runtime-preparation/supervisor-authority.py').read_text()}})
        authority = self.pod('authority', ['python3', '/fixture/authority.py'], {'role': 'authority'})
        authority['spec']['containers'][0]['volumeMounts'] = [
            {'name': 'code', 'mountPath': '/fixture', 'readOnly': True}, {'name': 'authority', 'mountPath': '/authority', 'readOnly': True}]
        authority['spec']['volumes'] = [{'name': 'code', 'configMap': {'name': 'authority-code'}},
            {'name': 'authority', 'secret': {'secretName': 'authority'}}]
        self.apply(authority)
        self.apply({'apiVersion': 'v1', 'kind': 'Service', 'metadata': {'name': 'authority'},
            'spec': {'selector': self.labels | {'role': 'authority'}, 'ports': [{'port': 8080, 'targetPort': 8080}]}})
        self.kube('wait', '--for=condition=Ready', 'pod/authority', '--timeout=90s')
        self.start_forward()
        self.claim = seed_claim(self.args.worker_image, self.operation)
        self.request('/configure', self.claim)
        self.apply({'apiVersion': 'v1', 'kind': 'ServiceAccount', 'metadata': {'name': 'supervisor'}})
        self.apply({'apiVersion': 'rbac.authorization.k8s.io/v1', 'kind': 'Role', 'metadata': {'name': 'supervisor'},
            'rules': [{'apiGroups': [''], 'resources': ['pods', 'persistentvolumeclaims', 'secrets'],
                'verbs': ['get', 'list', 'create', 'delete']}, {'apiGroups': [''], 'resources': ['pods/log'], 'verbs': ['get']}]})
        self.apply({'apiVersion': 'rbac.authorization.k8s.io/v1', 'kind': 'RoleBinding', 'metadata': {'name': 'supervisor'},
            'roleRef': {'apiGroup': 'rbac.authorization.k8s.io', 'kind': 'Role', 'name': 'supervisor'},
            'subjects': [{'kind': 'ServiceAccount', 'name': 'supervisor', 'namespace': self.namespace}]})
        owned = {'app.kubernetes.io/managed-by': 'ayni-runtime-supervisor', 'ayni.build/operation': self.operation,
            'ayni.build/stage': 'build', 'ayni.build/generation': '1'}
        for prefix, labels in [('obsolete', owned), ('unrelated', {'role': 'unrelated'})]:
            self.apply(self.pod(prefix, ['/bin/sh', '-c', 'exec sleep 600'], labels))
            self.apply({'apiVersion': 'v1', 'kind': 'PersistentVolumeClaim', 'metadata': {'name': prefix, 'labels': labels},
                'spec': {'accessModes': ['ReadWriteOnce'], 'resources': {'requests': {'storage': '8Mi'}}}})
            self.apply({'apiVersion': 'v1', 'kind': 'Secret', 'metadata': {'name': prefix, 'labels': labels},
                'immutable': True, 'stringData': {'fixture': 'synthetic-only'}})
        self.unrelated_uids = {(kind, name): self.get(kind, name)['metadata']['uid']
            for kind, name in [('pod', 'unrelated'), ('pvc', 'unrelated'), ('secret', 'unrelated'), ('secret', 'authority')]}

    def supervisor(self):
        pod = self.pod('supervisor', ['/usr/local/bin/ayni-runtime-supervisor'], {'role': 'supervisor'})
        pod['spec']['automountServiceAccountToken'] = True
        pod['spec']['serviceAccountName'] = 'supervisor'
        pod['spec']['containers'][0]['env'] = [
            {'name': 'ORCHESTRATOR_RUNTIME_NAMESPACE', 'value': self.namespace},
            {'name': 'ORCHESTRATOR_RUNTIME_AUTHORITY_URL', 'value': 'http://authority.' + self.namespace + '.svc:8080'},
            {'name': 'ORCHESTRATOR_RUNTIME_AUTHORITY_TOKEN_PATH', 'value': '/runtime-authority/token'},
            {'name': 'ORCHESTRATOR_RUNTIME_PULL_POLICY', 'value': 'Never'}]
        pod['spec']['containers'][0]['volumeMounts'] = [{'name': 'authority', 'mountPath': '/runtime-authority', 'readOnly': True}]
        pod['spec']['volumes'] = [{'name': 'authority', 'secret': {'secretName': 'authority'}}]
        return pod

    def qualify(self):
        self.apply(self.supervisor())
        result = self.until(lambda: (r if (r := self.request('/result'))['pod_uid'] else None), 'cleanup_not_bound')
        cleanup_name = 'runtime-' + uuid.UUID(self.operation).hex[:20] + '-cleanup-7'
        cleanup_uid = result['pod_uid']
        cleanup = self.get('pod', cleanup_name)
        if not cleanup or cleanup['metadata']['uid'] != cleanup_uid:
            raise RuntimeError('bound_cleanup_identity_missing')
        if cleanup['spec']['containers'][0]['image'] != self.args.worker_image or cleanup['spec'].get('automountServiceAccountToken') is not False:
            raise RuntimeError('cleanup_workload_identity_mismatch')
        if any('secret' in volume for volume in cleanup['spec'].get('volumes', [])):
            raise RuntimeError('cleanup_has_unexpected_secret_mount')
        self.evidence['cleanup_pod_uid'] = cleanup_uid
        self.evidence['cleanup_immutable_image'] = cleanup['spec']['containers'][0]['image']
        self.until(lambda: self.request('/result')['blocked_reports'] > 0, 'cleanup_report_not_held')
        old_supervisor = self.get('pod', 'supervisor')['metadata']['uid']
        self.kube('delete', 'pod', 'supervisor', '--wait=true', '--timeout=60s')
        polls = self.request('/result')['polls']
        blocked = self.request('/result')['blocked_reports']
        self.apply(self.supervisor())
        self.until(lambda: self.request('/result')['polls'] > polls and self.request('/result')['blocked_reports'] > blocked,
            'restarted_supervisor_did_not_reconcile')
        current = self.get('pod', cleanup_name)
        if not current or current['metadata']['uid'] != cleanup_uid:
            raise RuntimeError('cleanup_recreated_after_supervisor_restart')
        new_supervisor = self.get('pod', 'supervisor')['metadata']['uid']
        if old_supervisor == new_supervisor:
            raise RuntimeError('supervisor_not_restarted')
        self.evidence.update(supervisor_before_uid=old_supervisor, supervisor_after_uid=new_supervisor,
            same_cleanup_uid_after_restart=True)
        for kind in ('pod', 'pvc', 'secret'):
            if self.get(kind, 'obsolete') is not None:
                raise RuntimeError('owned_cleanup_resource_retained')
        self.request('/release')
        self.until(lambda: self.request('/result')['report_accepted'], 'cleanup_report_not_accepted')
        self.until(lambda: self.get('pod', cleanup_name) is None, 'cleanup_pod_not_retired')
        before = self.request('/result')
        if before['stopped']:
            raise RuntimeError('stopped_before_external_absence_confirmation')
        self.request('/observe-absent', {'pod_uid': cleanup_uid})
        final = self.until(lambda: (r if (r := self.request('/result'))['stopped'] else None), 'stopped_not_confirmed')
        for (kind, name), uid in self.unrelated_uids.items():
            resource = self.get(kind, name)
            if not resource or resource['metadata']['uid'] != uid:
                raise RuntimeError('unrelated_resource_changed')
        self.evidence.update(owned_resources_deleted=True, unrelated_resources_retained=True,
            stopped_only_after_exact_pod_absence=True, authority_result=final, state='passed')

    def cleanup(self):
        if self.forward:
            self.forward.terminate()
            self.forward.wait(timeout=10)
        if self.created:
            namespace = self.get('namespace', self.namespace)
            if namespace is None or namespace['metadata'].get('labels', {}).get('ayni.build/qualification') != self.namespace:
                raise RuntimeError('namespace_cleanup_ownership_mismatch')
            self.kube('delete', 'namespace', self.namespace, '--wait=true', '--timeout=120s')
            self.evidence['namespace_cleaned'] = True


def self_test():
    module = authority_module()
    claim = seed_claim(DEFAULT_IMAGE, str(uuid.uuid4()))
    state = module.State('a' * 64)
    assert state.master('/configure', claim)[0] == 200
    binding = state.binding() | {'pod_uid': str(uuid.uuid4())}
    assert state.master('/v1/runtime-supervisor', {'action': 'bind', 'binding': binding})[0] == 200
    assert state.master('/v1/runtime-supervisor', {'action': 'stopped', 'binding': binding})[0] == 409
    report = dict(binding=binding, outcome='completed', safe_reason='', evidence=claim['operation']['evidence'] | {'cleanup_complete': True})
    path = '/v1/runtime-worker/{operation_id}/{stage}/{generation}'.format(**binding)
    args = (path, {'action': 'report', 'report': report}, 'Bearer ' + module.capability(state.token, binding), binding['pod_uid'], binding['command_digest'])
    assert state.worker(*args)[0] == 503
    assert state.worker(*args[:-2], str(uuid.uuid4()), args[-1])[0] == 403
    assert state.worker(path, args[1], 'Bearer ' + state.token, *args[-2:])[0] == 403
    state.master('/release', {})
    assert state.worker(*args)[0] == 200
    assert state.master('/v1/runtime-supervisor', {'action': 'stopped', 'binding': binding})[0] == 409
    state.master('/observe-absent', {'pod_uid': binding['pod_uid']})
    assert state.master('/v1/runtime-supervisor', {'action': 'stopped', 'binding': binding})[0] == 200
    assert state.master('/v1/runtime-supervisor', {'action': 'poll'}) == (200, None)
    for changed in ('operation_id', 'stage', 'generation', 'command_digest'):
        other = binding | {changed: 9 if changed == 'generation' else 'other'}
        assert module.capability(state.token, binding) != module.capability(state.token, other)
    try:
        module.validate_claim(claim | {'extra': True})
    except ValueError:
        pass
    else:
        raise AssertionError('strict_fixture_schema')
    print('supervisor fixture protocol self-test passed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker-image', default=DEFAULT_IMAGE)
    parser.add_argument('--context', default=os.environ.get('AYNI_RUNTIME_KUBE_CONTEXT', 'docker-desktop'),
        choices=('docker-desktop', 'kind-ayni-runtime-qualification'))
    parser.add_argument('--evidence', default='/tmp/ayni-runtime-supervisor-qualification.json')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not re.fullmatch(r'(?:docker\.io/)?local/ayni-runtime-worker@sha256:[a-f0-9]{64}', args.worker_image):
        raise SystemExit('immutable_local_worker_image_required')
    if args.context == 'kind-ayni-runtime-qualification' and os.environ.get('KUBECONFIG') != '/tmp/ayni-runtime-cluster/kubeconfig':
        raise SystemExit('qualification_cluster_requires_exact_dedicated_kubeconfig')
    qualification = Qualification(args)
    try:
        qualification.setup()
        qualification.qualify()
    except Exception as error:
        qualification.evidence.update(state='failed', safe_reason=str(error) if isinstance(error, RuntimeError) else type(error).__name__)
        raise
    finally:
        try:
            qualification.cleanup()
        finally:
            Path(args.evidence).write_text(json.dumps(qualification.evidence, indent=2) + '\n')
    print(json.dumps(qualification.evidence, indent=2))


if __name__ == '__main__':
    main()
