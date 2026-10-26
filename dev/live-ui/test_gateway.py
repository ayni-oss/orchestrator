"""Exercise rollout reconnect and subprocess cleanup without a cluster or sockets."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

MODULE = Path(__file__).with_name('gateway.py')
spec = importlib.util.spec_from_file_location('gateway', MODULE)
gateway = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gateway)


class GatewayTests(unittest.TestCase):
    def test_ready_selection_excludes_terminating_and_unready_pods(self):
        def pod(name, timestamp, ready='True', deleting=False):
            metadata = {'name': name, 'creationTimestamp': timestamp}
            if deleting:
                metadata['deletionTimestamp'] = timestamp
            return {'metadata': metadata, 'status': {'conditions': [{'type': 'Ready', 'status': ready}]}}
        document = {'items': [pod('old', '1'), pod('new', '2'),
                              pod('terminating', '4', deleting=True), pod('unready', '5', ready='False')]}
        self.assertEqual(gateway.ready_pod(document), 'new')
        with self.assertRaises(RuntimeError):
            gateway.ready_pod({'items': []})

    def test_rollout_reconnect_stops_gateway_and_shutdown_cleans_children(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            kubectl = root / 'kubectl'
            binary = root / 'gateway'
            record = root / 'record'
            pid_record = root / 'pids'
            kubectl.write_text(f'''#!{sys.executable}
import json, os, pathlib, sys, time
root = pathlib.Path({directory!r})
if 'get' in sys.argv:
    print(json.dumps({{'items': [{{'metadata': {{'name': 'control', 'creationTimestamp': '1'}}, 'status': {{'conditions': [{{'type': 'Ready', 'status': 'True'}}]}}}}]}}))
else:
    with (root / 'record').open('a') as out: out.write('forward\\n')
    with (root / 'pids').open('a') as out: out.write(str(os.getpid()) + '\\n')
    count = (root / 'record').read_text().count('forward')
    print('Forwarding from 127.0.0.1:45678 -> 50051', flush=True)
    if count == 1:
        deadline = time.monotonic() + 10
        while 'gateway' not in (root / 'record').read_text() and time.monotonic() < deadline:
            time.sleep(.05)
        time.sleep(.5)
    else:
        time.sleep(30)
''')
            binary.write_text(f'''#!{sys.executable}
import os, pathlib, time
root = pathlib.Path({directory!r})
with (root / 'record').open('a') as out: out.write('gateway\\n')
with (root / 'pids').open('a') as out: out.write(str(os.getpid()) + '\\n')
time.sleep(30)
''')
            kubectl.chmod(0o755)
            binary.chmod(0o755)
            environment = dict(os.environ, PATH=f'{directory}:{os.environ["PATH"]}')
            process = subprocess.Popen([sys.executable, str(MODULE), 'ayni-orchestrator-test', str(binary)],
                                       env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline:
                    if record.exists() and record.read_text().count('gateway') >= 2:
                        break
                    if process.poll() is not None:
                        self.fail(f'Gateway wrapper exited: {process.communicate()}')
                    time.sleep(.05)
                self.assertGreaterEqual(record.read_text().count('gateway'), 2)
                process.send_signal(signal.SIGTERM)
                stdout, stderr = process.communicate(timeout=8)
                self.assertEqual(process.returncode, 0, (stdout, stderr))
                for pid in pid_record.read_text().splitlines():
                    with self.assertRaises(ProcessLookupError):
                        os.kill(int(pid), 0)
            finally:
                if process.poll() is None:
                    process.send_signal(signal.SIGTERM)
                    process.communicate(timeout=8)


if __name__ == '__main__':
    unittest.main()
