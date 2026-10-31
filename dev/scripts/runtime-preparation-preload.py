#!/usr/bin/env python3
"""Preload a pinned public arm64 image into only the separate runtime cluster.

Docker verifies registry TLS. The original index bytes and digest are retained;
no certificate verification bypass or index-to-child digest substitution is used.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

NODE = 'ayni-runtime-qualification-control-plane'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image', help='Public fully qualified repository@sha256 reference')
    args = parser.parse_args()
    match = re.fullmatch(r'((?:docker\.io|quay\.io|ghcr\.io)/[a-z0-9._/-]+)@sha256:([a-f0-9]{64})', args.image)
    if not match:
        raise SystemExit('a fully qualified pinned public image is required')
    repository, expected = match.groups()
    if shutil.disk_usage(Path.cwd()).free < 9 * 1024 ** 3:
        raise SystemExit('at least nine GiB host headroom is required')
    with tempfile.TemporaryDirectory(prefix='ayni-runtime-preload-') as state:
        config = Path(state)
        (config / 'config.json').write_text(json.dumps({'auths': {},
            'cliPluginsExtraDirs': ['/Applications/Docker.app/Contents/Resources/cli-plugins']}))
        env = os.environ | {'DOCKER_CONFIG': state,
            'DOCKER_HOST': 'unix://' + str(Path.home() / '.docker/run/docker.sock')}
        env.pop('DOCKER_CONTEXT', None)

        def docker(*command, **kwargs):
            return subprocess.run(['docker', *command], env=env, check=True, **kwargs)

        labels = docker('inspect', NODE, '--format', '{{index .Config.Labels "io.x-k8s.kind.cluster"}} {{index .Config.Labels "io.x-k8s.kind.role"}}', capture_output=True, text=True).stdout.strip()
        if labels != 'ayni-runtime-qualification control-plane':
            raise SystemExit('dedicated node identity mismatch')
        raw = docker('buildx', 'imagetools', 'inspect', '--raw', args.image, capture_output=True).stdout
        if hashlib.sha256(raw).hexdigest() != expected:
            raise SystemExit('registry manifest identity mismatch')
        descriptor = json.loads(raw)
        selected = 'sha256:' + expected
        if 'manifests' in descriptor:
            choices = [item for item in descriptor['manifests'] if item.get('platform', {}).get('os') == 'linux'
                and item.get('platform', {}).get('architecture') == 'arm64']
            if len(choices) != 1:
                raise SystemExit('expected exactly one native arm64 manifest')
            selected = choices[0]['digest']
        if not re.fullmatch('sha256:[a-f0-9]{64}', selected):
            raise SystemExit('invalid platform digest')
        docker('pull', '--platform', 'linux/arm64', repository + '@' + selected)
        platform = docker('image', 'inspect', repository + '@' + selected,
            '--format', '{{.Os}}/{{.Architecture}}', capture_output=True, text=True).stdout.strip()
        if platform != 'linux/arm64':
            raise SystemExit('native image platform mismatch')
        with tempfile.TemporaryFile() as archive:
            docker('save', '--platform', 'linux/arm64', repository + '@' + selected, stdout=archive)
            archive.seek(0)
            docker('exec', '-i', NODE, 'ctr', '--namespace', 'k8s.io', 'images', 'import',
                '--platform', 'linux/arm64', '--digests', '--base-name', repository, '-', stdin=archive)
        # The native image is cached. Import the original verified root manifest
        # with its native descendants already present in containerd's content store.
        index = {'schemaVersion': 2, 'manifests': [{'mediaType': descriptor['mediaType'],
            'digest': 'sha256:' + expected, 'size': len(raw),
            'annotations': {'org.opencontainers.image.ref.name': args.image}}]}
        with tempfile.TemporaryFile() as archive:
            with tarfile.open(fileobj=archive, mode='w') as bundle:
                for name, data in [('oci-layout', b'{"imageLayoutVersion":"1.0.0"}'),
                    ('index.json', json.dumps(index, separators=(',', ':')).encode()),
                    ('blobs/sha256/' + expected, raw)]:
                    entry = tarfile.TarInfo(name)
                    entry.size, entry.mode = len(data), 0o644
                    bundle.addfile(entry, io.BytesIO(data))
            archive.seek(0)
            docker('exec', '-i', NODE, 'ctr', '--namespace', 'k8s.io', 'images', 'import',
                '--platform', 'linux/arm64', '-', stdin=archive)
        retained = docker('exec', NODE, 'ctr', '--namespace', 'k8s.io', 'images', 'list',
            capture_output=True, text=True).stdout.splitlines()
        if not any(row.split()[0] == args.image and row.split()[2] == 'sha256:' + expected for row in retained[1:]):
            raise SystemExit('pinned root manifest alias was not retained')
        print('preloaded=' + args.image + ' platform=linux/arm64 native=' + selected)


if __name__ == '__main__':
    main()
