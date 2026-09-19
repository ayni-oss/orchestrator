# Multi-repository work and evidence

Status: exploratory. The orchestrator should support multi-repository projects;
the execution topology and project-level evidence format remain open.

## Project context and task scope

A project groups repositories, architecture knowledge, instructions, integrations,
and policy references. A task selects the repositories it needs. It does not
necessarily start every environment or grant write access to every project repo.

One agent can inspect and edit several repos through a workspace interface.
Additional agents are a task strategy, not a required one-agent-per-repo mapping.
Project context should preserve discoveries while recording their source and
freshness; learned instructions can become stale as repositories change.

## Delegate discovery to the agent

The user should not have to predeclare every dependency edge or test combination.
The agent can inspect manifests, service clients, tests, documentation, and prior
project knowledge to discover what a request affects.

| Responsibility | Proposed owner |
| --- | --- |
| Identify affected repositories and necessary services | Agent |
| Propose coordinated changes and useful integration checks | Agent |
| Decide whether delegation helps | Agent/runtime within task budgets |
| Grant repository, service, tool, and credential access | Orchestrator policy |
| Record actual source, environment, and execution inputs | Platform automatically |
| Evaluate configured repository measurements | Ayni |
| Explain gaps, assumptions, and product-level acceptance | Agent and reviewer |

An agent may propose the combination to test. The platform records the combination
actually executed. This should be an automatic consequence of tool execution,
not a manifest the agent must accurately transcribe by hand.

## Example: API and frontend change

Request: add a response field and display it in checkout.

1. The agent discovers the API and frontend repositories and reads their contracts.
2. The task pins starting revisions and prepares the needed environments.
3. The agent edits the API and its consumer, then uses focused Ayni verification.
4. If dependencies change, candidate environments are prepared from updated inputs.
5. The agent starts the candidate services with a known fixture and runs an
   integration check for the new behavior.
6. A final controlled run evaluates the repository contracts and required
   integration checks against frozen candidate inputs.
7. The result links the proposed changes to those execution records and describes
   compatibility and merge ordering.

The exact checks may be discovered by the agent. Required repository policy
remains the minimum acceptance standard; agent-chosen extra checks add evidence.

## What to capture automatically

The proposed task-level record binds together:

- Repository identities, starting revisions, candidate commits, and measured
  source fingerprints. A branch name alone is mutable and insufficient.
- Policy digests, environment lock fingerprints, executor/image identities,
  platform, and preparation identities.
- Service images, fixture/init revisions, and uncontrolled external dependencies.
- Executed commands, selected scope, exit status, timestamps, and artifact references.
- Agent profile identity and attempt identity for operational diagnosis.
- Complete per-repository Ayni artifacts plus separately identified integration results.

This is a conceptual record, not a selected schema. Avoid copying every artifact
into a new format when references and hashes can preserve its original contract.
Exclude credentials and apply retention/access policy to logs and source details.

## Repository completion and project completion

Existing Ayni distinguishes focused verification from repository completion.
Its current artifact records policy, source, environment, tool versions, and
target accounting. An unscoped completed check is the repository gate; focused
success alone does not establish it.

An orchestrator should add project-level integration accounting without claiming
that today's Ayni schema already provides it. Passing checks in each repository
does not prove that an API and its consumer work together.

Configured required checks must not disappear because an agent did not select
them. Treat missing or incomplete required evidence as an explicit incomplete
outcome. Distinguish infrastructure/setup failure from a code-quality failure and
from an unverified product claim.

Reference: [Ayni signal schema v4](https://github.com/ayni-oss/ayni/blob/main/docs/product/signals/v4.md).

## Evidence trust

Current Ayni artifacts are local mutable files, not signed independent
attestations. The agent can use them for rapid repair, but an acceptance service
must not trust a passing file solely because it exists in an editable workspace.

Freeze candidate inputs, run validation under the applicable approved policy,
and collect outputs through a controlled execution path. Record changed tests and
policy along with changed source; an agent lowering thresholds must not silently
change the acceptance standard. Exact approval rules remain an orchestrator decision.

Deterministic measurements are useful but do not prove the completeness of an
agent's test selection. State that limitation in the outcome rather than requiring
humans to specify every workflow in advance.

Reference: [Ayni evidence limitations](https://github.com/ayni-oss/ayni/blob/main/docs/product/security.md#persistent-state-and-evidence-limitations).

## Concurrent edits and integration

Independent tasks need separate checkouts or worktrees and isolated mutable
state. Agents collaborating within one task need explicit write ownership or
separate branches with a deliberate integration step. Shared project knowledge
does not imply unsynchronized shared writable source.

If a shared library changes, consumers must test the candidate artifact rather
than accidentally fetching the old published version. A task-local registry,
artifact handoff, or an explicit local dependency override are alternatives to
evaluate. Capture whichever mechanism supplies the actual bytes.

Separate repositories do not have an atomic merge operation. Agents can discover
and propose a compatible merge order, staged migration, or feature flag. The
platform should track prerequisites and per-repo outcomes. If candidate inputs
change, previously recorded evidence must not silently apply to the new combination.

## Questions to keep open

- How does an agent search across repos before all execution environments start?
- Which integration expectations should persist as project policy after discovery?
- When should service fixtures be immutable artifacts versus initialization scripts?
- Which runtime supports coherent cross-repo edits with the least custom tooling?
- How do large projects present relevant context without loading every repository?
