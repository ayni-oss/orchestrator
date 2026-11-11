# Development

Run `cargo xtask dev` or `make dev`, then open **http://127.0.0.1:5174**.
Debug capture and the pinned local runtime profile are enabled automatically.

The Rust controller owns one Vite frontend, one gateway and the existing
`ayni-orchestrator-live` deployment on `docker-desktop`. Use
`cargo xtask dev status`, `cargo xtask dev redeploy` and `cargo xtask dev stop`.
Storage is retained across restarts. There are no alternate namespace or port
selectors and no separate acceptance frontend.

See the [development guide](../docs/technical/operations/local-development.md)
for prerequisites, reload behavior, migrations, validation and retained data.
