# Runtime component qualification

`dev/scripts/runtime-preparation-qualify.py` exercises actual worker binaries and independent static probes in a uniquely created namespace on an explicit local cluster. Docker Desktop is the default; the separate policy-enforcing cluster requires `--context kind-ayni-runtime-qualification --kubeconfig /absolute/state/kubeconfig`. Only those targets are accepted, with exact Docker node identity checks. It creates only disposable fixture pods, PVCs, ConfigMaps and synthetic Secrets, then removes its namespace and imported candidate image aliases. It uses an empty Docker configuration and the current user's explicit Desktop socket. It does not use saved provider connections. See the [separate cluster setup](cluster/README.md).

Build and import the local worker/probe images first with `dev/scripts/runtime-preparation-images.sh`. Supply the reviewed Linux arm64 Ayni CLI and its SHA256 explicitly. Then run:

```sh
python3 dev/scripts/runtime-preparation-qualify.py \
  --context kind-ayni-runtime-qualification \
  --kubeconfig /tmp/ayni-runtime-cluster/kubeconfig \
  --worker-image local/ayni-runtime-worker:<tag> \
  --probe-image local/ayni-runtime-probes:<tag> \
  --cli /absolute/path/to/ayni-linux-arm64 \
  --mise /absolute/path/to/verified-mise \
  --docker-config /absolute/path/to/empty-docker-config
```

The driver verifies the pinned CLI and Mise checksums before creating resources. Keep enough free host space for the VFS build and the nine GiB guard; this is independent of PVC requests and asynchronous kubelet eviction limits. The fixture uses Node 22.14.0 with no external package dependencies and a real configured size check. Its committed policy contains only a public test key. Each fresh run creates a new ephemeral signing key, regenerates the lock through Ayni, and places the seed only in its own immutable Kubernetes Secret. It never publishes externally.

A successful full run covers worker build, independent freeze, public Ayni certification, final OCI assembly, a digest-preserving isolated local registry round trip, content verification and managed quality compatibility. Failure cases reject a wrong signer, altered protected content, altered source, and unsafe parent identity/capability contexts. The cancellation check observes the nested processes within the owned pod's mapped UID range and confirms their removal and emptyDir cleanup after pod deletion. A network-policy check uses an explicit positive control and a denied candidate pod.

The final log prints an evidence directory containing `gate.json`, safe stage reports and probe results. Each JSON gate lists what that invocation actually covered and excluded. `--reuse-public-evidence <directory>` reprobes an earlier public signed OCI/source pair and runs freeze and failure checks; it does not repeat positive build/certification/publication. `--probes-only` further skips those repeated worker stages. `--diagnose-quality` enables a bounded stdout/stderr observer only for this synthetic fixture; it does not change production pod confinement.

`--registry-denial-only --reuse-public-evidence <directory>` invokes the real worker's read-only GHCR discovery transport using deliberately invalid synthetic credentials. It requires an `unavailable` report with `registry_access_denied`. It neither reads a real credential nor publishes an image.

These are component checks. They do not establish real GitHub acquisition, production control-plane admission, live GHCR publication or supervisor restart/authority fencing. The default Desktop runtime remains disabled because its policy negative failed. The separate Kubernetes 1.36.4/Cilium 1.20.4 cluster passed the complete fresh Node component pipeline, all listed negative checks, and the independent supervisor restart/cleanup fixture. A public cold CRI GHCR pull and real-worker invalid-credential GHCR denial also passed there. Only that measured local profile is enabled for operator/UI acceptance; real provider acquisition and private publication remain untested by this fixture. Arbitrary Rust repositories have not been qualified within the tested eight GiB builder allowance.
