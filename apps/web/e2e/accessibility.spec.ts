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
 for(const path of ["/projects","/projects/project:1","/repositories","/repositories/repository:1","/profiles/profile:1","/source-control","/container-registries","/certificates"]){
  await page.goto(path);
  await expect(page.locator("html")).toHaveAttribute("data-hydrated","true");
  await expectAccessible(page);
 }
 await page.screenshot({path:"/tmp/orchestrator-profile-ui.png",fullPage:true});
});

test("project creation supports keyboard selection and accessible validation", async ({page})=>{
 await page.goto("/projects");
 await page.getByRole("button",{name:"Create project",exact:true}).click();
 await expectAccessible(page);
 await page.getByLabel("Project name",{exact:true}).fill("Keyboard project");
  await page.getByLabel("Goal",{exact:true}).fill("Improve project behavior");
  await page.getByLabel("Workspace",{exact:true}).click();
  await page.getByRole("option",{name:"Shared workspace",exact:true}).click();
 await page.getByRole("button",{name:"Repositories",exact:true}).click();
 await page.getByRole("textbox",{name:"Search repositories",exact:true}).fill("sandbox");
 await expectAccessible(page);
 await page.getByRole("checkbox",{name:"ayni-oss/sandbox",exact:true}).press("Space");

 await expect(page.getByRole("checkbox",{name:"ayni-oss/sandbox",exact:true})).toBeChecked();
 await page.keyboard.press("Escape");
 await expect(page.getByRole("textbox",{name:"Search repositories",exact:true})).toBeHidden();
 await page.keyboard.press("Escape");
 await expect(page.getByRole("dialog")).toBeHidden();
 await expect(page.getByRole("button",{name:"Create project",exact:true})).toBeFocused();
});

test("GHCR connection onboarding dialog has accessible labels and keyboard dismissal", async ({page,request}) => {
  await request.post("http://127.0.0.1:50062/reset-connections");
  await page.goto("/container-registries/github");
  await page.getByRole("button", {name:"Add connection",exact:true}).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expectAccessible(page);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
});


test("certificate import dialog supports accessible labels and keyboard dismissal",async({page,request})=>{
  await request.post("http://127.0.0.1:50062/reset-certificates");await page.goto("/certificates");await page.getByRole("button",{name:"Add certificate",exact:true}).click();
  await page.getByLabel("Import",{exact:true}).check();await expectAccessible(page);await page.getByLabel("Signing seed",{exact:true}).fill("a".repeat(64));await page.keyboard.press("Escape");await expect(page.getByRole("dialog")).toBeHidden();
});
