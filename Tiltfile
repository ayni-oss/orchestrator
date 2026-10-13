# Development-only live updates for the Helm-managed Docker Desktop release.
# `dev/ctl hot-reload` creates the release before this file attaches to it.

allow_k8s_contexts('docker-desktop')

namespace = os.getenv('AYNI_ORCHESTRATOR_NAMESPACE', 'ayni-orchestrator-dev')

def attach(name, container, deps, live_update, port_forwards=[], sync_web=False, sync_control_plane=False):
    if sync_control_plane:
        apply_cmd = [
            "bash", "-c",
            'bash dev/scripts/sync-control-plane-source.sh && exec kubectl --context docker-desktop --namespace "${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-dev}" get deployment/control-plane --output=yaml',
        ]
    elif sync_web:
        apply_cmd = [
            'bash', '-c',
            'bash dev/scripts/sync-web-source.sh && exec kubectl --context docker-desktop --namespace "${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-dev}" get deployment/control-plane --output=yaml',
        ]
    else:
        apply_cmd = [
            'kubectl', '--context', 'docker-desktop', '--namespace', namespace,
            'get', 'deployment/control-plane', '--output=yaml',
        ]
    k8s_custom_deploy(
        name,
        apply_cmd=apply_cmd,
        delete_cmd=['echo', 'Skipping delete; Helm manages deployment/control-plane.'],
        deps=deps,
        live_update=live_update,
        container_selector=container,
    )
    k8s_resource(name, port_forwards=port_forwards)

attach(
    name='control-plane-live',
    container='control-plane',
    deps=['backend', 'proto', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'dev/scripts/sync-control-plane-source.sh', 'dev/scripts/restart-control-plane.sh'],
    sync_control_plane=True,
    live_update=[
        sync('backend', '/opt/source/backend'),
        sync('proto', '/opt/source/proto'),
        sync('Cargo.toml', '/opt/source/Cargo.toml'),
        sync('Cargo.lock', '/opt/source/Cargo.lock'),
        sync('rust-toolchain.toml', '/opt/source/rust-toolchain.toml'),
        sync('dev/scripts/restart-control-plane.sh', '/opt/source/dev/scripts/restart-control-plane.sh'),
        # Compilation shares the container's memory budget with the running server.
        run('cargo build --locked --jobs 1 --package ayni-orchestrator-control-plane --bins'),
        run('sh dev/scripts/restart-control-plane.sh'),
    ],
    port_forwards=['18081:8080'],
)

attach(
    name='web-live',
    container='web',
    deps=['apps/web', 'packages/bff', 'clients/typescript', 'tsconfig.base.json', 'dev/scripts/sync-web-source.sh'],
    sync_web=True,
    live_update=[
        sync('apps/web', '/opt/source/apps/web'),
        sync('packages/bff', '/opt/source/packages/bff'),
        sync('clients/typescript', '/opt/source/clients/typescript'),
        sync('tsconfig.base.json', '/opt/source/tsconfig.base.json'),
    ],
    port_forwards=['18080:3000'],
)
