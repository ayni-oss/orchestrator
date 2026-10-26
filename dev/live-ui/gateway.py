#!/usr/bin/env python3
"""Reconnect the trusted local gateway after a Pod rollout; no workflow authority."""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time

STOP = False


def stop_requested(_signal, _frame):
    global STOP
    STOP = True


def stop_process(process):
    if process is None:
        return
    # Include descendants: kubectl and gateway get separate process groups.
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
    except ProcessLookupError:
        process.wait()


def ready_pod(document):
    pods = [pod for pod in document['items']
            if not pod['metadata'].get('deletionTimestamp')
            and any(condition.get('type') == 'Ready' and condition.get('status') == 'True'
                    for condition in pod.get('status', {}).get('conditions', []))]
    if not pods:
        raise RuntimeError('No ready control-plane Pod; waiting for rollout')
    return max(pods, key=lambda pod: pod['metadata']['creationTimestamp'])['metadata']['name']


def run(namespace, binary, port=3000):
    kubectl = ['kubectl', '--context', 'docker-desktop', '--namespace', namespace]
    while not STOP:
        forward = gateway = None
        try:
            result = subprocess.run(
                [*kubectl, 'get', 'pods', '-l',
                 'app.kubernetes.io/name=ayni-orchestrator-control-plane', '-o', 'json'],
                capture_output=True, check=True, timeout=15, text=True)
            pod = ready_pod(json.loads(result.stdout))
            identity = binary.stat().st_mtime_ns
            with tempfile.TemporaryFile(mode='w+') as output:
                forward = subprocess.Popen(
                    [*kubectl, 'port-forward', '--address', '127.0.0.1', f'pod/{pod}', ':50051'],
                    stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
                endpoint = None
                deadline = time.monotonic() + 15
                while not STOP and time.monotonic() < deadline and forward.poll() is None:
                    output.seek(0)
                    match = re.search(r'Forwarding from 127\.0\.0\.1:([0-9]+) -> 50051', output.read())
                    if match:
                        endpoint = f'127.0.0.1:{match[1]}'
                        break
                    time.sleep(.1)
                if STOP:
                    break
                if endpoint is None:
                    raise RuntimeError('Port-forward unavailable; waiting for rollout')
                gateway = subprocess.Popen(
                    [str(binary), f'127.0.0.1:{port}', endpoint], start_new_session=True)
                print(f'Live gateway connected to {pod}', flush=True)
                while not STOP and forward.poll() is None and gateway.poll() is None:
                    if binary.stat().st_mtime_ns != identity:
                        print('Gateway binary changed; restarting local adapter', flush=True)
                        break
                    time.sleep(.2)
        except (OSError, ValueError, KeyError, subprocess.SubprocessError, RuntimeError) as error:
            # Do not print subprocess output, which may contain local configuration.
            print(f'Live gateway waiting ({type(error).__name__}); retrying', flush=True)
        finally:
            stop_process(gateway)
            stop_process(forward)
        for _ in range(10):
            if STOP:
                break
            time.sleep(.1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('namespace')
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    if not re.fullmatch(r'[a-z0-9]([-a-z0-9]*[a-z0-9])?', args.namespace) or len(args.namespace) > 54:
        parser.error('Invalid local namespace')
    signal.signal(signal.SIGINT, stop_requested)
    signal.signal(signal.SIGTERM, stop_requested)
    port = int(os.environ.get('AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT', '3000'))
    if not 1024 <= port <= 65535:
        parser.error('Live gateway port must be from 1024 to 65535')
    run(args.namespace, args.binary.resolve(), port)


if __name__ == '__main__':
    main()
