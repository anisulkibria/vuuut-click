import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_EXECUTABLE_PATH || undefined, headless: true });
try {
  const context = await browser.newContext({viewport: {width: 1440, height: 1050}});
  await context.addCookies([{name:'ghost-admin-api-session',value:process.env.NATIVE_TEST_COOKIE,url:process.env.NATIVE_TEST_CMS}]);
  const page = await context.newPage();
  await page.route('**/*',route => {
    const u = new URL(route.request().url());
    return ['127.0.0.1','localhost'].includes(u.hostname) ? route.continue() : route.abort();
  });
  if (process.argv[2] === 'collect') {
    let collected = 0;
    page.on('response',r => { if(r.url().includes('/v1/collect/') && r.status() === 202) collected++; });
    await page.goto(process.env.NATIVE_TEST_CMS, {waitUntil:'networkidle'});
    await page.waitForFunction(() => Boolean(window.vuuutClick));
    await page.evaluate(() => window.vuuutClick.flush());
    assert.equal(collected,0,'required consent blocks automatic collection');
    const response = page.waitForResponse(r => r.url().includes('/v1/collect/') && r.status() === 202);
    await page.evaluate(async () => {window.vuuutClick.consent(true);window.vuuutClick.track('native_browser_proof');await window.vuuutClick.flush();});
    await response;
  } else {
    await page.goto(`${process.env.NATIVE_TEST_CMS}/ghost/analytics/click/`,{waitUntil:'networkidle'});
    await page.getByRole('heading',{name:'Vuuut Click',exact:true}).waitFor();
    assert.match(await page.locator('body').innerText(),/native_browser_proof/);
    assert.match(await page.locator('body').innerText(),/Pageviews/);
    await page.getByRole('heading',{name:'Member growth and subscriptions',exact:true}).waitFor();
    await page.getByRole('heading',{name:'Newsletter performance',exact:true}).waitFor();
    await page.getByRole('link',{name:'/',exact:true}).click();
    await page.getByRole('heading',{name:'Selected post traffic',exact:true}).waitFor();
    const selected=page.locator('section').filter({has:page.getByRole('heading',{name:'Selected post traffic',exact:true})});
    assert.match(await selected.innerText(),/Pageviews\s+1/);
    await page.getByRole('navigation',{name:'Report period'}).getByRole('link',{name:'7 days',exact:true}).click();
    assert.equal(new URL(page.url()).searchParams.get('path'),'/');
    await page.getByRole('heading',{name:'Selected post traffic',exact:true}).waitFor();

    assert.equal(await page.locator('input[type=password]').inputValue(),'');
    const directory = process.env.NATIVE_TEST_SCREENSHOTS;
    fs.mkdirSync(directory,{recursive:true});
    await page.screenshot({path:`${directory}/native-admin-desktop.png`,fullPage:true});
    await page.setViewportSize({width:390,height:844});
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),'native reports fit a phone');
    await page.screenshot({path:`${directory}/native-admin-mobile.png`,fullPage:true});
    await page.getByRole('button',{name:'Test connection',exact:true}).click();
    await page.getByRole('status').filter({hasText:'Connection updated'}).waitFor();
  }
  await context.close();
} finally {await browser.close();}
