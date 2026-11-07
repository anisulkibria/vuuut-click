# Browser tracker

The server and dashboard are Rust. Only the browser SDK uses TypeScript/JavaScript. Checked-in bundles mean running the app requires no Node server or frontend compilation. Rebuild with `npm ci && npm run check && npm run build`. CDN/IIFE bundle is served at `/tracker.js`; ESM bundle is `tracker/dist/index.js`. Npm publishing/package signing remains a release task.

```html
<script defer src="https://analytics.example/tracker.js"
  data-website="vc_PUBLIC_COLLECTION_ID" data-consent="required"></script>
```

Call `window.vuuutClick.consent(true)` after your consent manager approves analytics. Call `.consent(false)` to clear the in-memory queue and SDK storage. Events recorded before withdrawal cannot be recalled by this call. `.track('newsletter_signup')` records a custom event. Never put personal data in event names or campaign labels. `.flush()` submits pending batches; `.destroy()` removes listeners. Explicit anonymous mode (`data-consent="anonymous"`) uses memory-only random identity; page reloads create new identities. Required-consent mode keeps session IDs in sessionStorage with 30-minute inactivity timeout. Optional `data-identity="persistent"` after consent uses first-party localStorage with 90-day expiry. Cross-tab shared-session/returning-user interpretation is limited and must be explained to customers.

SPA pushState/replaceState/popstate navigation is tracked when pathname changes. Hash/query-only changes are intentionally ignored. Only UTM source, medium, campaign labels are collected; no arbitrary query/form values. Referrers become origins before sending and hosts in storage. SDK respects Do Not Track and GPC, including after initialization. 2-second batching, stable UUID event IDs, 5 bounded retries, 100-event memory queue and best-effort beacon support. Terminal failures warn without logging payloads; navigation/beacon delivery is best effort and cannot guarantee browser/network delivery before durable acceptance. Backend acceptance is durable.

HTML, Ghost themes, WordPress, Shopify and Webflow can paste the script before `</head>` using their HTML/code injection mechanism. React/Next, Vue/Nuxt can load it once in the root layout with their script mechanism; do not remount a tracker on every component render. Consent must still be wired explicitly. For module clients, import `createTracker` from the ESM build and pass `{endpoint:'https://analytics.example/v1/collect/vc_...',consent:'required'}`. These install patterns do not imply completed native CMS/payment integrations.

A first-party proxy must forward the exact collection route, preserve the site's Origin and only set forwarding headers at an explicitly trusted proxy. Never reflect arbitrary collection origins or forward account cookies. Country lookup uses the actual peer unless the proxy network is allowlisted.

## Optional Web Vitals and backend conversion identity

Load `/vitals.js` with defer immediately after the base tracker. The optional MIT-licensed web-vitals addon measures LCP, INP, CLS, FCP and TTFB when supported; the base tracker remains dependency-free. Consent/DNT/GPC settings apply. Reports use the latest value per metric instance, so CLS/INP updates do not inflate sample counts. The addon captures the path at measurement initialization. Metric occurrence times use milliseconds with a monotonic per-page event clock for ordered funnels.

`window.vuuutClick.identity()` returns the consented random session_id/visitor_id (or null when disallowed). If needed, carry them through your own server transaction to correlate authorized backend conversion events. Never include an integration key in browser scripts. Server TypeScript API client: `tracker/src/api.ts`, bundled ESM `tracker/dist/api.js`; Rust client: `crates/click-sdk`. Call backendEvent with a stable external_id for retries. Publishing versioned SDK packages is still pending.
