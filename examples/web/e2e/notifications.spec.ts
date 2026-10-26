import { expect, test } from "@playwright/test";

test("action results show one dismissible toast per request, including consecutive saves", async ({ page }) => {
  await page.goto("/projects/project:1/settings");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await page.getByRole("button", { name: "Save settings", exact: true }).click();
  const toasts = page.locator("[data-sonner-toast]");
  await expect(toasts).toHaveCount(1);
  await expect(toasts).toContainText("Project updated.");
  await toasts.getByRole("button", { name: "Close toast" }).click();
  await expect(toasts).toHaveCount(0);
  await page.getByRole("button", { name: "Save settings", exact: true }).click();
  await expect(toasts).toHaveCount(1);
  await expect(toasts).toContainText("Project updated.");
  await page.screenshot({ path: "/private/tmp/orchestrator-sonner-success.png" });
});

test("creation redirects preserve the success toast after the dialog closes", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "New project", exact: true }).click();
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
