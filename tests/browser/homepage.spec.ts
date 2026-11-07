import {test,expect} from '@playwright/test';

test('public homepage has working sign-in, signup, section links and accessible responsive layouts',async({page})=>{
 const response=await page.goto('/');
 expect(response?.status()).toBe(200);expect(response?.headers()['location']).toBeUndefined();
 await expect(page).toHaveURL(/\/$/);
 await expect(page.getByRole('heading',{level:1})).toContainText('Behind every');
 await expect(page.getByRole('link',{name:'Log in',exact:true})).toHaveAttribute('href','/signin');
 await expect(page.getByRole('link',{name:'Get started',exact:false}).first()).toHaveAttribute('href','/signup');
 await expect(page.getByText('Product illustration',{exact:true})).toBeVisible();
 await page.screenshot({path:'test-results/homepage-desktop.png',fullPage:true});
 await page.getByRole('link',{name:'Take a closer look'}).click();await expect(page).toHaveURL(/#features$/);
 await expect(page.locator('#features')).toBeInViewport();
 await page.getByRole('link',{name:'Good questions',exact:true}).click();await expect(page).toHaveURL(/#faq$/);
 await page.locator('summary').filter({hasText:'How does visitor privacy work?'}).click();
 await expect(page.locator('details[open]')).toContainText('random identifiers');
 await page.getByRole('link',{name:'Log in',exact:true}).click();await expect(page).toHaveURL(/\/signin$/);
 await page.goto('/');
 for(const width of [768,390,320]) {
  await page.setViewportSize({width,height:900});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`overflow at ${width}px`).toBe(true);
  await expect(page.getByRole('link',{name:'Log in',exact:true})).toBeVisible();
  await expect(page.getByRole('link',{name:'Get started',exact:false}).first()).toBeVisible();
  if(width===390)await page.screenshot({path:'test-results/homepage-mobile.png',fullPage:true});
 }
 await page.setViewportSize({width:1440,height:1000});
 await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});
 await page.screenshot({path:'test-results/homepage-dark.png',fullPage:true});
 await page.getByRole('link',{name:'Find your next insight'}).click();await expect(page).toHaveURL(/\/signup$/);
});
