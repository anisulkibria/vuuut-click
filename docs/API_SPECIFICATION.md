# API specification

Machine-readable OpenAPI 3.1: `docs/openapi.json`, served at `/v1/openapi.json`.

Register/login with JSON `{email,password,organization?}`. Password minimum is 12 characters. Send `Origin: PUBLIC_URL` for mutations, including login. Use the returned HttpOnly session cookie for account/organization/website operations. Read API keys are website-scoped credentials and cannot manage accounts or create goals. Collection IDs are public identifiers with zero read permission.

`GET /v1/organizations` → memberships, IDs and names. `POST /v1/websites` accepts `{organization_id,name,origin}` and verifies membership and site quota transactionally. HTTPS origins in production; localhost HTTP is allowed only with the explicit development flag. Register DNS TXT `_vuuut-click.<host>` with value `vuuut-click=<verification_token>`, then POST `/v1/websites/{id}/verify`. After lookup, the commit rechecks the active workspace and management role and requires the origin/challenge to be unchanged; a concurrent origin change returns 409 and requires starting verification again.

`POST /v1/collect/{collection_id}` accepts a JSON `{events:[...]}` batch, 1–50 items, maximum request size 64 KiB. Content-Type may be application/json or text/plain (beacon-compatible). Origin and each event URL must match a verified website. Response 202 reports `{accepted,duplicates}`; acceptance means PostgreSQL committed, not immediate ClickHouse visibility. Worker retries with exponential backoff capped at 60 seconds, dead letters after 20 failures. Operator requeue command: `click-server requeue-dead`.

Browser event fields: UUID event_id, UUID session_id, optional UUID visitor_id, ASCII name (pageview or custom event), URL, optional referrer and utm_source/utm_medium/utm_campaign labels. No arbitrary properties or client organization IDs. Server validates optional occurred_at_ms (7-day past/60-second future bounds) and stamps country; strips queries/fragments and referrer path; scopes identity hashes to website. Clients must reuse event IDs on retries.

Overview and breakdown queries accept `days=1|7|30|90` and use UTC days including today. Breakdown dimensions: countries/pages/referrers/campaigns/devices/browsers/events; limit 20 ordered by count, shares are labeled as top-20 share. API returns full integer counts even where UI abbreviates. Overview contains pageviews, visitors, sessions, bounce_rate (0–100), realtime and daily timeseries. Authoritative backend revenue uses the separate integration-key endpoint below; browsers cannot submit server.* names or monetary fields.

Goals: GET/POST `/v1/websites/{id}/goals`. Owners/admins/analysts can create `{name,event_name}` or PATCH `/goals/{goal}` with the same complete definition. Dashboard completions count exact unique event IDs; viewer keys cannot write. Goals match named events. Ordered session funnels are a separate resource described below.

Create a read credential using POST `/v1/websites/{id}/keys` with `{label}` as an owner/admin. Raw key returned once; DELETE `/v1/websites/{id}/keys/{key_id}` revokes it. Use `Authorization: Bearer vck_...` on website overview/breakdown/goals reads. `/v1/organizations/{id}/usage` returns the current monthly accepted allowance and usage. No paid subscription is activated by this API.

JSON errors use `{error:{code,message}}`. Invalid routes/access use 404; absent session 401; origin failure 403; duplicate website 409; quota/rate limits 429. Malformed-body, UUID and query rejections use the same JSON envelope. HTML navigations receive themed error pages; fetch/API requests retain JSON. Report concurrency limits return 503 with `Retry-After: 2`; per-IP limits return 429 with `Retry-After: 60`. Report requests are capped at four per workspace and sixteen per process. Cursor pagination on control collections remains future work.

## Platform owner console

`GET /owner/signin`, `POST /owner/auth/login` (HTML form email/password with exact Origin), `POST /owner/auth/logout`, `GET /owner/dashboard`, and `GET /owner/v1/me` use a separate owner session. `/signin` and `/v1/auth/login` accept customer accounts only. Owner login never changes the customer cookie, and either portal can log out independently. The console currently reports real platform totals and the latest 5 customer workspaces, with dedicated paginated directories for workspaces, users, websites, subscriptions, event delivery and audit records; workspace suspension/reactivation, manual plan grants and dead-letter retry controls are available.

To grant an existing account platform ownership, an operator with server/database access runs `click-server grant-owner <email>`. This invalidates that account's prior customer sessions. No public registration or API grants this privilege. Removing a row from platform_owners immediately invalidates its owner sessions.

Owner sidebar routes: `/owner/workspaces`, `/owner/users`, `/owner/websites`, `/owner/subscriptions`, `/owner/delivery`, `/owner/audit`. They require the owner session, show up to 100 records per page, and accept `?page=1` (1–10000). These directories omit passwords, credential hashes, collector IDs and event payloads. `/owner/appearance` changes the color scheme and returns to the owner console; it never uses or changes customer authentication.

## SaaS management and advanced reports

All customer workflows are also available through Rust-rendered `/settings` tabs. `POST /v1/organizations` creates a workspace; `PATCH /v1/organizations/{org}` renames it. Team members are listed at `/members`; PATCH/DELETE `/members/{user}` changes/removes access with last-owner protection. POST `/invitations` accepts `{email,role}` and returns `{id,accept_url,emailed}` with a one-time shareable fragment URL, valid seven days; email is queued in the same transaction when configured (`emailed` means queued); DELETE `/invitations/{id}` revokes it. POST `/v1/invitations/accept` accepts `{token}` only for the invited email. Invitations reserve plan seats. Owners alone grant owner/admin roles; admins manage lower roles.

PATCH `/v1/websites/{id}` accepts `{name,origin,active}`; an origin change requires verification again. POST `/keys` accepts `{label,scope:"read"|"integration"}`. PUT `/keys/{key_id}` rotates atomically and returns the replacement once; integration keys cannot read reports or manage settings. DELETE `/goals/{goal}` deletes a goal.

POST `/v1/websites/{id}/funnels` accepts `{name,window_seconds,steps:[{kind:"page"|"event",value}]}`, with 2–8 steps and a 60-second to 7-day window. GET lists definitions; GET `/funnels/{funnel}?days=7` reports ordered-session counts, conversion percentages and drop-off. Page steps match only pageview events. Event times must strictly increase. DELETE removes a definition. PATCH `/funnels/{funnel}` edits the name, steps and window using the same complete definition as creation.

GET `/journeys?days=7` returns the latest 25 pseudonymous sessions, the first 100 events each; `events` retains the total in-period count. The aggregation state is bounded to 100 steps after selecting those sessions. GET `/vitals` returns latest-instance samples and p50/p75/p95 per metric. GET `/revenue?days=7&model=last_non_direct&dimension=source` supports first_touch/last_touch/last_non_direct and source/campaign/landing, with a 30-day session acquisition lookback. Monetary totals are integer minor-unit strings, net of refunds and separated by currency. No currency conversion or ad-spend ROI is inferred.

POST `/backend-events` requires an integration Bearer credential. Required: `external_id` and `kind` (payment, refund, member_signup, subscription_started, subscription_cancelled, newsletter_sent, newsletter_open, newsletter_click). Optional: consented visitor_id/session_id UUIDs, local path, occurred_at_ms. Payments/refunds require positive amount_minor and supported uppercase currency (USD/EUR/GBP/CAD/AUD/NZD/SGD/INR/BDT/JPY); refunds are negated on the server. Retries with the same website/kind/external_id deduplicate delivery and usage. Keep credentials and external identifiers on your backend; do not send personal data. Rust `click-sdk` and TypeScript `tracker/src/api.ts` expose typed report/conversion clients.

PUT `/retention` accepts `{days:7..90}` within the plan maximum. POST `/exports` accepts `{days:1|7|30|90}` and queues a durable CSV job; GET lists jobs, GET `/v1/exports/{id}/download` downloads within 24 hours. Exports need analyst or higher, wait for pending delivery and fail explicitly if incomplete/dead or over 100,000 rows. DELETE website accepts `{confirmation:exactOrigin}`, stops collection, and queues ClickHouse/control deletion; GET `/v1/deletions/{id}` returns requester-only status. Workspace deletion is available through settings and requires exact workspace-name confirmation and cancellation of active provider subscriptions.

PATCH `/v1/account/profile` updates display_name. POST `/password` verifies current_password and sets new_password, revoking older user sessions and outstanding customer reset links. POST `/sessions/revoke-others` revokes other user sessions. POST `/v1/account/reset/request` queues eligible reset mail, POST `/reset` consumes `{token,password}`. POST `/verification` queues verification mail; POST `/verify` consumes `{token}`. Tokens are hashed, expiring and single use; links use URL fragments removed by local JavaScript. Platform-owner recovery is not exposed through customer reset.

Billing checkout/portal actions are on `/settings/action`, owner role only. `POST /v1/billing/webhooks/stripe` requires a valid raw-body Stripe signature with timestamp tolerance and matching test/live mode. A checkout return URL does not activate entitlements. Verified notifications fetch the current provider subscription and update entitlements with transactional event deduplication. Without configuration, local test-plan changes are explicitly labeled and record no payment. No external paid checkout has been performed during development.

POST `/owner/actions` uses independent owner authentication plus exact Origin, accepts form action=status|plan|requeue, organization_id, and optional value. Status changes are active/suspended. Manual plans cannot override provider subscriptions. Retrying dead letters preserves event IDs and discards expired payloads. Actions appear in the owner audit log.

Owner account management lives at `/owner/account` (sidebar **Account & security**). Forms POST `/owner/account/action` with `action=profile|password|sessions|enroll|confirm|codes|disable`; see OpenAPI for fields. Setup displays an authenticator key, and confirmation displays ten recovery codes once. Current password is required for security changes; an enabled factor is also required for password changes, disabling MFA and replacing recovery codes. All owner account writes and platform actions recheck the session and grant inside their transaction.

With MFA enabled, `/owner/auth/login` redirects to `/owner/auth/verify` and sets a five-minute pending cookie with audience `owner_pending`. POST a fresh authenticator or unused recovery `code` to that endpoint to get a full owner session; the challenge allows five attempts. A pending cookie cannot access either portal's private data. `/owner/forgot-password` and POST `/owner/account/reset/request` start separate owner email recovery. GET/POST `/owner/account/reset` completes with `token`, `password`, and `code` if MFA is enabled. Owner reset links are valid 30 minutes, single-use, and have five factor attempts. Recovery invalidates all sessions and tokens while preserving MFA. These are HTML form routes with exact Origin checks, not the customer JSON reset endpoints.


## Native integration v1

Operator credentials authorize `PUT /native/v1/tenants/{tenant}` with
`{revision,name,origin,plan_id,state:"active"|"suspended"|"deleted"}`. Tenant IDs
are 1–100 alphanumeric/underscore/hyphen characters; revisions start at 1.
Identical retries return the same website UUID, public collection ID,
website-scoped integration key, verification challenge/status and revision.
Conflicting/stale revisions return 409. Deleted mappings are terminal, even
after durable erasure. No cookie or public collection ID authorizes this API.

Website integration credentials authorize POST
`/native/v1/websites/{id}/connect` (`{issuer,jwks}`), `/exchange` (`{token}`),
`/verify`, `/rotate` (random 64-hex `X-Click-Replacement-Key` header), and
`/disconnect`. Connect pins keys only for the exact verified website origin;
exchange validates the dedicated staff assertion and returns an opaque
website-read-only bearer valid 60 seconds. Disconnect returns 204 and revokes
the connector and delegated access. Rotation requires persisting the replacement
before sending it, to recover uncertain responses. `/verify` independently
checks the current origin/challenge's TXT record outside the DB lock and
rechecks authorization and origin inside the commit transaction.

See [GHOST_INTEGRATION.md](GHOST_INTEGRATION.md) for the full pinned-issuer/JWKS,
consent, lifecycle, retry, privacy and server-only credential contract. Generic
read keys still cannot write, integration keys still cannot directly read
reports, and native access never creates customer/platform-owner membership.

POST `/native/v1/websites/{id}/events` additionally requires
`X-Click-Signature: t=<send-seconds>,v1=<HMAC-SHA256-hex>`, authenticating
`timestamp + "." + raw JSON body` with the integration credential. The send
window is 5 minutes past / 60 seconds future, body at most 16 KiB; the original
occurred_at_ms is preserved across retries. Payload/202 schema matches
`/backend-events`. Native CMS workers use this signed endpoint.

`GET /v1/websites/{id}/native-report?days=7&path=/article/` returns
`{activity:[{label,count}],page:{path,pageviews,visitors,sessions}|null}`.
The optional post path groups CMS slash/non-slash aliases; traffic counts
pageviews only and remains correct before ClickHouse merges. Eight authoritative
CMS event kinds are aggregated independently of general top-100 event lists.
Existing website read keys and native staff tokens authorize only their site,
and normal report concurrency/period limits apply.

## Extended analytics (Priority 3)

All new GET routes below use the normal website read authorization (including
native read tokens), tenant isolation, UTC period presets and report budgets.
See [metric definitions](ANALYTICS_METRICS.md) before interpreting a rate.

- `/comparison`: current and immediately preceding equal-duration snapshots,
  exact boundaries, pageviews/visitors/sessions/bounced sessions and measured
  visible time. Missing engagement remains unavailable; zero baselines have
  no relative percentage change.
- `/content`: top 100 paths with views, visitors, sessions, entries/exits,
  associated goal-converting sessions, visible milliseconds, measured sessions
  and nullable mean maximum scroll depth.
- `/acquisition?dimension=channel|source|medium|campaign|landing`: top 100 groups
  using each session's first observed pageview and distinct goal-session rates.
- `/retention`: provided-visitor coverage and weekly cohorts, retained history
  boundary, separate session-only/unclassified counts and nullable week 1–4
  follow-up cells. No fingerprinting or automatic member/browser stitching.
- `/vitals/comparison`: latest-instance p50/p75/p95 and sample counts for both
  equal-duration windows.
- `/newsletters`: top 100 editions by captured sends; distinct matched sent,
  opened and clicked recipient IDs, link-recipient facts and explicit unassigned/
  outside-send-cohort diagnostics. Backend newsletter facts optionally carry
  `newsletter_id` and `recipient_id`, both 64-character lowercase hex hashes.
- `/events?event=&path=&campaign=`: top 100 matching context groups, completions,
  sessions and visitor IDs. Empty event selects custom/backend events; automatic
  pageview/vital/engagement telemetry is excluded unless explicitly selected.
- `/campaign-costs?model=first_touch|last_touch|last_non_direct`: currency-separated
  supplied costs and recorded campaign revenue, explicit daily coverage and
  nullable ROI/ROAS. Missing revenue/attribution, incomplete dates and zero cost
  cannot manufacture returns. No FX conversion or provider verification.

`POST /campaign-costs` requires a manager session with exact Origin, or the
website's backend integration credential. Body: `{reference,revision,day,
campaign,currency,amount_minor,source}`. UTC dates must be within retained
history and not future; amounts are nonnegative supported-currency minor units.
Source is a lowercase identifier (1–32 characters); references are hashed.
An exact version replay returns the same record; changed/stale versions return
409. Corrections increase the version, replacing the same source/reference.
Fifty thousand retained records per website are allowed; further creation
returns 409. Cost rows are audited, expire with retention and cascade on erasure.

Funnel definitions additionally accept `scope: session|visitor` (default session)
and `page_prefix` steps. Visitor scope uses explicitly provided IDs across
sessions. Reports add `scope` and per-step `participants`; the legacy `sessions`
field stays a session count and is zero for visitor-scoped reports. Ordered
occurrence-time windows and permission/allowance checks remain in force.
Revenue additionally groups by medium, channel, acquisition country and device.
`dimension_present` is 0/1 and prevents missing/ambiguous campaign attribution
from implying a calculable ROI; up to 1,000 attribution groups are returned,
with an explicit refusal above that limit.

Optional browser `web_engagement` events carry `engagement:{duration_ms,
scroll_percent}`, bounded to 30,000 milliseconds / 100%. The tracker enables
these only through `engagement:true` or `data-engagement="true"`; they consume
normal event allowance. Persisted visitor mode, coarse OS and telemetry are
additive ClickHouse fields; older data stays unclassified. CSV exports include
these minimized fields and opaque newsletter edition keys, excluding recipient
hashes. Periods retain UTC collection-time semantics; ordering/windows use
validated occurrence time.

## Priority 4 commercial lifecycle

Checkout persists an exact request, has a one-hour provider expiry and allows
one unexpired selection per workspace. Retries reuse its idempotency key.
Settings action `cancel_checkout` requires the workspace Owner, exact Origin and
provider-confirmed expiry before clearing the pending selection. It returns a
303 to Billing. Current provider subscriptions use the portal; delayed checkout
notifications cannot replace a different active mapping. Portal authorization
is rechecked after its provider request. Manual test downgrades reserve pending
invitation seats and cannot override a hosting/provider plan.

Past-due/unpaid and incomplete/paused states pause collection while billing
recovery, existing reports/export and deletion remain possible. Authoritative
cancellation returns to Free and its retention limits. Existing members/sites
are preserved; adding new ones obeys current limits. No event/return URL clears
operator suspension. Cancellation of a retired price still reduces entitlements.

`GET /owner/v1/commercial` uses the separate full owner session and returns
provider modes, durable maintenance timestamps/failure codes and queue counts.
`GET /owner/commercial` renders those values. `POST /owner/commercial/retry-mail`
is a form `{id}` with full owner and exact Origin checks; it audits a bounded
retry of a dead message only while its source link is valid. Bodies/credentials
never enter these responses. A missing/stale check is not healthy. See the
[commercial runbook](COMMERCIAL_VALIDATION.md) for real-provider evidence.


Billing-inactive collection keeps HTTP 429 for retry-compatible collectors and
returns `Collection is paused until billing is resolved`. Monthly quota refusal
remains separate. Retries containing only already accepted IDs still return 202
with duplicate counts after a downgrade/payment pause, without incrementing
usage. New events and mixed batches obey current entitlement/quota atomically.
