<script lang="ts">
 import { enhance } from "$app/forms";
 import type { PageProps } from "./$types";
 import PageHeader from "$lib/components/page-header.svelte";
 import Feedback from "$lib/components/feedback.svelte";
 import CodeValue from "$lib/components/code-value.svelte";
 import StatusBadge from "$lib/components/status-badge.svelte";
 import PollStatus from "$lib/components/poll-status.svelte";
 import { Button } from "$lib/components/ui/button/index.js";
 import * as Card from "$lib/components/ui/card/index.js";
 import { statusTone,words,formatDate } from "$lib/presentation.js";
 let {data,form}:PageProps=$props();
 const guidance:Record<string,string>={setup_required:"Inspecting the connected repository's default revision.",setup_pr_open:"Preparing the immutable runtime identity.",awaiting_environment:"Resolving and verifying the registry image for this exact lock fingerprint. A missing image is built by an Orchestrator-managed Kubernetes Job.",ready:"The signed runtime image matches the checked lock and is ready for use.",updating:"A new lock fingerprint is being resolved. Existing requests retain their frozen image.",degraded:"Repair provider access, registry access, build configuration or certificate trust, then retry reconciliation.",failed:"The committed Ayni configuration or runtime image could not be verified. Review the failure and retry after repair."};
</script>
<svelte:head><title>Runtime profile · Ayni Orchestrator</title></svelte:head>
<div class="space-y-6">
 {#if data.error}<Feedback kind="error" title="Profile unavailable" message={data.error.message} />{/if}
 {#if form?.error}<Feedback kind="error" title="Action failed" message={form.error.message} />{:else if form?.success}<Feedback kind="success" title="Request accepted" message={form.success} />{/if}
 {#if data.profile && data.repository}
 {@const profile=data.profile}
 <Button href={`/repositories/${encodeURIComponent(data.repository.id)}`} variant="ghost">Back to {data.repository.displayName}</Button>
 <PageHeader eyebrow="Runtime profile" title={words(profile.kind)} description={guidance[profile.state] ?? "Waiting for a status update."} />
 <div class="flex flex-wrap items-center gap-4"><StatusBadge status={profile.state} tone={statusTone(profile.state)} /><PollStatus /></div>
 {#if profile.support!=="operational"}<Feedback kind="warning" title="Unsupported runtime" message="Custom Runtime execution is not available in this release." />{/if}
 {#if profile.kind==="ayni_managed"}<p class="text-sm">Certificate: {#if profile.effectiveCertificate}{profile.effectiveCertificate.displayName} · {profile.effectiveCertificate.keyId} · revision {profile.effectiveCertificate.revision}{profile.certificateId ? "" : " (global default)"}{:else}<a href="/certificates" class="underline">Add a signing certificate</a>{/if}</p>{/if}
 <p class="text-sm" role="status">Preparation: {words(profile.preparationPhase)}</p>
 {#each profile.findings as finding (finding.code)}<Feedback kind="warning" title={finding.message} message={finding.remediation} />{/each}
 {#if profile.failure && !profile.findings.length}<Feedback kind="warning" title={words(profile.failure.reason)} message={profile.failure.message} />{#if profile.failure.remediation}<p class="text-sm">{profile.failure.remediation}</p>{/if}{/if}
 <div class="flex flex-wrap gap-3">
 <form method="POST" action="?/reconcile" use:enhance><input type="hidden" name="idempotencyKey" value={data.keys.reconcile} /><Button type="submit" variant="outline">{profile.failure ? "Retry analysis" : "Analyze / Sync"}</Button></form>
 <form method="POST" action="?/reconcile" use:enhance><input type="hidden" name="idempotencyKey" value={data.keys.reconcile} /><input type="hidden" name="forceRebuild" value="true" /><Button type="submit" variant="outline" disabled={!profile.rebuildAvailable}>Rebuild runtime</Button></form>
 {#if profile.state==="ready"}<Button href={`/tasks?repository=${encodeURIComponent(profile.repositoryConnectionId)}&profile=${encodeURIComponent(profile.id)}#new`}>New request</Button>{/if}
 </div>
 <Card.Root><Card.Header><Card.Title>Current runtime identity</Card.Title><Card.Description>A signed image must pass Orchestrator verification before use. The registry tag is lookup metadata; the digest is the frozen runtime identity.</Card.Description></Card.Header><Card.Content><dl class="space-y-3 text-sm">
 {#if data.revision}<div><dt>Source snapshot</dt><dd><CodeValue value={data.revision.repositoryRevision} /></dd></div>{#if data.revision.managed}<div><dt>Full lock fingerprint</dt><dd><CodeValue value={data.revision.managed.fingerprint} /></dd></div><div><dt>Registry repository</dt><dd><CodeValue value={profile.desiredImageRepository || data.revision.managed.repository} /></dd></div><div><dt>Ayni version</dt><dd>{data.revision.managed.ayniVersion}</dd></div>{/if}{/if}
 {#each profile.reconciliation as entry (entry.key)}<div><dt>{words(entry.key)}</dt><dd>{#if entry.key==="certificate-trust-pr"}<a href={entry.value} target="_blank" rel="noreferrer" class="underline">Review certificate trust update</a>{:else}<CodeValue value={entry.value} />{/if}</dd></div>{/each}
 </dl></Card.Content></Card.Root>
 <section class="space-y-3"><h2 class="text-lg font-semibold">Environment verification</h2>
 {#each data.artifacts ?? [] as artifact (artifact.id)}<Card.Root><Card.Content class="space-y-3 pt-5"><StatusBadge status={artifact.status} tone={statusTone(artifact.status)} /><CodeValue value={`${artifact.ociRepository}@${artifact.resolvedDigest}`} /><p class="text-sm">{artifact.platform.operatingSystem}/{artifact.platform.architecture} · {artifact.verifiedAt ? `Verified ${formatDate(artifact.verifiedAt)}` : "Not verified"}</p>{#if artifact.failure}<p class="text-sm">{artifact.failure.message}</p>{/if}</Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No verified environment is available yet. The expected image is checked automatically when runtime execution is enabled.</p>{/each}
 </section>
 {/if}
</div>
