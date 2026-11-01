"""Explicit local cluster selection; never modify the user's current context."""
import os
import subprocess
import sys

QUALIFICATION_CONTEXT = 'kind-ayni-runtime-qualification'
QUALIFICATION_KUBECONFIG = '/tmp/ayni-runtime-cluster/kubeconfig'
QUALIFICATION_NAMESPACE = 'ayni-orchestrator-runtime-qualification'
NODES = {'docker-desktop': ('desktop-control-plane', 'desktop'),
    QUALIFICATION_CONTEXT: ('ayni-runtime-qualification-control-plane', 'ayni-runtime-qualification')}


def selected_context(environment=None):
    env = os.environ if environment is None else environment
    context = env.get('AYNI_ORCHESTRATOR_CONTEXT', 'docker-desktop')
    if context not in NODES:
        raise ValueError('Only docker-desktop and the explicit isolated runtime qualification context are supported')
    if context == QUALIFICATION_CONTEXT:
        if env.get('KUBECONFIG') != QUALIFICATION_KUBECONFIG:
            raise ValueError('The runtime qualification context requires its exact dedicated KUBECONFIG')
        if env.get('AYNI_ORCHESTRATOR_NAMESPACE') != QUALIFICATION_NAMESPACE:
            raise ValueError('The runtime qualification context requires its separate qualification namespace')
    return context


def checked_context(environment=None):
    env = dict(os.environ if environment is None else environment)
    context = selected_context(env)
    if context == 'docker-desktop':
        command = ['kubectl', 'config', 'current-context']
    else:
        command = ['kubectl', 'config', 'get-contexts', context, '-o', 'name']
    result = subprocess.run(command, env=env, text=True, capture_output=True, timeout=15)
    if result.returncode or result.stdout.strip() != context:
        raise ValueError('Selected local Kubernetes context is unavailable; no fallback is permitted')
    return context


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ('check', 'node', 'cluster'):
        raise ValueError('usage: local_context.py check|node|cluster')
    context = checked_context() if sys.argv[1] == 'check' else selected_context()
    print(context if sys.argv[1] == 'check' else NODES[context][0 if sys.argv[1] == 'node' else 1])


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(str(error) if isinstance(error, ValueError) else 'Local context validation unavailable', file=sys.stderr)
        raise SystemExit(1)
