# Cumulative recovery acceptance

Current checks use the [single local environment](local-development.md).
There is one frontend at **http://127.0.0.1:5174** and no separate acceptance stack.

```text
cargo xtask test durability
```

These checks establish their named contracts only. The PostgreSQL and worker
Rust suites retain admission, frozen-input, cancellation, stale-generation,
publication and replay assertions. Native frontend assertions run through
`cargo xtask test deployment browser`; debug source collection and recovery use
`cargo xtask test deployment debug` and `cargo xtask test deployment recovery`.
Deployment checks serialize against the canonical environment.

The combined milestone harness that previously occupied this page depended on
retired browser flows. It has been removed rather than reported as qualified.
Its [historical protocol and evidence](https://github.com/ayni-oss/orchestrator/blob/9de06f53/docs/technical/operations/recovery-acceptance.md)
remain available in Git history. Those reports are not current-source acceptance
or evidence of live provider/GitHub operation, production capacity, or Docker
Desktop NetworkPolicy enforcement.
