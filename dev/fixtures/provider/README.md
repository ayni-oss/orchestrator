# Deterministic provider fixture

This server implements the bounded foreground Responses profile without calling
OpenAI. It uses the public dummy credential `provider-fixture-key` and model
`fixture-model`. It rejects provider state, native tools and unsupported request
profiles. It retains request counters and client call IDs; it does not retain
prompts, response text or authorization headers.

Deploy only to the local Docker Desktop context, with the same namespace as the
control plane. For example:

```sh
kubectl --context docker-desktop -n ayni-orchestrator-dev create configmap provider-fixture \
  --from-file=server.mjs=dev/fixtures/provider/server.mjs --dry-run=client -o yaml \
  | kubectl --context docker-desktop -n ayni-orchestrator-dev apply -f -
kubectl --context docker-desktop -n ayni-orchestrator-dev apply -f dev/fixtures/provider/workload.yaml
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout status deployment/provider-fixture
kubectl --context docker-desktop -n ayni-orchestrator-dev port-forward service/provider-fixture 18080:8080
```

Configure the local chart with `ayni-orchestrator.providers.openai.enabled=true`,
`ayni-orchestrator.providers.openai.fixtureEgress=true` and
`ayni-orchestrator.providers.openai.baseUrl=http://provider-fixture.ayni-orchestrator-dev.svc:8080/v1`.
Create the local authority bearer with `dev/scripts/configure-development-secrets.sh`
before upgrading. Start `dev/ctl console` and run:

```sh
PROVIDER_FIXTURE_URL=http://127.0.0.1:18080 \
  node apps/console/provider-acceptance.mjs
```

The acceptance script creates an isolated workspace and a workspace-scoped
Kubernetes Secret containing only the public dummy credential. It uses the
explicit `docker-desktop` context and `<namespace>-sessions`; set
`AYNI_ORCHESTRATOR_NAMESPACE` to select another local deployment namespace.
It preserves accepted application data and retained ledgers.

`fixture:no-usage` omits token usage, `fixture:disconnect` closes the connection
after accepting the request, and `fixture:delay` delays the result for a
cancellation race. Other inputs return a completed response with reported usage.
`GET /stats` exposes counts and correlation IDs for proving that replay does not
make another provider request. These fixtures establish protocol and recovery
behavior, not live-provider latency or availability.
