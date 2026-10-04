import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

async function expectAccessible(page: import("@playwright/test").Page): Promise<void> {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
    .analyze();
  expect(results.violations, results.violations.map((violation) => `${violation.id}: ${violation.help}`).join("\n")).toEqual([]);
}

test("overview and task workflows meet automated accessibility checks", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await expectAccessible(page);

  await page.setViewportSize({ width: 375, height: 720 });
  await page.getByRole("button", { name: "Toggle color theme" }).click();
  await page.goto("/tasks/00000000-0000-4000-8000-000000000039");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await expectAccessible(page);
});

test("repository and runtime onboarding meet accessibility checks",async({page})=>{
 for(const path of ["/projects","/projects/project:1","/repositories","/repositories/repository:1","/profiles/profile:1"]){
  await page.goto(path);
  await expect(page.locator("html")).toHaveAttribute("data-hydrated","true");
  await expectAccessible(page);
 }
 await page.screenshot({path:"/tmp/orchestrator-profile-ui.png",fullPage:true});
});
