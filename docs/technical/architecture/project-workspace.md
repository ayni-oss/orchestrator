# Product workspace status

The historical product workspace and contributor console remain removed. The new
`apps/web` application provides local workspace/project creation and discovery,
plus manual GitHub App configuration. See [integration connections](integration-connections.md).
Registry, OpenAI, repository association and delivery screens remain planned.
[The example-derived application](../../../apps/web/README.md) is the only frontend.
Its duplicate example and standalone diagnostic UI have been removed.

Backend discovery, conversations, execution timelines, plans, task evidence,
recovery and publication continue through canonical v1 APIs. The local
`dev/ctl gateway` command exposes an allowlisted JSON/SSE adapter without pages,
static assets or legacy backend wiring. See [the gateway boundary](contributor-console.md).

## Further interface

The [next roadmap focus](../../../ROADMAP.md#next-focus-repository-preparation-with-a-usable-ui)
delivers repository preparation and the new UI together. Each slice includes a
usable browser journey against canonical APIs and its own qualification evidence.
The connection foundation above is delivered; repository association and delivery
journeys in this table remain planned.

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
