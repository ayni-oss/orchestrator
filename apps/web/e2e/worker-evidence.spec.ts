import {test,expect,type Page} from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
async function choose(page:Page,label:string,option:string){await page.getByLabel(label,{exact:true}).click();await page.getByRole("option",{name:option,exact:true}).click();}
async function approved(page:Page){
 await page.goto("/workspaces/workspace:1?view=settings");
 await choose(page,"Project agent connection","Admin App");await choose(page,"Project agent model","gpt-5.6-sol");
 await choose(page,"Coding agent connection","Admin App");await choose(page,"Coding agent model","gpt-5.6-sol");
 await page.getByRole("button",{name:"Save agent defaults",exact:true}).click();
 await expect(page.getByRole("region",{name:"Workspace agent defaults",exact:true})).toContainText("Workspace settings updated.");
 await page.goto("/projects/project:1");await page.getByRole("button",{name:"New conversation",exact:true}).click();
 await page.getByLabel("Title",{exact:true}).fill("Worker evidence");await page.getByRole("button",{name:"Create conversation",exact:true}).click();
 await page.getByLabel("Message",{exact:true}).fill("Preserve API compatibility");await page.getByRole("button",{name:"Send message",exact:true}).click();
 await page.getByRole("button",{name:"Approve version 1 and submit tasks",exact:true}).click();
 await expect(page.getByRole("region",{name:"Worker updates",exact:true})).toBeVisible();
}
test.beforeEach(async({request})=>{await request.post("http://127.0.0.1:50062/reset-project");await request.post("http://127.0.0.1:50062/admin-connection?kind=openai");});
for(const [scenario,execution,quality,publication] of [
 ["success","Succeeded","Passed","Published"],
 ["failed_quality","Succeeded","Failed","Published"],
 ["unavailable","Succeeded","Unavailable","Published"],
 ["publication_failed","Succeeded","Passed","Failed"],
 ["failed","Failed","Unavailable","Not required"],
 ["cancelled","Cancelled","Unavailable","Not required"],
] as const)test(`conversation shows independent worker outcomes for ${scenario}`,async({page,request})=>{
 await approved(page);
 await request.post(`http://127.0.0.1:50062/worker-evidence?case=${scenario}`);
 const updates=page.getByRole("region",{name:"Worker updates",exact:true});
 await expect(updates.getByText("Execution",{exact:true}).locator("..").locator("dd")).toHaveText(execution);
 await expect(updates.getByText("Quality",{exact:true}).locator("..").locator("dd")).toHaveText(quality);
 await expect(updates.getByText("Publication",{exact:true}).locator("..").locator("dd")).toHaveText(publication);
 await expect(updates.getByRole("region",{name:"Task attempt 1",exact:true})).toHaveCount(1);
 if(publication==="Published")await expect(updates.getByRole("link",{name:"Open draft PR #9",exact:true})).toBeVisible();
 if(quality==="Unavailable")await expect(updates).toContainText("Quality evidence unavailable");
 await updates.getByRole("link",{name:"Open execution attempt",exact:true}).click();
 await expect(page.getByRole("navigation",{name:"Task origin",exact:true})).toContainText("Approved plan · version 1");
 await page.getByRole("link",{name:"Originating conversation",exact:true}).click();
 await expect(page).toHaveURL(/view=overview&conversation=conversation%3A1/u);
});
test("worker questions preserve provenance, drafts, and explicit follow-up approval",async({page,request})=>{
 await approved(page);
 await request.post("http://127.0.0.1:50062/worker-evidence?case=input_required");
 const updates=page.getByRole("region",{name:"Worker updates",exact:true});
 await expect(updates).toContainText("Which API compatibility behavior should be preserved?");
 await updates.getByRole("button",{name:"Discuss this question",exact:true}).click();
 const composer=page.getByLabel("Message",{exact:true});await expect(composer).toBeFocused();
 await expect(composer).toHaveValue(/For task .*attempt .*approved plan version 1/u);
 await composer.fill((await composer.inputValue())+"Preserve API v1.");
 await request.post("http://127.0.0.1:50062/worker-evidence?case=input_required&version=100");
 await expect(composer).toHaveValue(/Preserve API v1\./u);
 await page.getByRole("button",{name:"Send message",exact:true}).click();
 await choose(page,"Plan version","Version 2 · Pending");
 await expect(page.getByRole("region",{name:"Plan review",exact:true})).toContainText("Preserve API v1.");
 await expect(page.getByRole("button",{name:"Approve version 2 and submit tasks",exact:true})).toBeVisible();
 await request.post("http://127.0.0.1:50062/restart-conversations");await page.reload();
 await expect(updates.getByRole("region",{name:"Task attempt 1",exact:true})).toHaveCount(1);
 await page.setViewportSize({width:390,height:844});
 await expect.poll(()=>page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 expect((await new AxeBuilder({page}).include("main").analyze()).violations).toEqual([]);
});
