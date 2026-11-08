import {test,expect} from '@playwright/test';
test('real browser tracker, Rust dashboard, goals, theme, mobile and logout',async({page,baseURL})=>{
 const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('/signup');
 await page.getByLabel('Workspace name').fill('Browser test studio');
 await page.getByLabel('Email address').fill(`browser-${Date.now()}@test.example`);
 await page.getByLabel('Password',{exact:true}).fill('correct-horse-battery-staple');
 await page.getByRole('button',{name:'Create your workspace'}).click();
 await expect(page).toHaveURL(/\/websites$/);
 const publicHome=await page.context().newPage();const publicResponse=await publicHome.goto('/');
 expect(publicResponse?.status()).toBe(200);await expect(publicHome.getByRole('heading',{level:1})).toContainText('Behind every');
 expect((await page.request.get('/v1/me')).status()).toBe(200);await publicHome.close();

 await page.getByLabel('Website name').fill('Browser test publication');
 await page.getByLabel('Website origin').fill(baseURL!);
 await page.getByRole('button',{name:'Add website',exact:true}).click();
 await expect(page.getByText('Verified',{exact:true})).toBeVisible();
 const response=await page.request.get('/v1/websites');const websites=await response.json();const website=websites[0];
 const collected=page.waitForResponse(r=>r.url().includes('/v1/collect/')&&r.request().method()==='POST');
 await page.getByRole('link',{name:'Open real tracker test page'}).click();
 expect((await collected).status()).toBe(202);
 await page.waitForFunction(()=>!!window.vuuutClick);
 await page.evaluate(async()=>{window.vuuutClick!.track('newsletter_signup');await window.vuuutClick!.flush();});
 await expect.poll(async()=>{const r=await page.request.get(`/v1/websites/${website.id}/overview`);return(await r.json()).pageviews;},{timeout:15000}).toBe(1);
 await page.goto(`/dashboard?site=${website.id}`);
 await expect(page.getByRole('heading',{name:'The bigger picture.'})).toBeVisible();
 await expect(page.getByText('Unknown country',{exact:true})).toBeVisible();
 await expect(page.locator('.metric').filter({hasText:'Page views'}).locator('strong')).toHaveText('1');
 await page.screenshot({path:'test-results/dashboard-desktop.png',fullPage:true});
 await page.getByRole('link',{name:'Events & goals'}).click();
 await page.getByLabel('Goal name').fill('Newsletter signup');
 await page.getByLabel('Event name',{exact:true}).fill('newsletter_signup');
 await page.getByRole('button',{name:'Create goal'}).click();
 await expect(page.getByRole('cell',{name:'Newsletter signup',exact:true})).toBeVisible();
 await expect(page.getByRole('row').filter({has:page.getByRole('cell',{name:'Newsletter signup',exact:true})})).toContainText('1');

 // Editing a goal in place. The name and event stay readable table cells;
 // the fields live in a disclosure, so they are scoped to it rather than to
 // the create form above, which carries the same labels.
 const goalEdit=page.locator('details.row-edit').filter({hasText:'Newsletter signup'});
 await goalEdit.locator('summary').click();
 await goalEdit.getByLabel('Goal name').fill('Newsletter subscribers');
 await goalEdit.getByLabel('Event name',{exact:true}).fill('newsletter_subscribed');
 await goalEdit.getByRole('button',{name:'Save goal'}).click();
 await expect(page.getByRole('cell',{name:'Newsletter subscribers',exact:true})).toBeVisible();
 await expect(page.getByRole('cell',{name:'newsletter_subscribed',exact:true})).toBeVisible();

 // A funnel's steps are editable too, pre-filled with the funnel as it is.
 await page.goto(`/insights?site=${website.id}&view=funnels`);
 await page.getByLabel('Funnel name').fill('Reader to subscriber');
 await page.getByLabel('Steps, one per line').fill('page:/\nevent:newsletter_subscribed');
 await page.getByRole('button',{name:'Create funnel'}).click();
 await expect(page.getByRole('heading',{name:'Reader to subscriber'})).toBeVisible();
 const funnelEdit=page.locator('details.row-edit');
 await funnelEdit.locator('summary').click();
 await expect(funnelEdit.getByLabel('Steps, one per line')).toHaveValue('page:/\nevent:newsletter_subscribed');
 await funnelEdit.getByLabel('Funnel name').fill('Reader to member');
 await funnelEdit.getByLabel('Steps, one per line').fill('page:/\nevent:newsletter_subscribed\npage:/thanks');
 await funnelEdit.getByRole('button',{name:'Save funnel'}).click();
 await expect(page.getByRole('heading',{name:'Reader to member'})).toBeVisible();
 await expect(page.getByRole('cell',{name:'3. /thanks · page'})).toBeVisible();

 // Removing asks first, and the row goes when it is confirmed.
 page.on('dialog',d=>d.accept());
 await page.goto(`/dashboard?site=${website.id}&view=events`);
 await page.getByRole('button',{name:'Remove',exact:true}).click();
 await expect(page.getByRole('cell',{name:'Newsletter subscribers',exact:true})).toHaveCount(0);
 await expect(page.getByText('No goals yet.')).toBeVisible();

 await page.getByTitle('Dark theme',{exact:true}).click();
 await expect(page.locator('html')).toHaveClass('dark');
 await page.screenshot({path:'test-results/dashboard-dark.png',fullPage:true});
 await page.setViewportSize({width:390,height:844});
 await page.screenshot({path:'test-results/dashboard-mobile.png',fullPage:true});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
 await page.setViewportSize({width:1440,height:1000});
 await page.getByRole('button',{name:'Sign out',exact:true}).click();
 await expect(page).toHaveURL(/\/signin$/);
 expect(errors).toEqual([]);
});

test('required consent and DNT suppress collection; SPA navigation creates real page views',async({page,baseURL})=>{
 const email=`consent-${Date.now()}@test.example`;
 await page.request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email,password:'correct-horse-battery-staple'}});
 const orgs=await(await page.request.get('/v1/organizations')).json();
 const site=await(await page.request.post('/v1/websites',{headers:{Origin:baseURL!},data:{organization_id:orgs[0].id,name:'Consent site',origin:baseURL}})).json();
 await page.goto('/signin');
 await page.addScriptTag({url:'/tracker.js'});
 await page.evaluate((endpoint)=>{(window as any).testTracker=(window as any).VuuutClickSDK.createTracker({endpoint,consent:'required'});},`${baseURL}/v1/collect/${site.collection_id}`);
 await page.evaluate(async()=>{await(window as any).testTracker.flush();});
 expect((await(await page.request.get(`/v1/websites/${site.id}/overview`)).json()).pageviews).toBe(0);
 await page.evaluate(async()=>{(window as any).testTracker.consent(true);await(window as any).testTracker.flush();history.pushState({},'', '/consent-spa?token=sensitive');await(window as any).testTracker.flush();});
 await expect.poll(async()=> (await(await page.request.get(`/v1/websites/${site.id}/overview`)).json()).pageviews,{timeout:15000}).toBe(2);
 const pages=await(await page.request.get(`/v1/websites/${site.id}/breakdown/pages`)).json();
 expect(pages.some((p:any)=>p.label==='/consent-spa')).toBe(true);expect(JSON.stringify(pages)).not.toContain('sensitive');
 await page.evaluate(async()=>{Object.defineProperty(navigator,'doNotTrack',{value:'1',configurable:true});(window as any).testTracker.track('blocked');await(window as any).testTracker.flush();});
 const events=await(await page.request.get(`/v1/websites/${site.id}/breakdown/events`)).json();expect(events).toEqual([]);
 await page.evaluate(()=>{(window as any).testTracker.consent(false);(window as any).testTracker.destroy();});
});
