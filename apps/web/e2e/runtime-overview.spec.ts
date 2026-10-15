import { expect, test } from "@playwright/test";

test("certificate trust proposal and progress appear in Ayni onboarding", async ({page, request}) => {
  const href = "https://github.com/ayni-oss/sandbox/pull/17";
  try {
    await request.post(`http://127.0.0.1:50062/certificate-trust?state=open&href=${encodeURIComponent(href)}`);
    await page.goto("/repositories/repository:1");
    const onboarding = page.getByRole("region", {name:"Ayni onboarding",exact:true});
    await expect(onboarding).toContainText("Setup PR open");
    await expect(onboarding).toContainText("Review and merge");
    const proposal = onboarding.getByRole("link", {name:"Review certificate trust update"});
    await expect(proposal).toHaveAttribute("href", href);
    await expect(proposal).toHaveAttribute("target", "_blank");
    await expect(page.getByRole("region", {name:"Ayni managed",exact:true}).getByRole("link", {name:"Review certificate trust update"})).toHaveCount(0);
    for (const [state, description] of [
      ["propose", "Creating a pull request"],
      ["closed", "closed without merging"],
      ["merged", "The trust update was merged"],
      ["verified", "trusts the selected signing certificate"],
    ]) {
      await request.post(`http://127.0.0.1:50062/certificate-trust?state=${state}${["merged","verified"].includes(state!) ? `&href=${encodeURIComponent(href)}` : ""}`);
      await page.reload();
      await expect(onboarding).toContainText(description!);
      await expect(onboarding.getByRole("link", {name:"Review certificate trust update"})).toHaveCount(0);
      if (["merged","verified"].includes(state!)) {
        await page.goto("/profiles/profile:1");
        await expect(page.getByRole("link",{name:"Review certificate trust update"})).toHaveCount(0);
        await page.goto("/repositories/repository:1");
      }
    }
  } finally {
    await request.post("http://127.0.0.1:50062/certificate-trust");
  }
});

test("repository overview shows identity and readiness separately from runtime configuration",async({page,request})=>{
  await request.post("http://127.0.0.1:50062/admin-connection?kind=ghcr");
  await page.goto("/repositories/repository:1");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated","true");
  const header=page.locator("main header").filter({has:page.getByRole("heading",{name:"ayni-oss/sandbox"})});
  await expect(header.getByRole("heading",{name:"ayni-oss/sandbox"})).toBeVisible();
  const information = page.getByRole("region", {name:"Information",exact:true});
  await expect(information.getByRole("link",{name:"https://github.com/ayni-oss/sandbox"})).toHaveAttribute("href","https://github.com/ayni-oss/sandbox");
  await expect(information).toContainText("Connected");
  await expect(information).toContainText("main");
  await expect(information).not.toContainText("runtimes ready");
  await expect(information).not.toContainText("Ayni lock");
  await expect(page.getByText("Automatic runtime inspection",{exact:true})).toHaveCount(0);
  await expect(page.getByLabel("GHCR connection")).toHaveCount(0);
  const runtime=page.getByRole("region", {name:"Ayni managed", exact:true});
  await expect(runtime).toContainText("Ayni lock");
  await expect(runtime).toContainText("c".repeat(64));
  await expect(runtime.getByRole("button")).toHaveCount(1);
  await runtime.getByRole("button",{name:"Ayni managed actions",exact:true}).click();await page.getByRole("menuitem",{name:"Edit",exact:true}).click();
  const dialog=page.getByRole("dialog");
  await dialog.getByLabel("GHCR connection").click();
  await page.getByRole("option",{name:"Admin App",exact:true}).click();
  await expect(dialog.getByLabel("Image namespace")).toHaveCount(0);
  await expect(dialog.locator('input[name="imageNamespace"]')).toHaveCount(0);
  await expect(dialog.getByText("Image destination: ghcr.io/ayni-oss/sandbox",{exact:true})).toBeVisible();
  await request.post("http://127.0.0.1:50062/profile-version");
  await page.waitForTimeout(11000);
  await dialog.getByRole("button",{name:"Build / Retry",exact:true}).click();
  await expect(page.getByText("Runtime sync requested.",{exact:true})).toBeVisible();
  await dialog.getByRole("button",{name:"Save image destination",exact:true}).click();
  await expect(page.locator('[data-sonner-toast][data-type="error"]')).toContainText("Close and reopen");
  await page.keyboard.press("Escape");
  await runtime.getByRole("button",{name:"Ayni managed actions",exact:true}).click();await page.getByRole("menuitem",{name:"Edit",exact:true}).click();
  await dialog.getByLabel("GHCR connection").click();
  await page.getByRole("option",{name:"Admin App",exact:true}).click();
  await dialog.getByRole("button",{name:"Save image destination",exact:true}).click();
  await expect(page.getByText("Runtime image destination updated.",{exact:true})).toBeVisible();
  await expect(runtime).toContainText("ghcr.io/ayni-oss/sandbox");
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await dialog.getByRole("button",{name:"Build / Retry",exact:true}).click();
  await expect(page.getByText("Runtime sync requested.",{exact:true})).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  // Success toasts expire automatically; dedicated notification tests verify dismissal.
  await expect(page.getByText("Runtime sync requested.",{exact:true})).toHaveCount(0, {timeout:10_000});
});


test("runtime preparation shows progress, findings and guarded corrective actions", async({page,request}) => {
  await request.post("http://127.0.0.1:50062/admin-certificate");
  await request.post("http://127.0.0.1:50062/preparation?phase=building");
  await page.goto("/repositories/repository:1");
  await expect(page.getByRole("region", {name:"Ayni managed", exact:true})).toContainText("Preparation Building");
  for(const phase of ["publishing", "verifying", "ready"]) {
    await request.post(`http://127.0.0.1:50062/preparation?phase=${phase}`);
    await page.reload();
    await expect(page.getByRole("region", {name:"Ayni managed", exact:true})).toContainText(`Preparation ${phase[0]!.toUpperCase()}${phase.slice(1)}`);
  }
  await request.post("http://127.0.0.1:50062/preparation?phase=verifying&code=runtime_certificate_invalid");
  await page.reload();
  await expect(page.getByRole("region", {name:"Ayni managed", exact:true})).toContainText("The image certificate is invalid.");
  await expect(page.getByRole("region", {name:"Ayni managed", exact:true})).toContainText("Review trust and rebuild.");
  await page.getByRole("button",{name:"Ayni managed actions",exact:true}).click();await page.getByRole("menuitem",{name:"Edit",exact:true}).click();
  await page.getByRole("dialog").getByRole("button",{name:"Rebuild runtime",exact:true}).click();
  await expect(page.getByText("Runtime rebuild requested.", {exact:true})).toBeVisible();
  await request.post("http://127.0.0.1:50062/preparation?phase=setup_required&code=lock_missing&rebuild=false");
  await page.reload();
  await page.getByRole("button",{name:"Ayni managed actions",exact:true}).click();await page.getByRole("menuitem",{name:"Edit",exact:true}).click();
  await expect(page.getByRole("dialog").getByRole("button",{name:"Rebuild runtime",exact:true})).toBeDisabled();
  await page.getByRole("dialog").getByRole("button",{name:"Retry",exact:true}).click();
  await expect(page.getByText("Runtime sync requested.", {exact:true})).toBeVisible();
});


test("repository analysis retains onboarding independently of connection access", async ({ page }) => {
  await page.goto("/repositories/repository:1");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  const onboarding = page.getByRole("region", { name: "Ayni onboarding" });
  await expect(onboarding).toContainText("Awaiting trust");
  await expect(onboarding).toContainText("Merge the trust proposal before preparing the runtime.");
  await expect(page.getByRole("region", {name:"Information",exact:true})).toContainText("Connected");
  await onboarding.getByRole("button", { name: "Ayni onboarding actions" }).click();await page.getByRole("menuitem", { name: "Analyze repository" }).click();
  await expect(onboarding).toContainText("Analyzing");
  await page.reload();
  await expect(onboarding).toContainText("Analyzing");
});


test("exhausted runtime offers explicit retry without an automatic timestamp", async ({page, request}) => {
  try {
    await request.post("http://127.0.0.1:50062/preparation?phase=verifying&retry=operator_required");
    await page.goto("/profiles/profile:1");
    await expect(page.getByText("Manual retry required", {exact:true})).toBeVisible();
    await expect(page.getByText("runtime_verification_failed", {exact:true})).toBeVisible();
    await expect(page.getByText("Next retry", {exact:true})).toHaveCount(0);
    await page.getByRole("region", {name:"Current runtime identity",exact:true}).getByRole("button").click();
    await page.getByRole("menuitem", {name:"Retry",exact:true}).click();
    await expect(page.getByText("Reconciliation requested.", {exact:true})).toBeVisible();
  } finally {
    await request.post("http://127.0.0.1:50062/preparation");
  }
});


test("repository dialogs preview publishing ownership without exposing the authentication account", async ({page, request}) => {
  for (const [namespace, destination] of [
    ["ayni-oss", "ghcr.io/ayni-oss/sandbox"],
    ["ghcr.io/credential-owner", "ghcr.io/ayni-oss/sandbox"],
  ]) {
    await request.post(`http://127.0.0.1:50062/admin-connection?kind=ghcr&namespace=${namespace}`);
    const response = await page.goto("/repositories");
    expect(await response!.text()).not.toContain("registry-auth-account");
    await page.getByRole("button", {name:"Connect repository", exact:true}).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("Repository", {exact:true}).click();
    await page.getByRole("option", {name:"sandbox", exact:true}).click();
    await dialog.getByRole("checkbox", {name:"Use Ayni", exact:true}).check();
    await dialog.getByLabel("GHCR connection").click();
    await page.getByRole("option", {name:"Admin App", exact:true}).click();
    await expect(dialog.getByText(`Image destination: ${destination}`, {exact:true})).toBeVisible();
    await page.screenshot({path:`/tmp/orchestrator-registry-${namespace}.png`});
    await page.keyboard.press("Escape");
    await page.goto("/repositories/repository:1");
    await page.getByRole("button", {name:"Ayni managed actions", exact:true}).click();
    await page.getByRole("menuitem", {name:"Edit", exact:true}).click();
    await dialog.getByLabel("GHCR connection").click();
    await page.getByRole("option", {name:"Admin App", exact:true}).click();
    await expect(dialog.getByText(`Image destination: ${destination}`, {exact:true})).toBeVisible();
  }
});
