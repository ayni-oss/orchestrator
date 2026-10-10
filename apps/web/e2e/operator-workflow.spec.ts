import { expect, test, type Page } from "@playwright/test";

async function choose(page: Page, label: string, option: string): Promise<void> {
  await page.getByLabel(label, { exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}


test("operator inspects profile readiness and submits frozen work", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await expect(page.getByRole("heading", { name: "Projects", exact:true })).toBeVisible();

  await page.goto("/repositories");
  await page.getByRole("link", { name: "Manage repository" }).first().click();
  await expect(page.getByRole("heading", { name: "Runtimes" })).toBeVisible();
  await expect(page.locator("article").getByText("Ready", { exact: true })).toBeVisible();

  await page.goto("/tasks");
  await choose(page, "Project", "Sandbox");
  await choose(page, "Runtime profile", "Ayni managed");
  await choose(page, "Model", "GPT-5");
  await page.getByLabel("Request").fill("Document the bounded retry behavior");
  await page.getByLabel("Acceptance criteria").fill("Retry count is explicit\nExisting behavior remains covered");
  await page.getByRole("button", { name: "Submit task" }).click();
  await expect(page).toHaveURL(/\/tasks\/00000000-0000-4000-8000-000000000040/u);
  await expect(page.getByRole("heading", { name: "Document the bounded retry behavior" })).toBeVisible();
  await expect(page.getByText("Frozen repository and runtime", { exact: true })).toBeVisible();
  await expect(page.getByText("Frozen request", { exact: true })).toBeVisible();
});

// Provider operations are fixture-backed; this tests browser/BFF behavior, not live GitHub.
test("connected runtime and truthful worker draft delivery are visible", async ({ page }) => {
 await page.goto("/profiles/profile:1");
 await page.getByRole("link", {name:"New request",exact:true}).click();
 await choose(page, "Model", "GPT-5");
 await page.getByLabel("Request", {exact:true}).fill("Add a bounded example");
 await page.getByLabel("Acceptance criteria").fill("The example is covered");
 await page.getByRole("button", {name:"Submit task",exact:true}).click();
 await expect(page.getByRole("link", {name:"Open pull request #9",exact:true})).toHaveAttribute("href", "https://github.com/ayni-oss/sandbox/pull/9");
 await expect(page.getByText("Attempt 1 · Draft published · quality checks failed", {exact:true})).toBeVisible();
 await expect(page.getByText("Repair the failing test", {exact:true})).toBeVisible();
 await expect(page.getByText("Turns consumed: 5 / 24 · Quality round: 2 · Repair limit: 3", {exact:true})).toBeVisible();
 await page.screenshot({path:"/tmp/orchestrator-task-ui.png",fullPage:true});
});

test("every managed profile state explains the next step",async({page,request})=>{
 const states=["Inspecting the connected repository's default revision.", "Preparing the immutable runtime identity.", "Resolving and verifying the registry image for this exact lock fingerprint. A missing image is built by an Orchestrator-managed Kubernetes Job.", "The signed runtime image matches the checked lock and is ready for use.", "A new lock fingerprint is being resolved. Existing requests retain their frozen image.", "Repair provider access, registry access, build configuration or certificate trust, then retry reconciliation.", "The committed Ayni configuration or runtime image could not be verified. Review the failure and retry after repair."];
 try { for(let index=0;index<states.length;index++) {
  await request.post(`http://127.0.0.1:50062/profile-state?value=${index+1}`);
  await page.goto("/profiles/profile:1");
  await expect(page.getByText(states[index]!,{exact:false}).first()).toBeVisible();
  if(index!==3) await expect(page.getByRole("link",{name:"New request",exact:true})).toHaveCount(0);
 } } finally { await request.post("http://127.0.0.1:50062/profile-state?value=4"); }
});

test("projects retain identity through rename and archive", async ({ page }) => {
  await page.goto("/projects");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await page.getByRole("button", {name:"Create project",exact:true}).click();
  await page.getByLabel("Project name", { exact: true }).fill("Payments");
  await page.getByRole("button", {name:"Repositories",exact:true}).click();
  await page.getByRole("checkbox",{name:"ayni-oss/sandbox",exact:true}).check();
  await page.getByRole("button",{name:"Done",exact:true}).click();
  await expect(page.getByRole("button",{name:"Done",exact:true})).toBeHidden();
  await page.getByRole("dialog").getByRole("button", { name: "Create project", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Payments", exact: true })).toBeVisible();
  await page.getByLabel("Project name", { exact: true }).fill("Ledger");
  await page.getByRole("button", { name: "Save settings", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Ledger", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Archive project", exact: true }).click();
  await expect(page.getByText("New tasks and retries are disabled for this archived project.", {exact:true})).toBeVisible();
  await expect(page.getByRole("link", { name: "New task", exact: true })).toHaveCount(0);
  await page.screenshot({path:"/tmp/orchestrator-project-ui.png",fullPage:true});
  // Restore the shared browser fixture for later scenarios; real lifecycle is tested natively.
  await page.goto("/projects");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await page.getByRole("button", {name:"Create project",exact:true}).click();
  await page.getByLabel("Project name", { exact: true }).fill("Sandbox");
  await page.getByRole("button", {name:"Repositories",exact:true}).click();
  await page.getByRole("checkbox",{name:"ayni-oss/sandbox",exact:true}).check();
  await page.getByRole("button",{name:"Done",exact:true}).click();
  await expect(page.getByRole("button",{name:"Done",exact:true})).toBeHidden();
  await page.getByRole("dialog").getByRole("button", { name: "Create project", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Sandbox", exact: true })).toBeVisible();
});

test("projects select several repositories and tasks require one target",async({page})=>{
 await page.goto("/projects");
 await page.getByRole("button",{name:"Create project",exact:true}).click();
 await page.getByLabel("Project name",{exact:true}).fill("Shared work");
 await page.getByRole("button",{name:"Repositories",exact:true}).click();
 await page.getByRole("checkbox",{name:"ayni-oss/sandbox",exact:true}).check();
 await page.getByRole("checkbox",{name:"ayni-oss/library",exact:true}).check();
 await page.getByRole("button",{name:"Done",exact:true}).click();
 await expect(page.getByRole("button",{name:"Done",exact:true})).toBeHidden();
 await page.getByRole("dialog").getByRole("button",{name:"Create project",exact:true}).click();
 await expect(page.getByRole("heading",{name:"Shared work",exact:true})).toBeVisible();
 await page.getByRole("button",{name:"New task",exact:true}).click();
 await expect(page.getByLabel("Repository",{exact:true})).toHaveText("Select a repository");
 await choose(page, "Repository", "ayni-oss/library");
 await expect(page.getByRole("button",{name:"Submit task",exact:true})).toBeDisabled();
 await choose(page, "Repository", "ayni-oss/sandbox");
 await choose(page, "Runtime profile", "Ayni managed");
 await choose(page, "Model", "GPT-5");
 await page.getByLabel("Request",{exact:true}).fill("Work only in the selected repository");
 await page.getByLabel("Acceptance criteria",{exact:true}).fill("The other repository is unchanged");
 await page.getByRole("button",{name:"Submit task",exact:true}).click();
 await expect(page.getByRole("heading",{name:"Work only in the selected repository",exact:true})).toBeVisible();
 await page.getByRole("button",{name:"Cancel",exact:true}).click();
 await expect(page).toHaveURL(/notice=cancelled/u);
});

test("publication retry resumes retained final work and is distinct from task retry", async ({page}) => {
 await page.goto("/tasks/00000000-0000-4000-8000-000000000044");
 await expect(page.getByRole("button",{name:"Retry task · new attempt",exact:true})).toBeVisible();
 await page.getByRole("button",{name:"Retry publication",exact:true}).click();
 await expect(page).toHaveURL(/notice=publication-retried/u);
 await expect(page.getByText(/Attempt 1 · publication retry/u)).toBeVisible();
});
