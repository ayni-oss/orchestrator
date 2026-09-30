import { expect, test } from "@playwright/test";

const revision = "a".repeat(40);

test("operator inspects profile readiness and submits frozen work", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await expect(page.getByRole("heading", { name: "Development control plane" })).toBeVisible();
  await expect(page.locator("[data-bff-status]")).toHaveAttribute("data-bff-status", "connected");

  await page.getByRole("link", { name: "Repositories" }).click();
  await page.getByRole("link", { name: "Profiles and identity" }).click();
  await expect(page.getByRole("heading", { name: "Runtime profiles" })).toBeVisible();
  await expect(page.getByText("Ready", { exact: true })).toBeVisible();

  await page.getByRole("link", { name: "Tasks" }).click();
  await page.getByLabel("Repository connection").selectOption("repository:1");
  await page.getByLabel("Runtime profile").selectOption("profile:1");
  await page.getByLabel("Model").selectOption("gpt-5");
  await page.getByLabel("Exact repository revision").fill(revision);
  await page.getByLabel("Request").fill("Document the bounded retry behavior");
  await page.getByLabel("Acceptance criteria").fill("Retry count is explicit\nExisting behavior remains covered");
  await page.getByRole("button", { name: "Submit task" }).click();
  await expect(page).toHaveURL(/\/tasks\/00000000-0000-4000-8000-000000000040/u);
  await expect(page.getByRole("heading", { name: "Document the bounded retry behavior" })).toBeVisible();
  await expect(page.getByText("Frozen repository and runtime", { exact: true })).toBeVisible();
  await expect(page.getByText("Frozen request", { exact: true })).toBeVisible();
});
