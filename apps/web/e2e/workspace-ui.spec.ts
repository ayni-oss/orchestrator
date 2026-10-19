import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

test("workspace pages share navigation", async ({ page }) => {
  await page.goto("/operations");
  await expect(page.getByRole("navigation", { name: "Configuration navigation" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Operations", exact: true })).toBeVisible();
  await page.getByRole("link", { name: /Projects/ }).first().click();
  await expect(page.getByRole("heading", { name: "Projects", exact: true })).toBeVisible();
  await page.getByRole("link", { name: "Activity", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Activity", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Configuration", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Configuration navigation" })).toBeVisible();
});

test("project keeps chat, plans, tasks, and settings in the current UI", async ({ page }) => {
  await page.goto("/projects/project:1");
  await expect(page.getByRole("tab", { name: "Chat", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open conversation plan" })).toHaveCount(0);
  await page.getByRole("tab", { name: "Plans", exact: true }).click();
  await expect(page).toHaveURL(/view=plans/u);
  await page.getByRole("tab", { name: "Tasks", exact: true }).click();
  await expect(page).toHaveURL(/view=tasks/u);
  await page.getByRole("link", { name: "Project settings" }).click();
  await expect(page.getByRole("tab", { name: "Repositories", exact: true })).toBeVisible();
});

test("runtime details are nested under their repository", async ({ page }) => {
  await page.goto("/repositories/repository:1");
  await expect(page.getByRole("heading", { name: "ayni-oss/sandbox", exact: true })).toBeVisible();
  await page.goto("/repositories/repository:1/runtimes/profile:1");
  await expect(page.getByRole("heading", { level: 1, name: /runtime/ })).toBeVisible();
  await expect(page.getByRole("link", { name: /Back to ayni-oss\/sandbox/ })).toBeVisible();
});

test("primary workspace pages have no obvious accessibility violations", async ({ page }) => {
  for (const path of ["/", "/operations", "/activity", "/projects/project:1", "/workspaces/workspace:1/settings", "/repositories"]) {
    await page.goto(path);
    await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
    expect((await new AxeBuilder({ page }).include("main").analyze()).violations).toEqual([]);
  }
});
