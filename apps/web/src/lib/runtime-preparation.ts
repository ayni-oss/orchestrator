export type RuntimePreparation = {
 id: string; idempotencyKey: string; repositoryId: string; inspectionId: string;
 registryConnectionId: string; signerId: string; platform: string; state: string; stage: string;
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
 | { method: 'CancelRuntimePreparation' | 'RetryRuntimePreparation'; request: { idempotencyKey: string; id: string; expectedGeneration: string } };
export const runtimeStages: Record<string, string> = {
 acquire_source: 'Acquire exact source', discover: 'Discover registry image', pull_existing: 'Pull existing image',
 build: 'Build unsigned runtime', freeze_candidate: 'Freeze independent candidate', certify: 'Certify protected content',
 finalize: 'Assemble certified image', publish: 'Publish immutable image', pull_final: 'Independently pull published image',
 verify_content: 'Verify certification and content', verify_compatibility: 'Check managed compatibility', cleanup: 'Confirm cleanup',
};
export const runtimeReasons: Record<string, string> = {
 invalid_input: 'The frozen preparation inputs are unsupported. Review the selected inspection and deployment profile.',
 source_access_denied: 'GitHub denied source acquisition. Check the installation and repository Contents read permission before retrying.',
 source_rate_limited: 'GitHub is rate limiting source acquisition. Wait, then retry within the retained allowance.',
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
 github_access_denied: 'GitHub denied source access. Check the installation and repository Contents read permission.',
 github_rate_limited: 'GitHub is rate limiting source access. Wait before retrying.',
 github_unavailable: 'GitHub could not be reached. Retry when source access is restored.',
};
