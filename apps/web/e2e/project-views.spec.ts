import {test,expect,type Page} from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

async function choose(page:Page,label:string,option:string){
  await page.getByLabel(label,{exact:true}).click();
  await page.getByRole("option",{name:option,exact:true}).click();
}
async function startPlan(page:Page,message="Make the outcome easy to follow"){
  await page.goto("/projects/project:1");
  await page.getByRole("button",{name:"Sandbox project actions"}).click();
  await page.getByRole("menuitem",{name:"Project settings",exact:true}).click();
  await choose(page,"Project agent connection","Admin App");
  await choose(page,"Project agent model","gpt-5.6-sol");
  await choose(page,"Coding agent connection","Admin App");
  await choose(page,"Coding agent model","gpt-5.6-sol");
  await page.getByRole("button",{name:"Save agent defaults",exact:true}).click();
  await expect(page.getByRole("dialog")).toContainText("Agent defaults updated.");
  await page.getByRole("dialog").getByRole("button",{name:"Close",exact:true}).first().click();
  await page.getByRole("button",{name:"New conversation",exact:true}).click();
  await page.getByLabel("Title",{exact:true}).fill("Project layout");
  await page.getByRole("button",{name:"Create conversation",exact:true}).click();
  await page.getByLabel("Message",{exact:true}).fill(message);
  await page.getByRole("button",{name:"Send message",exact:true}).click();
  await expect(page.getByRole("region",{name:"Plan review",exact:true})).toBeVisible();
  await expect(page.getByRole("button",{name:"Sending…",exact:true})).toHaveCount(0);
}
function view(page:Page,name:string){return page.getByRole("navigation",{name:"Project views"}).getByRole("link",{name,exact:true});}
test.beforeEach(async({request})=>{
  await request.post("http://127.0.0.1:50062/reset-project");
  await request.post("http://127.0.0.1:50062/admin-connection?kind=openai");
});

for(const width of [1280,390])test(`chat scrolls independently above a stable plan at ${width}px`,async({page,request})=>{
  await page.setViewportSize({width,height:900});
  await startPlan(page);
  const timeline=page.getByRole("region",{name:"Conversation timeline",exact:true});
  const review=page.getByRole("region",{name:"Plan review",exact:true});
  const top=await review.evaluate(node=>node.getBoundingClientRect().top+scrollY);
  const height=await timeline.evaluate(node=>node.clientHeight);
  const count=await page.getByLabel("Conversation messages").getByRole("article").count();
  await request.post("http://127.0.0.1:50062/append-message");
  await expect(page.getByLabel("Conversation messages").getByRole("article")).toHaveCount(count+1);
  expect(await timeline.evaluate(node=>node.clientHeight)).toBe(height);
  expect(await review.evaluate(node=>node.getBoundingClientRect().top+scrollY)).toBeCloseTo(top,0);
  expect(await timeline.evaluate(node=>node.scrollHeight>node.clientHeight)).toBe(true);
  await timeline.focus();await page.keyboard.press("End");
  await expect.poll(()=>timeline.evaluate(node=>node.scrollTop)).toBeGreaterThan(0);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect((await new AxeBuilder({page}).include("main").analyze()).violations).toEqual([]);
  await page.evaluate(()=>{(document.activeElement as HTMLElement)?.blur();scrollTo(0,0);});
  await page.screenshot({path:`/tmp/orchestrator-project-overview-${width}.png`,fullPage:true});
});

test("plans, research runs, approved tasks, and execution attempts remain navigable",async({page})=>{
  await startPlan(page);
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-hydrated","true");
  await view(page,"Plans").focus();
  await expect(view(page,"Plans")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(view(page,"Plans")).toHaveAttribute("aria-current","page");
  await expect(page.getByLabel("Message",{exact:true})).toHaveCount(0);
  await page.getByText("Research run · Completed",{exact:true}).click();
  await expect(page.getByRole("region",{name:"Plan review",exact:true})).toContainText("run:1");
  await page.getByRole("button",{name:"Approve version 1 and submit tasks",exact:true}).click();
  const taskHref=await page.getByRole("link",{name:/View task/u}).getAttribute("href");
  await view(page,"Tasks").click();
  await expect(page).toHaveURL(/view=tasks.*conversation=conversation%3A1/u);
  await page.getByRole("link",{name:"Approved plan · version 1",exact:true}).click();
  await expect(page).toHaveURL(/view=plans.*version=1/u);
  await page.reload();await expect(page.getByLabel("Plan version",{exact:true})).toContainText("Version 1 · Approved");
  await page.getByRole("link",{name:/View task/u}).click();
  await expect(page).toHaveURL(new RegExp(taskHref!.replace(/\//gu,"\\/"),"u"));
  await expect(page.getByRole("heading",{name:"Progress",exact:true})).toBeVisible();
  await page.getByRole("link",{name:"Project tasks",exact:true}).click();
  await expect(view(page,"Tasks")).toHaveAttribute("aria-current","page");
  await page.goBack();await expect(page.getByRole("heading",{name:"Progress",exact:true})).toBeVisible();
});

test("conversation failures recover and invalid version links do not show a different plan",async({page,request})=>{
  await startPlan(page);
  await request.post("http://127.0.0.1:50062/conversation-unavailable");
  await page.reload();await expect(page.getByText("Conversations unavailable",{exact:true})).toBeVisible();
  await request.post("http://127.0.0.1:50062/conversation-unavailable?value=false");
  await page.getByRole("button",{name:"Retry conversations",exact:true}).click();
  await expect(page.getByRole("region",{name:"Plan review",exact:true})).toBeVisible();
  await page.goto("/projects/project:1?view=plans&conversation=conversation:1&version=999");
  await expect(page.getByText("Plan version unavailable",{exact:true})).toBeVisible();
  await expect(page.getByRole("button",{name:/Approve version/u})).toHaveCount(0);
  await page.getByRole("button",{name:"Show current plan",exact:true}).click();
  await expect(page.getByLabel("Plan version",{exact:true})).toContainText("Version 1");
  await page.goto("/projects/project:1?conversation=conversation:missing");
  await expect(page.getByText("Conversations unavailable",{exact:true})).toBeVisible();
  await expect(page.getByLabel("Message",{exact:true})).toHaveCount(0);
});

test("settings expose ownership, saving, failure, and focus recovery",async({page})=>{
  await page.goto("/workspaces/workspace:1");
  await expect(page.getByRole("region",{name:"Shared configuration",exact:true})).toContainText("shared across projects");
  await page.goto("/projects/project:1");
  const actions=page.getByRole("button",{name:"Sandbox project actions"});
  await actions.click();await page.getByRole("menuitem",{name:"Project settings",exact:true}).click();
  const settings=page.getByRole("dialog",{name:"Project settings",exact:true});
  await expect(settings).toContainText("Repository instructions such as AGENTS.md remain in each repository.");
  await expect(settings).toContainText("Inherited from");
  await settings.getByLabel("Goal and planning instructions",{exact:true}).fill("Retain my unsaved planning instructions");
  let release:(()=>void)|undefined;
  const held=new Promise<void>(resolve=>{release=resolve;});
  await page.route("**/projects/project:1?*/update",async route=>{
    if(route.request().method()!=="POST")return route.continue();
    await held;
    await route.fulfill({status:200,contentType:"application/json",body:JSON.stringify({type:"failure",status:409,data:'[{"error":1},{"message":2},"Reload before saving these settings."]'})});
  });
  await settings.getByRole("button",{name:"Save settings",exact:true}).click();
  await expect(settings.getByRole("button",{name:"Saving…",exact:true})).toBeDisabled();
  release!();
  await expect(settings.getByRole("alert")).toContainText("Reload before saving these settings.");
  await expect(settings.getByLabel("Goal and planning instructions",{exact:true})).toHaveValue("Retain my unsaved planning instructions");
  await expect(settings.getByRole("button",{name:"Save settings",exact:true})).toBeEnabled();
  await page.keyboard.press("Escape");await expect(actions).toBeFocused();
});


test("reviewed dependency scope stays visible through approval and task details",async({page})=>{
  await startPlan(page,"Update Cargo dependencies and regenerate the derived environment lock");
  const review=page.getByRole("region",{name:"Plan review",exact:true});
  await expect(review).toContainText("Regenerate .ayni.lock for: Cargo.toml, Cargo.lock");
  await expect(review).toContainText("Policy and trust settings remain protected.");
  await review.getByRole("button",{name:"Approve version 1 and submit tasks",exact:true}).click();
  await page.getByRole("link",{name:/View task/u}).click();
  await expect(page.locator("main")).toContainText("Regenerate .ayni.lock for: Cargo.toml, Cargo.lock");
});
