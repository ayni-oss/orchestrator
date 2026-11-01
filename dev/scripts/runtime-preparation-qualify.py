#!/usr/bin/env python3
"""Run real isolated runtime stages on an explicit local cluster's owned namespace.

This is component qualification, not real GitHub/GHCR or control-plane admission.
Only synthetic fixtures and uniquely owned pods/PVCs/Secrets are used.
"""
import argparse
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import tarfile
import re
import tomllib
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[2]
RELEASE = json.loads((ROOT / 'scripts/ci/ayni-tool.json').read_text())


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs).stdout


def sha(data):
    return 'sha256:' + hashlib.sha256(data).hexdigest()


class Qualification:
    def __init__(self, args):
        self.args = args
        targets = {'docker-desktop': ('desktop-control-plane', 'desktop'),
            'kind-ayni-runtime-qualification': ('ayni-runtime-qualification-control-plane', 'ayni-runtime-qualification')}
        self.context = args.context
        self.node, self.cluster = targets[self.context]
        if args.kubeconfig:
            os.environ['KUBECONFIG'] = str(Path(args.kubeconfig).resolve())
        elif self.context != 'docker-desktop':
            raise RuntimeError('dedicated_context_requires_explicit_kubeconfig')
        self.namespace = 'ayni-runtime-qual-' + uuid.uuid4().hex[:10]
        self.operation = str(uuid.uuid4())
        self.directory = Path(tempfile.mkdtemp(prefix='ayni-runtime-qual-'))
        self.token = os.urandom(32).hex()
        self.evidence = {}
        self.forward = None
        self.claim = None
        self.counter = 0
        self.created_namespace = False
        self.registry_name = None
        self.signer_name = 'ayni-runtime-signer-fixture'
        self.imported_images = []
        self.owned = {'ayni.build/qualification': self.namespace}

    def kube(self, *args, body=None):
        command = ['kubectl', '--context', self.context, '-n', self.namespace, *args]
        return run(command, input=json.dumps(body) if body else None)

    def apply(self, value):
        value.setdefault('metadata', {}).setdefault('labels', {}).update(self.owned)
        self.kube('apply', '-f', '-', body=value)

    def headroom(self):
        if shutil.disk_usage(ROOT).free < 9 * 1024 ** 3:
            raise RuntimeError('host_headroom_guard')

    def wait(self, name, timeout=1800):
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            self.headroom()
            pod = json.loads(self.kube('get', 'pod', name, '-o', 'json'))
            if pod.get('status', {}).get('phase') in ('Succeeded', 'Failed'):
                return pod
            time.sleep(2)
        raise RuntimeError('qualification_stage_deadline')

    def request(self, path, value):
        request = urllib.request.Request('http://127.0.0.1:' + str(self.port) + path,
            data=json.dumps(value).encode(), headers={'Authorization': 'Bearer ' + self.token,
                'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)

    def setup(self):
        self.headroom()
        node = self.docker('inspect', self.node, '--format', '{{index .Config.Labels "io.x-k8s.kind.cluster"}} {{index .Config.Labels "io.x-k8s.kind.role"}}').strip()
        if node != self.cluster + ' control-plane':
            raise RuntimeError('Docker_Desktop_node_identity_mismatch')
        self.kube('create', '-f', '-', body={'apiVersion': 'v1', 'kind': 'Namespace', 'metadata': {'name': self.namespace, 'labels': self.owned}})
        self.created_namespace = True
        self.apply({'apiVersion': 'v1', 'kind': 'Secret', 'metadata': {'name': 'authority'},
            'immutable': True, 'stringData': {'token': self.token}})
        self.apply({'apiVersion': 'v1', 'kind': 'ConfigMap', 'metadata': {'name': 'authority-code'},
            'data': {'authority.py': (ROOT / 'dev/fixtures/runtime-preparation/authority.py').read_text()}})
        self.apply({'apiVersion': 'v1', 'kind': 'Pod', 'metadata': {'name': 'authority'}, 'spec': {
            'hostUsers': False, 'automountServiceAccountToken': False, 'restartPolicy': 'Never',
            'securityContext': {'runAsUser': 1000, 'runAsGroup': 1000, 'fsGroup': 1000},
            'containers': [{'name': 'authority', 'image': self.args.worker_image, 'imagePullPolicy': 'Never',
                'command': ['python3', '/fixture/authority.py'],
                'securityContext': {'allowPrivilegeEscalation': False, 'readOnlyRootFilesystem': True,
                    'capabilities': {'drop': ['ALL']}, 'seccompProfile': {'type': 'RuntimeDefault'}},
                'resources': {'limits': {'cpu': '500m', 'memory': '128Mi'}},
                'volumeMounts': [{'name': 'code', 'mountPath': '/fixture', 'readOnly': True},
                    {'name': 'authority', 'mountPath': '/authority', 'readOnly': True}]}],
            'volumes': [{'name': 'code', 'configMap': {'name': 'authority-code'}},
                {'name': 'authority', 'secret': {'secretName': 'authority'}}]}})
        self.kube('wait', '--for=condition=Ready', 'pod/authority', '--timeout=90s')
        self.apply({'apiVersion': 'v1', 'kind': 'Service', 'metadata': {'name': 'authority'}, 'spec': {
            'selector': self.owned | {'role': 'authority'}, 'ports': [{'port': 8080, 'targetPort': 8080}]}})
        self.kube('label', 'pod', 'authority', 'role=authority')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        self.forward = subprocess.Popen(['kubectl', '--context', self.context, '-n', self.namespace,
            'port-forward', 'pod/authority', str(self.port) + ':8080'], stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL)
        for _ in range(30):
            try:
                self.request('/result', {})
                break
            except (OSError, ValueError):
                time.sleep(0.2)
        self.apply({'apiVersion': 'networking.k8s.io/v1', 'kind': 'NetworkPolicy', 'metadata': {'name': 'deny-default'},
            'spec': {'podSelector': {}, 'policyTypes': ['Ingress', 'Egress']}})
        self.apply({'apiVersion': 'networking.k8s.io/v1', 'kind': 'NetworkPolicy', 'metadata': {'name': 'fixture-authority'},
            'spec': {'podSelector': {'matchLabels': {'role': 'authority'}}, 'policyTypes': ['Ingress'],
                'ingress': [{'from': [{'podSelector': {'matchLabels': {'role': 'worker'}}}],
                    'ports': [{'protocol': 'TCP', 'port': 8080}]}]}})
        self.apply({'apiVersion': 'networking.k8s.io/v1', 'kind': 'NetworkPolicy', 'metadata': {'name': 'worker-authority'},
            'spec': {'podSelector': {'matchLabels': {'role': 'worker'}}, 'policyTypes': ['Egress'],
                'egress': [{'to': [{'podSelector': {'matchLabels': {'role': 'authority'}}}],
                    'ports': [{'protocol': 'TCP', 'port': 8080}]},
                    {'to': [{'namespaceSelector': {'matchLabels': {'kubernetes.io/metadata.name': 'kube-system'}}}],
                        'ports': [{'protocol': 'UDP', 'port': 53}, {'protocol': 'TCP', 'port': 53}]}]}})
        self.apply({'apiVersion': 'networking.k8s.io/v1', 'kind': 'NetworkPolicy', 'metadata': {'name': 'builder-public'},
            'spec': {'podSelector': {'matchLabels': {'stage': 'build'}}, 'policyTypes': ['Egress'],
                'egress': [{'to': [{'ipBlock': {'cidr': '0.0.0.0/0', 'except': ['0.0.0.0/8','10.0.0.0/8',
                    '100.64.0.0/10','127.0.0.0/8','169.254.0.0/16','172.16.0.0/12','192.168.0.0/16','224.0.0.0/4','240.0.0.0/4']}}],
                    'ports': [{'protocol': 'TCP', 'port': 80}, {'protocol': 'TCP', 'port': 443}]}]}})
        self.apply({'apiVersion': 'v1', 'kind': 'ConfigMap', 'metadata': {'name': 'engine'}, 'data': {
            'storage.conf': '[storage]\ndriver="vfs"\ngraphroot="/scratch/store"\nrunroot="/scratch/run"\n',
            'containers.conf': '[containers]\ncgroups="disabled"\n'}})
        for role in ['source', 'candidate', 'frozen', 'certification', 'final', 'pull']:
            self.apply({'apiVersion': 'v1', 'kind': 'PersistentVolumeClaim', 'metadata': {'name': role},
                'spec': {'accessModes': ['ReadWriteOnce'], 'resources': {'requests': {'storage': '8Gi'}}}})

    def make_source(self):
        source = self.directory / 'source'
        private = self.directory / 'private.pem'
        if self.args.reuse_public_evidence:
            previous = Path(self.args.reuse_public_evidence)
            shutil.copytree(previous / 'source', source)
            policy = tomllib.loads((source / '.ayni.toml').read_text())
            self.public = policy['environment']['certificate']['trusted_keys']['runtime-fixture']
            seed = os.urandom(32).hex()
            for stage in ['build', 'freeze_candidate', 'certify', 'finalize']:
                report = json.loads((previous / (stage + '.report.json')).read_text())
                for key, value in report['evidence'].items():
                    if value:
                        self.evidence[key] = value
        else:
            shutil.copytree(ROOT / 'dev/fixtures/runtime-preparation/source', source)
            private = self.directory / 'private.pem'
            run(['openssl', 'genpkey', '-algorithm', 'ED25519', '-out', str(private)])
            os.chmod(private, 0o600)
            public = subprocess.check_output(['openssl', 'pkey', '-in', str(private), '-pubout', '-outform', 'DER'])
            seed_der = subprocess.check_output(['openssl', 'pkey', '-in', str(private), '-outform', 'DER'])
            if len(public) != 44 or len(seed_der) != 48:
                raise RuntimeError('unexpected_ed25519_encoding')
            self.public = public[-32:].hex()
            seed = seed_der[-32:].hex()
            policy = source / '.ayni.toml'
            policy.write_text(policy.read_text().replace(
                'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a', self.public))
            env = os.environ | {'DOCKER_CONFIG': self.args.docker_config,
                'DOCKER_HOST': 'unix://' + str(Path.home() / '.docker/run/docker.sock')}
            env.pop('DOCKER_CONTEXT', None)
            run(['docker', 'run', '--rm', '--user', str(os.getuid()) + ':' + str(os.getgid()),
                '--mount', f'type=bind,src={source},dst=/source',
                '--mount', f'type=bind,src={self.args.cli},dst=/tools/ayni,readonly',
                '--mount', f'type=bind,src={self.args.mise},dst=/tools/mise,readonly',
                '--entrypoint', '/bin/sh', self.args.executor_image,
                '-c', 'export PATH=/tools:$PATH HOME=/tmp; /tools/ayni env lock --repo-root /source'], env=env)
        self.source = source
        self.apply({'apiVersion': 'v1', 'kind': 'Secret', 'metadata': {'name': self.signer_name},
            'immutable': True, 'stringData': {'seed': seed}})
        private.unlink(missing_ok=True)
        self.signer_uid = self.kube('get', 'secret', self.signer_name, '-o', 'jsonpath={.metadata.uid}')
        files = {p.name: p.read_text() for p in source.iterdir() if p.is_file()}
        self.apply({'apiVersion': 'v1', 'kind': 'ConfigMap', 'metadata': {'name': 'source-fixture'}, 'data': files})
        command = 'mkdir -p /outputs/checkout /outputs/locked; ' + '; '.join(
            f'cp -L /fixture/{name} /outputs/checkout/{name}; cp -L /fixture/{name} /outputs/locked/{name}; chmod 0644 /outputs/checkout/{name} /outputs/locked/{name}' for name in sorted(files))
        pod = self.pod('populate-source', 'source', [], ['/bin/sh', '-ec', command])
        pod['spec']['volumes'].append({'name': 'fixture', 'configMap': {'name': 'source-fixture'}})
        pod['spec']['containers'][0]['volumeMounts'].append({'name': 'fixture', 'mountPath': '/fixture', 'readOnly': True})
        self.apply(pod)
        result = self.wait('populate-source', 90)
        if result['status']['phase'] != 'Succeeded':
            raise RuntimeError('source_population_failed')
        self.kube('delete', 'pod', 'populate-source', '--wait=true')
        digest = hashlib.sha256(b'ayni.runtime.source.v1\0')
        for name in sorted(files):
            data = files[name].encode()
            encoded = name.encode()
            digest.update(struct.pack('>Q', len(encoded)) + encoded + struct.pack('>Q', len(data))
                + struct.pack('>I', 0o644) + hashlib.sha256(data).digest())
        self.evidence['source_digest'] = 'sha256:' + digest.hexdigest()
        self.files = files

    def pod(self, name, output, inputs, command=None, builder=False):
        volumes = [{'name': 'scratch', 'emptyDir': {'sizeLimit': '8Gi'}},
            {'name': 'tmp', 'emptyDir': {'sizeLimit': '256Mi'}},
            {'name': 'home', 'emptyDir': {'sizeLimit': '64Mi'}}]
        mounts = [{'name': 'scratch', 'mountPath': '/scratch'}, {'name': 'tmp', 'mountPath': '/tmp'},
            {'name': 'home', 'mountPath': '/home/podman'}]
        for role in inputs + ([output] if output else []):
            volumes.append({'name': role, 'persistentVolumeClaim': {'claimName': role, 'readOnly': role != output}})
            mounts.append({'name': role, 'mountPath': '/outputs' if role == output else '/inputs/' + role,
                'readOnly': role != output})
        security = {'runAsUser': 1000, 'runAsGroup': 1000, 'allowPrivilegeEscalation': builder,
            'readOnlyRootFilesystem': True, 'capabilities': {'drop': ['ALL']},
            'seccompProfile': {'type': 'Unconfined' if builder else 'RuntimeDefault'}}
        if builder:
            security['capabilities']['add'] = ['SETUID', 'SETGID']
            security['procMount'] = 'Unmasked'
            volumes.append({'name': 'engine', 'configMap': {'name': 'engine'}})
            mounts.append({'name': 'engine', 'mountPath': '/runtime-config', 'readOnly': True})
        container = {'name': 'stage', 'image': self.args.worker_image, 'imagePullPolicy': 'Never',
            'command': command or ['/usr/local/bin/ayni-runtime-worker'], 'securityContext': security,
            'volumeMounts': mounts, 'resources': {'limits': {'cpu': '2', 'memory': '2Gi', 'ephemeral-storage': '12Gi'}}}
        return {'apiVersion': 'v1', 'kind': 'Pod', 'metadata': {'name': name}, 'spec': {
            'hostUsers': False, 'automountServiceAccountToken': False, 'restartPolicy': 'Never',
            'activeDeadlineSeconds': 1800, 'terminationGracePeriodSeconds': 5,
            'securityContext': {'fsGroup': 1000, 'seccompProfile': {'type': 'RuntimeDefault'}},
            'containers': [container], 'volumes': volumes}}

    def stage(self, stage, output, inputs, expected_failure=None, expected_outcome='failed'):
        self.counter += 1
        name = 'stage-' + stage.replace('_', '-') + '-' + str(self.counter)
        now = int(time.time() * 1000)
        lock = json.loads(self.files['.ayni.lock'])
        profile = {'profile_id': 'rootless-podman-vfs-userns-v1', 'qualified': False,
            'platform': 'linux/arm64', 'worker_image': self.args.worker_image,
            'executor_image': self.args.executor_image, 'verifier_image': self.args.probe_image,
            'trusted_cli_sha256': self.args.executor_cli_sha256, 'max_cpu_millis': 2000,
            'max_memory_mib': 2048, 'max_storage_mib': 8192, 'max_seconds': 1800, 'max_builds': 1}
        frozen = {key: 'fixture' for key in ['repository_id', 'inspection_id', 'source_tree',
            'source_connection_id', 'source_credential_revision', 'repository_full_name',
            'registry_connection_id', 'registry_credential_revision', 'signer_id', 'signer_active_revision']}
        frozen.update(source_commit='1' * 40, source_connection_revision=1, installation_id=1,
            provider_repository_id=1, registry_connection_revision=1,
            registry_destination='ghcr.io/ayni-oss/runtime-qualification', discovery_tag='fixture',
            reusable_image='', publication_tag='fixture', signer_revision=1, signer_key_id='runtime-fixture',
            signer_public_key=self.public, signer_secret_name=self.signer_name, signer_secret_uid=self.signer_uid,
            policy=self.files['.ayni.toml'], lock=self.files['.ayni.lock'],
            policy_digest=sha(self.files['.ayni.toml'].encode()), lock_digest=sha(self.files['.ayni.lock'].encode()),
            lock_fingerprint=lock['fingerprint'], sources=[{'path': name, 'digest': sha(data.encode()),
                'content': list(data.encode())} for name, data in sorted(self.files.items())],
            profile_json=json.dumps(profile, separators=(',', ':')))
        operation = dict(id=self.operation, idempotency_key='fixture', repository_id='fixture',
            inspection_id='fixture', registry_connection_id='fixture', signer_id='fixture',
            platform='linux/arm64', state='running', stage=stage, safe_reason='', generation=self.counter,
            allow_build=True, allow_replace=False, max_builds=1, builds_started=1,
            deadline_unix_ms=now + 1800000, accepted_at_unix_ms=now, updated_at_unix_ms=now,
            frozen_input_digest=sha(json.dumps(frozen, sort_keys=True).encode()), frozen=frozen, evidence=self.evidence)
        attempt = dict(id=str(uuid.uuid4()), operation_id=self.operation, stage=stage, generation=self.counter,
            command_digest=sha((self.operation + stage + str(self.counter)).encode()), pod_uid='', state='claimed',
            deadline_unix_ms=now + 1800000, accepted_at_unix_ms=now, report_digest='')
        self.claim = {'operation': operation, 'attempt': attempt}
        self.request('/configure', self.claim)
        pod = self.pod(name, output, inputs, builder=stage == 'build')
        pod['metadata']['labels'] = {'role': 'worker', 'stage': stage}
        container = pod['spec']['containers'][0]
        values = {'OPERATION': self.operation, 'STAGE': stage, 'GENERATION': str(self.counter),
            'COMMAND_DIGEST': attempt['command_digest'], 'CAPABILITY_PATH': '/runtime-capability/token',
            'AUTHORITY_URL': f'http://authority.{self.namespace}.svc.cluster.local:8080'}
        container['env'] = [{'name': 'ORCHESTRATOR_RUNTIME_' + key, 'value': value} for key, value in values.items()]
        container['env'].append({'name': 'ORCHESTRATOR_RUNTIME_POD_UID', 'valueFrom': {'fieldRef': {'fieldPath': 'metadata.uid'}}})
        pod['spec']['volumes'].append({'name': 'authority', 'secret': {'secretName': 'authority', 'defaultMode': 288}})
        container['volumeMounts'].append({'name': 'authority', 'mountPath': '/runtime-capability', 'readOnly': True})
        if stage == 'certify':
            pod['spec']['volumes'].append({'name': 'signer', 'secret': {'secretName': self.signer_name, 'defaultMode': 288}})
            container['volumeMounts'].append({'name': 'signer', 'mountPath': '/runtime-signer', 'readOnly': True})
        self.apply(pod)
        result = self.wait(name)
        logs = self.kube('logs', name)
        (self.directory / (stage + '.log')).write_text(logs)
        report = json.loads(logs) if stage == 'certify' else self.request('/result', {})
        (self.directory / (stage + '.report.json')).write_text(json.dumps(report, indent=2))
        self.kube('delete', 'pod', name, '--wait=true', '--timeout=60s')
        if expected_failure is not None:
            if not report or report.get('outcome') != expected_outcome or report.get('safe_reason') != expected_failure:
                raise RuntimeError('negative_stage_did_not_fail_closed')
            (self.directory / ('negative-' + stage + '.json')).write_text(json.dumps(report, indent=2))
            print(stage + ': expected failure ' + expected_failure, flush=True)
            return
        if stage == 'certify' and report and report.get('outcome') == 'completed':
            identity = self.kube('get', 'secret', self.signer_name, '-o', 'jsonpath={.metadata.uid}:{.immutable}')
            if identity != self.signer_uid + ':true':
                raise RuntimeError('certification_signer_identity_changed')
        if not report or report['outcome'] != 'completed' or result['status']['phase'] != 'Succeeded':
            if stage == 'build' and (report or {}).get('safe_reason') == 'execution_failed':
                self.diagnose_build()
            raise RuntimeError(stage + ':' + (report or {}).get('safe_reason', 'report_unavailable'))
        for key, value in report['evidence'].items():
            if value:
                self.evidence[key] = value
        print(stage + ': completed', flush=True)

    def registry_denial(self):
        # Read-only public GHCR request through the real worker transport. Only
        # deliberately invalid fixture credentials can be returned by authority.
        self.apply({'apiVersion': 'networking.k8s.io/v1', 'kind': 'NetworkPolicy',
            'metadata': {'name': 'discover-public'}, 'spec': {
                'podSelector': {'matchLabels': {'stage': 'discover'}}, 'policyTypes': ['Egress'],
                'egress': [{'to': [{'ipBlock': {'cidr': '0.0.0.0/0', 'except': ['0.0.0.0/8',
                    '10.0.0.0/8', '100.64.0.0/10', '127.0.0.0/8', '169.254.0.0/16',
                    '172.16.0.0/12', '192.168.0.0/16', '224.0.0.0/4', '240.0.0.0/4']}}],
                    'ports': [{'protocol': 'TCP', 'port': 443}]}]}})
        retained = self.evidence
        self.evidence = {}
        try:
            self.stage('discover', None, [], expected_failure='registry_access_denied',
                expected_outcome='unavailable')
        finally:
            self.evidence = retained

    def diagnose_build(self):
        # No authority capability or signer is mounted in this diagnostic pod.
        # Only the synthetic source can contribute process output.
        command = 'set -eu; export HOME=/home/podman XDG_RUNTIME_DIR=/scratch/run CONTAINERS_STORAGE_CONF=/runtime-config/storage.conf CONTAINERS_CONF=/runtime-config/containers.conf BUILDAH_LAYERS=false AYNI_OCI_RUNTIME=podman; mkdir -p /scratch/run; cp -a /inputs/source/checkout /scratch/source; /usr/local/bin/ayni --version; podman info --format "{{.Host.Security.Rootless}}"; /usr/local/bin/ayni env build --repo-root /scratch/source --executor-image ' + self.args.executor_image + ' --tag localhost/qualification:diagnostic'
        pod = self.pod('diagnostic-build', None, ['source'], ['/bin/sh', '-ec', command], builder=True)
        pod['metadata']['labels'] = {'role': 'worker', 'stage': 'build'}
        self.apply(pod)
        self.wait('diagnostic-build')
        (self.directory / 'diagnostic-build.log').write_text(self.kube('logs', 'diagnostic-build'))
        self.kube('delete', 'pod', 'diagnostic-build', '--wait=true', '--timeout=60s')

    def tool_readiness(self, role="final"):
        command = """set -eu
export HOME=/home/podman TMPDIR=/tmp
/usr/bin/git --version
/usr/bin/podman --version
/usr/bin/skopeo --version
/usr/bin/umoci --version
/usr/local/bin/ayni --version
/usr/bin/skopeo copy --preserve-digests oci:/inputs/final/layout:runtime oci:/scratch/layout:roundtrip
python3 - <<'PYJSON'
import json
print(json.load(open('/scratch/layout/index.json'))['manifests'][0]['digest'])
PYJSON
"""
        command = command.replace('/inputs/final/', '/inputs/' + role + '/')
        self.apply(self.pod('tool-readiness', None, [role], ['/bin/sh', '-ec', command]))
        result = self.wait('tool-readiness', 120)
        logs = self.kube('logs', 'tool-readiness')
        (self.directory / 'tool-readiness.log').write_text(logs)
        self.kube('delete', 'pod', 'tool-readiness', '--wait=true')
        if result['status']['phase'] != 'Succeeded' or logs.strip().splitlines()[-1] != self.evidence['expected_manifest_digest']:
            raise RuntimeError('nonroot_worker_tool_readiness_failed')
        print('nonroot_worker_tools_and_OCI_copy: completed', flush=True)

    def freeze_public_baseline(self, layout):
        # Standard OCI transport only: preserve every referenced blob and normalize
        # the fixture tag. The real worker still performs the independent freeze.
        archive_path = self.directory / 'baseline-input.tar'
        index = json.loads((layout / 'index.json').read_text())
        index['manifests'][0].setdefault('annotations', {})['org.opencontainers.image.ref.name'] = 'runtime'
        import io
        with tarfile.open(archive_path, 'w') as archive:
            for name, data in [('index.json', json.dumps(index, separators=(',', ':')).encode()),
                    ('oci-layout', (layout / 'oci-layout').read_bytes())]:
                entry = tarfile.TarInfo(name)
                entry.size, entry.mode = len(data), 0o644
                archive.addfile(entry, io.BytesIO(data))
            for path in sorted((layout / 'blobs/sha256').iterdir()):
                if not re.fullmatch('[0-9a-f]{64}', path.name) or sha(path.read_bytes()) != 'sha256:' + path.name:
                    raise RuntimeError('baseline_blob_identity_invalid')
                archive.add(path, arcname='blobs/sha256/' + path.name, recursive=False)
        self.apply(self.pod('load-baseline', 'candidate', [], ['/bin/sh', '-ec', 'mkdir -p /outputs/layout; exec sleep 300']))
        self.kube('wait', '--for=condition=Ready', 'pod/load-baseline', '--timeout=90s')
        with archive_path.open('rb') as source:
            subprocess.run(['kubectl', '--context', self.context, '-n', self.namespace,
                'exec', '-i', 'load-baseline', '--', 'tar', '--no-same-owner', '-C', '/outputs/layout', '-xf', '-'],
                stdin=source, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
        self.kube('delete', 'pod', 'load-baseline', '--wait=true')
        archive_path.unlink()
        self.evidence['artifact_digest'] = self.evidence['expected_manifest_digest']
        self.stage('freeze_candidate', 'frozen', ['candidate'])
        self.wrong_signer()
        self.tool_readiness('candidate')
        self.cancellation()

    def wrong_signer(self):
        original = (self.signer_name, self.signer_uid)
        self.signer_name = 'ayni-runtime-signer-wrong'
        self.apply({'apiVersion': 'v1', 'kind': 'Secret', 'metadata': {'name': self.signer_name},
            'immutable': True, 'stringData': {'seed': os.urandom(32).hex()}})
        self.signer_uid = self.kube('get', 'secret', self.signer_name, '-o', 'jsonpath={.metadata.uid}')
        try:
            self.stage('certify', 'certification', ['source', 'frozen'], expected_failure='certification_failed')
        finally:
            self.signer_name, self.signer_uid = original

    def cancellation(self):
        command = 'set -eu; export HOME=/home/podman XDG_RUNTIME_DIR=/scratch/run TMPDIR=/tmp CONTAINERS_STORAGE_CONF=/runtime-config/storage.conf CONTAINERS_CONF=/runtime-config/containers.conf; mkdir -p /scratch/run; exec podman run --rm --network none --cgroups disabled docker.io/library/busybox@sha256:a4b7bbbb14a7e50bec3f6cbc4e2560c9b6792a7ae7062637bdfc25793817d4ff sleep 600'
        pod = self.pod('cancel-builder', None, [], ['/bin/sh', '-ec', command], builder=True)
        pod['metadata']['labels'] = {'role': 'worker', 'stage': 'build'}
        self.apply(pod)
        self.kube('wait', '--for=condition=Ready', 'pod/cancel-builder', '--timeout=90s')
        uid = self.kube('get', 'pod', 'cancel-builder', '-o', 'jsonpath={.metadata.uid}')
        mapping = self.kube('exec', 'cancel-builder', '--', 'cat', '/proc/self/uid_map').split()
        lower, count = int(mapping[1]), int(mapping[2])
        observed = []
        for _ in range(60):
            rows = self.docker('exec', self.node, 'ps', '-eo', 'pid,uid,comm').splitlines()[1:]
            # Desktop's procps renders high user-namespace UIDs as signed i32.
            observed = [row for row in rows if lower <= (int(row.split()[1]) & 0xffffffff) < lower + count]
            if any(row.split()[-1] == 'conmon' for row in observed) and any(row.split()[-1] == 'sleep' for row in observed):
                break
            self.headroom()
            time.sleep(1)
        else:
            (self.directory / 'cancel-builder.log').write_text(self.kube('logs', 'cancel-builder'))
            raise RuntimeError('nested_cancellation_probe_did_not_start')
        self.kube('delete', 'pod', 'cancel-builder', '--wait=true', '--timeout=60s')
        rows = self.docker('exec', self.node, 'ps', '-eo', 'pid,uid,comm').splitlines()[1:]
        retained = [row for row in rows if lower <= (int(row.split()[1]) & 0xffffffff) < lower + count]
        if retained:
            raise RuntimeError('nested_cancellation_process_retained')
        path = '/var/lib/kubelet/pods/' + uid + '/volumes/kubernetes.io~empty-dir/scratch'
        for _ in range(20):
            try:
                self.docker('exec', self.node, 'sh', '-c', 'test ! -e "$1"', 'sh', path)
                break
            except subprocess.CalledProcessError:
                time.sleep(1)
        else:
            raise RuntimeError('cancelled_scratch_retained')
        (self.directory / 'cancel-cleanup.json').write_text(json.dumps({'pod_uid': uid,
            'mapped_uid_start': lower, 'mapped_uid_count': count,
            'nested_processes_before': observed, 'remaining_processes': [], 'scratch_removed': True}, indent=2))
        print('nested_builder_cancellation_and_storage_cleanup: completed', flush=True)

    def docker(self, *args):
        env = os.environ | {'DOCKER_CONFIG': self.args.docker_config,
            'DOCKER_HOST': 'unix://' + str(Path.home() / '.docker/run/docker.sock')}
        env.pop('DOCKER_CONTEXT', None)
        return run(['docker', *args], env=env)

    def copy_oci_layout(self):
        pod = self.pod('export-final', None, ['final'], ['/bin/sh', '-c', 'exec sleep 600'])
        self.apply(pod)
        self.kube('wait', '--for=condition=Ready', 'pod/export-final', '--timeout=90s')
        archive_path = self.directory / 'final.tar'
        with archive_path.open('wb') as output:
            subprocess.run(['kubectl', '--context', self.context, '-n', self.namespace, 'exec',
                'export-final', '--', 'tar', '-C', '/inputs/final/layout', '-cf', '-', '.'],
                check=True, stdout=output, stderr=subprocess.PIPE)
        self.kube('delete', 'pod', 'export-final', '--wait=true')
        layout = self.directory / 'layout'
        layout.mkdir()
        total = 0
        with tarfile.open(archive_path) as archive:
            for entry in archive:
                if entry.isdir():
                    continue
                name = entry.name.removeprefix('./')
                if not entry.isfile() or not (name in ('index.json', 'oci-layout')
                        or re.fullmatch(r'blobs/sha256/[0-9a-f]{64}', name)):
                    raise RuntimeError('unexpected_OCI_archive_entry')
                total += entry.size
                if entry.size > 8 * 1024**3 or total > 8 * 1024**3:
                    raise RuntimeError('OCI_archive_bounds')
                destination = layout / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(entry) as source, destination.open('xb') as output:
                    digest = hashlib.sha256()
                    while chunk := source.read(65536):
                        digest.update(chunk)
                        output.write(chunk)
                if name.startswith('blobs/') and digest.hexdigest() != name.rsplit('/', 1)[1]:
                    raise RuntimeError('OCI_blob_identity_mismatch')
        index = json.loads((layout / 'index.json').read_text())
        if len(index['manifests']) != 1 or index['manifests'][0]['digest'] != self.evidence['expected_manifest_digest']:
            raise RuntimeError('final_manifest_identity_mismatch')
        return layout

    def registry_roundtrip(self, layout):
        self.registry_name = self.namespace + '-registry'
        registry = 'registry@sha256:fa647fc1e5d5df7d8d923fb6332aab8e78783f8fca1a1394efb4011f68f5a793'
        self.docker('run', '-d', '--rm', '--name', self.registry_name, '--network', 'none',
            '--label', 'ayni.build.qualification=' + self.namespace, '--read-only', '--memory', '256m', '--cpus', '1', '--pids-limit', '128',
            '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', registry)
        time.sleep(2)
        # This private HTTP registry has no external interface. Production transport
        # remains GHCR-only with TLS; there is no product override for this fixture.
        prefix = ['run', '--rm', '--network', 'container:' + self.registry_name, '--user', '0:0',
            '--read-only', '--tmpfs', '/tmp:rw,size=256m', '--cap-drop', 'ALL',
            '--security-opt', 'no-new-privileges', '--mount', f'type=bind,src={layout},dst=/layout,readonly']
        destination = '127.0.0.1:5000/runtime/fixture'
        self.docker(*prefix, '--entrypoint', '/usr/bin/skopeo', self.args.worker_image,
            'copy', '--preserve-digests', '--dest-tls-verify=false', 'oci:/layout:runtime', 'docker://' + destination + ':prepared')
        raw = self.docker(*prefix, '--entrypoint', '/usr/bin/skopeo', self.args.worker_image,
            'inspect', '--tls-verify=false', '--raw', 'docker://' + destination + ':prepared')
        # subprocess text decoding preserves JSON bytes; skopeo prints one terminal LF.
        expected = self.evidence['expected_manifest_digest']
        if sha(raw.encode()) != expected and sha(raw.removesuffix('\n').encode()) != expected:
            raise RuntimeError('registry_manifest_identity_mismatch')
        pulled = self.directory / 'pulled'
        pulled.mkdir()
        self.docker(*prefix, '--mount', f'type=bind,src={pulled},dst=/pulled',
            '--entrypoint', '/usr/bin/skopeo', self.args.worker_image, 'copy', '--preserve-digests',
            '--src-tls-verify=false', 'docker://' + destination + '@' + expected, 'oci:/pulled:runtime')
        index = json.loads((pulled / 'index.json').read_text())
        if index['manifests'][0]['digest'] != expected:
            raise RuntimeError('registry_readback_identity_mismatch')
        for path in (pulled / 'blobs/sha256').iterdir():
            if sha(path.read_bytes()) != 'sha256:' + path.name:
                raise RuntimeError('registry_readback_blob_mismatch')
        print('local_registry_digest_preserving_roundtrip: completed', flush=True)
        return pulled

    def import_candidate(self, layout, suffix):
        index = json.loads((layout / 'index.json').read_text())
        digest = index['manifests'][0]['digest']
        tag = 'docker.io/local/' + self.namespace + ':' + suffix
        index['manifests'][0].setdefault('annotations', {})['org.opencontainers.image.ref.name'] = tag
        (layout / 'index.json').write_text(json.dumps(index, separators=(',', ':')))
        archive_path = self.directory / (suffix + '-import.tar')
        with tarfile.open(archive_path, 'w') as archive:
            for path in sorted(layout.rglob('*')):
                if path.is_file():
                    archive.add(path, arcname=str(path.relative_to(layout)), recursive=False)
        env = os.environ | {'DOCKER_CONFIG': self.args.docker_config,
            'DOCKER_HOST': 'unix://' + str(Path.home() / '.docker/run/docker.sock')}
        env.pop('DOCKER_CONTEXT', None)
        with archive_path.open('rb') as source:
            subprocess.run(['docker', 'exec', '-i', self.node, 'ctr', '--namespace',
                'k8s.io', 'images', 'import', '--platform', 'linux/arm64', '-'], stdin=source,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, env=env)
        self.imported_images.append(tag)
        immutable = 'docker.io/local/' + self.namespace + '@' + digest
        self.docker('exec', self.node, 'ctr', '--namespace', 'k8s.io', 'images', 'tag', tag, immutable)
        self.imported_images.append(immutable)
        return immutable

    def probe(self, image, name, compatibility=False, parent_uid=0, parent_caps=None, no_new_privs=True):
        pod = self.pod(name, None, ['source'])
        pod['spec']['volumes'].extend([{'name': 'trusted', 'emptyDir': {'sizeLimit': '64Mi'}}])
        strict = {'runAsUser': 10001, 'runAsGroup': 10001, 'readOnlyRootFilesystem': True,
            'allowPrivilegeEscalation': False, 'capabilities': {'drop': ['ALL']},
            'seccompProfile': {'type': 'RuntimeDefault'}}
        install = {'name': 'install-probes', 'image': self.args.probe_image, 'imagePullPolicy': 'Never',
            'command': ['/runtime-worker', '--install-probes'], 'securityContext': copy.deepcopy(strict) | {'runAsUser': 0, 'runAsGroup': 0},
            'volumeMounts': [{'name': 'trusted', 'mountPath': '/trusted'}]}
        container = pod['spec']['containers'][0]
        container['image'] = image
        container['securityContext'] = strict
        container['volumeMounts'].append({'name': 'trusted', 'mountPath': '/trusted', 'readOnly': True})
        pod['spec']['initContainers'] = [install]
        if compatibility:
            strict['runAsUser'] = parent_uid
            strict['runAsGroup'] = parent_uid
            strict['allowPrivilegeEscalation'] = not no_new_privs
            pod['spec']['securityContext']['fsGroup'] = 10002
            strict['capabilities']['add'] = ['SETUID', 'SETGID'] if parent_caps is None else parent_caps
            for role in ['workspace', 'runtime-evidence', 'compat-cache', 'quality']:
                pod['spec']['volumes'].append({'name': role, 'emptyDir': {'sizeLimit': '2Gi'}})
            values = {'TRUSTED_CLI_SHA256': self.args.executor_cli_sha256,
                'CERTIFICATE_DIGEST': self.evidence['certificate_digest'],
                'LOCK_FINGERPRINT': json.loads(self.files['.ayni.lock'])['fingerprint'],
                'SOURCE_DIGEST': self.evidence['source_digest'], 'PLATFORM': 'linux/arm64', 'MAX_SECONDS': '120'}
            container['env'] = [{'name': 'ORCHESTRATOR_RUNTIME_' + key, 'value': value} for key, value in values.items()]
            prepare = {'name': 'compat-prepare', 'image': image, 'imagePullPolicy': 'Never',
                'command': ['/trusted/runtime-worker', '--compat-prepare'], 'env': container['env'],
                'securityContext': strict, 'volumeMounts': [{'name': 'trusted', 'mountPath': '/trusted', 'readOnly': True},
                    {'name': 'source', 'mountPath': '/inputs/source', 'readOnly': True},
                    {'name': 'workspace', 'mountPath': '/workspace'},
                    {'name': 'runtime-evidence', 'mountPath': '/runtime-evidence'},
                    {'name': 'compat-cache', 'mountPath': '/tmp'}]}
            if self.args.diagnose_quality:
                observer = "import os,time,stat\nseen={}; total=0\nwhile True:\n for name in os.listdir('/capture')[:64]:\n  if not name.startswith('.tmp'): continue\n  parent=None\n  try:\n   parent=os.open('/capture/'+name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)\n   for stream in ['stdout','stderr']:\n    fd=None\n    try:\n     fd=os.open(stream,os.O_RDONLY|os.O_NOFOLLOW,dir_fd=parent)\n     info=os.fstat(fd)\n     data=os.read(fd,8192) if stat.S_ISREG(info.st_mode) else b''\n     key=name+'/'+stream\n     if data and seen.get(key)!=data and total<65536:\n      print(key+': '+data.decode('utf-8','replace'),flush=True);seen[key]=data;total+=len(data)\n    except OSError: pass\n    finally:\n     if fd is not None: os.close(fd)\n  except OSError: pass\n  finally:\n   if parent is not None: os.close(parent)\n time.sleep(.001)\n"
                pod['spec']['initContainers'].append({'name': 'fixture-stderr-observer', 'restartPolicy': 'Always',
                    'image': self.args.worker_image, 'imagePullPolicy': 'Never', 'command': ['python3', '-c', observer],
                    'securityContext': {'runAsUser': 0, 'runAsGroup': 0, 'readOnlyRootFilesystem': True,
                        'allowPrivilegeEscalation': False, 'capabilities': {'drop': ['ALL']}},
                    'resources': {'limits': {'cpu': '100m', 'memory': '64Mi'}},
                    'volumeMounts': [{'name': 'compat-cache', 'mountPath': '/capture', 'readOnly': True}]})
            pod['spec']['initContainers'].append(prepare)
            container['command'] = ['/trusted/runtime-worker', '--compat-launch']
            container['volumeMounts'] = [m for m in container['volumeMounts'] if m['mountPath'] != '/tmp']
            container['volumeMounts'].extend([{'name': 'workspace', 'mountPath': '/workspace', 'readOnly': True},
                {'name': 'runtime-evidence', 'mountPath': '/runtime-evidence', 'readOnly': True},
                {'name': 'quality', 'mountPath': '/workspace/.ayni'}, {'name': 'compat-cache', 'mountPath': '/tmp'}])
        else:
            container['command'] = ['/trusted/ayni-runtime-verifier', '--source', '/source',
                '--expected-platform', 'linux/arm64', '--expected-cli-sha256', self.args.executor_cli_sha256]
            container['volumeMounts'].append({'name': 'source', 'mountPath': '/source', 'subPath': 'locked', 'readOnly': True})
        self.apply(pod)
        result = self.wait(name, 300)
        (self.directory / (name + '.status.json')).write_text(json.dumps(result, indent=2))
        prepare_report = None
        for init in pod['spec']['initContainers']:
            init_logs = self.kube('logs', name, '-c', init['name'])
            (self.directory / (name + '-' + init['name'] + '.log')).write_text(init_logs)
            if init['name'] == 'compat-prepare' and init_logs.strip():
                prepare_report = init_logs.strip()
        try:
            logs = self.kube('logs', name, '-c', 'stage')
        except subprocess.CalledProcessError:
            logs = prepare_report or '{"state":"invalid","safe_reason":"init_failed"}'
        (self.directory / (name + '.log')).write_text(logs)
        self.kube('delete', 'pod', name, '--wait=true')
        return json.loads(logs)

    def negative_parent_contexts(self, image):
        for name, options in [('parent-nonroot', {'parent_uid': 10001}),
                ('parent-no-caps', {'parent_caps': []}), ('parent-with-elevation', {'no_new_privs': False})]:
            result = self.probe(image, 'negative-' + name, compatibility=True, **options)
            if result.get('state') != 'invalid' or result.get('safe_reason') != 'invalid_input':
                raise RuntimeError('unsafe_parent_context_was_accepted')
        print('unsafe_parent_contexts_rejected: completed', flush=True)

    def negative_source(self, image):
        command = "printf '\\ntampered_source\\n' >> /outputs/checkout/index.js"
        pod = self.pod('tamper-source', 'source', [], ['/bin/sh', '-ec', command])
        self.apply(pod)
        self.wait('tamper-source', 90)
        self.kube('delete', 'pod', 'tamper-source', '--wait=true')
        try:
            result = self.probe(image, 'negative-source', compatibility=True)
            if result.get('state') != 'invalid' or result.get('safe_reason') != 'source_identity_mismatch':
                raise RuntimeError('changed_source_was_accepted')
        finally:
            pod = self.pod('restore-source', 'source', [], ['/bin/sh', '-ec',
                'cp -L /fixture/index.js /outputs/checkout/index.js; chmod 0644 /outputs/checkout/index.js'])
            pod['spec']['volumes'].append({'name': 'fixture', 'configMap': {'name': 'source-fixture'}})
            pod['spec']['containers'][0]['volumeMounts'].append({'name': 'fixture', 'mountPath': '/fixture', 'readOnly': True})
            self.apply(pod)
            self.wait('restore-source', 90)
            self.kube('delete', 'pod', 'restore-source', '--wait=true')
        print('changed_source_rejected: completed', flush=True)

    def negative_content(self, layout):
        tampered = self.directory / 'tampered'
        shutil.copytree(layout, tampered, copy_function=os.link)
        (tampered / 'index.json').unlink()
        (tampered / 'index.json').write_bytes((layout / 'index.json').read_bytes())
        original_index = json.loads((tampered / 'index.json').read_text())
        original_tag = original_index['manifests'][0]['annotations']['org.opencontainers.image.ref.name']
        layer = self.directory / 'tamper.tar'
        import io
        with tarfile.open(layer, 'w') as archive:
            data = b'tampered executable must never be executed\n'
            entry = tarfile.TarInfo('usr/local/bin/ayni')
            entry.size, entry.mode, entry.uid, entry.gid, entry.mtime = len(data), 0o555, 0, 0, 0
            archive.addfile(entry, io.BytesIO(data))
        self.docker('run', '--rm', '--network', 'none', '--user', '0:0', '--read-only',
            '--tmpfs', '/tmp:rw,size=64m', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
            '--mount', f'type=bind,src={tampered},dst=/layout', '--mount', f'type=bind,src={layer},dst=/tamper.tar,readonly',
            '--entrypoint', '/usr/bin/umoci', self.args.worker_image, 'raw', 'add-layer',
            '--image', '/layout:' + original_tag, '--tag', 'tampered', '--history.created',
            '1970-01-01T00:00:00Z', '--history.created_by', 'qualification-negative', '--compress', 'none', '/tamper.tar')
        index = json.loads((tampered / 'index.json').read_text())
        index['manifests'] = [m for m in index['manifests'] if m.get('annotations', {}).get('org.opencontainers.image.ref.name') == 'tampered']
        if len(index['manifests']) != 1:
            raise RuntimeError('tamper_fixture_manifest_invalid')
        # Break the shared index inode before changing its fixture-only annotation.
        (tampered / 'index.json').unlink()
        (tampered / 'index.json').write_text(json.dumps(index))
        image = self.import_candidate(tampered, 'tampered')
        result = self.probe(image, 'negative-content')
        if result.get('state') != 'invalid' or result.get('reason') != 'cli_digest_mismatch':
            raise RuntimeError('tampered_content_was_accepted')
        print('tampered_protected_content_rejected: completed', flush=True)

    def network_policy(self, image):
        self.apply({'apiVersion': 'networking.k8s.io/v1', 'kind': 'NetworkPolicy', 'metadata': {'name': 'network-control'},
            'spec': {'podSelector': {'matchLabels': {'stage': 'network-control'}}, 'policyTypes': ['Egress'],
                'egress': [{'to': [{'ipBlock': {'cidr': '1.1.1.1/32'}}], 'ports': [{'protocol': 'TCP', 'port': 443}]}]}})
        for allowed in [True, False]:
            name = 'network-control' if allowed else 'network-denied'
            # Public pinned mise resolves the already installed locked Node runtime;
            # no private filesystem layout or Ayni implementation is duplicated.
            script = "const s=require('net').connect({host:'1.1.1.1',port:443});s.setTimeout(2500);s.on('connect',()=>{console.log('connected');s.destroy();process.exit(" + ('0' if allowed else '7') + ")});s.on('error',()=>{console.log('rejected');process.exit(" + ('8' if allowed else '0') + ")});s.on('timeout',()=>{console.log('timeout');s.destroy();process.exit(" + ('9' if allowed else '0') + ")})"
            pod = self.pod(name, None, [], ['/usr/local/bin/mise', 'exec', 'node@22.14.0', '--', 'node', '-e', script])
            pod['metadata']['labels'] = {'stage': name}
            container = pod['spec']['containers'][0]
            container['image'] = image
            container['securityContext'].update(runAsUser=10002, runAsGroup=10002)
            container['env'] = [{'name': 'HOME', 'value': '/tmp'}, {'name': 'MISE_DATA_DIR', 'value': '/opt/ayni/mise'},
                {'name': 'MISE_CACHE_DIR', 'value': '/tmp/mise'}, {'name': 'MISE_AUTO_INSTALL', 'value': '0'}]
            self.apply(pod)
            result = self.wait(name, 60)
            (self.directory / (name + '.status.json')).write_text(json.dumps(result, indent=2))
            (self.directory / (name + '.log')).write_text(self.kube('logs', name))
            self.kube('delete', 'pod', name, '--wait=true')
            if result['status']['phase'] != 'Succeeded':
                raise RuntimeError('candidate_network_policy_probe_failed')
        print('candidate_network_policy_with_positive_control: completed', flush=True)

    def cleanup(self):
        if self.forward:
            self.forward.terminate()
            self.forward.wait(timeout=10)
        if self.registry_name:
            try:
                owner = self.docker('inspect', '--format', '{{ index .Config.Labels "ayni.build.qualification" }}', self.registry_name).strip()
            except subprocess.CalledProcessError:
                owner = None
            if owner == self.namespace:
                self.docker('rm', '-f', '-v', self.registry_name)
        for image in self.imported_images:
            self.docker('exec', self.node, 'ctr', '--namespace', 'k8s.io', 'images', 'remove', image)
        if self.created_namespace:
            self.kube('delete', 'namespace', self.namespace, '--wait=true', '--timeout=90s')
        (self.directory / 'private.pem').unlink(missing_ok=True)
        print('evidence=' + str(self.directory), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['worker-image', 'probe-image', 'cli', 'mise', 'docker-config']:
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--context', choices=['docker-desktop', 'kind-ayni-runtime-qualification'], default='docker-desktop')
    parser.add_argument('--kubeconfig', help='Explicit dedicated kubeconfig; required for the separate qualification cluster')
    parser.add_argument('--diagnose-quality', action='store_true', help='Observe bounded stderr only for these synthetic fixtures; no production pod changes')
    parser.add_argument('--reuse-public-evidence', help='Reprobe the retained public OCI/source only; this does not rerun build/certification')
    parser.add_argument('--probes-only', action='store_true', help='Reprobe public baseline without repeating worker freeze, wrong-signer or cancellation checks')
    parser.add_argument('--network-only', action='store_true', help='Test actual candidate network-policy enforcement using a retained public baseline')
    parser.add_argument('--engine-only', action='store_true', help='Test actual nested rootless execution and cancellation cleanup only')
    parser.add_argument('--registry-denial-only', action='store_true', help='Read-only real GHCR denial with deliberately invalid synthetic credentials; never publishes')
    parser.add_argument('--cli-sha256', default=RELEASE['linux_arm64_cli_sha256'])
    parser.add_argument('--executor-image', default=RELEASE['linux_arm64_executor_image'])
    parser.add_argument('--executor-cli-sha256', default='sha256:' + RELEASE['linux_arm64_cli_sha256'])
    args = parser.parse_args()
    if sha(Path(args.cli).read_bytes()) != 'sha256:' + args.cli_sha256.removeprefix('sha256:'):
        raise RuntimeError('CLI_checksum_mismatch')
    if sha(Path(args.mise).read_bytes()) != 'sha256:e3578b43b90a9031fd777d462c96a010f347f7e56f67f11b243cfc08fbff831d':
        raise RuntimeError('mise_checksum_mismatch')
    q = Qualification(args)
    print('namespace=' + q.namespace, flush=True)
    summary = {'kind': 'runtime_component_qualification', 'namespace': q.namespace,
        'context': q.context, 'cluster': q.cluster, 'node': q.node,
        'worker_image': args.worker_image, 'probe_image': args.probe_image, 'passed': False,
        'ayni_version': RELEASE['cli_version'], 'ayni_source_revision': RELEASE['source_revision'],
        'cli_sha256': args.cli_sha256, 'executor_image': args.executor_image,
        'executor_cli_sha256': args.executor_cli_sha256,
        'excluded': ['control_plane_admission', 'github_acquisition', 'ghcr_publication', 'supervisor_restart', 'production_stage_authority_fencing']}
    try:
        q.setup()
        summary['node_image'] = q.docker('inspect', q.node, '--format', '{{.Config.Image}}').strip()
        node_info = json.loads(q.kube('get', 'node', q.node, '-o', 'json'))['status']['nodeInfo']
        summary['node_runtime'] = {key: node_info[key] for key in ['architecture', 'operatingSystem',
            'osImage', 'kernelVersion', 'containerRuntimeVersion', 'kubeletVersion']}
        if q.context == 'kind-ayni-runtime-qualification':
            summary['cilium_image'] = q.kube('get', 'daemonset', 'cilium', '-n', 'kube-system',
                '-o', 'jsonpath={.spec.template.spec.containers[0].image}')
            for name in ['kind', 'cilium']:
                summary[name + '_config_sha256'] = sha((ROOT / ('dev/fixtures/runtime-preparation/cluster/' + name + '.yaml')).read_bytes())
        q.make_source()
        if args.registry_denial_only:
            q.registry_denial()
            summary['passed'] = True
            summary['covered'] = ['real_worker_ghcr_access_denied']
            print('REGISTRY_DENIAL_QUALIFICATION_PASSED', flush=True)
            return
        if args.engine_only:
            q.cancellation()
            summary['passed'] = True
            summary['covered'] = ['nested_rootless_execution', 'nested_cancellation_cleanup']
            return
        if args.network_only:
            if not args.reuse_public_evidence:
                raise RuntimeError('network_only_requires_public_baseline')
            image = q.import_candidate(Path(args.reuse_public_evidence) / 'pulled', 'network-candidate')
            q.network_policy(image)
            summary['passed'] = True
            summary['covered'] = ['candidate_network_policy']
            return
        if args.reuse_public_evidence:
            pulled = Path(args.reuse_public_evidence) / 'pulled'
            summary['reused_public_baseline'] = str(Path(args.reuse_public_evidence))
            if not args.probes_only:
                q.freeze_public_baseline(pulled)
        else:
            for stage, output, inputs in [('build', 'candidate', ['source']),
                    ('freeze_candidate', 'frozen', ['candidate']), ('certify', 'certification', ['source', 'frozen']),
                    ('finalize', 'final', ['frozen', 'certification'])]:
                if stage == 'certify':
                    q.wrong_signer()
                q.stage(stage, output, inputs)
            q.tool_readiness()
            q.cancellation()
            layout = q.copy_oci_layout()
            pulled = q.registry_roundtrip(layout)
        image = q.import_candidate(pulled, 'candidate')
        content = q.probe(image, 'verify-content')
        if content.get('state') != 'content_verified':
            raise RuntimeError('independent_content_verification_failed')
        managed = q.probe(image, 'verify-compatibility', compatibility=True)
        if managed.get('state') != 'managed_compatible':
            raise RuntimeError('independent_managed_verification_failed')
        q.negative_parent_contexts(image)
        q.negative_source(image)
        q.negative_content(pulled)
        q.network_policy(image)
        (q.directory / 'final-evidence.json').write_text(json.dumps(q.evidence, indent=2))
        summary['passed'] = True
        summary['covered'] = ['worker_build', 'independent_freeze', 'public_ayni_certification', 'oci_finalization', 'local_registry_roundtrip', 'independent_content', 'managed_compatibility', 'wrong_signer_rejected', 'nested_cancellation_cleanup', 'unsafe_parent_contexts_rejected', 'source_tamper_rejected', 'protected_content_tamper_rejected', 'candidate_network_policy', 'nonroot_worker_tool_readiness']
        if args.reuse_public_evidence:
            summary['covered'] = ['worker_freeze_public_baseline', 'wrong_signer_rejected', 'nonroot_worker_tool_readiness', 'nested_cancellation_cleanup', 'independent_content', 'managed_compatibility', 'unsafe_parent_contexts_rejected', 'source_tamper_rejected', 'protected_content_tamper_rejected', 'candidate_network_policy']
        if args.probes_only:
            summary['covered'] = ['independent_content', 'managed_compatibility', 'unsafe_parent_contexts_rejected', 'source_tamper_rejected', 'protected_content_tamper_rejected', 'candidate_network_policy']
        print('COMPONENT_QUALIFICATION_PASSED', flush=True)
    except RuntimeError as error:
        summary['safe_failure'] = str(error)
        raise
    finally:
        (q.directory / 'gate.json').write_text(json.dumps(summary, indent=2))
        q.cleanup()


if __name__ == '__main__':
    main()
