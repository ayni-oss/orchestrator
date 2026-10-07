import { test, expect, type Page } from "@playwright/test";
test.beforeEach(async ({request})=>{await request.post("http://127.0.0.1:50062/reset-connections");});
test("GitHub import and rotation use dialogs and the canonical route",async ({page,request})=>{
  await page.goto("/setup/github");
  await expect(page).toHaveURL(/\/source-control\/github$/u);
  await expect(page.getByRole("button",{name:"Enroll a repository"})).toHaveCount(0);
  await expect(page.getByLabel("Private key (PEM)")).toHaveCount(0);
  await page.getByRole("button",{name:"Add GitHub App",exact:true}).click();
  await page.getByRole("button",{name:"Import existing App"}).click();
  await page.getByLabel("Connection name").fill("My App");
  await page.getByLabel("App ID",{exact:true}).fill("42");
  await page.getByLabel("Private key (PEM)").fill("test-private-value");
  await page.getByRole("button",{name:"Save and test"}).click();
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(page.getByRole("heading",{name:"My App"})).toBeVisible();
  await expect(page.locator("body")).not.toContainText("test-private-value");
  await expect(page.getByRole("button",{name:"Add GitHub App",exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:"Edit",exact:true}).click();
  await page.getByLabel("New private key").fill("replacement-private-value");
  await request.post("http://127.0.0.1:50062/reject-connection-mutation");
  await page.getByRole("button",{name:"Replace credentials"}).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toBeVisible();
  await expect(page.getByLabel("New private key")).toHaveValue("");
  await page.getByLabel("New private key").fill("accepted-private-value");
  await page.getByRole("button",{name:"Replace credentials"}).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toHaveCount(0);
  await expect(page.getByLabel("New private key")).toHaveValue("");
  await page.getByRole("button",{name:"Test connection / Retry"}).click();
  await page.getByRole("button",{name:"Disable",exact:true}).click();
  await expect(page.getByRole("dialog").getByText("Disabled",{exact:true})).toBeVisible();
  await page.getByRole("button",{name:"Enable",exact:true}).click();
  await expect(page.getByRole("dialog").getByText("Ready",{exact:true})).toBeVisible();
  await page.getByRole("button",{name:"Delete",exact:true}).click();
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(page.getByRole("heading",{name:"My App"})).toHaveCount(0);
});
test("admin GitHub App has a compact read-only details dialog",async ({page,request})=>{
  await request.post("http://127.0.0.1:50062/admin-connection");await page.goto("/source-control/github");
  await expect(page.getByText("Managed by your administrator",{exact:true})).toBeVisible();
  await page.getByRole("button",{name:"Details"}).click();
  await expect(page.getByRole("dialog").getByText("Managed by your administrator.",{exact:false})).toBeVisible();
  await expect(page.getByRole("button",{name:"Replace credentials"})).toHaveCount(0);
  await expect(page.getByLabel("Private key (PEM)")).toHaveCount(0);
});
test("GitHub setup dialog clears dismissed credentials and prepares a canonical manifest",async ({page})=>{
  await page.goto("/source-control/github");
  await page.getByRole("button",{name:"Add GitHub App",exact:true}).click();
  await page.getByLabel("GitHub organization").fill("example-org");
  const create = page.getByRole("button",{name:"Create App for example-org"});
  await expect(create).toBeVisible();
  const manifest=JSON.parse(await page.locator('input[name="manifest"]').first().inputValue());
  expect(manifest.redirect_url).toBe("http://127.0.0.1:4173/source-control/github/complete");
  expect(manifest.default_permissions).toEqual({contents:"write",metadata:"read",pull_requests:"write",workflows:"write"});
  await page.getByRole("button",{name:"Import existing App"}).click();
  await page.getByLabel("Private key (PEM)").fill("discarded-secret");
  await page.getByRole("button",{name:"Cancel",exact:true}).click();
  await page.getByRole("button",{name:"Add GitHub App",exact:true}).click();
  await page.getByRole("button",{name:"Import existing App"}).click();
  await expect(page.getByLabel("Private key (PEM)")).toHaveValue("");
});
test("canonical and legacy manifest callbacks enforce session state and reject replay",async ({page})=>{
  await page.goto("/source-control/github");
  const origin="http://127.0.0.1:4173";
  const foreign=await page.request.post("/source-control/github/start",{headers:{origin:"https://foreign.example"}});
  expect(foreign.status()).toBe(403);
  for(const prefix of ["/source-control/github","/setup/github"]){
    const missing=await page.request.get(`${prefix}/complete?code=fixture-code`);
    expect(missing.status()).toBe(400);
    const start=await page.request.post(`${prefix}/start`,{headers:{origin}});
    expect(start.status()).toBe(200);
    expect(start.headers()["set-cookie"]).toContain("Path=/;");
    const {state}=await start.json();
    const complete=await page.request.get(`${prefix}/complete?code=fixture-code&state=${state}`,{headers:{cookie:`github_app_manifest_state=${state}`}});
    // The fixture rejects provider codes; consumed states must still never be replayed.
    expect(complete.status()).toBe(502);
    const replay=await page.request.get(`${prefix}/complete?code=fixture-code&state=${state}`,{headers:{cookie:`github_app_manifest_state=${state}`}});
    expect(replay.status()).toBe(400);
  }
});
async function fillRegistry(page: Page, name = "Team images") {
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Connection name").fill(name);
  await dialog.getByLabel("GitHub username").fill("team");
  await dialog.getByLabel("Image namespace").fill("team");
  await dialog.getByLabel("Push token", {exact:true}).fill("push-only-value");
  await dialog.getByLabel("Separate pull-only token", {exact:true}).fill("pull-only-value");
}
async function createRegistry(page: Page) {
  await page.goto("/container-registries/github");
  await page.getByRole("button", {name:"Add connection",exact:true}).click();
  await fillRegistry(page);
  await page.getByRole("dialog").getByRole("button", {name:"Save and test"}).click();
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(page.getByRole("region", {name:"Team images"}).getByText("Ready", {exact:true})).toBeVisible();
}

test("GHCR creation lives in a dialog and clears credentials on close and rejected submission", async ({page,request})=>{
  await page.goto("/container-registries/github");
  await expect(page.getByLabel("Push token", {exact:true})).toHaveCount(0);
  await page.getByRole("button", {name:"Add connection",exact:true}).click();
  await fillRegistry(page);
  await page.getByRole("dialog").getByRole("button", {name:"Cancel",exact:true}).click();
  await page.getByRole("button", {name:"Add connection",exact:true}).click();
  await expect(page.getByLabel("Push token", {exact:true})).toHaveValue("");
  await expect(page.getByLabel("Separate pull-only token", {exact:true})).toHaveValue("");
  await fillRegistry(page);
  await request.post("http://127.0.0.1:50062/reject-connection-mutation");
  await page.getByRole("dialog").getByRole("button", {name:"Save and test"}).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toBeVisible();
  await expect(page.getByLabel("Push token", {exact:true})).toHaveValue("");
  await expect(page.getByLabel("Separate pull-only token", {exact:true})).toHaveValue("");
  await expect(page.getByRole("region", {name:"Team images"})).toHaveCount(0);
  await page.getByLabel("Push token", {exact:true}).fill("fresh-push-value");
  await page.getByLabel("Separate pull-only token", {exact:true}).fill("fresh-pull-value");
  await page.getByRole("dialog").getByRole("button", {name:"Save and test"}).click();
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(page.getByRole("region", {name:"Team images"})).toBeVisible();
  await page.getByRole("button", {name:"Add connection",exact:true}).click();
  await expect(page.getByLabel("Push token", {exact:true})).toHaveValue("");
  await expect(page.getByLabel("Separate pull-only token", {exact:true})).toHaveValue("");
});

test("GHCR editing clears credentials on success, failure, and dismissal and supports connection actions", async ({page,request})=>{
  await createRegistry(page);
  await expect(page.getByLabel("New push token")).toHaveCount(0);
  await page.getByRole("region", {name:"Team images"}).getByRole("button", {name:"Edit",exact:true}).click();
  const dialog=page.getByRole("dialog");
  await dialog.getByLabel("New push token").fill("replacement-push-value");
  await dialog.getByLabel("Separate pull-only token").fill("replacement-pull-value");
  await request.post("http://127.0.0.1:50062/reject-connection-mutation");
  await dialog.getByRole("button", {name:"Replace credentials"}).click();
  await expect(dialog.getByRole("alert")).toBeVisible();
  await expect(dialog.getByLabel("New push token")).toHaveValue("");
  await expect(dialog.getByLabel("Separate pull-only token")).toHaveValue("");
  await dialog.getByLabel("New push token").fill("accepted-push-value");
  await dialog.getByLabel("Separate pull-only token").fill("accepted-pull-value");
  await dialog.getByRole("button", {name:"Replace credentials"}).click();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await expect(dialog.getByLabel("New push token")).toHaveValue("");
  await expect(dialog.getByLabel("Separate pull-only token")).toHaveValue("");
  await dialog.getByRole("button", {name:"Test connection / Retry"}).click();
  await dialog.getByRole("button", {name:"Disable",exact:true}).click();
  await expect(dialog.getByText("Disabled", {exact:true})).toBeVisible();
  await dialog.getByRole("button", {name:"Enable",exact:true}).click();
  await expect(dialog.getByText("Ready", {exact:true})).toBeVisible();
  await dialog.getByLabel("New push token").fill("discarded-push-value");
  await dialog.getByLabel("Separate pull-only token").fill("discarded-pull-value");
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await page.getByRole("region", {name:"Team images"}).getByRole("button", {name:"Edit",exact:true}).click();
  await expect(dialog.getByLabel("New push token")).toHaveValue("");
  await expect(dialog.getByLabel("Separate pull-only token")).toHaveValue("");
  await dialog.getByRole("button", {name:"Delete",exact:true}).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("region", {name:"Team images"})).toHaveCount(0);
});

test("GHCR open edit dialog observes new revisions and shows cleanup warnings on ready rows", async ({page,request})=>{
  await page.clock.install();
  await createRegistry(page);
  await page.getByRole("region", {name:"Team images"}).getByRole("button", {name:"Edit",exact:true}).click();
  await request.post("http://127.0.0.1:50062/connection-state?status=ready&cleanup=failed");
  await page.clock.fastForward(30_000);
  await expect(page.getByRole("region", {name:"Team images"}).getByText("Cleanup required", {exact:true})).toBeVisible();
  const dialog=page.getByRole("dialog");
  await expect(dialog.getByText("registry_cleanup_permission_required", {exact:false})).toBeVisible();
  await dialog.getByLabel("New push token").fill("current-revision-push");
  await dialog.getByLabel("Separate pull-only token").fill("current-revision-pull");
  await dialog.getByRole("button", {name:"Replace credentials"}).click();
  await expect(dialog.getByLabel("New push token")).toHaveValue("");
  await expect(dialog.getByRole("alert")).toHaveCount(0);
});

test("GHCR administrator connections expose status and readonly details", async ({page,request})=>{
  await request.post("http://127.0.0.1:50062/admin-connection?kind=ghcr");
  await page.goto("/container-registries/github");
  const row=page.getByRole("region", {name:"Admin App"});
  await expect(row.getByText("Managed by your administrator", {exact:true})).toBeVisible();
  await expect(row.getByRole("button", {name:"Edit",exact:true})).toHaveCount(0);
  await row.getByRole("button", {name:"Details",exact:true}).click();
  const dialog=page.getByRole("dialog");
  await expect(dialog.getByText("Managed by your administrator.", {exact:false})).toBeVisible();
  await expect(dialog.getByRole("button", {name:"Replace credentials"})).toHaveCount(0);
  await expect(dialog.locator("input[type=password]")).toHaveCount(0);
});
