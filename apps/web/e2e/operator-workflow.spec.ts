import { expect, test } from "@playwright/test";

const revision = "a".repeat(40);

test("operator prepares a repository and submits frozen work", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await expect(page.getByRole("heading", { name: "Development control plane" })).toBeVisible();
  await expect(page.locator("[data-bff-status]")).toHaveAttribute("data-bff-status", "connected");

  await page.getByRole("link", { name: "Repositories" }).click();
  await page.getByRole("link", { name: "Readiness and history" }).click();
  await page.getByLabel("Git commit revision").fill(revision);
  await page.getByRole("button", { name: "Prepare revision" }).click();
  await expect(page.getByText("Preparation accepted")).toBeVisible();

  await page.getByRole("link", { name: "Tasks" }).click();
  await page.getByLabel("Repository").selectOption("sandbox");
  await page.getByLabel("Provider model").selectOption("gpt-5");
  await page.getByLabel("Exact start revision").fill(revision);
  await page.getByLabel("Request").fill("Document the bounded retry behavior");
  await page.getByLabel("Acceptance criteria").fill("Retry count is explicit\nExisting behavior remains covered");
  await page.getByRole("button", { name: "Submit development task" }).click();
  await expect(page).toHaveURL(/\/tasks\/00000000-0000-4000-8000-000000000040/u);
  await expect(page.getByRole("heading", { name: "Document the bounded retry behavior" })).toBeVisible();
  await expect(page.getByText("Frozen scope", { exact: true })).toBeVisible();
  await expect(page.getByText("Provider usage", { exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Retained evidence" })).toBeVisible();
});

test("requested input creates a linked follow-up", async ({ page }) => {
  await page.goto("/tasks/00000000-0000-4000-8000-000000000039");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await expect(page.getByText("Which retry path should remain backward compatible?")).toBeVisible();
  await page.getByLabel("Response").fill("Keep the public retry call backward compatible.");
  await page.getByRole("button", { name: "Submit follow-up" }).click();
  await expect(page).toHaveURL(/00000000-0000-4000-8000-000000000042/u);
  await expect(page.getByText("Follow-up submitted")).toBeVisible();
  await expect(page.getByText("Keep the public retry call backward compatible.")).toBeVisible();
});
