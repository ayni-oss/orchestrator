#!/usr/bin/env python3
"""Validate the executed cumulative proof bundle and write a bounded M4 summary."""
import json
import hashlib
import subprocess
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "lib"))
from publication_accounting import verify


def summarize(root, source):
    files = subprocess.check_output(['git', '-C', str(source), 'ls-files', '--cached', '--others', '--exclude-standard', '-z']).decode().split('\0')
    manifest = {name: hashlib.sha256((source / name).read_bytes()).hexdigest()
                for name in files if name and (source / name).is_file()}
    with (root / 'source-manifest-after.json').open('x') as output:
        json.dump(manifest, output, sort_keys=True, indent=2)
        output.write('\n')
    if manifest != json.loads((root / 'quality/source-manifest.json').read_text()):
        raise ValueError('repository source changed during acceptance')
    revision = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD']).decode().strip()
    if revision != (root / 'quality/source-revision.txt').read_text().strip():
        raise ValueError('source revision changed during acceptance')
    quality = json.loads((root / 'quality/outcome.json').read_text())
    if quality['result'] != 'passed':
        raise ValueError('managed quality acceptance did not pass')
    remote = json.loads((root / 'cleanup-remote.json').read_text())
    baseline = json.loads((root / 'fixture-baseline.json').read_text())
    accounting = verify(baseline, remote, 4, 3)
    cases = {
        'retained-fixture-baseline': ['fixture-baseline.json', 'cleanup-remote.json'],
        'frozen-harness-source': ['quality/source-manifest.json', 'source-manifest-after.json'],
        'approval-and-preparation-restart': ['quality/plans-broker-unavailable.json', 'cleanup-plans.txt', 'cleanup-preparation.txt'],
        'coding-pod-loss': ['quality/task-pod-loss-result.txt'],
        'late-coding-report-fenced': ['quality/task-fence-worker-exit.txt', 'quality/task-fence-result.txt'],
        'managed-quality-pass-and-failure': ['cleanup-task-quality-run.txt', 'cleanup-task-quality-failed-run.txt'],
        'browser-publication-retry-and-cancel': ['console/outcome.json'],
        'publisher-loss-before-create': ['before-create-prepared-identity.json', 'before-create-recovered-identity.json'],
        'publisher-loss-after-remote-create': ['after-create-prepared-identity.json', 'after-create-recovered-identity.json', 'after-create-cancel-rejected.txt'],
        'uncertain-without-visible-pr': ['uncertain-cancel-rejected.txt', 'uncertain-recovered.txt'],
        'existing-branch-conflict': ['collision-final.json'],
        'deployed-credential-and-source-isolation': ['task-topology.json'],
        'cleanup-and-redeployment': ['cleanup.txt', 'cleanup-redeployed.txt', 'cleanup-original-draft.txt', 'cleanup-cancelled.txt'],
    }
    owners = ['control-plane', 'task-publisher', 'publication-fixture', 'task-supervisor', 'conversation-supervisor', 'jetstream', 'postgres']
    for owner in owners:
        cases[f'restart-{owner}'] = [f'replay-{owner}.txt', f'cancel-replay-{owner}.txt', f'before-create-replay-{owner}.txt', f'after-create-replay-{owner}.txt', f'uncertain-replay-{owner}.txt']
    for files in cases.values():
        for name in files:
            if not (root / name).read_text().strip():
                raise ValueError(f'missing proof: {name}')
    if (root / 'quality/task-fence-worker-exit.txt').read_text().strip() != '1':
        raise ValueError('stale worker did not reject its late result')
    if json.loads((root / 'console/outcome.json').read_text())['result'] != 'passed':
        raise ValueError('browser authority gate did not pass')
    if json.loads((root / 'collision-final.json').read_text())['safe_reason'] != 'publication_ref_conflict':
        raise ValueError('remote branch conflict was not retained')
    if json.loads((root / 'task-topology.json').read_text())['result'] != 'passed':
        raise ValueError('deployed task isolation did not pass')
    return {
        'schema': 'm4-acceptance.v1', 'result': 'passed',
        'cases': {name: {'result': 'passed', 'evidence': files} for name, files in cases.items()},
        'source': {'revision': (root / 'quality/source-revision.txt').read_text().strip(),
                   'manifest': 'quality/source-manifest.json', 'diff': 'quality/source-diff.patch'},
        'runtime': {'workloads': json.loads((root / 'quality/image-identities.json').read_text()),
                    'managed_quality': (root / 'quality/quality-runtime-image.txt').read_text().strip(),
                    'publication_fixture': (root / 'fixture-image.txt').read_text().strip()},
        'external_actions': accounting,
        'integration': {'coding': 'scripted fixture', 'quality': 'real managed Ayni',
                        'git': 'real Git transport', 'github': 'retained protocol fixture',
                        'live_paid_provider': False, 'live_github_authentication': False},
        'retention': 'parent and worker PVCs and application Secret identities preserved; exact receipts replayed after reinstall',
    }


if __name__ == '__main__':
    root = Path(sys.argv[1])
    result = summarize(root, Path(sys.argv[2]))
    with (root / 'acceptance.json').open('x') as output:
        json.dump(result, output, indent=2, sort_keys=True)
        output.write('\n')
