"""Fresh internal runtime authority provisioning without existing credential access."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DevelopmentSecretTests(unittest.TestCase):
    def test_runtime_authority_is_fresh_immutable_and_reused_without_reading_material(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tool = root / 'kubectl'
            tool.write_text('''#!/usr/bin/env python3
import json, os, pathlib, re, sys
args=sys.argv[1:]
root=pathlib.Path(os.environ['SECRET_TEST_ROOT'])
with (root/'calls').open('a') as output: output.write(json.dumps(args)+'\\n')
if 'get-contexts' in args: print('kind-ayni-runtime-qualification')
if 'get' in args and 'workflow-runtime-authority' in args and not (root/'created').exists(): sys.exit(1)
if 'create' in args and 'workflow-runtime-authority' in args:
    path=next(arg.split('=',2)[2] for arg in args if arg.startswith('--from-file=token='))
    value=pathlib.Path(path).read_text().strip()
    assert re.fullmatch('[a-f0-9]{64}',value)
    (root/'created').write_text('synthetic fixture exists')
''')
            tool.chmod(0o755)
            env = dict(os.environ, PATH=str(root) + ':' + os.environ['PATH'], SECRET_TEST_ROOT=str(root),
                AYNI_ORCHESTRATOR_CONTEXT='kind-ayni-runtime-qualification',
                KUBECONFIG='/tmp/ayni-runtime-cluster/kubeconfig',
                AYNI_ORCHESTRATOR_NAMESPACE='ayni-orchestrator-runtime-qualification')
            for _ in range(2):
                result = subprocess.run(['bash', 'dev/scripts/configure-development-secrets.sh'], cwd=ROOT,
                    env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in (root / 'calls').read_text().splitlines()]
            runtime = [call for call in calls if 'workflow-runtime-authority' in call]
            self.assertEqual(sum('create' in call for call in runtime), 1)
            self.assertEqual(sum('patch' in call for call in runtime), 1)
            self.assertTrue(any('{"immutable":true}' in call for call in runtime))
            self.assertTrue(all('--context' in call and 'kind-ayni-runtime-qualification' in call for call in runtime))
            self.assertFalse(any('-o' in call or '--output' in call for call in runtime if 'get' in call))
            self.assertFalse(any('docker-desktop' in call for call in calls))


if __name__ == '__main__':
    unittest.main()
