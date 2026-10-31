"""Context and image-import fencing without a cluster or user credentials."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('local_context', ROOT / 'dev/lib/local_context.py')
context = importlib.util.module_from_spec(spec)
spec.loader.exec_module(context)


class ContextTests(unittest.TestCase):
    def qualified(self):
        return dict(AYNI_ORCHESTRATOR_CONTEXT=context.QUALIFICATION_CONTEXT,
            KUBECONFIG=context.QUALIFICATION_KUBECONFIG,
            AYNI_ORCHESTRATOR_NAMESPACE=context.QUALIFICATION_NAMESPACE)

    def test_only_exact_dedicated_configuration_selects_the_new_cluster(self):
        self.assertEqual(context.selected_context({}), 'docker-desktop')
        env = self.qualified()
        self.assertEqual(context.selected_context(env), context.QUALIFICATION_CONTEXT)
        for key, value in [('AYNI_ORCHESTRATOR_CONTEXT', 'production'),
            ('KUBECONFIG', '/tmp/other'), ('KUBECONFIG', context.QUALIFICATION_KUBECONFIG + ':/tmp/other'),
            ('AYNI_ORCHESTRATOR_NAMESPACE', 'ayni-orchestrator-live')]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                context.selected_context(env | {key: value})
        for key in env:
            if key != 'AYNI_ORCHESTRATOR_CONTEXT':
                with self.subTest(missing=key), self.assertRaises(ValueError):
                    context.selected_context({k: v for k, v in env.items() if k != key})

    def test_context_check_never_switches_global_state_or_falls_back(self):
        env = self.qualified()
        with patch.object(context.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, context.QUALIFICATION_CONTEXT + '\n')) as run:
            self.assertEqual(context.checked_context(env), context.QUALIFICATION_CONTEXT)
        self.assertEqual(run.call_args.args[0], ['kubectl', 'config', 'get-contexts', context.QUALIFICATION_CONTEXT, '-o', 'name'])
        for output in ('docker-desktop', 'production', ''):
            with patch.object(context.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, output)), self.assertRaises(ValueError):
                context.checked_context(env)
        with patch.object(context.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'production')), self.assertRaises(ValueError):
            context.checked_context({})

    def test_import_checks_selected_node_labels_before_any_image_transfer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tool = root / 'tool'
            tool.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
name=pathlib.Path(sys.argv[0]).name
args=sys.argv[1:]
with open(os.environ['CONTEXT_TEST_RECORD'],'a') as output: output.write(json.dumps([name,*args])+'\\n')
if name=='kubectl': print(os.environ['AYNI_ORCHESTRATOR_CONTEXT'])
elif args[:1]==['inspect']:
    print(os.environ['CONTEXT_TEST_LABELS'] if 'Labels' in args[-1] else 'selected-node-id')
elif args[:2]==['image','inspect']: print('linux/arm64')
elif 'list' in args: print('docker.io/local/fixture:test')
elif args[:1]==['exec'] and 'import' in args: sys.stdin.buffer.read()
''')
            tool.chmod(0o755)
            for name in ('kubectl', 'docker'):
                (root / name).symlink_to(tool)
            record = root / 'record'
            env = dict(os.environ, **self.qualified(), PATH=str(root) + ':' + os.environ['PATH'],
                CONTEXT_TEST_RECORD=str(record), CONTEXT_TEST_LABELS='ayni-runtime-qualification control-plane')
            command = ['bash', '-c', 'set -euo pipefail; source dev/lib/common.sh; source dev/lib/images.sh; dev_load_image local/fixture:test']
            result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in record.read_text().splitlines()]
            self.assertTrue(any(call[:3] == ['docker', 'inspect', 'ayni-runtime-qualification-control-plane'] for call in calls))
            self.assertFalse(any('desktop-control-plane' in call for call in calls))
            record.write_text('')
            result = subprocess.run(command, cwd=ROOT, env=env | {'CONTEXT_TEST_LABELS': 'desktop control-plane'}, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            calls = [json.loads(line) for line in record.read_text().splitlines()]
            self.assertFalse(any('save' in call or 'exec' in call for call in calls))


if __name__ == '__main__':
    unittest.main()
