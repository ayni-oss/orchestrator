# Workflow schema migrations

This fresh-install catalog starts at `001`. Each table is defined once in its
owning layer, including its final columns, checks, keys and indexes. Earlier
changes to the same table have been folded into that definition.

| Migration | Layer |
| --- | --- |
| `001` | Workspaces, projects and rename receipts |
| `002` | Connections, credentials and connection operations |
| `003` | Repository catalog, inspection, onboarding and hierarchy membership |
| `004` | Runtime signers, revisions and provisioning |
| `005` | Runtime preparations, attempts, selections and receipts |
| `006` | Project configuration revisions, heads and receipts |
| `007` | Conversations, messages, turns, activity, progress and replay |
| `008` | Dispatch, outbox, reports, quarantine and broker capacity |
| `009` | Sessions, frozen context, provider calls and uncertainty receipts |
| `010` | Plans, versions, proposals and approvals |
| `011` | Tasks, preparations, attempts, runs and quality |
| `012` | Task publication and publication reports |
| `013` | Task feedback and recovery |
| `014` | Outcome assessment authorization and results |
| `015` | GitHub source connection deletion with retained references |
| `016` | Provider connection and runtime signer deletion with retained references |

Layers are ordered by their foreign-key dependencies. Circular references between
messages/turns, plans/versions and signers/revisions are completed at the end of
their owning file. They are installed in the same transaction as the tables.
The migration ledger is created by the runner, outside this catalog.

The old 41-step catalog and the temporary single migration at `0042` require a
fresh database. The runner rejects their retained history without changing data;
use a new local namespace rather than deleting an existing installation.

Once installed, these files are immutable. Add subsequent changes from `017`
and register them in `src/migrations.rs`. Before applying pending migrations, the
runner validates the complete applied prefix and its checksums. After changing
schema definitions, regenerate and verify the catalog fingerprint with
`scripts/ci/generate-workflow-schema-manifest.py --write` from the repository root.
