import {test,expect} from '@playwright/test';
import {execFileSync} from 'node:child_process';

test('demo reports are distinct and workspace workflows work in the browser',async({page,baseURL})=>{
 const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
 const email=`saas-${Date.now()}@test.example`;
 await page.goto('/signup');
 await page.getByLabel('Workspace name').fill('Browser SaaS studio');
 await page.getByLabel('Email address').fill(email);
 await page.getByLabel('Password',{exact:true}).fill('correct-horse-battery-staple');
 await page.getByRole('button',{name:'Create your workspace'}).click();
 await expect(page).toHaveURL(/\/websites$/);
 await page.getByLabel('Website name').fill('Live browser site');
 await page.getByLabel('Website origin').fill(baseURL!);
 await page.getByRole('button',{name:'Add website',exact:true}).click();
 const orgs=await(await page.request.get('/v1/organizations')).json();const org=orgs[0].id;
 execFileSync('./target/debug/click-server',['seed-demo',email],{stdio:'pipe'});
 const sites=await(await page.request.get('/v1/websites')).json();const demo=sites.find((s:any)=>s.is_demo);const live=sites.find((s:any)=>!s.is_demo);
 await page.goto('/dashboard');await expect(page.getByText('Demo data',{exact:true})).toBeVisible();
 expect(await page.locator('.metric').filter({hasText:'Page views'}).locator('strong').innerText()).not.toBe('0');
 const reports=[['audience','Meet your audience.','Around the world'],['acquisition','See what brings them here.','Your campaigns'],['content','Stories that find their people.','Pages worth a visit'],['events','Small actions. Real progress.','Your goals']];
 for(const [view,title,panel] of reports){
  const r=await page.goto(`/dashboard?site=${demo.id}&view=${view}`);expect(r?.status()).toBe(200);
  await expect(page.getByRole('heading',{name:title,exact:true})).toBeVisible();await expect(page.getByRole('heading',{name:panel,exact:true})).toBeVisible();
  await expect(page.locator('.traffic-chart')).toHaveCount(0);
  expect(await page.getByRole('row').count()).toBeGreaterThan(2);
 }
 for(const view of ['funnels','journeys','revenue','performance']){
  const r=await page.goto(`/insights?site=${demo.id}&view=${view}`);expect(r?.status()).toBe(200);
  await expect(page.getByText('Demo data',{exact:true})).toBeVisible();
  if(view==='funnels'){await expect(page.getByRole('heading',{name:'From visitor to trial'})).toBeVisible();const counts=await page.locator('tbody tr td:nth-child(2)').allTextContents();expect(counts.length).toBe(3);expect(counts.every(n=>Number(n)>0)).toBe(true);}
  if(view==='journeys')expect(await page.locator('.journey-list li').count()).toBeGreaterThan(0);
  if(view==='revenue')await expect(page.getByRole('cell',{name:/USD \d+/}).first()).toBeVisible();
  if(view==='performance')await expect(page.getByRole('cell',{name:'LCP',exact:true})).toBeVisible();
  await page.screenshot({path:`test-results/demo-${view}.png`,fullPage:true});
 }
 await page.goto(`/settings?tab=team&org=${org}`);
 await page.getByLabel('Teammate email').fill(`invited-${Date.now()}@test.example`);
 await page.getByRole('combobox',{name:/Team role/}).selectOption('analyst');
 await page.getByRole('button',{name:'Create invitation'}).click();
 await expect(page.getByRole('heading',{name:'Save this securely'})).toBeVisible();await expect(page.locator('pre')).toContainText('/invite#');
 await page.getByRole('link',{name:'Continue',exact:true}).click();
 await page.getByRole('button',{name:'Revoke invitation'}).click();
 await expect(page.getByRole('button',{name:'Revoke invitation'})).toHaveCount(0);
 await page.goto(`/settings?tab=keys&org=${org}&site=${live.id}`);
 await page.getByLabel('Credential name').fill('Browser integration');await page.getByRole('combobox',{name:/Permission/}).selectOption('integration');
 await page.getByRole('button',{name:'Create credential'}).click();await expect(page.getByRole('heading',{name:'Save this securely'})).toBeVisible();
 await page.getByRole('link',{name:'Continue',exact:true}).click();await expect(page.getByRole('cell',{name:'Browser integration',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Rotate',exact:true}).click();await expect(page.getByRole('heading',{name:'Save this securely'})).toBeVisible();
 await page.goto(`/settings?tab=billing&org=${org}`);await expect(page.getByRole('heading',{name:'Development billing mode'})).toBeVisible();
 await page.getByRole('combobox',{name:/Test plan/}).selectOption('pro');await page.getByRole('button',{name:'Apply test plan'}).click();await expect(page.locator('.settings-stat-grid')).toContainText('pro');
 await page.goto(`/settings?tab=privacy&org=${org}&site=${live.id}`);await page.getByRole('button',{name:'Create export'}).click();
 await expect(page).toHaveURL(new RegExp(`/settings\\?tab=privacy&org=${org}&site=${live.id}$`));
 await expect.poll(async()=>{await page.reload();return page.getByRole('link',{name:'Download CSV'}).count();},{timeout:15000}).toBe(1);
 expect((await(await page.request.get(`/v1/websites/${live.id}/exports`)).json()).length).toBe(1);
 const download=page.waitForEvent('download');await page.getByRole('link',{name:'Download CSV'}).click();expect((await download).suggestedFilename()).toMatch(/\.csv$/);
 for(const tab of ['workspaces','team','websites','keys','billing','privacy','account']){
  await page.goto(`/settings?tab=${tab}&org=${org}&site=${live.id}`);expect(await page.locator('h1').count()).toBe(1);
  await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
  if(tab==='team')await page.screenshot({path:'test-results/settings-team-mobile.png',fullPage:true});
  await page.setViewportSize({width:1440,height:1000});
 }
 expect(errors).toEqual([]);
});
