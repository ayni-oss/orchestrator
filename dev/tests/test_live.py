"""Authority/reload regressions; no Docker or credentials required."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('live', Path(__file__).parents[1] / 'scripts/live.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


class LiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'status.json'
        self.old = {'control-plane': 'a' * 64, 'scripted-executor': 'b' * 64, 'schema': 'c' * 64}
        self.state = {'active': self.old, 'images': {'control-plane': 'old-control', 'scripted-executor': 'old-executor'}}

    def test_schema_edit_never_builds_or_deploys_automatically(self):
        changed = dict(self.old, schema='d' * 64)
        with patch.object(live, 'inputs', return_value=changed), patch.object(live, 'run') as run:
            self.assertFalse(live.reconcile(self.path, self.state, {}))
        run.assert_not_called()
        self.assertEqual(self.state['active'], self.old)
        self.assertEqual(json.loads(self.path.read_text())['phase'], 'migration_required')

    def test_failed_compilation_keeps_active_identity_and_never_deploys(self):
        changed = dict(self.old, **{'control-plane': 'd' * 64})
        with patch.object(live, 'inputs', return_value=changed), patch.object(live, 'run', side_effect=subprocess.CalledProcessError(1, 'build')) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                live.reconcile(self.path, self.state, {})
        self.assertEqual(run.call_count, 1)
        self.assertEqual(self.state['active'], self.old)

    def test_source_change_during_build_prevents_mixed_revision_deployment(self):
        changed = dict(self.old, **{'control-plane': 'd' * 64})
        newer = dict(changed, **{'control-plane': 'e' * 64})
        with patch.object(live, 'inputs', side_effect=[changed, newer]), patch.object(live, 'run', return_value='new-image') as run:
            self.assertFalse(live.reconcile(self.path, self.state, {}))
        self.assertFalse(any('live-deploy.sh' in str(call) for call in run.call_args_list))
        self.assertEqual(self.state['active'], self.old)

    def test_explicit_migrate_uses_same_deploy_and_updates_only_after_success(self):
        changed = dict(self.old, schema='d' * 64)
        with patch.object(live, 'inputs', return_value=changed), patch.object(live, 'run', return_value='revision') as run:
            self.assertTrue(live.reconcile(self.path, self.state, {}, allow_migration=True))
        self.assertEqual(run.call_args_list[0].args[1]['AYNI_ORCHESTRATOR_LIVE_MIGRATE'], 'true')
        self.assertEqual(self.state['active'], changed)

    def test_namespace_restriction_and_single_owner(self):
        with patch.dict(live.os.environ, {'AYNI_ORCHESTRATOR_NAMESPACE': 'production'}):
            with self.assertRaises(ValueError):
                live.namespace()
        with live.exclusive(self.path.parent):
            with self.assertRaises(ValueError):
                with live.exclusive(self.path.parent):
                    self.fail('second owner acquired the lock')

    def test_file_content_and_deletion_change_identity(self):
        source = self.path.parent / 'source.rs'
        with patch.object(live, 'ROOT', self.path.parent):
            source.write_text('before')
            before = live.fingerprint([source])
            source.write_text('after')
            self.assertNotEqual(before, live.fingerprint([source]))
            source.unlink()
            self.assertNotEqual(before, live.fingerprint([source]))


if __name__ == '__main__':
    unittest.main()
