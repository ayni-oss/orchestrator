// Accepted task authority is immutable; observedAt is the live read clock.
export function retainedAttemptEvidence(response) {
  const retained = structuredClone(response);
  if (retained.attempt?.timing) delete retained.attempt.timing.observedAt;
  return retained;
}
