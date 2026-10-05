import { expect, test } from "@playwright/test";

const revision = "a".repeat(40);

test("operator inspects profile readiness and submits frozen work", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-hydrated", "true");
  await expect(page.getByRole("heading", { name: "Projects", exact:true })).toBeVisible();

  await page.goto("/repositories");
  await page.getByRole("link", { name: "Profiles and identity" }).first().click();
  await expect(page.getByRole("heading", { name: "Runtime profiles" })).toBeVisible();
  await expect(page.getByText("Ready", { exact: true })).toBeVisible();

  await page.goto("/tasks");
  await page.getByLabel("Project", {exact:true}).selectOption("project:1");
  await page.getByLabel("Runtime profile").selectOption("profile:1");
  await page.getByLabel("Model").selectOption("gpt-5");
  await page.getByLabel("Request").fill("Document the bounded retry behavior");
  await page.getByLabel("Acceptance criteria").fill("Retry count is explicit\nExisting behavior remains covered");
  await page.getByRole("button", { name: "Submit task" }).click();
  await expect(page).toHaveURL(/\/tasks\/00000000-0000-4000-8000-000000000040/u);
  await expect(page.getByRole("heading", { name: "Document the bounded retry behavior" })).toBeVisible();
  await expect(page.getByText("Frozen repository and runtime", { exact: true })).toBeVisible();
  await expect(page.getByText("Frozen request", { exact: true })).toBeVisible();
});

// Provider operations are fixture-backed; this tests browser/BFF behavior, not live GitHub.
test("repository setup PR and requested draft publication are visible", async ({page,request})=>{
 await request.post("http://127.0.0.1:50062/reset-onboarding");
 await page.goto("/repositories");
 await expect(page.getByLabel("GitHub owner",{exact:true})).toHaveValue("42");
 await page.getByLabel("Repository",{exact:true}).selectOption("sandbox");
 await page.getByRole("button",{name:"Connect repository",exact:true}).click();
 await expect(page.getByText("Connected ayni-oss/sandbox.",{exact:true})).toBeVisible();
 await page.getByRole("link",{name:"Profiles and identity"}).first().click();
 await page.getByRole("button",{name:"Inspect repository"}).click();
 await expect(page.getByText(".github/workflows/ayni-environment.yml",{exact:true})).toBeVisible();
 await page.getByRole("button",{name:"Set up runtime"}).click();
 await expect(page.getByRole("link",{name:"Open setup PR #8"})).toHaveAttribute("href","https://github.com/ayni-oss/sandbox/pull/8");
 await page.reload();
 await expect(page.getByRole("link",{name:"Open setup PR #8"})).toBeVisible();
 await page.getByRole("button",{name:"Retry reconciliation"}).click();
 await page.getByRole("link",{name:"New request"}).click();
 await page.getByLabel("Model").selectOption("gpt-5");
 await page.getByLabel("Request",{exact:true}).fill("Add a bounded example");
 await page.getByLabel("Acceptance criteria").fill("The example is covered");
 await page.getByRole("button",{name:"Submit task"}).click();
 await expect(page.getByRole("link",{name:"Open pull request",exact:true})).toHaveAttribute("href","https://github.com/ayni-oss/sandbox/pull/9");
 await expect(page.getByText("Coding-time checks are advisory.",{exact:true})).toBeVisible();
 await page.screenshot({path:"/tmp/orchestrator-task-ui.png",fullPage:true});
});

test("every managed profile state explains the next step",async({page,request})=>{
 const states=["Open the setup PR","Review and merge the setup PR","Checking the expected image","The signed environment passed verification","A new repository revision is being verified","Repair provider access","Review the failure below"];
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
 await expect(page.getByLabel("Repository",{exact:true})).toHaveValue("");
 await page.getByLabel("Repository",{exact:true}).selectOption("repository:2");
 await expect(page.getByRole("button",{name:"Submit task",exact:true})).toBeDisabled();
 await page.getByLabel("Repository",{exact:true}).selectOption("repository:1");
 await expect(page.getByLabel("Runtime profile",{exact:true})).toHaveValue("profile:1");
 await page.getByLabel("Model",{exact:true}).selectOption("gpt-5");
 await page.getByLabel("Request",{exact:true}).fill("Work only in the selected repository");
 await page.getByLabel("Acceptance criteria",{exact:true}).fill("The other repository is unchanged");
 await page.getByRole("button",{name:"Submit task",exact:true}).click();
 await expect(page.getByRole("heading",{name:"Work only in the selected repository",exact:true})).toBeVisible();
 await page.getByRole("button",{name:"Cancel",exact:true}).click();
 await expect(page).toHaveURL(/notice=cancelled/u);
});
