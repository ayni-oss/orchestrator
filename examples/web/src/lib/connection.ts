/** Safe connection metadata used by the operator views. Never add credentials here. */
export interface ConnectionView {
  id: string;
  kind: string;
  managementSource: string;
  displayName: string;
  appId: string;
  imageNamespace: string;
  status: string;
  failureReason: string;
  enabled: boolean;
  resourceVersion: string;
  testAuthentication: string;
  testPush: string;
  testPull: string;
  testCleanup: string;
  testCleanupFailureReason?: string;
}

export function registryCredentialStatus(connection: Pick<ConnectionView, "enabled" | "status" | "failureReason">) {
  if (!connection.enabled) return "Disabled";
  if (connection.status === "ready") return "Available";
  if (connection.status === "pending") return "Checking";
  return connection.failureReason === "connection_secret_unavailable" ? "Unavailable" : "Not verified";
}

export function registryFailureMessage(connection: Pick<ConnectionView, "status" | "failureReason">) {
  if (connection.status === "failed" && connection.failureReason === "connection_secret_unavailable") {
    return "The control plane could not read one or both configured push/pull credential Secrets. It will recheck automatically. Earlier test results do not confirm current access.";
  }
  return connection.failureReason;
}

export function registryConfiguration(connections: Pick<ConnectionView, "kind" | "enabled" | "testPush" | "testPull" | "status" | "failureReason">[]) {
  const registries = connections.filter(connection => connection.kind === "ghcr" && connection.enabled);
  if (registries.some(connection => connection.status === "ready" && connection.testPush === "passed" && connection.testPull === "passed")) {
    return { status: "ready", description: "Container registry push and pull permissions are verified." };
  }
  if (registries.some(connection => connection.status === "failed" && connection.failureReason === "connection_secret_unavailable")) {
    return { status: "warning", description: "Container registry credential Secrets are unavailable. Previous test results are historical." };
  }
  if (registries.some(connection => connection.status === "pending")) {
    return { status: "warning", description: "Container registry validation is pending. Previous test results do not confirm current access." };
  }
  if (registries.some(connection => connection.status === "ready" && connection.testPull === "passed")) {
    return { status: "warning", description: "Container registry pull access is verified, but push access is not ready." };
  }
  return { status: "missing", description: registries.length ? "No container registry has verified pull access." : "No enabled container registry is configured." };
}

/** Preview only; the control plane validates and derives the publishing destination. */
export function registryImageRepository(owner: string, repository: string): string {
  if (!owner || !repository) return "";
  return `ghcr.io/${owner.toLowerCase()}/${repository.toLowerCase()}`;
}
