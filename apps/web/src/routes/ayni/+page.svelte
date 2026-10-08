<script lang="ts">
  import { enhance } from "$app/forms";
  import { invalidateAll } from "$app/navigation";
  import { onMount } from "svelte";
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  let { data, form }: PageProps = $props();
  onMount(() => { const timer = setInterval(() => { if (["testing", "installing"].includes(data.settings?.status ?? "")) void invalidateAll(); }, 5000); return () => clearInterval(timer); });
  const inputClass = "mt-2 w-full rounded-md border bg-background px-3 py-2 text-sm";
</script>
<svelte:head><title>Ayni · Ayni Orchestrator</title></svelte:head>
<PageHeader title="Ayni" description="Install Ayni builders for repository runtime images." />
{#if data.error || form?.error}<p role="alert" class="rounded-md border border-destructive p-4">{(form?.error ?? data.error)?.message}</p>{/if}
{#if form?.success}<p role="status" class="rounded-md border p-4">{form.success}</p>{/if}
{#if data.settings}
  {@const settings = data.settings}
  <Card.Root><Card.Header><Card.Title>Ayni installation</Card.Title><Card.Description>Repositories keep their selected registry destination and signing certificate.</Card.Description></Card.Header><Card.Content class="space-y-4">
    <dl class="grid gap-4 sm:grid-cols-3"><div><dt class="text-sm text-muted-foreground">Status</dt><dd>{settings.status.replaceAll("_", " ")}</dd></div><div><dt class="text-sm text-muted-foreground">Supported Ayni version</dt><dd>{data.releases?.supportedVersion ?? settings.ayniVersion}</dd></div><div><dt class="text-sm text-muted-foreground">Tested configuration</dt><dd>{settings.testedRevision === "0" ? "Not tested" : `Revision ${settings.testedRevision}`}</dd></div></dl>
    {#if settings.securityMode === "local-development"}<p role="note" class="rounded-md border border-amber-500 bg-amber-500/10 p-3 text-sm">Development security mode: Docker runs in a privileged pod. A successful local test does not validate production isolation.</p>{/if}
    {#if settings.failureReason}<p role="alert" class="text-sm text-destructive">{settings.failureReason.replaceAll("_", " ")}</p>{/if}
    {#if settings.installation}
      <dl class="space-y-2 text-sm"><div><dt class="text-muted-foreground">Installation stage</dt><dd>{settings.installation.stage.replaceAll("-", " ")}</dd></div><div><dt class="text-muted-foreground">Builder tag</dt><dd class="break-all">{settings.installation.imageTag}</dd></div></dl>
    {/if}
    <div class="space-y-2 text-sm"><p class="text-muted-foreground">Installed builder image</p><p class="break-all">{settings.builderImage || "Not installed"}</p></div>
    {#if settings.status === "installing"}<p role="status" class="text-sm">Building and validating Ayni. New repository runtime builds are paused.</p>{/if}
    <form method="POST" action="?/install" use:enhance><input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-install`} /><input type="hidden" name="resourceVersion" value={settings.resourceVersion} /><Button type="submit" disabled={settings.resourceVersion === "0" || ["testing", "installing"].includes(settings.status)}>{settings.status === "installing" ? "Installing Ayni…" : settings.installation?.status === "ready" ? "Rebuild builder" : "Install Ayni"}</Button></form>
    {#if settings.status === "installing"}<form method="POST" action="?/cancelInstall" use:enhance><input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-cancel-install`} /><input type="hidden" name="resourceVersion" value={settings.resourceVersion} /><Button type="submit" variant="outline">Cancel installation</Button></form>{/if}
    <form method="POST" action="?/test" use:enhance><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={settings.resourceVersion} /><Button type="submit" disabled={settings.resourceVersion === "0" || !settings.builderImage || ["testing", "installing"].includes(settings.status) || settings.installation?.status === "failed" || settings.installation?.status === "cancelled"}>{settings.status === "testing" ? "Testing build service…" : settings.status === "failed" ? "Retry service test" : "Test build service"}</Button></form>
    {#if settings.status === "testing"}<form method="POST" action="?/cancel" use:enhance><input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-cancel`} /><input type="hidden" name="resourceVersion" value={settings.resourceVersion} /><Button type="submit" variant="outline">Cancel service test</Button></form>{/if}
  </Card.Content></Card.Root>
  <Card.Root><Card.Header><Card.Title>Configuration</Card.Title><Card.Description>Save the desired version and architecture, then install Ayni. Saving does not start a build.</Card.Description></Card.Header><Card.Content>
    <form method="POST" action="?/save" use:enhance class="space-y-5">
      <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={settings.resourceVersion} />
      <fieldset disabled={settings.managementSource === "administrator" || ["testing", "installing"].includes(settings.status)} class="space-y-5 disabled:opacity-70">
        <div class="grid gap-5 sm:grid-cols-2"><label class="text-sm">Ayni version<select name="ayniVersion" class={inputClass} value={settings.ayniVersion}><option value={settings.ayniVersion}>{settings.ayniVersion} · supported</option>{#each data.releases?.releases ?? [] as release (release.tag)}{#if release.version !== settings.ayniVersion}<option value={release.version} disabled={!release.supported}>{release.version}{release.supported ? " · supported" : " · unsupported"}</option>{/if}{/each}</select></label>
        <label class="text-sm">Architecture<select name="platform" class={inputClass} value={settings.platform}><option value="linux/arm64">Linux ARM64</option><option value="linux/amd64">Linux AMD64</option></select></label></div>
        <label class="block text-sm">Docker engine image<input name="engineImage" class={inputClass} value={settings.engineImage} maxlength="512" required placeholder="registry/image@sha256:…" /></label>
        <label class="block text-sm">Dependency substrate<input name="provisioningBase" class={inputClass} value={settings.provisioningBase} maxlength="512" required placeholder="ghcr.io/ayni-oss/ayni-provisioning@sha256:…" /></label>
        <p class="text-sm">Deployed engine mode: <strong>{settings.securityMode}</strong>. Managed by the deployment administrator.</p>
        <div class="grid gap-5 sm:grid-cols-2"><label class="text-sm">Registry for builders and service tests<select name="testRegistryId" class={inputClass} value={settings.testRegistryId} required><option value="">Select a validated registry</option>{#each data.registries ?? [] as registry (registry.id)}<option value={registry.id}>{registry.displayName}</option>{/each}</select></label>
        <label class="text-sm">Signing certificate for service tests<select name="testCertificateId" class={inputClass} value={settings.testCertificateId} required><option value="">Select a ready certificate</option>{#each data.certificates ?? [] as certificate (certificate.id)}<option value={certificate.id}>{certificate.displayName}</option>{/each}</select></label></div>
        <Button type="submit">Save configuration</Button>
      </fieldset>
      {#if settings.managementSource === "administrator"}<p class="text-sm text-muted-foreground">This configuration is managed by deployment settings.</p>{/if}
    </form>
  </Card.Content></Card.Root>
{/if}
<Card.Root><Card.Header><Card.Title>Ayni releases</Card.Title></Card.Header><Card.Content class="space-y-3">
  {#if data.releases?.discoveryStatus !== "ready"}<p role="status" class="text-sm text-muted-foreground">Release discovery is unavailable{data.releases?.cached ? "; showing cached releases" : ""}. <a href="/ayni" class="underline">Retry</a></p>{/if}
  {#each data.releases?.releases ?? [] as release (release.tag)}<div class="flex items-center justify-between gap-3 border-b py-2 text-sm"><a href={release.url} target="_blank" rel="noreferrer" class="underline">{release.version}</a><span class="text-muted-foreground">{release.supported ? "Supported" : "Unsupported"}</span></div>{/each}
  <div class="flex gap-4">{#if (data.page ?? 1) > 1}<a href={`/ayni?page=${(data.page ?? 1) - 1}`} class="text-sm underline">Previous releases</a>{/if}{#if data.releases?.nextPage}<a href={`/ayni?page=${data.releases.nextPage}`} class="text-sm underline">Older releases</a>{/if}</div>
</Card.Content></Card.Root>
