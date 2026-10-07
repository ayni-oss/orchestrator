import {test,expect,type Page} from "@playwright/test";
const seed="a".repeat(64);
test.beforeEach(async({request})=>{await request.post("http://127.0.0.1:50062/reset-certificates");});
test.afterEach(async({request})=>{await request.post("http://127.0.0.1:50062/reset-certificates");});
async function create(page:Page,name="Team signing",mode="generate") {
  await page.goto("/certificates");await page.getByRole("button",{name:"Add certificate",exact:true}).click();
  await page.getByLabel("Certificate name",{exact:true}).fill(name);await page.getByLabel("Key ID",{exact:true}).fill("team-key");
  if(mode==="import"){await page.getByLabel("Import",{exact:true}).check();await page.getByLabel("Signing seed",{exact:true}).fill(seed);}
  await page.getByRole("button",{name:"Save certificate"}).click();await expect(page.getByRole("dialog")).toBeHidden();
}
test("certificate import clears seeds on dismissal and rejected submission",async({page,request})=>{
  await page.goto("/certificates");await expect(page.getByText("The first certificate becomes the default.",{exact:false})).toBeVisible();
  await page.getByRole("button",{name:"Add certificate",exact:true}).click();await page.getByLabel("Import",{exact:true}).check();await page.getByLabel("Signing seed",{exact:true}).fill(seed);
  await page.getByRole("button",{name:"Cancel",exact:true}).click();await page.getByRole("button",{name:"Add certificate",exact:true}).click();await page.getByLabel("Import",{exact:true}).check();await expect(page.getByLabel("Signing seed",{exact:true})).toHaveValue("");
  await page.getByLabel("Certificate name",{exact:true}).fill("Team signing");await page.getByLabel("Key ID",{exact:true}).fill("team-key");await page.getByLabel("Signing seed",{exact:true}).fill(seed);
  await request.post("http://127.0.0.1:50062/reject-connection-mutation");await page.getByRole("button",{name:"Save certificate"}).click();await expect(page.getByRole("alert")).toBeVisible();await expect(page.getByLabel("Signing seed",{exact:true})).toHaveValue("");
  await page.getByLabel("Signing seed",{exact:true}).fill(seed);await page.getByRole("button",{name:"Save certificate"}).click();await expect(page.getByRole("dialog")).toBeHidden();await expect(page.locator("body")).not.toContainText(seed);
  await page.getByRole("region",{name:"Team signing"}).getByRole("button",{name:"Edit",exact:true}).click();await expect(page.getByRole("button",{name:"Disable",exact:true})).toBeDisabled();await expect(page.getByRole("button",{name:"Delete",exact:true})).toBeDisabled();await expect(page.getByText("[environment.certificate.trusted_keys]",{exact:false})).toBeVisible();
  await page.getByLabel("Import",{exact:true}).check();await page.getByLabel("New signing seed").fill(seed);await request.post("http://127.0.0.1:50062/reject-connection-mutation");await page.getByRole("button",{name:"Replace signing key"}).click();await expect(page.getByRole("alert")).toBeVisible();await expect(page.getByLabel("New signing seed")).toHaveValue("");
  await page.getByLabel("New signing seed").fill(seed);await page.getByRole("button",{name:"Replace signing key"}).click();await expect(page.getByLabel("New signing seed")).toHaveValue("");await expect(page.getByText("team-key / 2",{exact:true})).toBeVisible();
});
test("generated certificates support default changes, renaming and unused deletion",async({page})=>{
  await create(page);await create(page,"Next signing");await page.getByRole("region",{name:"Next signing"}).getByRole("button",{name:"Edit",exact:true}).click();await page.getByRole("button",{name:"Make default"}).click();await expect(page.getByRole("button",{name:"Disable",exact:true})).toBeDisabled();await page.keyboard.press("Escape");
  await page.getByRole("region",{name:"Team signing"}).getByRole("button",{name:"Edit",exact:true}).click();await page.getByLabel("Certificate name",{exact:true}).fill("Renamed signing");await page.getByRole("button",{name:"Save name"}).click();await expect(page.getByRole("region",{name:"Renamed signing"})).toBeVisible();await page.getByRole("button",{name:"Disable",exact:true}).click();await expect(page.getByRole("button",{name:"Enable",exact:true})).toBeVisible();await page.getByRole("button",{name:"Enable",exact:true}).click();await page.getByRole("button",{name:"Delete",exact:true}).click();await expect(page.getByRole("dialog")).toBeHidden();await expect(page.getByRole("region",{name:"Renamed signing"})).toHaveCount(0);
});
test("administrator certificates expose read-only public details",async({page,request})=>{
  await request.post("http://127.0.0.1:50062/admin-certificate");await page.goto("/certificates");await page.getByRole("button",{name:"Details",exact:true}).click();await expect(page.getByRole("heading",{name:"Certificate details"})).toBeVisible();await expect(page.getByText("Source Secret: admin-signing")).toBeVisible();await expect(page.getByRole("button",{name:"Replace signing key"})).toHaveCount(0);await expect(page.getByLabel("New signing seed")).toHaveCount(0);
});
test("Ayni runtime follows default and accepts explicit certificate selection; custom runtimes hide it",async({page,request})=>{
  await create(page);await create(page,"Next signing");await page.goto("/repositories/repository%3A1");await page.getByRole("button",{name:"Edit",exact:true}).click();await page.getByLabel("Signing certificate",{exact:true}).click();await page.getByRole("option",{name:"Next signing",exact:true}).click();await page.getByRole("button",{name:"Save certificate",exact:true}).click();await expect(page.getByText("Runtime certificate updated.",{exact:false})).toBeVisible();await page.keyboard.press("Escape");await expect(page.getByText("Next signing · revision 1",{exact:true})).toBeVisible();
  await request.post("http://127.0.0.1:50062/custom-profile");await page.reload();await page.getByRole("button",{name:"Edit",exact:true}).click();await expect(page.getByLabel("Signing certificate",{exact:true})).toHaveCount(0);
});
