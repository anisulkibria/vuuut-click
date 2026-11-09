# @vuuut/click

Vuuut Click's versioned browser tracker and server API client. Install a verified
release tarball with `npm install ./vuuut-click-0.1.0.tgz`; registry publication
requires the operator's npm organization and release configuration.

```ts
import { createTracker } from '@vuuut/click';
const tracker = createTracker({
  endpoint: 'https://analytics.example.com/v1/collect/PUBLIC_COLLECTION_ID',
  consent: 'required',
});
// Call from your consent manager after permission is granted:
tracker.consent(true);
```

The module accesses browser APIs when a tracker is created. Use it only in the
browser; required consent is the default. DNT/GPC suppresses collection. Opt-in
engagement and persistent random visitor IDs require an explicit site decision.
`tracker.js` is the standalone IIFE; `vitals.js` is an optional script addon that
bundles web-vitals under the license in THIRD_PARTY_NOTICES. Serve these files
from your own deployment and use the release manifest's SRI values when pinning
script contents. See the project's docs/TRACKER.md for script attributes.

Server-side report access uses a separate export:

```ts
import { ClickApi } from '@vuuut/click/api';
const click = new ClickApi('https://analytics.example.com', process.env.CLICK_READ_KEY!);
const overview = await click.overview('WEBSITE_UUID', 7);
```

Keep read and integration credentials on the server. Backend events require an
integration credential and a stable `external_id` and `kind` on every retry.
The client rejects redirects and uses a 15-second deadline. Reports cover basic
traffic, comparisons, engagement, retention, campaigns, newsletters and vitals;
the complete HTTP contract is shipped in the operator bundle's OpenAPI document.

Verify the release signature against an independently obtained public key before
installing. A public key included with an untrusted download is not a trust root.
