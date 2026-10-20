# Private-v1 replacement feasibility

This directory models the deliberate M1 reset of the private
`ayni.orchestrator.v1` workflow surface without changing canonical production
protos, generated clients, listener registration, or runtime authority.

## Candidate overlay

`proto/ayni/orchestrator/v1/` is overlaid onto a temporary copy of `proto/` by
`scripts/ci/check-workflow-private-v1-feasibility.sh`. The candidate keeps only:

- `ProjectService.CreateWorkspace` and `CreateProject`;
- `ProjectConversationService.CreateConversation`, `SendMessage`,
  `GetConversationSnapshot`, `ListConversationHistory`, and `WatchConversation`;
- the field numbers and reservations fixed by the M1 durable-foundation contract.

The approved feasibility shape makes snapshots self-contained with `title = 12` and
`created_at = 13`, uses a compact terminal-Turn summary at snapshot field 7, uses
`PresentationState` for activity/update presentation, uses enums for product-owned
roles/kinds/waiting dependencies, and retains bounded strings for safe diagnostic
reason codes. Watch heartbeats are a separate cursorless frame and never represent
workflow progress.

These are candidate wire choices. They become the private-v1 compatibility baseline
only in the coordinated authority-switch change; this directory is not compiled or
registered by production.

## Mechanical proof

From the repository root:

```sh
scripts/ci/check-workflow-private-v1-feasibility.sh
```

The check proves that:

1. the candidate formats and lints as an independent Buf module and as an overlay;
2. the current private-v1 baseline rejects the intentional breaking replacement;
3. the exact refreshed baseline passes `FILE` compatibility;
4. removing an RPC after that reset is detected by `FILE` compatibility;
5. two complete TypeScript generations of the overlay are byte-for-byte identical;
6. `consumer-inventory.txt` matches the current handwritten Rust, TypeScript, web,
   and shell consumer search;
7. a type-level listener model admits only a PostgreSQL workflow owner on the trusted
   listener; and
8. the custom-client listener has no workflow-registration operation.

The listener model proves a viable construction pattern, not that production has
already cut over. The authority-switch change must instantiate the same separation,
remove the SQLite-backed workflow adapters before registering the replacement, rerun
all affected Rust/TypeScript/BFF/web tests, refresh `proto/BASELINE`, and record the
resulting immutable revision.

## Generated and handwritten consumers

Rust bindings and the reflection descriptor are generated from every canonical
`proto/**/*.proto` by
`backend/servers/control-plane/crates/grpc-contract/build.rs`. TypeScript bindings are
checked in under `clients/typescript/src/generated/` and generated with `clean: true`.
Both sets must therefore change from the same canonical proto revision.

`consumer-inventory.txt` is the sorted output of the retained handwritten-consumer
search in the feasibility script. Generated TypeScript is intentionally excluded
from that list; its relevant outputs are currently:

- `clients/typescript/src/generated/ayni/orchestrator/v1/projects.ts`;
- `clients/typescript/src/generated/ayni/orchestrator/v1/project_conversations.ts`;
- `clients/typescript/src/generated/index.ayni.orchestrator.v1.ts`.

The eventual cutover must remove or adapt every inventoried consumer. M1 does not
silently preserve old aliases or route either service to SQLite.
