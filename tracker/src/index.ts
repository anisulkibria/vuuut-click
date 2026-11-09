/** Vuuut Click browser SDK. No form values, arbitrary query strings, fingerprinting or third-party storage. */
type Engagement = {duration_ms:number;scroll_percent:number};
type Event = { engagement?: Engagement; occurred_at_ms: number; vital?: {name:string;value:number;id?:string}; event_id: string; session_id: string; visitor_id?: string; name: string; url: string; referrer: string; utm_source: string; utm_medium: string; utm_campaign: string };
type Options = { endpoint: string; consent?: 'required' | 'anonymous'; identity?: 'session' | 'persistent'; engagement?: boolean };
export function createTracker(options: Options) {
  let allowed = options.consent === 'anonymous';
  let queue: Event[] = [];
  let inFlight: Promise<void> | undefined;
  let activeCount = 0;
  let consentEpoch = 0;
  let attempts = 0;
  let nextAttempt = 0;
  let session = crypto.randomUUID();
  let lastSeen = Date.now();
  let lastPath = '';
  let eventClock = 0;
  let measuredAt = performance.now();
  let measuredPage = '';
  let wasVisible = document.visibilityState === 'visible';
  let depth = 0;
  let sentDepth = 0;
  const key = `vuuut-click:${options.endpoint}`;
  const privacy = () => navigator.doNotTrack === '1' || (navigator as Navigator & { globalPrivacyControl?: boolean }).globalPrivacyControl === true;
  const clearStorage = () => { try { sessionStorage.removeItem(key); localStorage.removeItem(`${key}:visitor`); } catch { /* Storage may be disabled; memory mode still works. */ } };
  const identity = () => {
    if (Date.now() - lastSeen > 30 * 60 * 1000) session = crypto.randomUUID();
    lastSeen = Date.now();
    let visitor: string | undefined;
    if (options.consent !== 'anonymous') {
      try {
        const saved = JSON.parse(sessionStorage.getItem(key) || 'null') as { id: string; seen: number } | null;
        if (saved && /^[0-9a-f-]{36}$/.test(saved.id) && Date.now() - saved.seen < 30 * 60 * 1000) session = saved.id as typeof session;
        sessionStorage.setItem(key, JSON.stringify({ id: session, seen: lastSeen }));
        if (options.identity === 'persistent') {
          const raw = JSON.parse(localStorage.getItem(`${key}:visitor`) || 'null') as { id: string; expires: number } | null;
          visitor = raw && raw.expires > Date.now() && /^[0-9a-f-]{36}$/.test(raw.id) ? raw.id : crypto.randomUUID();
          localStorage.setItem(`${key}:visitor`, JSON.stringify({ id: visitor, expires: raw && raw.expires > Date.now() ? raw.expires : Date.now() + 90 * 86400000 }));
        }
      } catch { /* Fall back to random memory identifiers; never fingerprint. */ }
    }
    return { session_id: session, visitor_id: visitor };
  };
  const track = (name: string = 'pageview', vital?: {name:string;value:number;id?:string}, locationOverride?:string, engagement?:Engagement) => {
    if (!allowed || privacy()) return;
    if (!/^[a-zA-Z0-9_.:-]{1,64}$/.test(name)) throw new Error('Invalid analytics event name');
    if (queue.length + activeCount >= 100) { console.warn('Vuuut Click queue full; event dropped'); return; }
    const url = new URL(locationOverride || location.href);
    let referrer = '';
    try { referrer = new URL(document.referrer).origin; } catch { /* Direct visit. */ }
    const label = (key: string) => {
      const raw = (url.searchParams.get(key) || '').slice(0,120).replace(/[\u0000-\u001f\u007f-\u009f]/g,'');
      let output = '', bytes = 0;
      for (const character of raw) { const size = new TextEncoder().encode(character).length; if (bytes + size > 120) break; output += character; bytes += size; }
      return output;
    };
    queue.push({ engagement, occurred_at_ms: eventClock = Math.max(Date.now(),eventClock+1), vital, event_id: crypto.randomUUID(), ...identity(), name, url: url.origin + url.pathname, referrer, utm_source: label('utm_source'), utm_medium: label('utm_medium'), utm_campaign: label('utm_campaign') });
  };
  const recordDepth = () => {
    if (!options.engagement || !allowed || privacy() || document.visibilityState !== 'visible' || !measuredPage) return;
    const height = Math.max(document.documentElement.scrollHeight, innerHeight, 1);
    depth = Math.max(depth, Math.min(100, Math.round((scrollY + innerHeight) / height * 100)));
  };
  const measure = () => {
    const now = performance.now();
    const duration = Math.min(30000, Math.max(0, Math.round(now - measuredAt)));
    if (options.engagement && allowed && !privacy() && wasVisible && measuredPage) {
      recordDepth();
      if (duration || depth > sentDepth) {track('web_engagement', undefined, measuredPage, {duration_ms:duration,scroll_percent:depth});sentDepth=depth;}
    }
    measuredAt = now;
    wasVisible = document.visibilityState === 'visible';
  };
  const page = () => {
    if (allowed && !privacy() && lastPath !== location.pathname) {
      measure(); depth = 0; sentDepth = 0; lastPath = location.pathname; measuredPage = location.origin + location.pathname;
      measuredAt = performance.now(); track();
    }
  };
  const flush = async (beacon = false): Promise<void> => {
    if (!allowed || privacy()) { queue = []; return; }
    if (beacon && queue.length && navigator.sendBeacon) {
      let batch = queue.slice(0,50), payload = JSON.stringify({events:batch});
      while (batch.length > 1 && new TextEncoder().encode(payload).length > 60000) { batch = batch.slice(0,Math.ceil(batch.length/2)); payload = JSON.stringify({events:batch}); }
      if (navigator.sendBeacon(options.endpoint,new Blob([payload],{type:'text/plain'}))) {queue.splice(0,batch.length);return;}
    }
    if (inFlight) {await inFlight; return flush();}
    if (!queue.length || Date.now() < nextAttempt) return;
    let size = Math.min(50,queue.length), payload = JSON.stringify({events:queue.slice(0,size)});
    while (size > 1 && new TextEncoder().encode(payload).length > 60000) {size = Math.ceil(size/2);payload = JSON.stringify({events:queue.slice(0,size)});}
    const batch = queue.splice(0,size), epoch = consentEpoch;
    activeCount = batch.length;
    inFlight = (async () => {
      try {
        const response = await fetch(options.endpoint,{method:'POST',body:payload,headers:{'Content-Type':'text/plain'},credentials:'omit',mode:'cors',keepalive:true});
        if (!response.ok) {
          if (response.status >= 400 && response.status < 500 && response.status !== 429) {console.warn(`Vuuut Click rejected a batch (${response.status})`);return;}
          throw new Error('Collection unavailable');
        }
        if (epoch === consentEpoch) {attempts=0;nextAttempt=0;}
      } catch {
        if (allowed && !privacy() && epoch === consentEpoch) {
          attempts += 1;nextAttempt=Date.now()+Math.min(60000,1000*2**attempts);
          if (attempts < 5) queue=batch.concat(queue);
          else {attempts=0;console.warn('Vuuut Click exhausted delivery retries');}
        }
      } finally {activeCount=0;}
    })();
    try {await inFlight;} finally {inFlight=undefined;}
    if (queue.length && Date.now() >= nextAttempt) return flush();
  };
  const timer = setInterval(() => void flush(), 2000);
  const engagementTimer = options.engagement ? setInterval(measure,15000) : undefined;
  const hide = () => { measure(); if (document.visibilityState === 'hidden') void flush(true); };
  const leave = () => { measure(); void flush(true); };
  document.addEventListener('visibilitychange',hide);
  if (options.engagement) addEventListener('scroll',recordDepth,{passive:true});
  addEventListener('pagehide',leave);
  addEventListener('popstate',page);
  const originals = { pushState: history.pushState, replaceState: history.replaceState };
  const pushState: History['pushState'] = function(this: History, ...args) { originals.pushState.apply(this,args); page(); };
  const replaceState: History['replaceState'] = function(this: History, ...args) { originals.replaceState.apply(this,args); page(); };
  history.pushState = pushState; history.replaceState = replaceState;
  page();
  return {
    track, flush,
    trackVital(name: string, value: number, id?:string, page?:string) { track('web_vital', {name,value,id}, page); void flush(true); },
    identity() { return allowed && !privacy() ? { ...identity() } : null; },
    consent(value: boolean) { if (allowed && value) return; allowed = value; measuredAt = performance.now(); wasVisible = document.visibilityState === 'visible'; depth=0; sentDepth=0; if (!value) { consentEpoch += 1; attempts=0;nextAttempt=0;queue = []; clearStorage(); session = crypto.randomUUID(); lastPath = ''; measuredPage=''; } else page(); },
    destroy() { consentEpoch += 1; allowed=false; clearInterval(timer); if (engagementTimer !== undefined) clearInterval(engagementTimer); document.removeEventListener('visibilitychange',hide); removeEventListener('scroll',recordDepth); removeEventListener('pagehide',leave); removeEventListener('popstate',page); if (history.pushState === pushState) history.pushState = originals.pushState; if (history.replaceState === replaceState) history.replaceState = originals.replaceState; queue = []; }
  };
}
declare global { interface Window { vuuutClick?: ReturnType<typeof createTracker> } }
const script = typeof document !== 'undefined' ? document.currentScript as HTMLScriptElement | null : null;
if (script?.dataset.website) {
  const endpoint = new URL(`/v1/collect/${encodeURIComponent(script.dataset.website)}`,script.src).href;
  window.vuuutClick = createTracker({ endpoint, consent: script.dataset.consent === 'anonymous' ? 'anonymous' : 'required', identity: script.dataset.identity === 'persistent' ? 'persistent' : 'session', engagement: script.dataset.engagement === 'true' });
}
