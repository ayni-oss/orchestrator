#!/usr/bin/env python3
"""Build pinned tooling and emit Helm values for the local development profile."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'dev/lib'))
from local_context import checked_context, NODES


def source_paths():
    paths = [ROOT / name for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
        '.dockerignore', 'scripts/ci/ayni-tool.json', 'dev/scripts/runtime-development.py',
        'dev/scripts/runtime-preparation-images.sh')]
    for directory in ('backend', 'proto', 'deploy/images/runtime-preparation',
                      'dev/fixtures/task-source', 'dev/fixtures/task-quality', 'dev/fixtures/runtime-preparation'):
        paths.extend(path for path in (ROOT / directory).rglob('*')
                     if path.is_file() and '__pycache__' not in path.parts)
    return paths


def fingerprint():
    digest = hashlib.sha256()
    for path in sorted(source_paths()):
        digest.update(str(path.relative_to(ROOT)).encode() + b'\0' + path.read_bytes() + b'\0')
    return digest.hexdigest()


def run(*args, capture=True, **kwargs):
    return subprocess.run(args, check=True, text=True,
                          stdout=subprocess.PIPE if capture else sys.stderr, **kwargs).stdout


def profile_values(worker, verifier, tool):
    for image in (worker, verifier, tool['linux_arm64_executor_image']):
        if not re.fullmatch(r'[a-z0-9./_-]+@sha256:[a-f0-9]{64}', image):
            raise ValueError('runtime tooling must use immutable manifest references')
    return {'ayni-orchestrator': {'runtimePreparation': {
        'enabled': True, 'localDevelopment': True, 'publicEgress': True,
        'profile': {'profile_id': 'rootless-podman-vfs-userns-local-development-v1',
                    'qualified': False, 'platform': 'linux/arm64',
                    'worker_image': worker, 'verifier_image': verifier,
                    'executor_image': tool['linux_arm64_executor_image'],
                    'trusted_cli_sha256': 'sha256:' + tool['linux_arm64_cli_sha256']}}}}


def cached_values(path, identity, images):
    try:
        saved = json.loads(path.read_text())
        profile = saved['values']['ayni-orchestrator']['runtimePreparation']['profile']
        if saved['identity'] == identity and all(profile[key] in images
                for key in ('worker_image', 'verifier_image')):
            return saved['values']
    except (OSError, ValueError, KeyError):
        pass
    return None


def build(context, tool):
    with tempfile.TemporaryDirectory(prefix='ayni-dev-runtime-') as temporary:
        directory = Path(temporary)
        archive = directory / 'ayni.tar.gz'
        name = f'ayni-ayni-v{tool["cli_version"]}-aarch64-unknown-linux-gnu'
        run('curl', '--fail', '--location', '--retry', '3', '--proto', '=https',
            '--tlsv1.2', f'https://github.com/ayni-oss/ayni/releases/download/ayni-v{tool["cli_version"]}/{name}.tar.gz',
            '--output', str(archive), capture=False)
        if hashlib.sha256(archive.read_bytes()).hexdigest() != tool['linux_arm64_archive_sha256']:
            raise ValueError('Ayni release archive checksum mismatch')
        cli = directory / 'ayni'
        with tarfile.open(archive) as bundle:
            member = bundle.getmember(name + '/ayni')
            if not member.isfile():
                raise ValueError('Ayni release executable is not a regular file')
            cli.write_bytes(bundle.extractfile(member).read())
        if hashlib.sha256(cli.read_bytes()).hexdigest() != tool['linux_arm64_cli_sha256']:
            raise ValueError('Ayni executable checksum mismatch')
        cli.chmod(0o755)
        print('Preparing local runtime worker and verifier images (first build can take several minutes)', file=sys.stderr)
        output = run('bash', str(ROOT / 'dev/scripts/runtime-preparation-images.sh'),
                     str(cli), tool['linux_arm64_cli_sha256'],
                     env=dict(os.environ, AYNI_RUNTIME_KUBE_CONTEXT=context))
        references = dict(line.split('=', 1) for line in output.splitlines()
                          if line.startswith(('worker_immutable=', 'probes_immutable=')))
        return profile_values(references['worker_immutable'], references['probes_immutable'], tool)


def main():
    context = checked_context()
    enabled = os.environ.get('AYNI_ORCHESTRATOR_RUNTIME', 'true')
    if enabled not in ('true', 'false'):
        raise ValueError('AYNI_ORCHESTRATOR_RUNTIME must be true or false')
    directory = ROOT / '.local/runtime-development' / context
    directory.mkdir(parents=True, exist_ok=True)
    with (directory / 'build.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if enabled == 'false':
            values = {'ayni-orchestrator': {'runtimePreparation': {'enabled': False}}}
        else:
            node, cluster = NODES[context]
            labels = run('docker', 'inspect', node, '--format',
                '{{index .Config.Labels "io.x-k8s.kind.cluster"}} {{index .Config.Labels "io.x-k8s.kind.role"}}').strip()
            if labels != cluster + ' control-plane':
                raise ValueError('runtime node identity mismatch')
            arch = run('kubectl', '--context', context, 'get', 'nodes',
                       '-o', 'jsonpath={.items[*].status.nodeInfo.architecture}').strip()
            if arch != 'arm64':
                raise ValueError('Local runtime tooling currently requires one native arm64 node; set AYNI_ORCHESTRATOR_RUNTIME=false to deploy without runtime builds')
            identity = fingerprint()
            images = set(run('docker', 'exec', node, 'ctr', '--namespace', 'k8s.io', 'images', 'list', '--quiet').splitlines())
            cache = directory / 'images.json'
            values = cached_values(cache, identity, images)
            if values is None:
                values = build(context, json.loads((ROOT / 'scripts/ci/ayni-tool.json').read_text()))
                if fingerprint() != identity:
                    raise ValueError('Runtime sources changed during build; retry deployment')
                pending = cache.with_suffix('.tmp')
                pending.write_text(json.dumps({'identity': identity, 'values': values}) + '\n')
                pending.replace(cache)
            print('Runtime builds enabled with the local-development profile (qualified=false)', file=sys.stderr)
        # Separate immutable value paths keep concurrent deployments from observing
        # another invocation's enable/disable selection after releasing the lock.
        encoded = json.dumps(values, sort_keys=True) + '\n'
        path = directory / (hashlib.sha256(encoded.encode()).hexdigest() + '.json')
        path.write_text(encoded)
        print(path)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        print(f'Local runtime preparation failed: {error}', file=sys.stderr)
        raise SystemExit(1)
