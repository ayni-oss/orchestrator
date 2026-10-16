import {test,expect} from "@playwright/test";
test.beforeEach(async({request})=>{await request.post("http://127.0.0.1:50062/reset-project");await request.post("http://127.0.0.1:50062/reset-connections");});
test.afterEach(async({request})=>{await request.post("http://127.0.0.1:50062/reset-project");});
test("global history routes old creation bookmarks into project selection",async({page})=>{
 await page.goto("/tasks");await expect(page.getByRole("heading",{name:"Task history",exact:true})).toBeVisible();await expect(page.getByRole("button",{name:"Submit task",exact:true})).toHaveCount(0);
 await page.goto("/tasks#new");await expect(page.getByRole("heading",{name:"Choose a project to submit directly",exact:true})).toBeVisible();await page.getByRole("link",{name:"Sandbox",exact:true}).click();await expect(page.getByRole("dialog",{name:"Submit task directly",exact:true})).toBeVisible();
});
test("configuration failures disable creation without inventing browser capabilities",async({page,request})=>{
 await request.post("http://127.0.0.1:50062/capabilities-unavailable");await page.goto("/projects/project:1");await expect(page.getByText("Agent configuration unavailable",{exact:true})).toBeVisible();
 await page.getByRole("button",{name:"Sandbox project actions",exact:true}).click();await expect(page.getByRole("menuitem",{name:"Submit task directly",exact:true})).toBeDisabled();
 await page.getByRole("tab",{name:"Settings",exact:true}).click();await expect(page.getByRole("region",{name:"Agent capability profiles",exact:true})).toBeVisible();await expect(page.getByRole("button",{name:"Save planner profile",exact:true})).toBeDisabled();
});
