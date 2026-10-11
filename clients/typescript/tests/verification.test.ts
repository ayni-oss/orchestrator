import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { VerificationResultAvailable } from '../src/generated/ayni/orchestrator/events/v1/verification.ts';

// Shared with Rust: generated ProtoJSON codecs agree on enum names, oneof shape,
// uint64 strings and message presence. This is not an ingress validator.
for (const name of ['report', 'failure', 'diagnostic']) {
  const event = JSON.parse(readFileSync(new URL(`../../../proto/testdata/verification/${name}.json`, import.meta.url), 'utf8'));
  const payload = VerificationResultAvailable.fromJSON(event.data);
  assert.deepEqual(VerificationResultAvailable.toJSON(payload), event.data);
  assert.deepEqual(VerificationResultAvailable.decode(VerificationResultAvailable.encode(payload).finish()), payload);
}
