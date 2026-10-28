export type Repository = { id: string; connectionId: string; installationId: string; providerRepositoryId: string; fullName: string; defaultBranch: string; private: boolean; connectedAtUnixMs: string };
export type GithubInstallation = { installationId: string; accountLogin: string; suspended: boolean };
export type GithubRepository = { providerRepositoryId: string; fullName: string; defaultBranch: string; private: boolean };
export type ConnectRequest = { idempotencyKey: string; connectionId: string; expectedConnectionRevision: string; installationId: string; providerRepositoryId: string };
export type RepositoryOperation = { id: string; idempotencyKey: string; connectionId: string; connectionRevision: string; installationId: string; providerRepositoryId: string; state: string; safeReason?: string; repository?: Repository; acceptedAtUnixMs: string; updatedAtUnixMs: string };
export const repositoryReasons: Record<string, string> = {
 github_key_invalid: 'The saved App key is invalid. Update the source connection.',
 github_authentication_failed: 'GitHub rejected the App credential. Update the source connection.',
 github_access_denied: 'GitHub denied source access. Check the installation, suspension status, and Contents read permission.',
 github_rate_limited: 'GitHub is rate limiting requests. Wait, then retry the same operation.',
 github_unavailable: 'GitHub could not be reached. Retry the same operation when access is restored.',
 github_response_invalid: 'GitHub returned unsupported repository metadata. No repository was connected.',
 github_repository_unavailable: 'The installation or repository is unavailable to this App. Check its installation access.',
 github_source_changed: 'The source connection changed. Refresh and connect again using its current revision.',
 repository_already_connected: 'This repository is already connected through another source connection. Use its existing catalog entry.'
};
