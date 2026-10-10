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

export function registryConfiguration(connections: Pick<ConnectionView, "kind" | "enabled" | "testPush" | "testPull" | "status">[]) {
  const registries = connections.filter(connection => connection.kind === "ghcr" && connection.enabled);
  if (registries.some(connection => connection.status === "ready" && connection.testPush === "passed" && connection.testPull === "passed")) {
    return { status: "ready", description: "Container registry push and pull permissions are verified." };
  }
  if (registries.some(connection => connection.testPull === "passed")) {
    return { status: "warning", description: "Container registry pull access is verified, but push access is not ready." };
  }
  return { status: "missing", description: registries.length ? "No container registry has verified pull access." : "No enabled container registry is configured." };
}
