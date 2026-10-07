# Retry-backoff evaluation fixture

This small standalone program is intentionally wrong. It is an evaluation input,
not orchestrator retry policy. The coding task is to correct the formula to
`min(60, 2 × 2^(attempt−1))` seconds and add regression tests. The expected
observations are attempt `1 → 2`, `2 → 4`, `3 → 8`, and `6 → 60` seconds.

The evaluation starts from a pinned commit containing this defect. The worker commits the agent's
changes and runs the mandatory full repository check against that final commit.
The evaluator verifies these numbers and the retained full-check evidence
independently of tests written by the agent.
