#!/usr/bin/env python3
"""Dev-only build/reconcile loop. No database credentials or schema shortcuts."""
import argparse
import contextlib
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import socket
import threading
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
COMPONENTS = ('control-plane', 'conversation-supervisor')


def namespace():
    value = os.environ.get('AYNI_ORCHESTRATOR_NAMESPACE', 'ayni-orchestrator-live')
    if not re.fullmatch(r'ayni-orchestrator-[a-z0-9](?:[-a-z0-9]*[a-z0-9])?', value) or len(value) > 54:
        raise ValueError('live requires an ayni-orchestrator-* namespace of at most 54 characters')
    return value


def fingerprint(paths):
    digest = hashlib.sha256()
    for path in sorted(set(paths)):
        try:
            if path.is_symlink():
                content = os.readlink(path).encode()
            elif path.is_file():
                content = path.read_bytes()
            else:
                continue
        except FileNotFoundError:
            continue  # Atomic editor saves/deletes are detected on the next scan.
        digest.update(str(path.relative_to(ROOT)).encode() + b'\0')
        digest.update(content + b'\0')
    return digest.hexdigest()


def inputs():
    shared = [ROOT / name for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'dev/live.Dockerfile', '.dockerignore')]
    shared += list((ROOT / 'proto').rglob('*')) + list((ROOT / 'backend/libs').rglob('*'))
    control = list((ROOT / 'backend/servers/control-plane').rglob('*'))
    control += list((ROOT / 'dev/fixtures/task-source').rglob('*')) + list((ROOT / 'dev/fixtures/task-quality').rglob('*'))
    executor = list((ROOT / 'backend/workers/conversation-supervisor').rglob('*'))
    schema = list((ROOT / 'backend/servers/control-plane/crates/workflow-postgres/migrations').glob('*.sql'))
    schema += [ROOT / 'backend/servers/control-plane/crates/workflow-postgres/src/migrations.rs']
    return {COMPONENTS[0]: fingerprint(shared + control), COMPONENTS[1]: fingerprint(shared + executor), 'schema': fingerprint(schema)}


def read_state(path):
    return json.loads(path.read_text()) if path.exists() else {}


def write_state(path, state, **updates):
    state.update(updates)
    state['updated_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(state, indent=2) + '\n')
    temporary.replace(path)


def run(command, env, capture=False):
    # Children own process groups, so Ctrl-C cannot orphan a compiler/port-forward.
    child = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True,
                             stdout=subprocess.PIPE if capture else None, text=True)
    try:
        output, _ = child.communicate()
    except BaseException:
        stop(child)
        raise
    if child.returncode:
        raise subprocess.CalledProcessError(child.returncode, command)
    return (output or '').strip()


def stop(child):
    if child.poll() is None:
        os.killpg(child.pid, signal.SIGTERM)
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()


def reconcile(path, state, env, allow_migration=False):
    desired = inputs()
    write_state(path, state, desired=desired)
    active = state.get('active', {})
    if active and active['schema'] != desired['schema'] and not allow_migration:
        write_state(path, state, phase='migration_required', message='Schema inputs changed. Stop live and run dev/ctl live migrate; applied migration edits require a forward migration or explicit reset.')
        return False
    images = dict(state.get('images', {}))
    for component in COMPONENTS:
        if active.get(component) != desired[component] or component not in images:
            write_state(path, state, phase='building', message=f'Building {component}; previous deployed build remains active')
            images[component] = run([str(ROOT / 'dev/scripts/live-images.sh'), component, desired[component]], env, capture=True)
    if active.get(COMPONENTS[0]) != desired[COMPONENTS[0]]:
        write_state(path, state, phase='building_gateway', message='Building the local API gateway')
        run(['cargo', 'build', '--locked', '-p', 'ayni-orchestrator-control-plane', '--bin', 'workflow-gateway'], env)
    if desired != inputs():
        write_state(path, state, phase='changed_during_build', message='Source changed during compilation; discarding this deployment and rebuilding')
        return False
    write_state(path, state, phase='deploying', message='Rolling out built images; checking migration and workload readiness')
    deploy_env = dict(env, AYNI_ORCHESTRATOR_LIVE_MIGRATE='true' if allow_migration else 'false')
    run([str(ROOT / 'dev/scripts/live-deploy.sh'), *(images[c] for c in COMPONENTS)], deploy_env)
    write_state(path, state, phase='ready', active=desired, images=images,
                source_revision=run(['git', 'rev-parse', 'HEAD'], env, capture=True),
                message='Development images ready; active identifies source bytes, not production acceptance')
    return True


@contextlib.contextmanager
def exclusive(directory):
    with (directory / 'owner.lock').open('w') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError('Another live command owns this namespace; stop it before migrate/reset/once') from error
        yield


def heartbeat(path, stopped):
    while not stopped.wait(5):
        try:
            path.touch(exist_ok=True)
        except OSError:
            pass


def require_free_ports():
    ports = []
    for key, fallback in (('AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT', 3000), ('AYNI_ORCHESTRATOR_LIVE_UI_PORT', 5174)):
        value = os.environ.get(key, str(fallback))
        if not value.isdigit() or not 1024 <= int(value) <= 65535:
            raise ValueError(f'{key} must be a port from 1024 to 65535')
        ports.append(int(value))
    if len(set(ports)) != len(ports):
        raise ValueError('UI and gateway ports must differ')
    for port in ports:
        with socket.socket() as listener:
            try:
                listener.bind(('127.0.0.1', port))
            except OSError as error:
                raise ValueError(f'127.0.0.1:{port} is occupied; stop that local service before starting live') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', nargs='?', choices=('watch', 'once', 'status', 'migrate', 'reset'), default='watch')
    parser.add_argument('confirmation', nargs='?')
    args = parser.parse_args()
    selected = namespace()
    env = dict(os.environ, AYNI_ORCHESTRATOR_NAMESPACE=selected, HELM_NO_PLUGINS='1')
    directory = ROOT / '.local/live' / selected
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / 'status.json'
    state = read_state(path)
    if args.command == 'status':
        print(json.dumps(state or {'phase': 'not_started', 'namespace': selected}, indent=2))
        return 0
    if args.confirmation and args.command != 'reset':
        parser.error('namespace confirmation is only accepted for reset')
    with exclusive(directory):
        context = run(['kubectl', 'config', 'current-context'], env, capture=True)
        if context != 'docker-desktop':
            raise ValueError('live only supports the explicitly selected docker-desktop context')
        if args.command == 'reset':
            if args.confirmation != selected:
                parser.error(f'reset permanently deletes all retained state; supply exact namespace: {selected}')
            run([str(ROOT / 'dev/ctl'), 'purge', selected], env)
            write_state(path, state, phase='reset', active={}, images={}, message='Owned environment deleted; run live to recreate it')
            return 0
        children = []
        stopped = threading.Event()
        try:
            write_state(path, state, namespace=selected, owner_pid=os.getpid(), phase='starting')
            if args.command == 'watch':
                require_free_ports()
                threading.Thread(target=heartbeat, args=(path, stopped), daemon=True).start()
                for command in ([str(ROOT / 'dev/scripts/live-gateway.sh')],
                                ['pnpm', '--dir', 'apps/web', 'dev']):
                    children.append(subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True))
            last = None
            while True:
                current = inputs()
                if current != last:
                    # Short debounce; compile errors retry on the next edit, not in a busy loop.
                    time.sleep(0.4)
                    last = inputs()
                    try:
                        ready = reconcile(path, state, env, args.command == 'migrate')
                    except subprocess.CalledProcessError as error:
                        message = (f'Build failed ({error.returncode}); previous deployed services remain active. See terminal logs.'
                                   if state['phase'] in ('building', 'building_gateway') else
                                   f'Deployment failed ({error.returncode}); it may be partially updated. Active records last successful rollout; inspect Pods and terminal logs.')
                        write_state(path, state, phase='failed', message=message)
                        if args.command != 'watch':
                            return 1
                    else:
                        if args.command != 'watch':
                            return 0 if ready else 1
                        if state['phase'] == 'changed_during_build':
                            last = None
                if any(child.poll() is not None for child in children):
                    raise ValueError('Frontend or gateway exited; stopping live loop')
                time.sleep(1)
        finally:
            stopped.set()
            for child in children:
                stop(child)
            write_state(path, state, owner_pid=None, stopped_at=datetime.datetime.now(datetime.timezone.utc).isoformat())


if __name__ == '__main__':
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'live: {error}', file=sys.stderr)
        sys.exit(1)
