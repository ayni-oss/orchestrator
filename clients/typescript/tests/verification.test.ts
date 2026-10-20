import assert from 'node:assert/strict';
import assert from 'node:assert/strict';
import { createHash, webcrypto } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { ConversationReportDecision, ConversationTurnReport } from '../src/generated/ayni/orchestrator/events/v1/workflow.ts';
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

const coding = JSON.parse(readFileSync(new URL('../../../proto/testdata/coding/final.json', import.meta.url), 'utf8'));
const { CodingReportAvailable } = await import('../src/generated/ayni/orchestrator/events/v1/coding.ts');
const codingPayload = CodingReportAvailable.fromJSON(coding.data);
assert.deepEqual(CodingReportAvailable.toJSON(codingPayload), coding.data);
assert.deepEqual(CodingReportAvailable.decode(CodingReportAvailable.encode(codingPayload).finish()), codingPayload);

const reportDecision = ConversationReportDecision.fromJSON({
  reportSource: 'ayni://orchestrator/supervisors/018f0f7b-6a4d-7a11-8d77-010203040506',
  reportId: '018f0f7b-6a4d-7a11-8d77-111213141516',
  reportDigest: `sha256:${'ab'.repeat(32)}`,
  targetExecutorInstanceId: '018f0f7b-6a4d-7a11-8d77-010203040506',
  acceptedExecutorInstanceId: '018f0f7b-6a4d-7a11-8d77-010203040506',
  assignmentId: '018f0f7b-6a4d-7a11-8d77-616263646566',
  executorGeneration: '1',
  decision: 'CONVERSATION_REPORT_DECISION_KIND_ACCEPTED',
  safeReason: '',
  turnState: 'processing',
  workspaceId: 'workspace:018f0f7b-6a4d-7a11-8d77-212223242526',
  projectId: 'project:018f0f7b-6a4d-7a11-8d77-313233343536',
  conversationId: 'conversation:018f0f7b-6a4d-7a11-8d77-414243444546',
  turnId: 'turn:018f0f7b-6a4d-7a11-8d77-515253545556',
});
assert.deepEqual(
  ConversationReportDecision.decode(ConversationReportDecision.encode(reportDecision).finish()),
  reportDecision,
);

// Rust and TypeScript independently derive and verify the exact executor signing
// fixture so protobuf ordering or domain-separation changes cannot go unnoticed.
const signingFixture = JSON.parse(
  readFileSync(new URL('../../../proto/testdata/workflow/report-signing.json', import.meta.url), 'utf8'),
);
const signedReport = ConversationTurnReport.fromJSON(signingFixture.payload);
const unsignedReport = { ...signedReport, signature: new Uint8Array() };
const protobufBytes = ConversationTurnReport.encode(unsignedReport).finish();
const domain = Buffer.from('ayni-orchestrator-conversation-report-signature-v1');
const lengthPrefixed = (value: Uint8Array): Buffer => {
  const length = Buffer.alloc(8);
  length.writeBigUInt64BE(BigInt(value.byteLength));
  return Buffer.concat([length, value]);
};
const unsignedBinding = unsignedReport.binding;
assert.ok(unsignedBinding);
const unsigned64 = (value: string): Buffer => {
  const encoded = Buffer.alloc(8);
  encoded.writeBigUInt64BE(BigInt(value));
  return encoded;
};
const canonical = Buffer.concat([
  domain,
  lengthPrefixed(Buffer.from('build.ayni.orchestrator.conversation.turn.started.v1')),
  lengthPrefixed(Buffer.from(signingFixture.identity.source)),
  lengthPrefixed(Buffer.from(signingFixture.identity.id)),
  lengthPrefixed(Buffer.from(unsignedBinding.assignmentId)),
  unsigned64(unsignedBinding.executorGeneration),
  unsigned64(unsignedReport.reportSequence),
  lengthPrefixed(Buffer.from(unsignedBinding.commandEventId)),
  lengthPrefixed(protobufBytes),
]);
assert.equal(canonical.toString('hex'), signingFixture.canonicalHex);
const signingDigest = createHash('sha256').update(canonical).digest();
assert.equal(signingDigest.toString('hex'), signingFixture.digestHex);
const publicKey = await webcrypto.subtle.importKey(
  'raw',
  Buffer.from(signingFixture.publicKeyHex, 'hex'),
  { name: 'Ed25519' },
  false,
  ['verify'],
);
assert.equal(
  await webcrypto.subtle.verify({ name: 'Ed25519' }, publicKey, signedReport.signature, signingDigest),
  true,
);
