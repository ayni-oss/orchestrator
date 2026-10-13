import type { LayoutServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { requestContext } from "$lib/server/page.js";
import { registryConfiguration } from "$lib/connection.js";

export const load: LayoutServerLoad = async ({ locals, depends }) => {
  depends("app:certificates", "app:connections");
  const context = requestContext(locals);
  const [certificates, connections] = await Promise.allSettled([
    getBff().listCertificates(context),
    getBff().listConnections(context),
  ]);
  const sourceControlConfiguration = (() => {
    if (connections.status === "rejected") {
      return { status: "warning", description: "Source control configuration status is unavailable." };
    }
    const providers = connections.value.filter(connection => connection.kind === "github_app" && connection.enabled);
    if (!providers.length) return { status: "missing", description: "No enabled source control connection is configured." };
    if (providers.some(provider => provider.status !== "ready" || provider.failureReason)) {
      return { status: "warning", description: "A source control connection needs attention. Check its permissions and connection test." };
    }
    return { status: "ready", description: "Source control is configured and ready." };
  })();
  return {
    sourceControlConfiguration,
    llmConfiguration: connections.status === "rejected" ? {status:"warning",description:"LLM provider status is unavailable."} : connections.value.some(c=>c.kind==="openai" && c.enabled && c.status==="ready") ? {status:"ready",description:"An LLM connection is ready."} : {status:"missing",description:"No enabled LLM connection is ready."},
    registryConfiguration: connections.status === "fulfilled" ? registryConfiguration(connections.value) : { status: "warning", description: "Container registry permission status is unavailable." },
    certificateConfiguration: certificates.status === "rejected"
      ? { status: "warning", description: "Certificate configuration status is unavailable." }
      : certificates.value.length
        ? { status: "ready", description: "A signing certificate is configured." }
        : { status: "missing", description: "No signing certificate is configured." },
  };
};
