#!/usr/bin/env bash
set -euo pipefail
readonly repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repository_root"
# This host gate is required. A jailed or unavailable socket is a failure, never
# a reason to silently skip the protocol fixtures.
python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    listener.listen(1)
    with socket.create_connection(listener.getsockname(), timeout=5) as client:
        with listener.accept()[0] as server:
            server.settimeout(5)
            client.sendall(b'loopback')
            assert server.recv(8) == b'loopback'
print('required host loopback capability: passed')
PY
inventory="$(mktemp "${TMPDIR:-/tmp}/ayni-loopback-inventory.XXXXXX")"
trap 'rm -f "$inventory"' EXIT
cargo test --locked --workspace --all-targets requires_loopback -- --list --ignored --format terse > "$inventory"
python3 - "$inventory" <<'PY'
from collections import Counter
from pathlib import Path
import re
import sys

marker = 'requires loopback networking; run scripts/ci/check-workflow-loopback.sh'
attribute = re.compile(r'#\[ignore\s*=\s*"' + re.escape(marker) + r'"\s*\](?:(?!\bfn\b).)*?\bfn\s+(\w+)', re.S)
declared = []
for path in Path('backend').rglob('*.rs'):
    source = path.read_text()
    declared.extend(attribute.findall(source))
if not declared or any(not name.endswith('_requires_loopback') for name in declared):
    raise SystemExit('loopback test declarations are empty or lack the required suffix')
listed = re.findall(r'^([\w:]+): test$', Path(sys.argv[1]).read_text(), re.M)
actual = [name.rsplit('::', 1)[-1] for name in listed]
if Counter(declared) != Counter(actual):
    missing = Counter(declared) - Counter(actual)
    extra = Counter(actual) - Counter(declared)
    raise SystemExit(f'loopback Cargo inventory differs: missing={dict(missing)} extra={dict(extra)}')
print(f'required host loopback inventory: {len(actual)} tests')
PY
cargo test --locked --workspace --all-targets requires_loopback -- --ignored --nocapture
