import {test,expect,type Page} from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
async function choose(page:Page,label:string,option:string){await page.getByLabel(label,{exact:true}).click();await page.getByRole("option",{name:option,exact:true}).click();}
async function saveProfile(page:Page,role:"planner"|"worker"){
 await page.getByRole("button",{name:`Save ${role} profile`,exact:true}).click();
 const dialog=page.getByRole("dialog",{name:`Review ${role} profile changes`,exact:true});
 await expect(dialog.getByRole("columnheader",{name:"Current",exact:true})).toBeVisible();
 await expect(dialog.getByRole("columnheader",{name:"Proposed",exact:true})).toBeVisible();
 await dialog.getByRole("button",{name:"Confirm changes",exact:true}).click();
 await expect(dialog).toBeHidden();
 await expect(page.getByRole("status").filter({hasText:new RegExp(`${role} profile saved`,"i")})).toBeVisible();
}
async function setup(page:Page){
 await page.goto("/_old/workspaces/workspace:1?view=settings");
 await choose(page,"Project agent connection","Admin App");await choose(page,"Project agent model","gpt-5.6-sol");await choose(page,"Coding agent connection","Admin App");await choose(page,"Coding agent model","gpt-5.6-sol");
 await page.getByRole("button",{name:"Save agent defaults",exact:true}).click();await expect(page.getByRole("status")).toContainText("Workspace settings updated");
 await expect(page.getByRole("region",{name:"Agent capability profiles",exact:true})).toBeVisible();
 const planner=page.getByRole("form",{name:"Planner profile",exact:true});
 await planner.getByLabel("Use application capabilities").uncheck();
 await planner.getByLabel("Instructions",{exact:true}).fill("Review compatibility before preparing changes.");
 await planner.getByLabel("Maximum model calls").fill("4");await planner.getByLabel("Maximum duration (seconds)").fill("300");
 await planner.getByRole("button",{name:"Add pinned skill"}).click();
 await planner.getByLabel("Skill ID",{exact:true}).fill("compatibility");await planner.getByLabel("Revision",{exact:true}).fill("v1");await planner.getByLabel("Skill instructions",{exact:true}).fill("Describe compatibility risks. This text grants no permissions.");
 await saveProfile(page,"planner");
}
test.beforeEach(async({request})=>{await request.post("http://127.0.0.1:50062/reset-project");await request.post("http://127.0.0.1:50062/admin-connection?kind=openai");});
test("workspace capabilities persist and project selections expose inheritance",async({page})=>{
 await setup(page);await page.reload();await expect(page.getByRole("region",{name:"Agent capability profiles",exact:true})).toBeVisible();
 const planner=page.getByRole("form",{name:"Planner profile",exact:true});await expect(planner.getByLabel("Maximum model calls")).toHaveValue("4");await expect(planner.getByLabel("Revision",{exact:true})).toHaveValue("v1");await expect(planner.getByLabel("run_command",{exact:true})).toHaveCount(0);
 await page.goto("/_old/projects/project:1");await page.getByRole("button",{name:"Sandbox project actions",exact:true}).click();await page.getByRole("tab",{name:"Settings",exact:true}).click();await expect(page.getByRole("region",{name:"Agent capability profiles",exact:true})).toBeVisible();
 const projectPlanner=page.getByRole("form",{name:"Planner profile",exact:true});await expect(projectPlanner.getByLabel("Use workspace capabilities")).toBeChecked();await expect(projectPlanner.getByLabel("Maximum model calls")).toHaveValue("4");
 await projectPlanner.getByLabel("Use workspace capabilities").uncheck();await projectPlanner.getByLabel("Maximum model calls").fill("2");await projectPlanner.getByLabel("Additional instructions",{exact:true}).fill("Focus on the project API.");await saveProfile(page,"planner");
 await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);await expect(page.locator("[data-sonner-toast]")).toHaveCount(0,{timeout:10_000});expect((await new AxeBuilder({page}).analyze()).violations).toEqual([]);
});
test("research and tasks expose frozen profiles after later settings edits",async({page})=>{
 await setup(page);await page.goto("/_old/projects/project:1");await page.getByRole("button",{name:"New conversation",exact:true}).click();await expect(page).toHaveURL(/new=/u);await page.getByLabel("Message",{exact:true}).fill("Plan a compatibility improvement");await page.getByRole("button",{name:"Send message",exact:true}).click();
 await page.getByRole("tab",{name:"Plans",exact:true}).click();await page.getByText("Research run · Completed",{exact:true}).click();const summary=page.locator("summary").filter({hasText:/^Frozen planner profile/});await expect(summary).toBeVisible();await summary.click();await expect(page.getByText("compatibility @ v1",{exact:true})).toBeVisible();
 await page.getByRole("button",{name:"Approve version 1 and submit tasks",exact:true}).click();await page.getByRole("tab",{name:"Tasks",exact:true}).click();await page.getByRole("link",{name:"Open execution attempt",exact:true}).click();await expect(page).toHaveURL(/\/tasks\//);await expect(page.locator("summary").filter({hasText:/^Frozen worker profile/})).toBeVisible();
 const taskUrl=page.url(),frozenSummary=await page.locator("summary").filter({hasText:/^Frozen worker profile/}).textContent();
 await page.goto("/_old/workspaces/workspace:1?view=settings");await expect(page.getByRole("region",{name:"Agent capability profiles",exact:true})).toBeVisible();
 const worker=page.getByRole("form",{name:"Worker profile",exact:true});await worker.getByLabel("Use application capabilities").uncheck();await worker.getByLabel("Instructions",{exact:true}).fill("New worker instructions apply to new tasks.");await saveProfile(page,"worker");
 await page.goto(taskUrl);await expect(page.locator("summary").filter({hasText:/^Frozen worker profile/})).toHaveText(frozenSummary!);await page.locator("summary").filter({hasText:/^Frozen worker profile/}).click();await expect(page.getByText("New worker instructions apply to new tasks.",{exact:true})).toHaveCount(0);

});

test("profile changes are reviewed before saving and each form keeps its own draft",async({page})=>{
 await page.goto("/_old/workspaces/workspace:1?view=settings");
 const planner=page.getByRole("form",{name:"Planner profile",exact:true});
 const worker=page.getByRole("form",{name:"Worker profile",exact:true});
 await planner.getByLabel("Use application capabilities").uncheck();
 await planner.getByLabel("Maximum model calls").fill("4");
 await worker.getByLabel("Use application capabilities").uncheck();
 await worker.getByLabel("Maximum model calls").fill("8");
 await planner.getByRole("button",{name:"Save planner profile",exact:true}).click();
 const review=page.getByRole("dialog",{name:"Review planner profile changes",exact:true});
 const calls=review.getByRole("row").filter({hasText:"Maximum model calls"});
 await expect(calls.getByRole("cell").nth(1)).toHaveText("16");
 await expect(calls.getByRole("cell").nth(2)).toHaveText("4");
 await page.setViewportSize({width:390,height:844});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
 expect((await new AxeBuilder({page}).analyze()).violations).toEqual([]);
 await review.getByRole("button",{name:"Back to editing",exact:true}).click();
 await expect(planner.getByLabel("Maximum model calls")).toHaveValue("4");
 await saveProfile(page,"planner");
 await expect(worker.getByLabel("Maximum model calls")).toHaveValue("8");
 await page.reload();
 await expect(planner.getByLabel("Maximum model calls")).toHaveValue("4");
 await expect(worker.getByLabel("Use application capabilities")).toBeChecked();
 await expect(worker.getByLabel("Maximum model calls")).toHaveValue("128");
});
