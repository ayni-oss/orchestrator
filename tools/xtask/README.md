# Project tooling

`cargo xtask` is the Rust entry point for repository-owned tooling. The migration
is in progress; do not delete an old runner until its behavioral checks and
recovery cases have replacements.

Available commands:

- `cargo xtask check docs`, `docs-site`, `security`, `api`, `protocol`, `charts`.
- `cargo xtask generate protocol`, `schema [--write]`, `contract-bundle OUTPUT`.
- `cargo xtask test unit`, `frontend`, `loopback`, `postgres [SUITE]`.
- `cargo xtask evidence publication BASELINE CURRENT CREATES PULLS [IDENTITY]`.
- `cargo xtask evidence recovery DIRECTORY` and `manifest DIRECTORY [REPORT]`.
- `cargo xtask check tooling` inventories prohibited supporting-language files
  and deliberately fails until the migration is complete.

PostgreSQL tests create disposable loopback-only containers with public fixture
credentials and clean up their own container. They never connect to a development
database. Schema generation uses a disposable database without a published port.
Evidence checks reject changed artifacts, path traversal, symlinks, missing
replays, duplicate external effects and unsupported milestone closure claims.

The target development interface and cutover are recorded in the
[roadmap](../../ROADMAP.md#one-development-environment-and-rust-tooling).
