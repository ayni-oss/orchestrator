import { expect, test } from "@playwright/test";

test("action results show one dismissible toast per request, including consecutive saves", async ({ page }) => {
  await page.goto("/_old/projects/project:1");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await page.getByRole("tab",{name:"Settings",exact:true}).click();
  await page.getByRole("button", { name: "Save settings", exact: true }).click();
  const toasts = page.locator("[data-sonner-toast]");
  await expect(toasts).toHaveCount(1);
  await expect(toasts).toContainText("Project updated.");
  await toasts.getByRole("button", { name: "Close toast" }).click();
  await expect(toasts).toHaveCount(0);
  await page.getByRole("tab",{name:"Settings",exact:true}).click();
  await page.getByRole("button", { name: "Save settings", exact: true }).click();
  await expect(toasts).toHaveCount(1);
  await expect(toasts).toContainText("Project updated.");
  await page.screenshot({ path: "/private/tmp/orchestrator-sonner-success.png" });
});

test("safe settings errors keep the form available and provide retry feedback", async ({ page }) => {
  await page.goto("/_old/projects/project:1");
  // Intercept only this isolated browser's POST; do not mutate the fixture or a real project.
  await page.route("**/_old/projects/*?*/update", async route => {
    if (route.request().method() !== "POST") return route.continue();
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ type: "failure", status: 409, data: '[{"error":1},{"message":2,"requestId":3},"Refresh and try again.","notification-test"]' }) });
  });
  await page.getByRole("tab",{name:"Settings",exact:true}).click();
  await page.getByRole("button", { name: "Save settings", exact: true }).click();
  const toast = page.locator("[data-sonner-toast]");
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText("Refresh and try again.");
  await expect(toast).toContainText("notification-test");
  await expect(page.getByRole("region", {name:"Project settings"})).toBeVisible();
  await expect(page.getByRole("region", {name:"Project settings"})).toContainText("Refresh and try again.");
  await page.screenshot({ path: "/private/tmp/orchestrator-sonner-error.png" });
});

test("creation redirects preserve the success toast after the dialog closes", async ({ page }) => {
  await page.goto("/_old/workspaces/workspace:1");
  await page.getByRole("button", { name: "Create project", exact: true }).click();
  await page.getByLabel("Project name", { exact: true }).fill("Notification project");
  await page.getByLabel("Goal",{exact:true}).fill("Improve project behavior");
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  await page.getByRole("checkbox", { name: "ayni-oss/sandbox", exact: true }).check();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Create project", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Notification project", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(1);
  await expect(page.locator("[data-sonner-toast]")).toContainText("Project created.");
});
