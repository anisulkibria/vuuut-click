/** Server-side API client. Never expose an integration credential in browser code. */
export type Period=1|7|30|90;
export type Breakdown={label:string;count:number};
export type Overview={pageviews:number;visitors:number;sessions:number;realtime:number;bounce_rate:number;timeseries:Breakdown[]};
export type FunnelStep={kind:'page'|'page_prefix'|'event';value:string};
export type Funnel={id:string;name:string;window_seconds:number;scope:'session'|'visitor';steps:FunnelStep[]};
export type FunnelReport=Omit<Funnel,'steps'>&{steps:(FunnelStep&{sessions:number;participants:number;conversion_rate:number;drop_off:number})[]};
export type Journey={session_id:string;country:string;events:number;steps:[number,string,string][]};
export type Vital={metric:string;samples:number;p50:number;p75:number;p95:number};
export type Revenue={label:string;currency:string;total_minor:string;payments:number;dimension_present:0|1};
export type BackendEvent={external_id:string;kind:'payment'|'refund'|'member_signup'|'subscription_started'|'subscription_cancelled'|'newsletter_sent'|'newsletter_open'|'newsletter_click';session_id?:string;visitor_id?:string;path?:string;amount_minor?:number;currency?:string;occurred_at_ms?:number;newsletter_id?:string;recipient_id?:string};
export class ClickApi {
 private base:URL;
 constructor(base:string,private key:string){this.base=new URL(base);}
 private async request<T>(site:string,suffix:string,params:Record<string,string|number|undefined>={},body?:unknown):Promise<T>{
  const url=new URL(`/v1/websites/${encodeURIComponent(site)}/${suffix}`,this.base);
  for(const [k,v]of Object.entries(params))if(v!==undefined)url.searchParams.set(k,String(v));
  const response=await fetch(url,{method:body?'POST':'GET',redirect:'error',headers:{Authorization:`Bearer ${this.key}`,...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined,signal:AbortSignal.timeout(15000)});
  if(!response.ok)throw new Error(`Vuuut Click request failed (${response.status})`);
  return response.json() as Promise<T>;
 }
 overview(site:string,days:Period=7){return this.request<Overview>(site,'overview',{days});}
 breakdown(site:string,dimension:'countries'|'pages'|'referrers'|'campaigns'|'devices'|'browsers'|'events'|'operating_systems',days:Period=7){return this.request<Breakdown[]>(site,`breakdown/${dimension}`,{days});}
 funnels(site:string){return this.request<Funnel[]>(site,'funnels');}
 funnel(site:string,id:string,days:Period=7){return this.request<FunnelReport>(site,`funnels/${encodeURIComponent(id)}`,{days});}
 journeys(site:string,days:Period=7){return this.request<Journey[]>(site,'journeys',{days});}
 vitals(site:string,days:Period=7){return this.request<Vital[]>(site,'vitals',{days});}
 revenue(site:string,days:Period=7,model:'first_touch'|'last_touch'|'last_non_direct'='last_non_direct',dimension:'source'|'campaign'|'landing'|'medium'|'channel'|'country'|'device'='source'){return this.request<Revenue[]>(site,'revenue',{days,model,dimension});}
 comparison(site:string,days:Period=7){return this.request<Comparison>(site,'comparison',{days});}
 content(site:string,days:Period=7){return this.request<ContentMetrics[]>(site,'content',{days});}
 acquisition(site:string,days:Period=7,dimension:'source'|'medium'|'campaign'|'landing'|'channel'='channel'){return this.request<AcquisitionMetrics[]>(site,'acquisition',{days,dimension});}
 retention(site:string,days:Period=30){return this.request<Retention>(site,'retention',{days});}
 vitalComparison(site:string,days:Period=7){return this.request<VitalComparison>(site,'vitals/comparison',{days});}
 newsletters(site:string,days:Period=7){return this.request<NewsletterReport>(site,'newsletters',{days});}
 campaignReturns(site:string,days:Period=7,model:'first_touch'|'last_touch'|'last_non_direct'='last_non_direct'){return this.request<CampaignReturn[]>(site,'campaign-costs',{days,model});}
 recordCost(site:string,input:CostInput){return this.request<{id:string;revision:number}>(site,'campaign-costs',{},input);}
 events(site:string,days:Period=7,filter:{event?:string;path?:string;campaign?:string}={}){return this.request<EventDetail[]>(site,'events',{days,...filter});}
 backendEvent(site:string,event:BackendEvent){return this.request<{event_id:string;accepted:number;duplicates:number}>(site,'backend-events',{},event);}
}

export type Snapshot={pageviews:number;visitors:number;sessions:number;bounced_sessions:number;visible_ms:number;measured_sessions:number};
export type Comparison={start:number;end:number;previous_start:number;previous_end:number;current:Snapshot;previous:Snapshot};
export type ContentMetrics={path:string;pageviews:number;visitors:number;sessions:number;entries:number;exits:number;converting_sessions:number;visible_ms:number;measured_sessions:number;mean_scroll_percent:number|null};
export type AcquisitionMetrics={label:string;sessions:number;pageviews:number;visitors:number;converting_sessions:number};
export type Retention={history_start:number;identity:{new_visitors:number;returning_visitors:number;session_only_sessions:number;unclassified_sessions:number};cohorts:{week:string;visitors:number;retained:(number|null)[]}[]};
export type VitalComparison={current:Vital[];previous:Vital[];start:number;end:number;previous_start:number;previous_end:number};
export type NewsletterReport={editions:{newsletter_id:string;sent_recipients:number;opened_recipients:number;clicked_recipients:number;link_click_facts:number}[];unassigned_events:number;activity_outside_send_cohort:number};
export type CostInput={reference:string;revision:number;day:string;campaign:string;currency:string;amount_minor:number;source:string};
export type CampaignReturn={campaign:string;currency:string;net_revenue_minor:string;revenue_recorded:boolean;attribution_complete:boolean;cost_minor:string|null;covered_days:number;expected_days:number;sources:string[];roi_percent:number|null;roas:number|null};
export type EventDetail={event:string;path:string;campaign:string;count:number;sessions:number;visitors:number};
