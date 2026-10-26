const signalStoreKey = Symbol.for("ayni.orchestrator.web.request-signals");
const signalHeader = "x-ayni-internal-request-signal";

type SignalStore = Map<string, AbortSignal>;

function signalStore(): SignalStore | undefined {
  const value = (globalThis as Record<symbol, unknown>)[signalStoreKey];
  return value instanceof Map ? value as SignalStore : undefined;
}

export function browserRequestSignal(request: Request): AbortSignal {
  const internalId = request.headers.get(signalHeader);
  const responseSignal = internalId === null ? undefined : signalStore()?.get(internalId);
  return responseSignal === undefined
    ? request.signal
    : AbortSignal.any([request.signal, responseSignal]);
}
