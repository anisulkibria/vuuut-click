/** Vuuut Click browser SDK. No form values, arbitrary query strings, fingerprinting or third-party storage. */
type Event = { occurred_at_ms: number; vital?: {name:string;value:number;id?:string}; event_id: string; session_id: string; visitor_id?: string; name: string; url: string; referrer: string; utm_source: string; utm_medium: string; utm_campaign: string };
type Options = { endpoint: string; consent?: 'required' | 'anonymous'; identity?: 'session' | 'persistent' };
export function createTracker(options: Options) {
  let allowed = options.consent === 'anonymous';
  let queue: Event[] = [];
  let inFlight = false;
  let attempts = 0;
  let nextAttempt = 0;
  let session = crypto.randomUUID();
  let lastSeen = Date.now();
  let lastPath = '';
  let eventClock = 0;
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
  const track = (name: string = 'pageview', vital?: {name:string;value:number;id?:string}, locationOverride?:string) => {
    if (!allowed || privacy()) return;
    if (!/^[a-zA-Z0-9_.:-]{1,64}$/.test(name)) throw new Error('Invalid analytics event name');
    if (queue.length >= 100) { console.warn('Vuuut Click queue full; event dropped'); return; }
    const url = new URL(locationOverride || location.href);
    let referrer = '';
    try { referrer = new URL(document.referrer).origin; } catch { /* Direct visit. */ }
    const label = (key: string) => (url.searchParams.get(key) || '').slice(0,120).replace(/[\u0000-\u001f\u007f]/g,'');
    queue.push({ occurred_at_ms: eventClock = Math.max(Date.now(),eventClock+1), vital, event_id: crypto.randomUUID(), ...identity(), name, url: url.origin + url.pathname, referrer, utm_source: label('utm_source'), utm_medium: label('utm_medium'), utm_campaign: label('utm_campaign') });
  };
  const page = () => { if (allowed && !privacy() && lastPath !== location.pathname) { lastPath = location.pathname; track(); } };
  const flush = async (beacon = false): Promise<void> => {
    if (!allowed || privacy()) { queue = []; return; }
    if (!queue.length || inFlight || Date.now() < nextAttempt) return;
    const batch = queue.slice(0,50);
    const payload = JSON.stringify({ events: batch });
    if (beacon && navigator.sendBeacon && navigator.sendBeacon(options.endpoint, new Blob([payload], { type: 'text/plain' }))) { queue.splice(0,batch.length); return; }
    inFlight = true;
    try {
      const response = await fetch(options.endpoint, { method: 'POST', body: payload, headers: { 'Content-Type': 'text/plain' }, credentials: 'omit', mode: 'cors', keepalive: true });
      if (!response.ok) {
        if (response.status >= 400 && response.status < 500 && response.status !== 429) { queue.splice(0,batch.length); console.warn(`Vuuut Click rejected a batch (${response.status})`); return; }
        throw new Error('Collection unavailable');
      }
      queue.splice(0,batch.length); attempts = 0; nextAttempt = 0;
    } catch {
      attempts += 1; nextAttempt = Date.now() + Math.min(60000, 1000 * 2 ** attempts);
      if (attempts >= 5) { queue.splice(0,batch.length); attempts = 0; console.warn('Vuuut Click exhausted delivery retries'); }
    } finally { inFlight = false; }
  };
  const timer = setInterval(() => void flush(), 2000);
  const hide = () => { if (document.visibilityState === 'hidden') void flush(true); };
  const leave = () => void flush(true);
  document.addEventListener('visibilitychange',hide);
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
    consent(value: boolean) { allowed = value; if (!value) { queue = []; clearStorage(); session = crypto.randomUUID(); lastPath = ''; } else page(); },
    destroy() { clearInterval(timer); document.removeEventListener('visibilitychange',hide); removeEventListener('pagehide',leave); removeEventListener('popstate',page); if (history.pushState === pushState) history.pushState = originals.pushState; if (history.replaceState === replaceState) history.replaceState = originals.replaceState; queue = []; }
  };
}
declare global { interface Window { vuuutClick?: ReturnType<typeof createTracker> } }
const script = typeof document !== 'undefined' ? document.currentScript as HTMLScriptElement | null : null;
if (script?.dataset.website) {
  const endpoint = new URL(`/v1/collect/${encodeURIComponent(script.dataset.website)}`,script.src).href;
  window.vuuutClick = createTracker({ endpoint, consent: script.dataset.consent === 'anonymous' ? 'anonymous' : 'required', identity: script.dataset.identity === 'persistent' ? 'persistent' : 'session' });
}
