# M3 retry-backoff evaluation fixture

This small standalone program is intentionally wrong. It is an evaluation input,
not orchestrator retry policy. The M3 coding task is to correct the formula to
`min(60, 2 × 2^(attempt−1))` seconds and add regression tests. The expected
observations are attempt `1 → 2`, `2 → 4`, `3 → 8`, and `6 → 60` seconds.

The evaluation starts from a pinned commit containing this defect. The agent's
candidate is checked from a fresh workspace; the evaluator must verify these
numbers independently of tests written by the agent.
