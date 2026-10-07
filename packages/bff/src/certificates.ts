import type { Certificate } from "@ayni-orchestrator/grpc-bindings/server";
import type { CertificateDto } from "./dto.js";

// Keep this allowlist independent of the wire type: credential fields must never reach page data.
export function mapCertificate(value: Certificate): CertificateDto {
  return { id: value.id, managementSource: value.managementSource, displayName: value.displayName,
    keyId: value.keyId, publicKey: value.publicKey, revision: value.revision, enabled: value.enabled,
    isDefault: value.isDefault, resourceVersion: value.resourceVersion, status: value.status,
    failureReason: value.failureReason, usageCount: value.usageCount,
    secretName: value.secretName, sourceSecretName: value.sourceSecretName };
}
