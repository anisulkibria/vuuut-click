import {test,expect} from '@playwright/test';
import {execFileSync} from 'node:child_process';

test('owner and user log in and out independently in the same browser',async({page,context,request,baseURL})=>{
 const stamp=Date.now();const password='correct-horse-battery-staple';
 const owner=`owner-${stamp}@test.example`;const user=`user-${stamp}@test.example`;
 const created=await request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email:owner,password,organization:'Owner session test'}});
 expect(created.status()).toBe(200);
 // Only the operator CLI can grant platform access. The public signup API cannot.
 execFileSync('target/debug/click-server',['grant-owner',owner],{env:process.env,stdio:'pipe'});
 const customer=await page.request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email:user,password,organization:'Customer session test'}});
 expect(customer.status()).toBe(200);
 await page.goto('/websites');await expect(page.locator('.email')).toHaveText(user);
 const ownerTab=await context.newPage();await ownerTab.goto('/owner/signin');
 await ownerTab.getByLabel('Email address').fill(owner);
 await ownerTab.getByLabel('Password',{exact:true}).fill(password);
 await ownerTab.getByRole('button',{name:'Sign in as owner',exact:true}).click();
 await expect(ownerTab).toHaveURL(/\/owner\/dashboard$/);
 await expect(ownerTab.getByRole('heading',{name:'Your platform at a glance.'})).toBeVisible();
 await expect(ownerTab.locator('.email')).toHaveText(owner);
 const publicHome=await context.newPage();const publicResponse=await publicHome.goto('/');
 expect(publicResponse?.status()).toBe(200);await expect(publicHome.getByRole('heading',{level:1})).toContainText('Behind every');
 expect((await page.request.get('/v1/me')).status()).toBe(200);expect((await ownerTab.request.get('/owner/v1/me')).status()).toBe(200);await publicHome.close();

 const sidebar=ownerTab.locator('.sidebar');await expect(sidebar).toBeVisible();
 for (const [label,path,heading] of [
  ['Overview','dashboard','Your platform at a glance.'],
  ['Workspaces','workspaces','A home for every customer.'],
  ['Users','users','The people behind your platform.'],
  ['Websites','websites','Every connected corner of the web.'],
  ['Subscriptions','subscriptions','Plans and usage, in one place.'],
  ['Event delivery','delivery','Keep the insights flowing.'],
  ['Audit log','audit','A record of what changed.'],
  ['Connections','connections','Connect your services.']
 ]) {
  await sidebar.getByRole('link',{name:label,exact:true}).click();
  await expect(ownerTab).toHaveURL(new RegExp('/owner/'+path+'$'));
  await expect(ownerTab.getByRole('heading',{name:heading,exact:true})).toBeVisible();
  await expect(sidebar.getByRole('link',{name:label,exact:true})).toHaveAttribute('aria-current','page');
 }
 await expect(ownerTab.locator('input[name="MAXMIND_LICENSE_KEY"]')).toHaveValue('');
 await expect(ownerTab.locator('input[name="SMTP_PASSWORD"]')).toHaveValue('');
 await ownerTab.getByRole('button',{name:'Generate private credential',exact:true}).click();
 await expect(ownerTab.locator('input[name="platform_key"]')).toHaveValue(/^[0-9a-f]{64}$/);
 await ownerTab.setViewportSize({width:390,height:844});
 expect(await ownerTab.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 await ownerTab.screenshot({path:'test-results/owner-connections-mobile.png',fullPage:true});
 await ownerTab.setViewportSize({width:1440,height:1000});
 await ownerTab.getByTitle('Dark theme',{exact:true}).click();
 await expect(ownerTab).toHaveURL(/\/owner\/dashboard$/);await expect(ownerTab.locator('html')).toHaveClass('dark');
 await ownerTab.screenshot({path:'test-results/owner-sidebar-dark.png',fullPage:true});
 await ownerTab.getByTitle('Light theme',{exact:true}).click();
 await ownerTab.setViewportSize({width:390,height:844});
 await sidebar.getByRole('link',{name:'Users',exact:true}).click();
 expect(await ownerTab.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 await ownerTab.screenshot({path:'test-results/owner-sidebar-mobile.png',fullPage:true});
 await ownerTab.setViewportSize({width:1440,height:1000});
 await sidebar.getByRole('link',{name:'Overview',exact:true}).click();

 await page.reload();await expect(page.locator('.email')).toHaveText(user);
 expect((await(await page.request.get('/v1/me')).json()).email).toBe(user);
 expect((await(await ownerTab.request.get('/owner/v1/me')).json()).email).toBe(owner);
 await ownerTab.getByRole('button',{name:'Sign out of owner console'}).click();
 await expect(ownerTab).toHaveURL(/\/owner\/signin$/);
 expect((await ownerTab.request.get('/owner/v1/me')).status()).toBe(401);
 await page.reload();await expect(page.locator('.email')).toHaveText(user);
 await ownerTab.getByLabel('Email address').fill(owner);await ownerTab.getByLabel('Password',{exact:true}).fill(password);
 await ownerTab.getByRole('button',{name:'Sign in as owner',exact:true}).click();
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page).toHaveURL(/\/signin$/);
 await ownerTab.reload();await expect(ownerTab.locator('.email')).toHaveText(owner);
 expect((await page.request.get('/v1/me')).status()).toBe(401);
 const ownerPublicHome=await context.newPage();expect((await ownerPublicHome.goto('/'))?.status()).toBe(200);
 await expect(ownerPublicHome.getByRole('heading',{level:1})).toContainText('Behind every');
 expect((await ownerTab.request.get('/owner/v1/me')).status()).toBe(200);await ownerPublicHome.close();

 await page.getByLabel('Email address').fill(owner);await page.getByLabel('Password',{exact:true}).fill(password);await page.getByRole('button',{name:'Sign in',exact:true}).click();
 await expect(page.getByRole('alert')).toContainText('Use the owner sign-in');
 const denied=await page.request.post('/owner/auth/login',{headers:{Origin:baseURL!},form:{email:user,password}});
 expect(denied.status()).toBe(403);
 await ownerTab.reload();await expect(ownerTab.locator('.email')).toHaveText(owner);
 const cookies=await context.cookies();const ownerCookie=cookies.find(c=>c.name==='click_owner_session');
 expect(ownerCookie?.path).toBe('/owner');expect(ownerCookie?.httpOnly).toBe(true);
 await ownerTab.screenshot({path:'test-results/owner-console.png',fullPage:true});
});
