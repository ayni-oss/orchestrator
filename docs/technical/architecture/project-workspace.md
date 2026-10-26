# Product workspace status

The initial product workspace and contributor console are removed. There is no
supported browser UI. [The former web app](../../../examples/web/EXAMPLE.md) is
preserved as an isolated reference for future design; its clients and backend
contracts are historical.

Backend discovery, conversations, execution timelines, plans, task evidence,
recovery and publication continue through canonical v1 APIs. The local
`dev/ctl gateway` command exposes an allowlisted JSON/SSE adapter without pages,
static assets or legacy backend wiring. See [the gateway boundary](contributor-console.md).

Future UI implementation and browser qualification are separate roadmap work.
