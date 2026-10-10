import tailwindcss from "@tailwindcss/vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [tailwindcss(), sveltekit()],
  // Compile Sonner from its package location so pnpm can resolve its nested
  // Svelte-only runed dependency rather than an externalized cached import.
  optimizeDeps: { exclude: ["svelte-sonner"] },
  ssr: {
    noExternal: [/^@ayni-orchestrator\/(?:bff|grpc-bindings)(?:\/|$)/u],
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
