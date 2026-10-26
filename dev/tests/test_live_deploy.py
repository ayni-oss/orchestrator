"""Exercise migration quiescing/readiness through the real dev shell entrypoint."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class LiveDeployTests(unittest.TestCase):
    def run_deploy(self, fail_upgrade=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record = root / 'calls.jsonl'
            executable = root / 'tool'
            executable.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['LIVE_TEST_RECORD'], 'a') as record:
    record.write(json.dumps([name, *args]) + '\\n')
if name == 'helm':
    if 'version' in args: print('v4.1.1')
    if 'upgrade' in args and os.environ.get('LIVE_TEST_FAIL') == '1': sys.exit(1)
if name == 'kubectl':
    if 'current-context' in args: print('docker-desktop')
    if 'deployment/control-plane' in args and 'get' in args:
        print('1' if any('readyReplicas' in arg for arg in args) else 'deployment.apps/control-plane')
    if 'deployment/task-publisher' in args and 'get' in args: print('deployment.apps/task-publisher')
''')
            executable.chmod(0o755)
            for name in ('kubectl', 'helm', 'openssl'):
                (root / name).symlink_to(executable)
            env = dict(os.environ, PATH=f'{root}:{os.environ["PATH"]}',
                       LIVE_TEST_RECORD=str(record), LIVE_TEST_FAIL='1' if fail_upgrade else '0',
                       AYNI_ORCHESTRATOR_NAMESPACE='ayni-orchestrator-test',
                       AYNI_ORCHESTRATOR_LIVE_MIGRATE='true')
            result = subprocess.run(['bash', 'dev/scripts/live-deploy.sh',
                                     'local/ayni-live-control-plane:test',
                                     'local/ayni-live-scripted-executor:test'],
                                    cwd=ROOT, env=env, capture_output=True, text=True)
            return result, [json.loads(line) for line in record.read_text().splitlines()]

    def test_migrate_quiesces_before_upgrade_and_restores_serving_singleton(self):
        result, calls = self.run_deploy()
        self.assertEqual(result.returncode, 0, result.stderr)
        down = next(i for i, call in enumerate(calls) if '--replicas=0' in call)
        upgrade = next(i for i, call in enumerate(calls) if 'upgrade' in call)
        up = next(i for i, call in enumerate(calls) if '--replicas=1' in call)
        self.assertLess(down, upgrade)
        self.assertLess(upgrade, up)
        self.assertTrue(any('deployment/task-publisher' in call and 'status' in call for call in calls))
        self.assertTrue(any(any('readyReplicas' in arg for arg in call) for call in calls))

    def test_failed_upgrade_does_not_claim_recovery_or_rollback_schema(self):
        result, calls = self.run_deploy(fail_upgrade=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any('--replicas=1' in call for call in calls))
        self.assertFalse(any('rollback' in call or 'delete' in call for call in calls))


if __name__ == '__main__':
    unittest.main()
