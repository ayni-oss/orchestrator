import { expect, test } from "@playwright/test";
test.beforeEach(async ({ request }) => { await request.post("http://127.0.0.1:50062/reset-project"); });
test("workspace conversation history stays in the sidebar and opens the selected project", async ({ page, request }) => {
 await page.goto("/_old/workspaces");
 await expect(page.getByRole("button", {name:"New conversation",exact:true})).toBeDisabled();
 await expect(page.getByText("Select a workspace to view conversations.", {exact:true})).toBeVisible();
 await request.post("http://127.0.0.1:50062/admin-connection?kind=openai");
 await page.goto("/_old/workspaces/workspace:1?view=settings");
 for(const [label,option] of [["Project agent connection","Admin App"],["Project agent model","gpt-5.6-sol"],["Coding agent connection","Admin App"],["Coding agent model","gpt-5.6-sol"]]){
  await page.getByLabel(label!,{exact:true}).click();await page.getByRole("option",{name:option!,exact:true}).click();
 }
 await page.getByRole("button",{name:"Save agent defaults",exact:true}).click();
 await expect(page.getByRole("region",{name:"Workspace agent defaults",exact:true})).toContainText("Workspace settings updated.");
 await expect(page.getByLabel("Current project",{exact:true})).toHaveText("No project");
 await page.getByRole("button", {name:"New conversation",exact:true}).click();
 const dialog=page.getByRole("dialog", {name:"New conversation",exact:true});
 await expect(dialog.getByLabel("Project", {exact:true})).toContainText("Sandbox");
 await dialog.getByRole("button", {name:"Start conversation",exact:true}).click();
 await expect(dialog).toBeHidden();
 await expect(page.getByLabel("Current project",{exact:true})).toHaveText("Sandbox");
 await page.getByLabel("Message",{exact:true}).fill("Workspace discussion");await page.getByLabel("Message",{exact:true}).press("Enter");
 await expect(page).toHaveURL(/\/projects\/project%3A1\?conversation=conversation%3A1/);
 await expect(page.getByRole("heading", {name:"Workspace discussion",exact:true})).toBeVisible();
 const history=page.getByRole("navigation", {name:"Conversation history",exact:true});
 await expect(history.getByRole("link", {name:"Workspace discussion Sandbox",exact:true})).toHaveAttribute("aria-current","page");
 await page.getByRole("tab", {name:"Tasks",exact:true}).click();
 await history.getByRole("link", {name:"Workspace discussion Sandbox",exact:true}).click();
 await expect(page.getByRole("tab", {name:"Conversation",exact:true})).toHaveAttribute("aria-selected","true");
 await expect(page.getByRole("region", {name:"Conversation",exact:true})).toBeVisible();
});
