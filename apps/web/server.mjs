import { randomUUID } from "node:crypto";
import { createServer } from "node:http";
import process from "node:process";

// This direct local server overwrites the protocol header below and retains the
// actual Host header, allowing adapter-node to reconstruct the request origin
// for its same-origin action protection without trusting an external proxy.
process.env.PROTOCOL_HEADER ??= "x-forwarded-proto";
process.env.HOST_HEADER ??= "host";
const { handler } = await import("./build/handler.js");

const host = process.env.HOST ?? "127.0.0.1";
const port = Number.parseInt(process.env.PORT ?? "3000", 10);
if (!Number.isInteger(port) || port < 1 || port > 65_535) {
  throw new Error("PORT must be a valid TCP port");
}

const signalStoreKey = Symbol.for("ayni.orchestrator.web.request-signals");
const signalHeader = "x-ayni-internal-request-signal";
const requestSignals = new Map();
globalThis[signalStoreKey] = requestSignals;

const server = createServer((request, response) => {
  // The supported runtime terminates no TLS and has no reverse proxy. Tell
  // adapter-node the actual scheme while retaining the request Host so its
  // same-origin form-action check works through a loopback port-forward.
  request.headers["x-forwarded-proto"] = "http";

  // adapter-node's Request signal covers an interrupted upload, but not a
  // browser disconnect after the request body has completed. Retain a private
  // response-lifetime signal so server loads, actions, and streams can cancel
  // native gRPC work when the response socket closes early.
  const internalId = randomUUID();
  const cancellation = new AbortController();
  request.headers[signalHeader] = internalId;
  requestSignals.set(internalId, cancellation.signal);
  const release = () => requestSignals.delete(internalId);
  response.once("finish", release);
  response.once("close", () => {
    if (!response.writableFinished) cancellation.abort();
    release();
  });

  try {
    handler(request, response);
  } catch (error) {
    release();
    throw error;
  }
});

server.listen(port, host, () => {
  console.log(`web server listening on ${host}:${port}`);
});

function shutdown() {
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(1), 10_000).unref();
}

process.once("SIGINT", shutdown);
process.once("SIGTERM", shutdown);
