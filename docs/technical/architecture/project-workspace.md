# Product workspace status

The initial product workspace and contributor console are removed. There is no
supported browser UI. [The former web app](../../../examples/web/EXAMPLE.md) is
preserved as an isolated reference for future design; its clients and backend
contracts are historical.

Backend discovery, conversations, execution timelines, plans, task evidence,
recovery and publication continue through canonical v1 APIs. The local
`dev/ctl gateway` command exposes an allowlisted JSON/SSE adapter without pages,
static assets or legacy backend wiring. See [the gateway boundary](contributor-console.md).

## Next interface

The [next roadmap focus](../../../ROADMAP.md#next-focus-repository-preparation-with-a-usable-ui)
delivers repository preparation and the new UI together. Each slice includes a
usable browser journey against canonical APIs and its own qualification evidence.
This is planned delivery; the API-only status above remains current.

| Surface | Planned user outcome |
| --- | --- |
| Shared repository catalog | Connect a repository once, opt into Ayni setup and inspect access, analysis and runtime history. |
| Integration setup | Configure source, registry and certificate references; inspect separate probe results and trust-review requirements. |
| Workspace/project | Create projects and associate shared repositories directly, including the same repository across workspaces. |
| Runtime inspector | Follow frozen inputs, build/publication/verification/cleanup, safe findings and explicit retries or rebuilds. |
| Chat, Plans and Tasks | Discuss, approve an exact plan, prepare/start with verified server-side inputs and inspect separate delivery outcomes. |
| Feedback and evidence | Follow questions, failures and confirmed recovery to the originating conversation and shared preparation evidence. |

The [repository preparation contract](repository-preparation.md) owns shared
catalog scope and authority. The [delivery plan](../research/repository-runtime-workspace-delivery.md)
owns the UI acceptance matrix, local prerequisites and interruption scenarios.
The historical example supplies design reference; its data access is not a
backend implementation. Reload, reconnect, concurrent tabs and scope changes
must reconstruct retained authority without duplicating commands.
