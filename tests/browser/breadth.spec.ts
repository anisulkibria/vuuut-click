import {test,expect} from '@playwright/test';
import {execFileSync} from 'node:child_process';

test('optional engagement measures visible time, preserves SPA paths and respects consent and DNT',async({page,baseURL})=>{
 const email=`engagement-${Date.now()}@test.example`;
 const registered=await page.request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email,password:'correct-horse-battery-staple'}});expect(registered.ok()).toBe(true);
 const orgs=await(await page.request.get('/v1/organizations')).json();
 const site=await(await page.request.post('/v1/websites',{headers:{Origin:baseURL!},data:{organization_id:orgs[0].id,name:'Engagement fixture',origin:baseURL}})).json();
 const requests:any[]=[];const statuses:number[]=[];
 page.on('request',r=>{if(r.url().includes('/v1/collect/')&&r.method()==='POST'&&r.postData())requests.push(...JSON.parse(r.postData()!).events);});
 page.on('response',r=>{if(r.url().includes('/v1/collect/')&&r.request().method()==='POST')statuses.push(r.status());});
 await page.goto('/signin');await page.clock.install();await page.clock.pauseAt(new Date(Date.now()+1000));await page.addScriptTag({url:'/tracker.js'});
 await page.evaluate(endpoint=>{history.replaceState({},'','/signin?utm_campaign='+encodeURIComponent('旅'.repeat(100))+'&private=secret');(window as any).engagementTracker=(window as any).VuuutClickSDK.createTracker({endpoint,consent:'required',identity:'persistent',engagement:true});},`${baseURL}/v1/collect/${site.collection_id}`);
 await page.clock.runFor(15000);await page.evaluate(()=>(window as any).engagementTracker.flush());expect(requests).toHaveLength(0);
 await page.evaluate(()=>{(window as any).engagementTracker.consent(true);document.body.style.minHeight='3000px';scrollTo(0,3000);dispatchEvent(new Event('scroll'));scrollTo(0,0);dispatchEvent(new Event('scroll'));});await page.clock.runFor(5000);
 const visibility=async(value:'visible'|'hidden')=>{await page.evaluate(value=>{Object.defineProperty(document,'visibilityState',{configurable:true,value});document.dispatchEvent(new Event('visibilitychange'));},value);};
 await visibility('hidden');await page.clock.runFor(20000);await visibility('visible');await page.clock.runFor(3000);
 await page.evaluate(()=>history.pushState({},'','/engaged-b'));await page.clock.runFor(2000);await visibility('hidden');
 await page.evaluate(()=>(window as any).engagementTracker.flush());
 // Chrome can omit Blob beacon bodies from request instrumentation. Verify
 // their committed content metrics, including path attribution and scroll max.
 await expect.poll(async()=>{const rows=await(await page.request.get(`/v1/websites/${site.id}/content?days=1`)).json();return rows.reduce((sum:number,r:any)=>sum+r.visible_ms,0);},{timeout:20000}).toBe(10000);
 const measured=await(await page.request.get(`/v1/websites/${site.id}/content?days=1`)).json();
 expect(measured.find((r:any)=>r.path==='/signin').visible_ms).toBe(8000);expect(measured.find((r:any)=>r.path==='/signin').mean_scroll_percent).toBe(100);expect(measured.find((r:any)=>r.path==='/engaged-b').visible_ms).toBe(2000);
 const firstView=requests.find(r=>r.name==='pageview');expect(firstView.visitor_id).toBeTruthy();expect(new TextEncoder().encode(firstView.utm_campaign).length).toBe(120);expect(JSON.stringify(requests)).not.toContain('private=secret');
 const identity=await(await page.request.get(`/v1/websites/${site.id}/retention?days=1`)).json();expect(identity.identity.new_visitors).toBe(1);expect(identity.identity.session_only_sessions).toBe(0);
 const count=requests.length;await page.evaluate(()=>Object.defineProperty(navigator,'doNotTrack',{configurable:true,value:'1'}));await visibility('visible');await page.clock.runFor(15000);await page.evaluate(()=>(window as any).engagementTracker.flush());expect(requests).toHaveLength(count);
 await page.evaluate(()=>(window as any).engagementTracker.consent(false));expect(await page.evaluate(()=>(window as any).engagementTracker.identity())).toBeNull();
 expect(await page.evaluate(()=>Object.keys(localStorage).some(k=>k.includes('vuuut-click:')))).toBe(false);
 await expect.poll(()=>statuses.length).toBeGreaterThan(0);expect(statuses.every(s=>s===202)).toBe(true);
 await page.clock.resume();
 await expect.poll(async()=>{const r=await page.request.get(`/v1/websites/${site.id}/content?days=1`);const rows=await r.json();return rows.reduce((sum:number,r:any)=>sum+r.visible_ms,0);},{timeout:20000}).toBe(10000);
 const comparison=await page.request.get(`/v1/websites/${site.id}/comparison?days=1`);expect(comparison.ok(),await comparison.text()).toBe(true);const content=await comparison.json();expect(content.current.pageviews).toBe(2);expect(content.current.sessions).toBe(1);expect(content.current.visible_ms).toBe(10000);
});

test('extended demo reports, filters, cohort cells and cost recording work on desktop and phone',async({page,baseURL})=>{
 const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
 const email=`breadth-${Date.now()}@test.example`;
 await page.request.post('/v1/auth/register',{headers:{Origin:baseURL!},data:{email,password:'correct-horse-battery-staple'}});
 execFileSync('target/debug/click-server',['seed-demo',email],{env:process.env,stdio:'pipe'});
 const sites=await(await page.request.get('/v1/websites')).json();const site=sites.find((s:any)=>s.is_demo);expect(site).toBeTruthy();
 await page.goto(`/dashboard?site=${site.id}&view=overview&days=7`);await expect(page.getByRole('heading',{name:'Compared with the previous period',exact:true})).toBeVisible();
 await page.goto(`/dashboard?site=${site.id}&view=content&days=7`);await expect(page.getByRole('heading',{name:'Entry, exit and content engagement',exact:true})).toBeVisible();
 const content=await(await page.request.get(`/v1/websites/${site.id}/content?days=7`)).json();expect(content.some((r:any)=>r.visible_ms>0&&r.measured_sessions>0)).toBe(true);
 await page.goto(`/dashboard?site=${site.id}&view=acquisition&days=7`);await page.getByLabel('Group sessions by').selectOption('medium');await page.getByRole('button',{name:'Apply grouping',exact:true}).click();await expect(page).toHaveURL(/dimension=medium/);await expect(page.getByRole('heading',{name:'Session acquisition',exact:true})).toBeVisible();
 await page.goto(`/dashboard?site=${site.id}&view=events&days=7`);await page.getByLabel('Filter event',{exact:true}).fill('trial_started');await page.getByRole('button',{name:'Filter events',exact:true}).click();
 const filtered=page.locator('section').filter({has:page.getByRole('heading',{name:'Event context and filters',exact:true})});expect(await filtered.getByRole('row').count()).toBeGreaterThan(1);await expect(filtered).not.toContainText('guide_download');
 for(const [view,heading] of [['retention','Weekly visitor retention'],['newsletters','Newsletter send cohorts'],['campaigns','Campaign costs and returns'],['performance','Web Vitals period comparison']]){
  await page.goto(`/insights?site=${site.id}&view=${view}&days=30`);await expect(page.getByRole('heading',{name:heading,exact:true})).toBeVisible();await expect(page.getByText('Demo data',{exact:true})).toBeVisible();
  if(view==='retention')await expect(page.getByText('Not complete',{exact:true}).first()).toBeVisible();
  if(view==='campaigns'){const rows=await(await page.request.get(`/v1/websites/${site.id}/campaign-costs?days=30`)).json();expect(rows.some((r:any)=>r.roi_percent!==null&&r.covered_days===30&&r.sources.includes('demo'))).toBe(true);await expect(page.getByRole('button',{name:'Save cost record',exact:true})).toHaveCount(0);}
  await page.screenshot({path:`test-results/priority3-${view}-desktop.png`,fullPage:true});await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:`test-results/priority3-${view}-mobile.png`,fullPage:true});await page.setViewportSize({width:1440,height:1000});
 }
 // Supplied cost records on a real website have no invented revenue or ROI.
 const orgs=await(await page.request.get('/v1/organizations')).json();const live=await(await page.request.post('/v1/websites',{headers:{Origin:baseURL!},data:{organization_id:orgs[0].id,name:'Cost input fixture',origin:baseURL}})).json();
 await page.goto(`/insights?site=${live.id}&view=campaigns&days=7`);
 await page.getByLabel('Campaign label').fill('Browser campaign');await page.getByLabel('Amount',{exact:true}).fill('5.25');await page.getByLabel('Source',{exact:true}).fill('browser_ledger');await page.getByLabel('Source record reference').fill('browser-cost-row');await page.getByRole('button',{name:'Save cost record',exact:true}).click();
 await expect(page.getByRole('row').filter({hasText:'Browser campaign'})).toContainText('USD 5.25');await expect(page.getByRole('row').filter({hasText:'Browser campaign'})).toContainText('No revenue recorded');
 const costs=await(await page.request.get(`/v1/websites/${live.id}/campaign-costs?days=7`)).json();expect(costs[0].cost_minor).toBe('525');expect(costs[0].roi_percent).toBeNull();expect(errors).toEqual([]);
});

test('retry batches retain event IDs and consent revocation discards in-flight retries',async({page,baseURL})=>{
 await page.goto('/signin');await page.clock.install();await page.clock.pauseAt(new Date(Date.now()+1000));await page.addScriptTag({url:'/tracker.js'});
 let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;});const batches:any[][]=[];
 await page.route('**/v1/collect/ownership-fixture',async route=>{batches.push(JSON.parse(route.request().postData()!).events);if(batches.length===1){await gate;await route.fulfill({status:503,body:'temporary outage'});}else await route.fulfill({status:202,body:'{}'});});
 await page.evaluate(endpoint=>{(window as any).retryTracker=(window as any).VuuutClickSDK.createTracker({endpoint,consent:'anonymous'});(window as any).retryTracker.track('first_batch');void(window as any).retryTracker.flush();},`${baseURL}/v1/collect/ownership-fixture`);
 await expect.poll(()=>batches.length).toBe(1);
 await page.evaluate(()=>{(window as any).retryTracker.track('queued_during_send');void(window as any).retryTracker.flush(true);});
 await expect.poll(()=>batches.length).toBe(2);expect(batches[1].map(r=>r.name)).toEqual(['queued_during_send']);
 await page.evaluate(()=>(window as any).retryTracker.consent(false));release();await page.evaluate(()=>(window as any).retryTracker.flush());await page.clock.runFor(20000);expect(batches).toHaveLength(2);await page.evaluate(()=>(window as any).retryTracker.destroy());
 const replay:any[][]=[];
 await page.route('**/v1/collect/replay-fixture',async route=>{replay.push(JSON.parse(route.request().postData()!).events);await route.fulfill({status:replay.length===1?503:202,body:'{}'});});
 await page.evaluate(endpoint=>{(window as any).replayTracker=(window as any).VuuutClickSDK.createTracker({endpoint,consent:'anonymous'});(window as any).replayTracker.track('stable_retry');return(window as any).replayTracker.flush();},`${baseURL}/v1/collect/replay-fixture`);
 expect(replay).toHaveLength(1);await page.clock.runFor(2000);await page.evaluate(()=>(window as any).replayTracker.flush());await expect.poll(()=>replay.length).toBe(2);
 expect(replay[1]).toEqual(replay[0]);await page.evaluate(()=>(window as any).replayTracker.destroy());
});
