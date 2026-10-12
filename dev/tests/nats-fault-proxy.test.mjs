import test from 'node:test';
import assert from 'node:assert/strict';
import { Frames, dropped } from './nats-fault-proxy.mjs';

test('NATS frames preserve bytes across every segmentation boundary and coalescing', () => {
  const body = '{"value":"PUB text"}\r\nPING\r\n';
  const input = Buffer.from(`INFO {}\r\nMSG inbox 1 ${Buffer.byteLength(body)}\r\n${body}\r\nPING\r\n`);
  for (let split = 0; split <= input.length; split++) {
    const frames = new Frames(), received = [];
    frames.push(input.subarray(0, split), ({ frame }) => received.push(frame));
    frames.push(input.subarray(split), ({ frame }) => received.push(frame));
    assert.equal(received.length, 3);
    assert.deepEqual(Buffer.concat(received), input);
  }
});
test('only the selected ACK direction is dropped; payload and protocol replies pass', () => {
  const ack = Buffer.from('{"stream":"ORCHESTRATOR_LIFECYCLE","seq":2}');
  assert.equal(dropped('server', ['MSG'], ack, 'publish-ack'), true);
  assert.equal(dropped('server', ['MSG'], ack, 'consumer-ack'), false);
  assert.equal(dropped('client', ['PUB', '$JS.ACK.stream.consumer.1.2.2.123.0'], Buffer.from('+ACK'), 'consumer-ack'), true);
  assert.equal(dropped('client', ['PUB', '$JS.ACK._.account.stream.consumer.1.2.7.123.0.token'], Buffer.from('+ACK'), 'consumer-ack'), true);
  assert.equal(dropped('client', ['PUB', '$JS.ACK._.account.stream.consumer.1.1.2.123.0.token'], Buffer.from('+ACK'), 'consumer-ack'), false);
  assert.equal(dropped('client', ['PUB', 'orchestrator.runtime.verification.accepted.v1'], ack, 'consumer-ack'), false);
  assert.equal(dropped('server', ['MSG'], Buffer.from('accepted event'), 'publish-ack'), false);
  assert.equal(dropped('server', ['PING'], Buffer.alloc(0), 'publish-ack'), false);
});
test('oversized and invalid frames fail within a bounded buffer', () => {
  assert.throws(() => new Frames().push(Buffer.alloc(1048577), () => {}));
  assert.throws(() => new Frames().push(Buffer.from('PUB subject -1\r\n'), () => {}));
});
test('header frames and one-byte TCP chunks keep protocol lengths intact', () => {
  const header = 'NATS/1.0\r\nNats-Msg-Id: stable\r\n\r\n';
  const body = 'event\r\nPUB inside payload';
  const size = Buffer.byteLength(header + body);
  const input = Buffer.from(`HPUB subject inbox ${Buffer.byteLength(header)} ${size}\r\n${header}${body}\r\nHMSG inbox 1 ${Buffer.byteLength(header)} ${size}\r\n${header}${body}\r\n`);
  const frames = new Frames(), received = [];
  for (const byte of input) frames.push(Buffer.from([byte]), ({ frame }) => received.push(frame));
  assert.equal(received.length, 2);
  assert.deepEqual(Buffer.concat(received), input);
});
