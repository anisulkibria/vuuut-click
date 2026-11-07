/** Server-side API client. Never expose an integration credential in browser code. */
export type Period=1|7|30|90;
export type Breakdown={label:string;count:number};
export type Overview={pageviews:number;visitors:number;sessions:number;realtime:number;bounce_rate:number;timeseries:Breakdown[]};
export type FunnelStep={kind:'page'|'event';value:string};
export type Funnel={id:string;name:string;window_seconds:number;steps:FunnelStep[]};
export type FunnelReport=Omit<Funnel,'steps'>&{steps:(FunnelStep&{sessions:number;conversion_rate:number;drop_off:number})[]};
export type Journey={session_id:string;country:string;events:number;steps:[number,string,string][]};
export type Vital={metric:string;samples:number;p50:number;p75:number;p95:number};
export type Revenue={label:string;currency:string;total_minor:string;payments:number};
export type BackendEvent={external_id:string;kind:'payment'|'refund'|'member_signup'|'subscription_started'|'subscription_cancelled'|'newsletter_sent'|'newsletter_open'|'newsletter_click';session_id?:string;visitor_id?:string;path?:string;amount_minor?:number;currency?:string;occurred_at_ms?:number};
export class ClickApi {
 private base:URL;
 constructor(base:string,private key:string){this.base=new URL(base);}
 private async request<T>(site:string,suffix:string,params:Record<string,string|number>={},body?:unknown):Promise<T>{
  const url=new URL(`/v1/websites/${encodeURIComponent(site)}/${suffix}`,this.base);
  for(const [k,v]of Object.entries(params))url.searchParams.set(k,String(v));
  const response=await fetch(url,{method:body?'POST':'GET',redirect:'error',headers:{Authorization:`Bearer ${this.key}`,...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined,signal:AbortSignal.timeout(15000)});
  if(!response.ok)throw new Error(`Vuuut Click request failed (${response.status})`);
  return response.json() as Promise<T>;
 }
 overview(site:string,days:Period=7){return this.request<Overview>(site,'overview',{days});}
 breakdown(site:string,dimension:'countries'|'pages'|'referrers'|'campaigns'|'devices'|'browsers'|'events',days:Period=7){return this.request<Breakdown[]>(site,`breakdown/${dimension}`,{days});}
 funnels(site:string){return this.request<Funnel[]>(site,'funnels');}
 funnel(site:string,id:string,days:Period=7){return this.request<FunnelReport>(site,`funnels/${encodeURIComponent(id)}`,{days});}
 journeys(site:string,days:Period=7){return this.request<Journey[]>(site,'journeys',{days});}
 vitals(site:string,days:Period=7){return this.request<Vital[]>(site,'vitals',{days});}
 revenue(site:string,days:Period=7,model:'first_touch'|'last_touch'|'last_non_direct'='last_non_direct',dimension:'source'|'campaign'|'landing'='source'){return this.request<Revenue[]>(site,'revenue',{days,model,dimension});}
 backendEvent(site:string,event:BackendEvent){return this.request<{event_id:string;accepted:number;duplicates:number}>(site,'backend-events',{},event);}
}
