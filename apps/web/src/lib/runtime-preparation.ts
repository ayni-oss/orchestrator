export type RuntimePreparation = {
 id: string; idempotencyKey: string; repositoryId: string; inspectionId: string;
 developmentOnly?: boolean; registryConnectionId: string; signerId: string; platform: string; state: string; stage: string;
 safeReason?: string; generation: string; allowBuild: boolean; allowReplace: boolean;
 maxBuilds: number; buildsStarted: number; deadlineUnixMs: string; frozenInputDigest: string;
 imageDigest?: string; candidateImage?: string; protectedContentRoot?: string; certificateDigest?: string;
 managedCheckDigest?: string; unsignedManifestDigest?: string; certificationLayerDigest?: string;
 expectedManifestDigest?: string; contentVerified?: boolean; managedCompatible?: boolean;
 cleanupComplete?: boolean; acceptedAtUnixMs: string; updatedAtUnixMs: string;
 sourceCommit?: string; sourceTree?: string; policyDigest?: string; lockDigest?: string; lockFingerprint?: string; registryDestination?: string;
};
export type RuntimeSigner = { id: string; displayName: string; keyId: string; publicKey: string; revision: string; enabled: boolean };
export type StartRuntimeRequest = {
 idempotencyKey: string; repositoryId: string; inspectionId: string; registryConnectionId: string;
 signerId: string; platform: string; maxSeconds: number; maxBuilds: number; allowBuild: boolean; allowReplace: boolean;
};
export type RuntimeCommand =
 | { method: 'StartRuntimePreparation'; request: StartRuntimeRequest }
 | { method: 'StartRepositoryRuntime'; request: RepositoryRuntimeSettings & {idempotencyKey: string} }
 | { method: 'CancelRuntimePreparation' | 'RetryRuntimePreparation'; request: { idempotencyKey: string; id: string; expectedGeneration: string } };
export const runtimeStages: Record<string, string> = {
 acquire_source: 'Acquire exact source', discover: 'Discover registry image', pull_existing: 'Pull existing image',
 build: 'Build unsigned runtime', freeze_candidate: 'Freeze independent candidate', certify: 'Certify protected content',
 finalize: 'Assemble certified image', publish: 'Publish immutable image', pull_final: 'Independently pull published image',
 verify_content: 'Verify certification and content', verify_compatibility: 'Check managed compatibility', cleanup: 'Confirm cleanup',
};
export const runtimeReasons: Record<string, string> = {
 invalid_input: 'The frozen preparation inputs are unsupported. Review the selected inspection and deployment profile.',
 source_access_denied: 'The source provider denied source acquisition. Check the installation and repository Contents read permission before retrying.',
 source_rate_limited: 'The source provider is rate limiting source acquisition. Wait, then retry within the retained allowance.',
 setup_failure: 'The isolated stage could not be configured. Ask the deployment administrator to check its required volumes, image and permissions before retrying.',
 source_unavailable: 'The exact source checkout could not be acquired. Check repository access before retrying.',
 source_identity_mismatch: 'The acquired source differs from the frozen commit or tree. Check the retained inspection before starting again.',
 source_path_unsupported: 'The checkout contains an unsupported path or file type. Commit supported repository files and inspect the new commit.',
 source_bounds_exceeded: 'The checkout exceeds supported source limits. Reduce the source snapshot before preparing it.',
 tool_unavailable: 'A required deployment tool is unavailable. Ask the administrator to restore the qualified worker image.',
 execution_failed: 'This stage’s tool failed. Review its stage and frozen inputs before retrying within the original allowance.',
 execution_deadline: 'The stage reached its deadline. Confirm cleanup before authorizing more time in a new preparation.',
 execution_output_exceeded: 'The stage exceeded its output limit. Reduce excessive tool output before preparing the source again.',
 artifact_invalid: 'An immutable artifact failed validation. Review the source and builder configuration before retrying.',
 artifact_conflict: 'Retained artifact bytes conflict with the expected identity. Preserve the evidence and start a new preparation after review.',
 artifact_bounds_exceeded: 'An artifact exceeds supported storage or content limits. Review the deployment allowance and source requirements.',
 platform_mismatch: 'The artifact or worker architecture differs from the selected platform. Select the qualified deployment platform.',
 certification_failed: 'Certification rejected the candidate. Check committed trust, the selected signer and independently frozen content evidence.',
 compatibility_failed: 'Managed environment compatibility failed. Review the lock and tooling requirements; content verification alone cannot make this runtime ready.',
 image_missing: 'No runtime image was found. Start a new preparation with an explicit bounded build allowance.',
 replacement_required: 'The existing image failed verification. Review the evidence and start a new preparation with explicit replacement authority.',
 build_allowance_exhausted: 'The build allowance is exhausted. Review retained evidence before authorizing a new preparation.',
 workload_interrupted: 'The workload stopped before retaining a result. Complete cleanup and retry within the original allowance.',
 selection_superseded: 'A newer source or configuration selection replaced this preparation. Its frozen history remains available.',
 cancelled: 'Preparation was cancelled. Its resources must finish cleanup.',
 stage_failed: 'This preparation stage failed. Review its evidence and configuration before retrying.',
 registry_access_denied: 'The registry denied this operation. Check the selected account’s permission for the configured package, then retry.',
 registry_authentication_failed: 'The registry rejected the selected account. Update the registry connection before starting a new preparation.',
 registry_rate_limited: 'The registry is rate limiting requests. Wait, then retry the retained preparation.',
 registry_unavailable: 'The registry could not be reached. Retry when access is restored.',
 registry_response_invalid: 'The registry returned an unsupported response. Check registry status before retrying.',
 registry_image_missing: 'No image was found. Start a new preparation with a bounded build allowance to produce one.',
 runtime_missing: 'No image was found. A build requires an explicit allowance.',
 runtime_invalid: 'The existing image failed verification. Review its evidence; replacement requires a new explicit allowance.',
 existing_image_invalid: 'The existing image failed verification. Review its evidence before authorizing replacement.',
 runtime_profile_unavailable: 'The qualified builder profile is unavailable. Ask the deployment administrator to restore it.',
 builder_unavailable: 'The isolated builder could not start. Check deployment prerequisites before retrying.',
 build_failed: 'The runtime build failed. Review the stage and frozen source before retrying within the remaining allowance.',
 publication_uncertain: 'Publication may have succeeded. Retry to reconcile the retained digest through registry readback.',
 cleanup_pending: 'Cleanup is incomplete. The supervisor will keep trying; refresh history to check progress. This runtime cannot become ready until cleanup is confirmed.',
 runtime_cancelled: 'Preparation was cancelled. Cleanup must finish before its resources can be released.',
 deadline_exceeded: 'The preparation reached its time limit. Review retained evidence and cleanup before starting a new allowance.',
 runtime_inputs_changed: 'The source, registry or signer configuration changed. Inspect the current source and start a new preparation.',
 github_access_denied: 'The source provider denied source access. Check the installation and repository Contents read permission.',
 github_rate_limited: 'The source provider is rate limiting source access. Wait before retrying.',
 github_unavailable: 'The source provider could not be reached. Retry when source access is restored.',
};

export function runtimeOverview(input: {
 loaded: boolean; error: string; pending: boolean; rejected?: boolean; inspectionPending?: boolean;
 inspectionLoading: boolean; inspectionError: string; inspectionReason?: string;
 inspection: import('./repositories').RepositoryInspection | null;
 operations: RuntimePreparation[]; registries: import('./api').Connection[]; signers: RuntimeSigner[];
 sourceReady: boolean; sourceRevision?: string; moreHistory?: boolean; operationId?: string;
}): { title: string; description: string; issues: string[]; nextAction: string; operation?: RuntimePreparation } {
 const candidate = input.inspection;
 const operation = input.operations.find(value => value.id === input.operationId) ||
  (candidate ? input.operations.find(value => value.inspectionId === candidate.id) || input.operations.find(value => ['pending','running','stopping','cancelling','cleanup_pending','publication_uncertain'].includes(value.state) || value.state !== 'ready' && !value.cleanupComplete) : input.operations[0]);
 if (input.inspectionPending) return {title:'Inspection result needs recovery',description:'The last check has an unconfirmed result. Older saved checks do not establish the result of this request.',issues:[],nextAction:'Recover the interrupted check using the controls above.'};
 if (input.rejected) return {title:'Preparation request rejected',description:input.error,issues:[],nextAction:'Review the rejected command below, dismiss it and correct the selection before trying again.'};
 if (input.pending) return {title:'Preparation response needs recovery',description:'The last command has an uncertain response. Its retained receipt determines whether work started.',issues:[],nextAction:'Recover the preparation receipt below before starting another command.'};
 if (input.error || input.inspectionError) return {title:'Runtime status unavailable',description:input.error || input.inspectionError,issues:[],nextAction:'Retry the default-branch recheck when access is restored.'};
 if (!input.loaded || input.inspectionLoading) return {title:'Checking runtime status',description:'Loading saved preparations, inspection results and configuration.',issues:[],nextAction:''};
 if (operation) {
  const description = `${operation.sourceCommit ? `Commit ${operation.sourceCommit.slice(0,12)} · ` : ''}${operation.platform} · ${runtimeStages[operation.stage] || operation.stage}`;
  const titles: Record<string,string> = {ready:'Runtime ready',pending:'Runtime preparation queued',running:'Runtime preparation in progress',stopping:'Runtime preparation stopping',cancelling:'Runtime preparation cancelling',cleanup_pending:'Runtime cleanup pending',publication_uncertain:'Publication needs reconciliation',blocked:'Runtime preparation blocked',failed:'Runtime preparation failed',unavailable:'Runtime preparation unavailable',cancelled:'Runtime preparation cancelled',superseded:'Runtime preparation superseded',stopped:'Runtime preparation stopped'};
  return {title:operation.developmentOnly && operation.state === 'ready' ? 'Development runtime ready' : titles[operation.state] || `Runtime preparation: ${operation.state.replaceAll('_',' ')}`,description,operation,
   issues:operation.safeReason ? [runtimeReasons[operation.safeReason] || 'Review the retained stage evidence before retrying.'] : [],
   nextAction:operation.state === 'ready' ? 'This saved preparation is ready for this exact inspection and platform.' : operation.state === 'publication_uncertain' ? 'Reconcile publication to confirm whether the saved image was published.' : ['pending','running','stopping','cancelling','cleanup_pending'].includes(operation.state) ? 'Status updates automatically while preparation and cleanup continue.' : 'Review the issue above, then retry this preparation when it is resolved.'};
 }
 if (input.moreHistory) return {title:'More runtime history available',description:'No preparation for this inspection appears in the loaded history. Earlier preparations have not been checked yet.',issues:[],nextAction:'Open preparation details and load more preparations to check earlier results.'};
 const issues: string[] = [];
 if (!input.sourceReady) issues.push('The source connection is unavailable, disabled or needs validation. Enable and validate it before inspecting a commit.');
 if (!candidate) issues.push('No saved Ayni inspection is available. Inspect an exact commit to check its policy, lock and trust requirements.');
 else {
  if (candidate.connectionRevision !== input.sourceRevision) issues.push('The source connection changed after this inspection. Inspect the commit again using the current connection.');
  if (candidate.state !== 'completed') issues.push(input.inspectionReason || 'The latest inspection did not complete. Recover or retry it before preparing this commit.');
  else if (!candidate.result) issues.push('The inspection has no retained inputs. Inspect the commit again.');
  else issues.push(...(candidate.result.findings || []).filter(value => !['adapter_discovery_pending','runtime_verification_pending'].includes(value.code)).map(value => value.action));
 }
 if (!input.registries.some(value => value.enabled && value.activeCredentialRevision)) issues.push('No enabled, validated runtime registry is available. Configure a registry connection.');
 const enabledSigners = input.signers.filter(value => value.enabled);
 if (!enabledSigners.length) issues.push('No enabled runtime signer is available. Configure a signer and commit its public trust entry.');
 else if (candidate?.result?.trustKeyIds?.length && !enabledSigners.some(value => candidate.result!.trustKeyIds!.includes(value.keyId))) issues.push(`The inspection trusts key names: ${candidate.result.trustKeyIds.join(', ')}. Enabled signing keys are named: ${enabledSigners.map(value => value.keyId).join(', ')}. Use the same key name and public key in the repository trust policy.`);
 return {title:issues.length ? 'Runtime setup needs attention' : 'Runtime not prepared',description:candidate ? `No preparation is saved for inspected commit ${candidate.commitSha.slice(0,12)}.` : 'No runtime preparation has been started for this repository.',issues:[...new Set(issues)],
 nextAction:issues.length ? 'Resolve the issue shown above, then recheck the default branch.' : 'Configure a runtime for the checked commit.'};
}

export type RepositoryRuntimeSettings = Omit<StartRuntimeRequest,'idempotencyKey'|'inspectionId'> & {expectedConnectionRevision: string; branch?: string; expectedCommitSha?: string; expectedRegistryRevision?: string; expectedSignerRevision?: string};
export type RepositoryRuntimeRun = {id:string; idempotencyKey:string; settings:RepositoryRuntimeSettings; state:string; safeReason?:string; commitSha?:string; inspectionId?:string; preparationId?:string};
export const onboardingReasons: Record<string,string> = {
 branch_head_changed: 'The branch advanced after review. Check the branch and review its new commit before starting again.',
 source_unavailable: 'The selected branch could not be read. Check the source control connection and repository access, then prepare again.',
 configuration_changed: 'The source, registry or signing configuration changed. Review the current settings before preparing again.',
 inspection_failed: 'The repository inspection could not complete. Check source access and prepare again.',
 inspection_findings: 'The repository setup needs changes. Resolve the inspection issues shown below, then prepare the latest commit.',
 runtime_profile_unavailable: 'Runtime building is not enabled on this deployment. Ask the deployment administrator to enable a qualified runtime build profile.',
 preparation_admission_failed: 'Preparation could not be admitted. Review signer trust, platform and allowance against the deployment profile, then prepare again.',
 step_unavailable: 'A preparation step could not complete. Review access and prepare again.',
};
export function runtimeRegistryImage(operation: RuntimePreparation | undefined, destination = '') {
 const target = operation?.registryDestination || destination;
 const match = /^ghcr\.io\/([a-z0-9][a-z0-9._-]*)\/([a-z0-9][a-z0-9._-]*)$/.exec(target);
 if (!match) return null;
 const digest = operation?.imageDigest;
 const confirmed = !!digest && /^sha256:[a-f0-9]{64}$/.test(digest) && (operation?.state === 'ready' || !!operation?.expectedManifestDigest && digest === operation.expectedManifestDigest && ['pull_final','verify_content','verify_compatibility','cleanup'].includes(operation.stage) && operation.state !== 'publication_uncertain');
 return {destination:target, url:`https://github.com/orgs/${match[1]}/packages/container/package/${match[2]}`, reference:confirmed ? `${target}@${digest}` : '', confirmed};
}


export function runtimeProgressKey(repositoryId: string) {
 return `ayni.repository.runtime.progress.${repositoryId}`;
}

export function readRuntimeProgress(repositoryId: string): {integrations: string; inspection: string; signingUpdate: string} | undefined {
 try {
  const saved = JSON.parse(localStorage.getItem(runtimeProgressKey(repositoryId)) ?? 'null');
  if (saved && typeof saved.integrations === 'string' && typeof saved.inspection === 'string' && typeof saved.signingUpdate === 'string') {
   return saved;
  }
 } catch {
  // Progress is optional when browser storage is unavailable.
 }
}

export type RuntimeIntegration = 'registry' | 'signer';

export function runtimeDefaultKey(kind: RuntimeIntegration) {
 return `ayni.runtime.default.${kind}`;
}
