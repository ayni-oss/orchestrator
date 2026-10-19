import { expect, test } from "@playwright/test";

test("cancellation updates the task with retained worker delivery", async ({ page }) => {
  await page.goto("/tasks/00000000-0000-4000-8000-000000000043");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await page.getByRole("button", { name: "Task actions", exact: true }).click();
  await page.getByRole("menuitem", { name: "Cancel task", exact: true }).click();
  await expect(page).toHaveURL(/notice=cancelled/u);
  await expect(page.getByText("Cancelled", { exact: true }).first()).toBeVisible();
});

test("shell remains operable at a narrow viewport with reduced motion", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 720 });
  await page.emulateMedia({ reducedMotion: "reduce", colorScheme: "light" });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await page.keyboard.press("Tab");
  await expect(page.getByRole("link", { name: "Skip to content" })).toBeFocused();
  await page.getByRole("button", { name: "Toggle color theme" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/u);
  await page.getByRole("button",{name:"Toggle workspace navigation",exact:true}).first().click();
  await page.getByRole("button",{name:"Configuration",exact:true}).click();
  await expect(page.getByRole("link",{name:/^Source control/u})).toBeVisible();
  await page.getByRole("link",{name:/^Source control/u}).click();
  await page.keyboard.press("Escape");
  await page.getByRole("button",{name:"Toggle color theme",exact:true}).click();
});
