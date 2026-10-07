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
