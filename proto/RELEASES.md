# Private protocol releases

The active private baseline is `workflow-m1-private-v1` (see `BASELINE`).

This baseline introduced the PostgreSQL-authoritative workflow contract in M1
Slice 1. Slice 2 retains the same private-v1 surface while adding durable delivery
and scripted execution behind it:

- `ProjectService.CreateWorkspace` and `CreateProject`;
- `ProjectConversationService.CreateConversation`, `SendMessage`,
  `GetConversationSnapshot`, `ListConversationHistory`, and
  `WatchConversation`;
- the canonical workflow projection, receipt, history, and bounded error types.

`WatchConversation` provides atomic snapshots and durable replay in runtime
Slice 3. Contributor console discovery adds bounded workspace, project and
conversation listing in the same v1 package. The service is registered only on
the trusted pod-loopback listener. Removed repository, runtime-profile, provider,
connection, certificate, development-task, browser/BFF, and custom-client
contracts are not compiled, reflected, or generated.

The reset replaces `repository-builders-v4`; it does not preserve wire
compatibility or authorize dual registration. Server and generated bindings must
move together. After this reset, unchanged baseline markers use normal Buf
breaking comparison. Any later reset requires a separately approved breaking
contract decision.

During alpha, the canonical interface version remains v1. Delivery milestones
extend this baseline in place, with generated consumers updated together. Data
revision IDs, session generations and SQL migration ordinals are independent of
interface versioning. Historical baseline names above retain their provenance;
they do not register additional supported interfaces.
