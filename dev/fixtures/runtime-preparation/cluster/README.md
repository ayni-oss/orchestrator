# Separate policy-enforcing local cluster

The default Docker Desktop cluster accepted NetworkPolicy objects but failed the actual deny-connectivity test. Keep its runtime profile disabled. This separate native arm64 cluster preserves that cluster's network, retained data and UI.

The pins are kind 0.33.0, Kubernetes 1.36.4 and Cilium 1.20.4. Cilium lists Kubernetes 1.36 in its [tested compatibility matrix](https://docs.cilium.io/en/stable/network/kubernetes/compatibility/). Configuration follows the [official kind installation guide](https://docs.cilium.io/en/stable/installation/kind/): disable the default CNI, retain kube-proxy, and use Kubernetes IPAM. The chart and macOS arm64 kind binary are checksum-verified; Cilium's native image digests are verified descendants of its official release indexes.

Create a new state directory with:

```sh
dev/scripts/runtime-preparation-cluster.sh /tmp/ayni-runtime-cluster
```

The script refuses an existing named node, Docker network or state directory. It creates only cluster `ayni-runtime-qualification`, node `ayni-runtime-qualification-control-plane`, and Docker network `ayni-runtime-qualification`. The pinned kind provider's `KIND_EXPERIMENTAL_DOCKER_NETWORK` override keeps that network separate; kind labels this override experimental. This is a measured local development topology, not a production Kubernetes installation procedure.

The normal parallel dev helper requires `/tmp/ayni-runtime-cluster/kubeconfig`. An alternative new state path works with the component drivers when passed explicitly, but does not satisfy that helper's fixed path. The kubeconfig lives only in the supplied state directory. Always pass both `--context kind-ayni-runtime-qualification` and its explicit kubeconfig, or set `KUBECONFIG` to that file for helpers that require it. No command falls back to an ambient context. Wait for Cilium and the node to be ready, then prove both allowed connectivity and denial; API admission alone is insufficient.

The fresh node rejected Quay's certificate chain. Its public CA bundle matched the existing Desktop node; public GHCR TLS and an actual cold CRI pull succeeded without trust changes. Bootstrap preloading addresses Quay without bypassing TLS: `runtime-preparation-preload.py` pulls public pinned images through Docker's verified TLS, checks the root manifest bytes, selects and verifies native arm64 content, and imports the original index plus the selected content into only this node. It neither disables TLS verification nor substitutes a child digest for an original index. For example:

```sh
python3 dev/scripts/runtime-preparation-preload.py \
  docker.io/library/postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3
```

Build/import worker tooling with `AYNI_RUNTIME_KUBE_CONTEXT=kind-ayni-runtime-qualification dev/scripts/runtime-preparation-images.sh ...`. The script checks the exact node's ownership labels and emits immutable manifest references. The qualification driver accepts `--context kind-ayni-runtime-qualification --kubeconfig /absolute/state/kubeconfig`. Its `--network-only --reuse-public-evidence /absolute/evidence` mode proves actual candidate connectivity and denial before another full build. Full runs must also pass native rootless engine execution, managed compatibility, cancellation, tampering and cleanup gates. Preserve at least nine GiB free host space; allow additional VFS build capacity before starting.

A separate dev stack uses its own application namespace and fresh internal authority/database credentials. Existing saved provider connections and user secrets are not copied. The actual supervisor restart/cleanup fixture passed on this cluster as a separate gate; see the [packaging README](../../../../deploy/images/runtime-preparation/README.md). Live GitHub acquisition and GHCR publication require their own operator/UI acceptance.

To remove this cluster later, deliberately use the pinned kind binary with `delete cluster --name ayni-runtime-qualification` and the explicit local Docker environment. Remove only the dedicated Docker network after its containers are gone. Never reset Docker Desktop, delete another context's resources, or prune global images/volumes as part of this workflow.

Completed qualification on this cluster includes the fresh Node build → independent freeze → Ayni certification → final OCI → local registry → content/managed verification chain, wrong-key and tampering rejection, unsafe-parent rejection, nested cancellation/storage cleanup, and actual policy positive/negative controls. The runtime profile is enabled only for the separate parallel local stack. Real GitHub acquisition and private GHCR publication remain operator/UI acceptance; arbitrary Rust builds are not qualified within the tested eight GiB allowance.
