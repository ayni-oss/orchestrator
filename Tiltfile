# Development-only live updates for the Helm-managed Docker Desktop release.
# `dev/ctl hot-reload` creates the release before this file attaches to it.

allow_k8s_contexts('docker-desktop')

namespace = os.getenv('AYNI_ORCHESTRATOR_NAMESPACE', 'ayni-orchestrator-dev')

def attach(name, container, deps, live_update, port_forwards=[]):
    k8s_custom_deploy(
        name,
        apply_cmd=[
            'kubectl', '--context', 'docker-desktop', '--namespace', namespace,
            'get', 'deployment/control-plane', '--output=yaml',
        ],
        delete_cmd=['echo', 'Skipping delete; Helm manages deployment/control-plane.'],
        deps=deps,
        live_update=live_update,
        container_selector=container,
    )
    k8s_resource(name, port_forwards=port_forwards)

attach(
    name='control-plane-live',
    container='control-plane',
    deps=['backend', 'proto', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'],
    live_update=[
        sync('backend', '/workspace/backend'),
        sync('proto', '/workspace/proto'),
        sync('Cargo.toml', '/workspace/Cargo.toml'),
        sync('Cargo.lock', '/workspace/Cargo.lock'),
        sync('rust-toolchain.toml', '/workspace/rust-toolchain.toml'),
        run('cargo build --locked --package ayni-orchestrator-control-plane'),
        run('kill -TERM 1'),
    ],
    port_forwards=['18081:8080'],
)

attach(
    name='web-live',
    container='web',
    deps=['apps/web', 'packages/bff', 'clients/typescript', 'tsconfig.base.json'],
    live_update=[
        sync('apps/web', '/workspace/apps/web'),
        sync('packages/bff', '/workspace/packages/bff'),
        sync('clients/typescript', '/workspace/clients/typescript'),
        sync('tsconfig.base.json', '/workspace/tsconfig.base.json'),
    ],
    port_forwards=['18080:3000'],
)
