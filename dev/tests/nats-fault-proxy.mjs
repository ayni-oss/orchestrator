// Disposable plaintext NATS fault injector. Never used by application deployments.
import net from 'node:net';
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';

// TCP segmentation is arbitrary; payloads can contain CRLF and protocol-looking text.
export class Frames {
  buffer = Buffer.alloc(0);
  push(chunk, consume) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    if (this.buffer.length > 1048576) throw new Error('fixture frame budget exceeded');
    while (this.buffer.length) {
      const end = this.buffer.indexOf('\r\n');
      if (end < 0) return;
      const line = this.buffer.subarray(0, end).toString();
      const fields = line.split(/\s+/);
      const payload = ['PUB', 'HPUB', 'MSG', 'HMSG'].includes(fields[0]);
      const size = payload ? Number(fields.at(-1)) : 0;
      if (!Number.isSafeInteger(size) || size < 0 || size > 1048576) throw new Error('invalid fixture frame');
      const length = end + 2 + (payload ? size + 2 : 0);
      if (this.buffer.length < length) return;
      const frame = this.buffer.subarray(0, length);
      const body = this.buffer.subarray(end + 2, end + 2 + size);
      consume({ frame, fields, body });
      this.buffer = this.buffer.subarray(length);
    }
  }
}

export function dropped(direction, fields, body, mode) {
  if (direction === 'client' && mode === 'consumer-ack') {
    const ack = fields[1]?.startsWith('$JS.ACK.') ? fields[1].slice(8).split('.') : [];
    const sequence = ack.length >= 9 ? ack[5] : ack.length === 7 ? ack[3] : undefined;
    return ['PUB', 'HPUB'].includes(fields[0]) && sequence === '2';
  }
  if (direction === 'server' && mode === 'publish-ack' && fields[0] === 'MSG') {
    try {
      const ack = JSON.parse(body.toString());
      return ack.stream === 'ORCHESTRATOR_LIFECYCLE' && Number.isSafeInteger(ack.seq);
    } catch { return false; }
  }
  return false;
}

function serve() {
  const statistics = { publisher: 0, consumer: 0 };
  const sockets = new Set();
  fs.writeFileSync('/tmp/mode', 'pass');
  fs.writeFileSync('/tmp/dropped.json', JSON.stringify(statistics));
  const server = net.createServer((client) => {
    if (sockets.size >= 32) { client.destroy(); return; }
    const broker = net.connect(4222, 'jetstream');
    sockets.add(client); sockets.add(broker);
    for (const [source, target, direction] of [[client, broker, 'client'], [broker, client, 'server']]) {
      const frames = new Frames();
      source.on('data', (chunk) => {
        try {
          frames.push(chunk, ({ frame, fields, body }) => {
            if (dropped(direction, fields, body, fs.readFileSync('/tmp/mode', 'utf8'))) {
              statistics[direction === 'client' ? 'consumer' : 'publisher']++;
              fs.writeFileSync('/tmp/dropped.json', JSON.stringify(statistics));
            } else if (!target.write(frame)) source.pause();
          });
        } catch { client.destroy(); broker.destroy(); }
      });
      target.on('drain', () => source.resume());
      source.on('error', () => { client.destroy(); broker.destroy(); });
      source.on('close', () => { sockets.delete(source); target.destroy(); });
    }
  });
  server.listen(4222, '0.0.0.0');
  process.on('SIGTERM', () => { for (const socket of sockets) socket.destroy(); server.close(); });
}
if (process.argv[1] && import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href) serve();
