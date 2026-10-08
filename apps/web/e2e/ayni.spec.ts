import { test, expect } from "@playwright/test";

test("Ayni displays the deployed boundary and supports service-test cancellation", async ({ page }) => {
  await page.goto("/ayni");
  await expect(page.getByRole("heading", { name: "Ayni", exact: true })).toBeVisible();
  await expect(page.getByText("dedicated-builder", { exact: true })).toBeVisible();
  await expect(page.locator('select[name="securityMode"]')).toHaveCount(0);
  await expect(page.locator('select[name="ayniVersion"] option[value="0.15.0"]')).toHaveAttribute("disabled", "");
  await page.getByRole("button", { name: "Test build service", exact: true }).click();
  await expect(page.getByRole("button", { name: "Cancel service test" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Save configuration" })).toBeDisabled();
  await page.getByRole("button", { name: "Cancel service test" }).click();
  await expect(page.getByText("cancelled", { exact: true })).toBeVisible();
});

test("Ayni installation builds explicitly and locks configuration until cancellation", async ({ page }) => {
  await page.goto("/ayni");
  await expect(page.locator('input[name="builderImage"]')).toHaveCount(0);
  await page.getByRole("button", {name:"Install Ayni",exact:true}).click();
  await expect(page.getByText("ghcr.io/ayni-oss/orchestrator:ayni-builder-0.14.5-arm64",{exact:true})).toBeVisible();
  await expect(page.getByRole("button",{name:"Save configuration"})).toBeDisabled();
  await expect(page.getByRole("button",{name:"Test build service",exact:true})).toBeDisabled();
  await page.getByRole("button",{name:"Cancel installation"}).click();
  await expect(page.getByRole("button",{name:"Install Ayni",exact:true})).toBeEnabled();
});
