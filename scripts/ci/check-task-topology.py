#!/usr/bin/env python3
"""Inspect deployed task/publisher isolation using Pod metadata, never Secret data."""
import argparse
import json
import subprocess


def pods(namespace):
    return json.loads(subprocess.check_output([
        'kubectl', '--context', 'docker-desktop', '--namespace', namespace,
        'get', 'pods', '-o', 'json',
    ]))['items']


def containers(pod):
    return pod['spec'].get('initContainers', []) + pod['spec']['containers']


def mounted_secrets(pod, container):
    volumes = {v['name']: v for v in pod['spec'].get('volumes', [])}
    return {volumes[m['name']]['secret']['secretName'] for m in container.get('volumeMounts', [])
            if 'secret' in volumes[m['name']]}


def inspect(parent, workers):
    publishers = [p for p in parent if p['metadata'].get('labels', {}).get('app.kubernetes.io/name') == 'ayni-task-publisher']
    tasks = [p for p in workers if p['metadata'].get('labels', {}).get('app.kubernetes.io/name') == 'ayni-task-worker']
    assert len(publishers) == 1 and len(tasks) >= 2, 'publisher and retained quality workers required'
    publisher = publishers[0]
    assert publisher['spec']['automountServiceAccountToken'] is False
    volumes = {v['name']: v for v in publisher['spec']['volumes']}
    assert set(volumes) == {'scratch', 'authority', 'github'}
    github = volumes['github']['secret']['secretName']
    assert github != volumes['authority']['secret']['secretName']
    assert 'emptyDir' in volumes['scratch']
    claims = set()
    runtime_workers = 0
    for pod in tasks:
        spec = pod['spec']
        name = pod['metadata']['name']
        assert spec['automountServiceAccountToken'] is False
        assert spec['securityContext']['runAsNonRoot'] is True
        assert pod['metadata']['annotations']['build.ayni/command-digest'].startswith('sha256:')
        volumes = {v['name']: v for v in spec['volumes']}
        claim = volumes['work']['persistentVolumeClaim']['claimName']
        assert claim == volumes['capability']['secret']['secretName'] == name
        assert claim not in claims
        claims.add(claim)
        assert all('hostPath' not in v and 'projected' not in v for v in volumes.values())
        stages = {c['name']: c for c in containers(pod)}
        assert {'initialize', 'coding', 'freeze', 'prepare', 'seal', 'quality', 'collector'} == set(stages)
        for stage, container in stages.items():
            assert github not in mounted_secrets(pod, container)
            assert not any('secretKeyRef' in e.get('valueFrom', {}) for e in container.get('env', []))
            assert container['securityContext']['allowPrivilegeEscalation'] is False
            assert container['securityContext']['readOnlyRootFilesystem'] is True
            if stage != 'coding':
                assert 'provider' not in {m['name'] for m in container['volumeMounts']}
        for stage in ['prepare', 'quality']:
            container = stages[stage]
            assert mounted_secrets(pod, container) == set()
            assert not any(e['name'].startswith(('ORCHESTRATOR_', 'OPENAI_')) for e in container['env'])
            assert set(container['securityContext']['capabilities']['add']) == {'SETUID', 'SETGID'}
        mounts = {m['mountPath']: m for m in stages['quality']['volumeMounts']}
        assert mounts['/workspace']['readOnly'] is True
        assert mounts['/workspace']['subPath'] == 'quality/source'
        assert mounts['/workspace/.ayni']['subPath'] == 'quality/artifacts'
        prepare_mounts = {m['mountPath']: m for m in stages['prepare']['volumeMounts']}
        assert prepare_mounts['/workspace/.git']['readOnly'] is True
        if '@sha256:' in stages['quality']['image']:
            runtime_workers += 1
    assert runtime_workers >= 2, 'passing and failing managed-runtime workers required'
    for pod in parent + workers:
        for container in containers(pod):
            if pod is not publisher:
                assert github not in mounted_secrets(pod, container)
    return {'result': 'passed', 'task_pods': len(tasks), 'separate_claims': len(claims),
            'managed_runtime_pods': runtime_workers, 'quality_secrets': 'none',
            'github_credential_mount': 'publisher only', 'network_policy_enforcement': 'not established'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--namespace', required=True)
    arguments = parser.parse_args()
    print(json.dumps(inspect(pods(arguments.namespace), pods(arguments.namespace + '-sessions')), sort_keys=True))
