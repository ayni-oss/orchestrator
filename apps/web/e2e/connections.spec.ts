import { test, expect } from "@playwright/test";
test.beforeEach(async ({request})=>{await request.post("http://127.0.0.1:50062/reset-connections");});
test("GitHub import clears private key and supports rotation",async ({page})=>{
  await page.goto("/setup/github");
  await page.getByLabel("Connection name").fill("My App");
  await page.getByLabel("App ID",{exact:true}).fill("42");
  await page.getByLabel("Private key (PEM)").fill("test-private-value");
  await page.getByRole("button",{name:"Save and test"}).click();
  await expect(page.getByRole("heading",{name:"My App"})).toBeVisible();
  await expect(page.locator("body")).not.toContainText("test-private-value");
  await page.getByLabel("New private key").fill("replacement-private-value");
  await page.getByRole("button",{name:"Replace credentials"}).click();
  await expect(page.getByLabel("New private key")).toHaveValue("");
});
test("admin connection is read only",async ({page,request})=>{
  await request.post("http://127.0.0.1:50062/admin-connection");await page.goto("/setup/github");
  await expect(page.getByText("Managed by your administrator.",{exact:false})).toBeVisible();
  await expect(page.getByRole("button",{name:"Replace credentials"})).toHaveCount(0);
  await expect(page.getByLabel("Private key (PEM)")).toHaveCount(0);
});
test("GHCR requires separate push and pull credentials",async ({page})=>{
  await page.goto("/container-registries/github");
  await page.getByLabel("Connection name").fill("Team images");await page.getByLabel("GitHub username").fill("team");await page.getByLabel("Image namespace").fill("team");
  await page.getByLabel("Push token",{exact:true}).fill("push-only-value");await page.getByLabel("Separate pull-only token",{exact:true}).fill("pull-only-value");
  await page.getByRole("button",{name:"Save and test"}).click();await expect(page.getByRole("heading",{name:"Team images"})).toBeVisible();
  await expect(page.locator("body")).not.toContainText("push-only-value");await expect(page.locator("body")).not.toContainText("pull-only-value");
});
