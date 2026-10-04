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
 const guidance:Record<string,string>={setup_required:"Open the setup PR, review its workflows, and merge it on GitHub.",setup_pr_open:"Review and merge the setup PR. The environment build starts after merge.",awaiting_environment:"Checking the expected image. Existing images are verified automatically; a missing image needs a successful environment build.",ready:"The signed environment passed verification. Select this profile on a new request.",updating:"A new repository revision is being verified. Existing requests keep their frozen image.",degraded:"Repair provider access, the image build or certificate trust as indicated below, then retry reconciliation.",failed:"Review the failure below, repair the repository or deployment configuration, then retry."};
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
 {#if profile.failure}<Feedback kind="warning" title={words(profile.failure.reason)} message={profile.failure.message} />{#if profile.failure.remediation}<p class="text-sm">{profile.failure.remediation}</p>{/if}{/if}
 <div class="flex flex-wrap gap-3">
 <form method="POST" action="?/reconcile" use:enhance><input type="hidden" name="idempotencyKey" value={data.keys.reconcile} /><Button type="submit" variant="outline">Retry reconciliation</Button></form>
 {#if !data.installations?.length}<form method="POST" action="?/install" use:enhance><input type="hidden" name="idempotencyKey" value={data.keys.install} /><Button type="submit">Set up runtime</Button></form>{/if}
 {#if profile.state==="ready"}<Button href={`/tasks?repository=${encodeURIComponent(profile.repositoryConnectionId)}&profile=${encodeURIComponent(profile.id)}#new`}>New request</Button>{/if}
 </div>
 <Card.Root><Card.Header><Card.Title>Repository automation</Card.Title></Card.Header><Card.Content class="space-y-3">
 {#each data.installations ?? [] as installation (installation.id)}<div class="flex flex-wrap items-center gap-3"><StatusBadge status={installation.state} tone={statusTone(installation.state)} /><span class="text-sm">{installation.version}</span>{#if installation.pullNumber}<a class="underline" href={`https://github.com/${data.repository.provider.owner}/${data.repository.provider.name}/pull/${installation.pullNumber}`} target="_blank" rel="noreferrer">Open setup PR #{installation.pullNumber}</a>{/if}{#if installation.reason}<p class="text-sm">{words(installation.reason)}</p>{/if}</div>{:else}<p class="text-sm">No setup PR has been requested.</p>{/each}
 <a class="text-sm underline" href={`https://github.com/${data.repository.provider.owner}/${data.repository.provider.name}/actions`} target="_blank" rel="noreferrer">View repository workflow runs</a>
 </Card.Content></Card.Root>
 <Card.Root><Card.Header><Card.Title>Current runtime identity</Card.Title><Card.Description>A signed image must pass verification before use. Unsigned local checks are development feedback. Tags discover images; the digest identifies the frozen image.</Card.Description></Card.Header><Card.Content><dl class="space-y-3 text-sm">
 {#if data.revision}<div><dt>Repository SHA</dt><dd><CodeValue value={data.revision.repositoryRevision} /></dd></div>{#if data.revision.managed}<div><dt>Full lock fingerprint</dt><dd><CodeValue value={data.revision.managed.fingerprint} /></dd></div><div><dt>Discovery image</dt><dd><CodeValue value={`${data.revision.managed.repository}:lock-${data.revision.managed.fingerprint}`} /></dd></div><div><dt>Ayni / workflow template</dt><dd>{data.revision.managed.ayniVersion} / {data.revision.managed.template}</dd></div>{/if}{/if}
 {#each profile.reconciliation as entry (entry.key)}<div><dt>{words(entry.key)}</dt><dd><CodeValue value={entry.value} /></dd></div>{/each}
 </dl></Card.Content></Card.Root>
 <section class="space-y-3"><h2 class="text-lg font-semibold">Environment verification</h2>
 {#each data.artifacts ?? [] as artifact (artifact.id)}<Card.Root><Card.Content class="space-y-3 pt-5"><StatusBadge status={artifact.status} tone={statusTone(artifact.status)} /><CodeValue value={`${artifact.ociRepository}@${artifact.resolvedDigest}`} /><p class="text-sm">{artifact.platform.operatingSystem}/{artifact.platform.architecture} · {artifact.verifiedAt ? `Verified ${formatDate(artifact.verifiedAt)}` : "Not verified"}</p>{#if artifact.failure}<p class="text-sm">{artifact.failure.message}</p>{/if}</Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No verified environment is available yet. The expected image is checked automatically when runtime execution is enabled.</p>{/each}
 </section>
 {/if}
</div>
