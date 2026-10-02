import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "*.spec.ts",
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: "pnpm exec tsx e2e/grpc-fixture.ts",
      url: "http://127.0.0.1:50062/healthz",
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command: `${process.env.PLAYWRIGHT_USE_EXISTING_BUILD === "1" ? "" : "pnpm build && "}HOST=127.0.0.1 PORT=4173 ORCHESTRATOR_GRPC_ADDRESS=127.0.0.1:50061 node server.mjs`,
      url: "http://127.0.0.1:4173/healthz",
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
});
