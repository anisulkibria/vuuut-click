# Native Vuuut / Vuuut-pro integration (v1)

Implemented across the three sibling repositories. Click remains independently
buildable and deployable; neither hosting nor a CMS is required for generic
websites. The optional three-repository test is outside the main workspace.

## Standalone CMS

1. Start Click at your own HTTPS origin (hosted or self-hosted). Register a
   customer, create the website with the CMS's exact public origin and verify
   it. Development permits localhost and managed `.localhost` origins.
2. In Click's Workspace & settings, issue a **website integration** credential.
3. In Vuuut Admin, Analytics → **Vuuut Click reports & connection**
   (`/ghost/analytics/click/`), the site owner enters the Click origin, website
   UUID and integration credential. Connect validates the server, verifies the
   origin and registers the CMS's public signing keys before saving anything.
4. The same page renders real overview, daily traffic, selected-post traffic, pages, sources,
   countries, devices, member/subscription/newsletter activity and currency-separated net revenue. Current CMS membership/subscription counts are labeled separately from period events. Post analytics link through the existing CMS permalink resolver; slash aliases share a post report. Dedicated exact native activity counts avoid top-list truncation.
   Administrator/Owner roles can report; only Owner can change the connector.
   Errors and empty reports are explicit. Existing local analytics remain
   available separately.

The credential lives in private `click_connection`, never in general settings,
public API responses, themes or report HTML. Rotation persists a random
replacement **before** sending it. A lost response is recovered by retrying the
replacement, then the previous credential. Disconnect revokes the credential
and staff access remotely, stops capture and clears queued deliveries locally.
Disconnect the old website before connecting another; queued records cannot be
retargeted to an unrelated website. A failed remote disconnect remains visibly
incomplete and can be retried.

`web_analytics` controls tracker injection through `ghost_foot`. Only Click's
public collection ID and server origin enter the theme context. Required
consent is the default; the site's consent UI calls the existing
`window.vuuutClick.consent(true)` API. Explicit anonymous mode is optional;
DNT/GPC remain respected by the tracker. No integration/platform key or staff
access token is delivered to the browser.

## Managed provisioning

An operator registers a platform at Click. Supply `NATIVE_PLATFORM_TOKEN`
privately in the environment (random, at least 32 characters), then run:

```sh
click-server native-platform vuuut-pro sites.example.com
```

Only the token hash is stored. The suffix is the operator's explicit delegated
DNS authority: exact host or dot-boundary subdomains can be verified on
provisioning. An unrelated custom domain is **never** auto-verified. Registering
again rotates the platform credential and revokes existing tenant keys/sessions;
restart the hosting worker to reconcile all tenants at higher revisions. Run
`click-server native-platform-disable vuuut-pro` to block provisioning,
collection, new exchanges and existing native access immediately. This operator
command does not erase data.

Configure the hosting platform's private `pro.json` (or `PRO_CLICK__*` variables):

```json
{
  "click": {
    "baseUrl": "https://click.example.com",
    "platformKey": "<private-operator-issued-platform-credential>",
    "defaultPlan": "free",
    "planMap": {"creator": "pro", "team": "pro", "business": "pro"},
    "consentRequired": true
  }
}
```

No Click block means the service is optional and no remote requests occur.
Plans map explicitly to Click's existing entitlement catalogue; hosting payment
providers never create analytics revenue from plan prices. Platform credentials
stay in the hosting process; only website-scoped keys enter private tenant
`hostSettings.click`. Downloadable hosting backups remove that block.

The hosting migrations add `click_sync` and transactional triggers for tenant
status/name/plan and primary-domain changes. Traffic/usage touches do not
continually rotate credentials. New sites synchronize after successful CMS
setup, once their state is active. Restoration after terminal erasure creates a fresh mapping under a new persisted incarnation; the old tombstone and its erased data remain terminal. The CMS remembers its queue target and clears old-target records before activating the new mapping. Pending deletion pauses analytics throughout
the grace period; actual deletion queues Click's durable whole-workspace erasure.
A terminal mapping tombstone prevents an old retry from recreating an erased
site. Suspension/rotation revoke old keys and delegated sessions.

The worker runs independently of provisioning/backups, keeps exact request
snapshots across uncertain responses, and uses monotonically increasing
revisions. Same revision + same body returns the same mapping/credential;
conflicting or stale revisions return 409. Tenant keys are derived with HMAC
from the authenticated platform credential, website and revision, making a lost
provisioning response recoverable without storing plaintext keys in Click.
Only the corresponding hashed key can authorize access.

Retries persist attempts/backoff; twenty failed deliveries leave visible dead
state. Per-site hosting controls provide retry, rotation, disconnect and
reconnect. The site detail page shows sync state, applied/current revision and
custom-domain verification instructions. A custom domain requires the exact
`_vuuut-click.<hostname>` TXT challenge; the worker retries DNS verification
without losing the website mapping. Collection/report access remain blocked
until verified. Local HTTP is confined to explicitly enabled development.

## Staff exchange

The ordinary Vuuut identity token retains its original site audience. Click
uses a separate RS256 assertion signed by the CMS's identity key, containing:

- Exact registered issuer `<website-origin>/ghost/api/admin/`.
- Dedicated audience `<click-origin>/native/v1`.
- Internal staff subject, explicit Owner/Administrator role and website UUID.
- Issued/expiry times (CMS emits 60 seconds; receiver caps at 300), unique jti.

An authenticated connector pins 1–4 RSA public JWKs (at least 2048 bits, unique
kid). Click selects only a pinned key, validates algorithm/signature, issuer,
audience, role, website and time bounds, and atomically consumes the assertion.
There is no URL discovery, JWKS fetch, arbitrary issuer access or SSRF surface
in the exchange. Re-registration supports CMS signing-key rotation and revokes
old access when the key set changes. Identical retries do not revoke access.

The exchange additionally requires the current website integration key. Its
result is an opaque, hashed, **60-second, website-read-only** bearer token. It
cannot create a customer/owner session, enumerate workspaces, write settings,
create goals or submit backend revenue. The CMS exchanges it and fetches reports
server-side for an authenticated current staff member; the bearer never enters
HTML. Connection generation/key revocation, origin changes, hosted suspension,
workspace suspension, disconnect and platform disable block further access.

## Native Admin reports

`/ghost/analytics/click/` includes website traffic, daily traffic, pages, sources,
countries, devices, events and currency-separated net revenue. Member growth,
subscription activations/cancellations and newsletter sent/open/click facts use
`GET /v1/websites/{id}/native-report`; exact authoritative counts are independent
of the general top-100 event list. Current CMS member/paid-member/active-or-trial
subscription counts are explicitly separate from the selected time period.

Existing post analytics link through the CMS permalink resolver into a selected
post report. Its optional `path` query returns replay-safe pageviews, visitors
and sessions for both slash forms. Other sections remain website-wide. The read
authorization, tenant isolation and report budgets match the other report APIs.
Richer cohorts, per-newsletter analysis and advanced attribution remain Priority 3.

## Transactional CMS events and privacy

SQLite/MySQL source-table triggers create outbox rows in the **same transaction**
as the durable member/subscription/payment/newsletter fact. Rolled-back source
writes cannot emit events. Capture begins on connection/worker startup; old
history is not silently backfilled. Source events include member signup,
subscription activation/cancellation, payment/refund, successfully sent
newsletter recipient, first open and unique link/recipient click.

Provider payment records now use stable hashed provider object IDs. Recurring
payments come from paid invoices (including the first), one-off payments from
checkout, and successful individual refunds from the signed cumulative
`charge.refunded` payload. Invoice aliases, webhook redelivery and cumulative
refund replay cannot inflate native totals. Existing legacy payment rows are
not rewritten. No browser event can provide money or entitlement changes. New CMS Stripe webhooks subscribe to invoice payments and refunds. For an existing endpoint, native Admin offers an explicit owner action to update its subscribed event list without rotating the signing secret. Truncated refund lists are fetched with bounded provider pagination before acknowledgement; live provider validation remains an external release gate.

The worker sends raw-body HMAC-SHA256 signed requests to `/native/v1/websites/{id}/events`. `X-Click-Signature` is `t=<send-seconds>,v1=<hex-HMAC(key, timestamp + "." + body)>`; Click verifies the signature in constant time with a five-minute past/60-second future send window, separately from the stable original event time. The worker hashes source IDs before transmission and sends only event kind,
original timestamp, integer money and supported uppercase currency. It sends
no email/member identity, raw provider ID, referrer URL, arbitrary query,
newsletter link URL or form values. There is no identity stitching: automatic
CMS events have no consented browser session and therefore appear under Direct
in attribution. Applications may use the existing backend event API to provide
consented session UUIDs; this connector does not invent them.

A batch contains at most 50 records. A 202 (including a duplicate) removes the
local row. Lost acknowledgements retry the same website/kind/opaque external ID,
so acceptance and metering stay idempotent. Transient failures use persisted
exponential backoff; malformed/too-old/unsupported events are dead-lettered,
with counts/retry controls in Admin. Twenty attempts stop automatic delivery;
dead records expire after 90 days. The receiver's seven-day backdate bound still
applies; old events are never relabeled with a new timestamp. Credential checks
are repeated under the acceptance transaction's workspace lock.

## Verification

Run the normal Click workspace suite against real PostgreSQL/ClickHouse, plus:

```sh
# From Vuuut-click, with its trusted local development .env sourced:
cargo test --manifest-path tests/native-bridge/Cargo.toml -- --include-ignored
# Siblings must be checked out as ../Vuuut and ../Vuuut-pro.
```

The bridge suite calls the actual hosting reconciler, CMS worker, JWT signer,
Admin renderer, theme renderer and Click API/stores. CMS `click_outbox` tests
run with SQLite or `GHOST_TEST_MYSQL_URL`; hosting has SQLite revision tests
and `PRO_TEST_MYSQL_URL` migration/trigger tests. See CONTINUE.md for the latest
completed results. No external public deployment, commercial release or
independent security review is implied by these local integration checks.
