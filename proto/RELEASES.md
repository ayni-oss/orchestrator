# Protocol release state

The active private baseline is `worker-owned-v2` (see `BASELINE`).

This explicitly authorized breaking replacement removes candidate/review APIs,
replaces task execution output with worker progress/results and pull request
receipts, freezes limits at admission, and adds publication-only retry. Deploy
the server and regenerated clients together. Older databases are rejected
without conversion. No compatibility implementation is retained.

Subsequent changes use the configured Buf comparison against the established
baseline. A future baseline reset requires an explicit breaking-contract
change. Private bundles identify the source revision and checksum; creation
of bundles or descriptors does not publish endpoints or SDKs.
