"""Local runtime setup must never substitute stale tooling or claim qualification."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('runtime_development', ROOT / 'dev/scripts/runtime-development.py')
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class RuntimeDevelopmentTests(unittest.TestCase):
    def setUp(self):
        self.tool = json.loads((ROOT / 'scripts/ci/ayni-tool.json').read_text())
        self.worker = 'docker.io/local/worker@sha256:' + 'a' * 64
        self.verifier = 'docker.io/local/verifier@sha256:' + 'b' * 64

    def test_local_profile_pins_tooling_without_claiming_qualification(self):
        values = runtime.profile_values(self.worker, self.verifier, self.tool)
        preparation = values['ayni-orchestrator']['runtimePreparation']
        self.assertTrue(preparation['enabled'])
        self.assertTrue(preparation['localDevelopment'])
        self.assertFalse(preparation['profile']['qualified'])
        self.assertEqual(preparation['profile']['executor_image'], self.tool['linux_arm64_executor_image'])
        self.assertEqual(preparation['profile']['trusted_cli_sha256'], 'sha256:' + self.tool['linux_arm64_cli_sha256'])
        with self.assertRaises(ValueError):
            runtime.profile_values('local/worker:latest', self.verifier, self.tool)

    def test_cache_requires_current_sources_and_both_node_manifests(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory) / 'images.json'
            values = runtime.profile_values(self.worker, self.verifier, self.tool)
            cache.write_text(json.dumps({'identity': 'current', 'values': values}))
            images = {self.worker, self.verifier}
            self.assertEqual(runtime.cached_values(cache, 'current', images), values)
            self.assertIsNone(runtime.cached_values(cache, 'edited', images))
            self.assertIsNone(runtime.cached_values(cache, 'current', {self.worker}))
            cache.write_text('interrupted')
            self.assertIsNone(runtime.cached_values(cache, 'current', images))

    def test_archive_checksum_failure_never_packages_tooling(self):
        def download(*args, **kwargs):
            self.assertEqual(args[0], 'curl')
            Path(args[-1]).write_bytes(b'wrong release')
        with patch.object(runtime, 'run', side_effect=download) as run:
            with self.assertRaisesRegex(ValueError, 'archive checksum mismatch'):
                runtime.build('docker-desktop', self.tool)
        self.assertEqual(run.call_count, 1)

    def test_source_hash_changes_with_edit_and_deletion(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'worker.rs'
            source.write_text('before')
            with patch.object(runtime, 'ROOT', root), patch.object(runtime, 'source_paths', return_value=[source]):
                previous = runtime.fingerprint()
                source.write_text('after')
                self.assertNotEqual(previous, runtime.fingerprint())
            with patch.object(runtime, 'ROOT', root), patch.object(runtime, 'source_paths', return_value=[]):
                self.assertNotEqual(previous, runtime.fingerprint())


if __name__ == '__main__':
    unittest.main()
