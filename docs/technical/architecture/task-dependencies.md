# Task dependencies and concurrent execution

An approved plan freezes each task's `depends_on` keys in the exact reviewed
version. Keys refer to tasks in that same approval. Validation rejects missing,
duplicate and self edges and cycles; forward references and diamond graphs are
valid. Dependency declarations grant no execution or publication authority.

Preparation requires an explicit upstream selection for every edge. Each selection
identifies one task, one accepted attempt and its retained published Git commit.
The application checks the full originating scope, approval, reviewed edge,
repository, completed coding, passed managed quality and accepted draft identity.
Missing selections produce retained blockers. Extra, cross-scope or mismatched
selections are rejected. A draft supplies a reviewable commit; it does not imply
that a user merged the work.

For multiple upstream commits, the operator selects an integrated source commit
in a trusted checkout. `dev/ctl prepare-task verify` checks that every selected
upstream commit is an ancestor of that source; it never performs a merge, fetches
or moves a branch. An explicit `source_commit` selects a local integrated commit;
otherwise verification resolves the approved source reference. Conflicts must be
resolved before verification. Starting the attempt checks ancestry again and
exports the exact source snapshot. The loopback API trusts the operator's Git
evidence, as it does for independent task preparation.

The complete upstream selection list remains in the immutable attempt inputs and
preparation receipt. New upstream outcomes do not rewrite it. Confirmed recovery
copies these frozen inputs into its linked attempt, preserving the selected
upstream commits and integrated source. Choosing different inputs requires new
reviewed work rather than an automatic rebase of an active or recovering task.

The supervisor admits up to four independent attempts across all replicas. Its
transaction lock serializes slot allocation; it does not serialize execution.
Each attempt has a distinct retained source archive, Pod capability, Pod UID and
working volume. A cancelling workload occupies its slot until stop confirmation.
Deadlines fence the old generation before cleanup. Fair reconciliation rotates
active attempts so one inaccessible Pod does not prevent the others from being
observed. Per-task pending uniqueness prevents simultaneous continuations of the
same task.

Four slots are the initial tested admission limit. This is not a production
throughput or provider latency claim. See [operational acceptance](../operations/operational-acceptance.md)
for saturation, isolation, cancellation and retained replay evidence.
