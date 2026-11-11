# Task source fixture

`cargo xtask generate fixtures` creates `source.pack` and `source.json` with Git, fixed fixture
identity and timestamps. The pack contains one commit, its tree and two file
blobs. It has no remote credentials or repository history. Regenerate with
`cargo xtask generate fixtures`; do not edit generated identities.

The lock is deliberately synthetic. Only the explicitly enabled deterministic
task worker uses it. This fixture establishes actual source reconstruction and
retained changes on Kubernetes, not Ayni image compatibility or paid provider use.
