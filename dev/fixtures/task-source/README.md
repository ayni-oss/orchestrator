# Task source fixture

`generate.py` creates `source.pack` and `source.json` with Git, fixed fixture
identity and timestamps. The pack contains one commit, its tree and two file
blobs. It has no remote credentials or repository history. Regenerate with
`python3 dev/fixtures/task-source/generate.py`; do not edit generated identities.

The lock is deliberately synthetic. Only the explicitly enabled deterministic
task worker uses it. This fixture establishes actual source reconstruction and
retained changes on Kubernetes, not Ayni image compatibility or paid provider use.
