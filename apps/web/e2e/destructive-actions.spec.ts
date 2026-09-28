import { expect, test } from "@playwright/test";

test("cancellation requires explicit confirmation and returns focus safely", async ({ page }) => {
  await page.goto("/tasks/00000000-0000-4000-8000-000000000043");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  const cancel = page.getByRole("button", { name: "Cancel task" });
  await cancel.click();
  const dialog = page.getByRole("dialog", { name: "Cancel this task?" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(cancel).toBeFocused();
  await expect(page.locator("body")).not.toHaveCSS("pointer-events", "none");

  await cancel.click();
  await page.getByRole("button", { name: "Confirm cancellation" }).click();
  await expect(page.getByText("Cancellation requested")).toBeVisible();
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
  await expect(page.getByRole("navigation", { name: "Primary" })).toBeVisible();
});
