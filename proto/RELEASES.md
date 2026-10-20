# Private protocol releases

The active private baseline is `workflow-m1-private-v1` (see `BASELINE`).

This intentional clean reset exposes only the PostgreSQL-authoritative M1 Slice 1
workflow contract:

- `ProjectService.CreateWorkspace` and `CreateProject`;
- `ProjectConversationService.CreateConversation`, `SendMessage`,
  `GetConversationSnapshot`, `ListConversationHistory`, and
  `WatchConversation`;
- the canonical workflow projection, receipt, history, and bounded error types.

`WatchConversation` is reserved in the canonical contract and returns
`UNIMPLEMENTED` until runtime Slice 3. The service is registered only on the
trusted pod-loopback listener. Removed repository, runtime-profile, provider,
connection, certificate, development-task, browser/BFF, and custom-client
contracts are not compiled, reflected, or generated.

The reset replaces `repository-builders-v4`; it does not preserve wire
compatibility or authorize dual registration. Server and generated bindings must
move together. After this reset, unchanged baseline markers use normal Buf
breaking comparison. Any later reset requires a separately approved breaking
contract decision.
