import { readdir, readFile, stat } from "node:fs/promises";
import { resolve, relative, sep } from "node:path";

const root = resolve(import.meta.dirname, "..");
const forbidden = [
  "@grpc/grpc-js",
  "@ayni-orchestrator/grpc-bindings",
  "/ayni.orchestrator.v1.",
];

async function files(directory) {
  const result = [];
  for (const entry of await readdir(directory)) {
    const path = resolve(directory, entry);
    if ((await stat(path)).isDirectory()) result.push(...await files(path));
    else result.push(path);
  }
  return result;
}

const sourceRoot = resolve(root, "src");
for (const path of await files(sourceRoot)) {
  if (!/\.(?:svelte|ts)$/u.test(path)) continue;
  const name = relative(sourceRoot, path).split(sep).join("/");
  if (name.startsWith("tests/") || /\.(?:test|spec)\.ts$/u.test(name)) continue;
  const serverOnly = name.includes("/server/") || name.endsWith(".server.ts") || name.endsWith("/+server.ts");
  const contents = await readFile(path, "utf8");
  const runtimeImports = contents.replace(/import\s+type\s+(?:[^;]+);?/gu, "");
  if (!serverOnly && ["@ayni-orchestrator/bff", "@ayni-orchestrator/grpc-bindings"]
    .some((marker) => runtimeImports.includes(marker))) {
    throw new Error(`server-only transport imported by browser source: ${name}`);
  }
}

const clientRoot = resolve(root, "build/client");
for (const path of await files(clientRoot)) {
  if (!/\.(?:js|css|html)$/u.test(path)) continue;
  const contents = await readFile(path, "utf8");
  for (const marker of forbidden) {
    if (contents.includes(marker)) {
      throw new Error(`browser bundle contains server transport marker ${marker}: ${relative(clientRoot, path)}`);
    }
  }
}

console.log("browser bundle contains no native gRPC transport code");
