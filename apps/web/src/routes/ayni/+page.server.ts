import { fail } from "@sveltejs/kit";
import { BffError } from "@ayni-orchestrator/bff/server";
import { getBff } from "$lib/server/bff.js";
import { actionKey, requestContext, safeError } from "$lib/server/page.js";
import type { Actions, PageServerLoad } from "./$types";
export const load: PageServerLoad = async ({ locals, url, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });
  try {
    const bff = getBff(), context = requestContext(locals);
    const page = Math.max(1, Math.min(100, Number(url.searchParams.get("page")) || 1));
    const [settings, releases, connections, certificates] = await Promise.all([
      bff.getBuildService(context), bff.listAyniReleases(page, context), bff.listConnections(context), bff.listCertificates(context),
    ]);
    return { settings, releases, page, registries: connections.filter(c => c.kind === "ghcr" && c.enabled && c.status === "ready"), certificates: certificates.filter(c => c.enabled && c.status === "ready"), idempotencyKey: actionKey() };
  } catch (error) { return { error: safeError(error, locals.requestId), idempotencyKey: actionKey() }; }
};
export const actions: Actions = {
  save: async ({ request, locals }) => {
    const form = await request.formData();
    try {
      const text = (key: string, max = 512) => {
        const value = form.get(key) ?? "";
        if (typeof value !== "string" || value.length > max) throw new BffError({ status: 400, reason: "invalid_input", message: "Build service input exceeds the supported bounds.", retryable: false });
        return value.trim();
      };
      await getBff().updateBuildService({ idempotencyKey: text("idempotencyKey", 128), expectedResourceVersion: text("resourceVersion", 20), settings: {
        resourceVersion: "0", ayniVersion: text("ayniVersion", 128), builderImage: "", engineImage: text("engineImage"), platform: text("platform", 32), securityMode: "", provisioningBase: text("provisioningBase"), testRegistryId: text("testRegistryId", 128), testCertificateId: text("testCertificateId", 128), status: "", failureReason: "", managementSource: "", testId: "", testedRevision: "0", engineHost: "", engineSecret: "", installation: undefined,
      } }, requestContext(locals));
      return { success: "Configuration saved. Install Ayni to build and activate its builder image." };
    } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { error: mapped }); }
  },
  install: async ({ request, locals }) => {
    const form = await request.formData();
    try {
      const key = form.get("idempotencyKey"), version = form.get("resourceVersion");
      if (typeof key !== "string" || key.length > 128 || typeof version !== "string" || !/^\d{1,20}$/u.test(version)) throw new Error("invalid input");
      await getBff().startBuilderInstallation({ idempotencyKey: key, expectedResourceVersion: version }, requestContext(locals));
      return { success: "Building the Ayni builder image. It will be published and tested before activation." };
    } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { error: mapped }); }
  },
  cancelInstall: async ({ request, locals }) => {
    const form = await request.formData();
    try {
      const key = form.get("idempotencyKey"), version = form.get("resourceVersion");
      if (typeof key !== "string" || key.length > 128 || typeof version !== "string" || !/^\d{1,20}$/u.test(version)) throw new Error("invalid input");
      await getBff().cancelBuilderInstallation({ idempotencyKey: key, expectedResourceVersion: version }, requestContext(locals));
      return { success: "Installation cancellation requested. Cleaning up build resources." };
    } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { error: mapped }); }
  },
  cancel: async ({ request, locals }) => {
    const form = await request.formData();
    try {
      const key = form.get("idempotencyKey"), version = form.get("resourceVersion");
      if (typeof key !== "string" || key.length > 128 || typeof version !== "string" || !/^\d{1,20}$/u.test(version)) throw new Error("invalid input");
      await getBff().cancelBuildServiceTest({ idempotencyKey: key, expectedResourceVersion: version }, requestContext(locals));
      return { success: "Cancellation requested. Cleaning up the service test." };
    } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { error: mapped }); }
  },
  test: async ({ request, locals }) => {
    const form = await request.formData();
    try {
      const key = form.get("idempotencyKey"), version = form.get("resourceVersion");
      if (typeof key !== "string" || key.length > 128 || typeof version !== "string" || !/^\d{1,20}$/u.test(version)) throw new Error("invalid input");
      await getBff().testBuildService({ idempotencyKey: key, expectedResourceVersion: version }, requestContext(locals));
      return { success: "Build service test started." };
    } catch (error) { const mapped = safeError(error, locals.requestId); return fail(mapped.status, { error: mapped }); }
  },
};
