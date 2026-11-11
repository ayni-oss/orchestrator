# Rust project tooling

`cargo xtask` owns development, validation, generation and acceptance.

- `dev [status|stop|redeploy]`: the singleton at http://127.0.0.1:5174.
- `check docs|docs-site|security|api|protocol|charts|tooling`.
- `generate protocol|schema [--write]|fixtures|contract-bundle OUTPUT`.
- `test unit|frontend|loopback|postgres [SUITE]|jetstream|durability`.
- `test deployment debug|recovery|browser|hmr|topology`: serial canonical checks.
- `evidence publication BASELINE CURRENT CREATES PULLS [IDENTITY]`.
- `evidence recovery DIRECTORY`, `manifest DIRECTORY [REPORT]`.
- `evidence traces INPUT_JSONL OUTPUT_DIRECTORY [conversation|full]`.
- `install helm|ayni`: checksum-verified Linux/amd64 CI bootstrap.

The standard-library-only bootstrap binaries in `tools/bootstrap` own image
build loops, PostgreSQL volume preparation and broker startup/provisioning.
They are also compiled directly with rustc during Docker bootstrap. PostgreSQL
role initialization is declarative SQL. No supporting Python, shell or
JavaScript implementation remains in these tooling paths.

Native frontend tests and workload example inputs retain their own languages.
Retired browser harnesses and their aggregate closure reports were removed;
they had not qualified the current product. Current backend contracts continue
through Rust PostgreSQL/worker/durable-handoff tests. Evidence verifiers reject
changed artifacts, unsafe paths, missing replay and duplicate external effects.

See [local development](../../docs/technical/operations/local-development.md)
for the supported environment and validation commands, and the
[roadmap](../../ROADMAP.md#one-development-environment-and-rust-tooling) for scope.
