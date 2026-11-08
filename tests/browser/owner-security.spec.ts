import {test as base, expect} from '@playwright/test';
import {spawn, execFileSync} from 'node:child_process';
import {createServer} from 'node:net';
import {createHmac} from 'node:crypto';

// A separate local server keeps this authentication-heavy scenario inside the
// real per-IP allowance, independently of the other specs' shared server.
const test=base.extend({baseURL:async({},use)=>{
 const port=await new Promise<number>((resolve,reject)=>{const server=createServer();server.on('error',reject);server.listen(0,'127.0.0.1',()=>{const address=server.address() as {port:number};server.close(()=>resolve(address.port));});});
 const url=`http://localhost:${port}`;
 const server=spawn('./target/debug/click-server',['serve'],{env:{...process.env,BIND:`127.0.0.1:${port}`,PUBLIC_URL:url},stdio:'ignore'});
 try{
  await expect.poll(async()=>{try{return(await fetch(url+'/readyz')).status;}catch{return 0;}},{timeout:15000}).toBe(200);
  await use(url);
 }finally{server.kill('SIGTERM');await new Promise<void>(resolve=>{if(server.exitCode!==null)resolve();else server.once('exit',()=>resolve());});}
}});
function authenticatorCode(secret:string):string {
 const alphabet='ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';let bits='';
 for(const char of secret)bits+=alphabet.indexOf(char).toString(2).padStart(5,'0');
 const key=Buffer.from(bits.match(/.{8}/g)!.map(b=>parseInt(b,2)));
 const counter=Buffer.alloc(8);counter.writeBigUInt64BE(BigInt(Math.floor(Date.now()/30000)));
 const hash=createHmac('sha1',key).update(counter).digest();const offset=hash[19]&15;
 return((hash.readUInt32BE(offset)&0x7fffffff)%1000000).toString().padStart(6,'0');
}
test('owner profile, authenticator, recovery codes and password work without disturbing customer login',async({page,request,baseURL})=>{
 const email=`security-browser-${Date.now()}@test.example`;const password='correct-horse-battery-staple';
 const created=await request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email,password,organization:'Owner security browser test'}});expect(created.status()).toBe(200);
 execFileSync('./target/debug/click-server',['grant-owner',email],{env:process.env,stdio:'pipe'});
 const customer=`customer-security-${Date.now()}@test.example`;
 expect((await page.request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email:customer,password,organization:'Customer security test'}})).status()).toBe(200);
 await page.goto('/owner/signin');await page.getByLabel('Email address').fill(email);await page.getByLabel('Password',{exact:true}).fill(password);await page.getByRole('button',{name:'Sign in as owner',exact:true}).click();
 await page.getByRole('link',{name:'Account & security',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Your account and security'})).toBeVisible();
 const action=(value:string)=>page.locator(`form:has(input[name="action"][value="${value}"])`);
 await action('profile').getByLabel('Display name').fill('Platform operator');await action('profile').getByRole('button',{name:'Save profile'}).click();await expect(action('profile').getByLabel('Display name')).toHaveValue('Platform operator');
 await action('enroll').getByLabel('Current password').fill(password);await action('enroll').getByRole('button',{name:'Set up two-factor authentication'}).click();
 const secret=(await page.locator('#totp-secret').textContent())!;
 await action('confirm').getByLabel('Current password').fill(password);await action('confirm').getByLabel('Authenticator or recovery code').fill(authenticatorCode(secret));await action('confirm').getByRole('button',{name:'Enable two-factor authentication'}).click();
 const initial=await page.locator('#recovery-codes code').allTextContents();expect(initial).toHaveLength(10);
 await page.getByRole('link',{name:'Return to your account'}).click();await expect(page.getByText('Two-factor authentication is enabled. Codes cannot be reused.')).toBeVisible();
 await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:'test-results/owner-security-mobile.png',fullPage:true});await page.setViewportSize({width:1440,height:1000});await page.screenshot({path:'test-results/owner-security-desktop.png',fullPage:true});
 await page.getByRole('button',{name:'Sign out of owner console'}).click();await page.getByLabel('Email address').fill(email);await page.getByLabel('Password',{exact:true}).fill(password);await page.getByRole('button',{name:'Sign in as owner',exact:true}).click();await expect(page).toHaveURL(/\/owner\/auth\/verify$/);
 expect((await page.request.get('/owner/v1/me')).status()).toBe(401);
 await page.getByLabel('Authenticator or recovery code').fill(initial[0]);await page.getByRole('button',{name:'Verify sign-in'}).click();await expect(page).toHaveURL(/\/owner\/dashboard$/);
 await page.getByRole('link',{name:'Account & security',exact:true}).click();await action('codes').getByLabel('Current password').fill(password);await action('codes').getByLabel('Authenticator or recovery code').fill(initial[1]);await action('codes').getByRole('button',{name:'Replace recovery codes'}).click();
 const codes=await page.locator('#recovery-codes code').allTextContents();expect(codes).toHaveLength(10);expect(codes).not.toEqual(initial);
 await page.getByRole('link',{name:'Return to your account'}).click();const newPassword='updated-owner-password-123';
 await action('password').getByLabel('Current password').fill(password);await action('password').getByLabel('Authenticator or recovery code').fill(codes[0]);await action('password').getByLabel('New password').fill(newPassword);await action('password').getByRole('button',{name:'Change password',exact:true}).click();await expect(page).toHaveURL(/\/owner\/account$/);
 await page.getByRole('button',{name:'Sign out of owner console'}).click();await page.getByLabel('Email address').fill(email);await page.getByLabel('Password',{exact:true}).fill(newPassword);await page.getByRole('button',{name:'Sign in as owner',exact:true}).click();await page.getByLabel('Authenticator or recovery code').fill(codes[1]);await page.getByRole('button',{name:'Verify sign-in'}).click();
 await page.getByRole('link',{name:'Account & security',exact:true}).click();await action('disable').getByLabel('Current password').fill(newPassword);await action('disable').getByLabel('Authenticator or recovery code').fill(codes[2]);await action('disable').getByRole('button',{name:'Disable two-factor authentication'}).click();await expect(action('enroll').getByRole('button')).toBeVisible();
 expect((await(await page.request.get('/v1/me')).json()).email).toBe(customer);
 expect((await(await page.request.get('/owner/v1/me')).json()).email).toBe(email);
});
