/** Safe certificate metadata for operator views. Never add signing seeds. */
export interface CertificateView {
  id:string;managementSource:string;displayName:string;keyId:string;publicKey:string;
  revision:string;enabled:boolean;isDefault:boolean;resourceVersion:string;status:string;
  failureReason:string;usageCount:string;secretName:string;sourceSecretName:string;
}
