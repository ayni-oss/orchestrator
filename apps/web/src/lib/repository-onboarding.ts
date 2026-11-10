import { RpcError } from './api.js';
import { repositoryReasons, type ConnectRequest, type GithubRepository, type RepositoryOperation } from './repositories.js';

export type ConnectionAttempt = {
  request: ConnectRequest;
  repository?: GithubRepository;
  status: 'pending' | 'failed';
  message?: string;
};

const storageKey = 'ayni.repository.connect.pending';

export function readConnectionAttempts(): Record<string, ConnectionAttempt> {
  const saved = sessionStorage.getItem(storageKey);
  if (!saved) return {};
  const value = JSON.parse(saved);
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  // Previous versions retained the original command without display metadata.
  if (typeof value.providerRepositoryId === 'string') {
    return { [value.providerRepositoryId]: { request: value, status: 'pending' } };
  }
  return value;
}

export function writeConnectionAttempt(id: string, attempt: ConnectionAttempt | null) {
  const attempts = readConnectionAttempts();
  if (attempt) attempts[id] = attempt;
  else delete attempts[id];
  if (Object.keys(attempts).length) sessionStorage.setItem(storageKey, JSON.stringify(attempts));
  else sessionStorage.removeItem(storageKey);
  return attempts;
}

export function connectionAttemptResult(attempt: ConnectionAttempt, operation: RepositoryOperation): ConnectionAttempt | null {
  if (operation.state === 'completed' && operation.repository) return null;
  return {
    ...attempt,
    status: operation.state === 'accepted' ? 'pending' : 'failed',
    message: repositoryReasons[operation.safeReason ?? ''] ??
      (operation.state === 'accepted' ? 'Source access is awaiting completion.' : 'The repository could not be connected.'),
  };
}

export function connectionAttemptError(attempt: ConnectionAttempt, error: unknown): ConnectionAttempt {
  if (error instanceof RpcError && (error.status === 400 || error.status === 409)) {
    return {
      ...attempt,
      status: 'failed',
      message: error.status === 409
        ? 'The source or repository selection changed. Refresh the catalog before connecting again.'
        : 'The repository selection was rejected. Check the source connection and try again.',
    };
  }
  return {
    ...attempt,
    message: 'The connection result is unknown. Check the saved status before submitting again.',
  };
}
