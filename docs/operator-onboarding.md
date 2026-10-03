# Repository onboarding and requests

Deploy the reusable `deploy/charts/ayni-orchestrator` chart in your own namespace.
The web BFF shares the control-plane Pod and reaches gRPC over loopback. Keep the
UI behind your trusted operator access boundary; the chart does not install an
internet-facing ingress or create a GitHub App. Persistent state and runtime
outputs must survive Pod restarts. This release uses one control-plane replica.

Build the private control-plane, web, and worker images from the same reviewed
commit using the repository Dockerfiles. Set `images.controlPlane`, `images.web`,
and `images.worker` to your private registry images. Runtime execution also needs
`runtime.workerImage` pinned by digest. This worker is part of Orchestrator; it is
separate from each repository's generated Ayni environment.

Provide the existing `github-app-credentials` and `openai-api` Secrets as described
in [local development](local-development.md) and
[managed runtime execution](managed-runtime-execution.md). Configure the existing
App for the selected repositories with contents, workflows and pull-request write
access, Actions read access, and the check/comment permissions described in
[managed PR assessments](managed-pr-assessments.md). No credentials enter the UI.

An operator enables each approved repository once, without knowing the future
profile ID. For example (replace the image digests and credential references):

```yaml
runtime:
  enabled: true
  workerImage: registry.example/private/worker@sha256:<worker-digest>
  managedRepositories:
    ayni-oss/sandbox:
      image: ghcr.io/ayni-oss/sandbox-ayni:lock-{lock}
      platform: linux/amd64
      service_account: runtime-coding
      pull_secret: private-environment-pull
      assessment:
        producer_image: registry.example/private/worker@sha256:<worker-digest>
        publication_policy: DRAFT_ONLY
```

`{lock}` is the full 64-character lock fingerprint of the selected profile
revision. Only an explicitly approved repository and its matching private GHCR
package can use this automatic binding. Existing `runtime.profiles` entries still
support explicit profile configuration and take precedence. Use the generated
workflow's `linux/amd64` platform; an arm64 image needs a separate reviewed build
configuration. The control plane and runtime Jobs must be able to share the
output claim; multi-node installations need storage access modes/topology suitable
for their scheduling. The supplied claim uses ReadWriteOnce.

1. Open **Repositories**, enter owner and repository name, and connect. The server
   uses the existing App to observe repository identity, installation and default
   branch; these are not user-entered IDs.
2. Open **Profiles and identity**, then **Inspect repository**. The default branch
   must already contain `.ayni.toml` and a generated `.ayni.lock` compatible with
   the centrally pinned release in `crates/runtime/ayni-release.json`. Inspection
   reports missing policy, incompatible lock, and missing certificate trust policy.
   A policy fingerprint is an observation, not proof of a valid signature.
3. Review the proposed files and select **Create profile and setup PR**. The
   durable controller creates a draft PR containing the environment build workflow
   and the assessment trigger/collector workflows. It never commits to the default
   branch or merges the PR. An interrupted response is recovered by the deterministic
   branch and existing PR identity; conflicting branch contents are not overwritten.
4. Configure the producer signing key and key ID in the repository's Actions
   signing configuration, with the matching public trust key committed in policy
   and lock. Review and merge the setup PR. The environment workflow runs on pushes
   to the default branch, builds with the pinned Ayni executor and publishes a
   private GHCR image tagged with the full lock fingerprint. It refuses an observed
   public package; a new GHCR package is private by default. Review package access
   so the deployment's pull credential can read it.
5. The profile page shows setup PR, workflow link, source SHA, template, fingerprint,
   discovery reference, verification state, resolved digest/platform, last success
   and retry information. Use **Retry reconciliation** after repairing a failure.
   No unsigned image is admitted as a verified managed runtime. An unsigned local
   `ayni check` remains development feedback only.
6. Select **New request**, choose this repository's ready profile and a model, and
   enter the request and acceptance criteria. **Create a draft PR when coding
   succeeds** is selected by default. The server freezes the verified source,
   runtime digest, profile revision, base branch and publication choice. Changes
   to tags, defaults, or the profile do not retarget an existing request.
7. Configure the assessment Actions variables from the admitted profile as listed
   in [managed PR assessments](managed-pr-assessments.md); this remains an operator
   step, and missing variables do not authorize readiness. Follow candidate, advisory check summary, model usage and draft PR on the task
   page. Publication resumes after controller restart. The reviews page separates
   current-head assessments from historical evidence and shows workflow, runtime
   and manifest identities. A coding success alone does not authorize readiness.

Custom Runtime is a separate legitimate profile model for code orchestration
without Ayni, but execution is explicitly unsupported in this release. Ayni itself
remains independently useful for code analysis and managed check environments.

Pages refresh canonical state through the BFF. Browser tests use a deterministic
provider fixture and do not prove GitHub permissions, certificate trust or cluster
execution. Live acceptance requires the private sandbox's supported policy/lock,
producer signing setup, private worker image and registry pull configuration; the
UI displays prerequisites rather than fabricating readiness when these are absent.
