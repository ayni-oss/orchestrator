# Live development

`make dev` is a thin alias for `cargo xtask dev`. Open **http://127.0.0.1:5174**.
Frontend changes use HMR; backend changes rebuild affected components and roll
out into `ayni-orchestrator-live`. Debug logs and runtime workers are enabled.

Use `cargo xtask dev status` for the owner, checkout, branch and readiness.
Use `cargo xtask dev stop` before changing checkouts. Start again with the same
command; the database, broker and retained logs remain intact.

See [local development](../docs/technical/operations/local-development.md) for
migration safeguards, failure handling, browser checks and Docker Desktop's
unqualified NetworkPolicy boundary.
