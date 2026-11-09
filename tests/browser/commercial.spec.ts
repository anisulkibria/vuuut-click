import {test,expect} from '@playwright/test';
import {execFileSync} from 'node:child_process';

test('commercial fulfillment stays owner-only and readable on desktop and phone',async({page,context,baseURL})=>{
 const email=`commercial-owner-${Date.now()}@test.example`;
 const customer=await page.request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email,password:'correct-horse-battery-staple'}});expect(customer.ok()).toBe(true);
 expect((await page.request.get('/owner/v1/commercial')).status()).toBe(401);
 execFileSync('target/debug/click-server',['grant-owner',email],{stdio:'pipe'});
 await page.goto('/owner/signin');await page.getByLabel('Email address').fill(email);await page.getByLabel('Password',{exact:true}).fill('correct-horse-battery-staple');await page.getByRole('button',{name:'Sign in as owner'}).click();
 await page.getByRole('link',{name:'Commercial status',exact:true}).click();await expect(page.getByRole('heading',{name:'Keep the promises behind your plans.',exact:true})).toBeVisible();
 for(const heading of ['Provider configuration','Fulfillment checks','Pending work','Recent dead mail'])await expect(page.getByRole('heading',{name:heading,exact:true})).toBeVisible();
 const report=await(await page.request.get('/owner/v1/commercial')).json();expect(report.mail_mode).toBe('local_file');expect(report.billing_mode).toBe('disabled');
 expect(report.queues.pending_mail).toBeGreaterThanOrEqual(0);expect(JSON.stringify(report)).not.toMatch(/recipient|token_hash|password|integration_key/);
 for(const width of [1440,768,390]){await page.setViewportSize({width,height:1000});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:`test-results/commercial-${width}.png`,fullPage:true});}
 const anonymous=await context.browser()!.newContext();expect((await anonymous.request.get(baseURL+'/owner/v1/commercial')).status()).toBe(401);await anonymous.close();
});


test('billing forms reach trusted Stripe hosts and reject lookalike hosts',async({page,baseURL})=>{
 const email=`billing-redirect-${Date.now()}@test.example`;
 const registration=await page.request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email,password:'correct-horse-battery-staple'}});expect(registration.ok()).toBe(true);
 const org=(await(await page.request.get('/v1/organizations')).json())[0].id;
 const cdp=await page.context().newCDPSession(page);let untrustedRequested=false;
 await cdp.send('Fetch.enable',{patterns:[{urlPattern:'https://checkout.stripe.com/*'},{urlPattern:'https://billing.stripe.com/*'},{urlPattern:'https://checkout.stripe.com.evil.invalid/*'}]});
 cdp.on('Fetch.requestPaused',async request=>{
  if(new URL(request.request.url).hostname==='checkout.stripe.com.evil.invalid')untrustedRequested=true;
  await cdp.send('Fetch.fulfillRequest',{requestId:request.requestId,responseCode:200,responseHeaders:[{name:'Content-Type',value:'text/html'}],body:Buffer.from('<h1>Hosted billing destination</h1>').toString('base64')});
 });
 for(const host of ['checkout.stripe.com','billing.stripe.com']){
  await page.route('**/settings/action',route=>route.fulfill({status:303,headers:{Location:`https://${host}/test-billing`}}));
  const settings=await page.goto(`/settings?tab=billing&org=${org}`);
  expect(settings!.headers()['content-security-policy']).toContain("form-action 'self' https://checkout.stripe.com https://billing.stripe.com;");
  await page.getByRole('button',{name:'Apply test plan',exact:true}).click();
  await expect(page).toHaveURL(`https://${host}/test-billing`);
  await expect(page.getByRole('heading',{name:'Hosted billing destination'})).toBeVisible();
  await page.unrouteAll();
 }
 await page.route('**/settings/action',route=>route.fulfill({status:303,headers:{Location:'https://checkout.stripe.com.evil.invalid/test-billing'}}));
 await page.goto(`/settings?tab=billing&org=${org}`);
 const violation=page.waitForEvent('console',message=>message.text().includes('form-action')&&message.type()==='error');
 await page.getByRole('button',{name:'Apply test plan',exact:true}).click({noWaitAfter:true});await violation;
 expect(new URL(page.url()).origin).toBe(baseURL);expect(untrustedRequested).toBe(false);
 const dashboard=await page.request.get('/dashboard');
 expect(dashboard.headers()['content-security-policy']).toContain("form-action 'self';");
 expect(dashboard.headers()['content-security-policy']).not.toContain('stripe.com');
 await cdp.detach();
});
