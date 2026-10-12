import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { VerificationDecisionRecorded, VerificationResultAvailable } from '../src/generated/ayni/orchestrator/events/v1/verification.ts';

// Shared with Rust: generated ProtoJSON codecs agree on enum names, oneof shape,
// uint64 strings and message presence. This is not an ingress validator.
for (const name of ['report', 'failure', 'diagnostic', 'resource-ids']) {
  const event = JSON.parse(readFileSync(new URL(`../../../proto/testdata/verification/${name}.json`, import.meta.url), 'utf8'));
  const payload = VerificationResultAvailable.fromJSON(event.data);
  assert.deepEqual(VerificationResultAvailable.toJSON(payload), event.data);
  assert.deepEqual(VerificationResultAvailable.decode(VerificationResultAvailable.encode(payload).finish()), payload);
}

const accepted = JSON.parse(readFileSync(new URL('../../../proto/testdata/verification/accepted.json', import.meta.url), 'utf8'));
const decision = VerificationDecisionRecorded.fromJSON(accepted.data);
assert.deepEqual(VerificationDecisionRecorded.toJSON(decision), accepted.data);
assert.equal(decision.observationId, undefined);
assert.deepEqual(VerificationDecisionRecorded.decode(VerificationDecisionRecorded.encode(decision).finish()), decision);
