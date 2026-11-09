import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { ClickApi } from '../../tracker/dist/api.js';

// Exercise actual wire requests; these tests also work against the packed API export.
const { ClickApi: PackedApi } = process.env.SDK_PACKAGE_URL ? await import(process.env.SDK_PACKAGE_URL) : {ClickApi};
test('server export imports without a DOM and preserves idempotent backend payloads', async () => {
  const calls=[];
  const server=createServer(async(req,res)=>{let body='';for await(const chunk of req)body+=chunk;calls.push({url:req.url,key:req.headers.authorization,body});res.setHeader('content-type','application/json');res.end(req.method==='POST'?'{"event_id":"e","accepted":1,"duplicates":0}':'{"pageviews":7}');});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  try {
    const api=new PackedApi(`http://127.0.0.1:${server.address().port}`,'fixture-key');
    assert.equal((await api.overview('site',7)).pageviews,7);
    const event={external_id:'stable-reference',kind:'payment',amount_minor:100,currency:'USD'};
    await api.backendEvent('site',event);await api.backendEvent('site',event);
    assert.equal(calls[0].url,'/v1/websites/site/overview?days=7');
    assert.equal(calls[0].key,'Bearer fixture-key');
    assert.equal(calls[1].body,calls[2].body);
    assert.deepEqual(JSON.parse(calls[1].body),event);
  } finally {await new Promise(resolve=>server.close(resolve));}
});
test('API rejects HTTP failures without exposing provider response bodies',async()=>{
  const previous=globalThis.fetch;
  globalThis.fetch=async()=>new Response('sensitive-provider-body',{status:503});
  try {await assert.rejects(new PackedApi('https://analytics.example.com','fixture-key').overview('site'),{message:'Vuuut Click request failed (503)'});}
  finally {globalThis.fetch=previous;}
});
test('API forbids redirects and provides a bounded request signal',async()=>{
  const previous=globalThis.fetch;
  globalThis.fetch=async(url,options)=>{assert.equal(options.redirect,'error');assert.ok(options.signal instanceof AbortSignal);return Response.json([]);};
  try {assert.deepEqual(await new PackedApi('https://analytics.example.com','fixture-key').events('site',7,{path:'/story'}),[]);}
  finally {globalThis.fetch=previous;}
});
