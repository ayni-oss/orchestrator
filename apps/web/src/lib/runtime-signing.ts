export type RuntimeSigner = {id:string;displayName:string;keyId:string;publicKey:string;secretName:string;secretUid:string;activeRevision:string;revision:string;enabled:boolean};
export type ProvisionMode = 'generate'|'import';
export type ProvisionIntent = {idempotencyKey:string;displayName:string;keyId:string;mode:ProvisionMode};
export type ProvisionOperation = ProvisionIntent & {id:string;state:string;safeReason?:string;publicKey?:string;secretName?:string;secretUid?:string;signerId?:string};
export type ProvisionResult = {operation:ProvisionOperation;signer?:RuntimeSigner};
export function signingSeed(value:string):string {
 if(!/^[0-9a-f]{64}\n?$/.test(value))throw new Error('Enter exactly 64 lowercase hexadecimal characters, with at most one final newline.');
 return value.slice(0,64);
}
export function validateSignerFields(displayName:string,keyId:string):void {
 if(!displayName.trim()||new TextEncoder().encode(displayName).length>120)throw new Error('Enter a shorter name with at least one non-space character.');
 if(!/^[A-Za-z0-9_.-]{1,128}$/.test(keyId)||keyId==='unsigned')throw new Error('Choose a Key ID using letters, numbers, dots, underscores or hyphens. The Key ID unsigned is reserved.');
}
export function provisionIntent(value:unknown):ProvisionIntent|null {
 if(!value||typeof value!=='object')return null;
 const input=value as Record<string,unknown>;
 if(typeof input.idempotencyKey!=='string'||!/^[0-9a-f-]{36}$/.test(input.idempotencyKey)||typeof input.displayName!=='string'||input.displayName.length>120||typeof input.keyId!=='string'||input.keyId.length>128||!['generate','import'].includes(String(input.mode)))return null;
 // Retain only public intent, including when reading existing browser storage.
 return {idempotencyKey:input.idempotencyKey,displayName:input.displayName,keyId:input.keyId,mode:input.mode as ProvisionMode};
}
export const provisioningReasons:Record<string,string>={
 signer_provisioning_unavailable:'Signer setup could not reach the signing service. Recover this setup when the service is available.',
 signer_provisioning_access_denied:'The deployment cannot create or read the signing Secret. Ask the administrator to check its permissions, then recover this setup.',
 signer_provisioning_conflict:'The retained signing identity or Secret conflicts with this request. Recover the original setup and ask the administrator to review it.',
 signer_provisioning_inputs_required:'Re-enter the same signing seed to resume this setup. A different seed cannot replace its retained identity.',
 signer_provisioning_disabled:'Signer setup is disabled in this deployment. Ask the administrator to enable it, then recover this setup.',
};
export function trustSnippet(signer:Pick<RuntimeSigner,'keyId'|'publicKey'>):string {
 return `[environment.certificate.trusted_keys]\n${JSON.stringify(signer.keyId)} = ${JSON.stringify(signer.publicKey)}`;
}
