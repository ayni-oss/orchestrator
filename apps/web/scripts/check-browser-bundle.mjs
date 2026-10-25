import { readdir, readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const directory = new URL('../build/assets/', import.meta.url);
for (const name of await readdir(directory)) {
  if (!name.endsWith('.js')) continue;
  const source = await readFile(new URL(name, directory), 'utf8');
  for (const forbidden of ['@grpc/grpc-js', 'grpc-bindings/server', 'node:fs', 'node:net', 'postgres://', 'postgresql://']) assert.equal(source.includes(forbidden), false, `Server capability in browser build: ${forbidden}`);
}
console.log('Browser build contains no native RPC, database or Node filesystem/network imports.');
