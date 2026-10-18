import { expect, test } from "@playwright/test";

test("task details and task history share bounded repair", async ({ page }) => {
  await page.clock.install();
  // Keep this browser's event streams quiet so only the repair policy triggers
  // task reads. The real task stream and snapshot relay have separate tests.
  await page.route("**/_old/updates/all", route => route.fulfill({
    contentType: "text/event-stream", body: "event: heartbeat\ndata: {}\n\n",
  }));
  await page.route("**/_old/tasks/*/updates", route => route.fulfill({
    contentType: "text/event-stream", body: "event: heartbeat\ndata: {}\n\n",
  }));
  let reads = 0;
  page.on("request", request => {
    if (new URL(request.url()).pathname.match(/^\/tasks(?:\/[^/]+)?\/__data\.json$/u)) reads += 1;
  });
  await page.goto("/_old/tasks/00000000-0000-4000-8000-000000000039");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  reads = 0;
  await page.clock.runFor(119_000);
  expect(reads).toBe(0);
  await page.clock.runFor(1_000);
  await expect.poll(() => reads).toBeGreaterThan(0);

  await page.goto("/_old/tasks");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  reads = 0;
  await page.clock.runFor(119_000);
  expect(reads).toBe(0);
  await page.clock.runFor(1_000);
  await expect.poll(() => reads).toBeGreaterThan(0);
});
