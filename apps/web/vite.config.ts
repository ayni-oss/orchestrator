import tailwindcss from "@tailwindcss/vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [tailwindcss(), sveltekit()],
  ssr: {
    noExternal: [/^@ayni-orchestrator\/(?:bff|grpc-bindings)(?:\/|$)/u],
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
