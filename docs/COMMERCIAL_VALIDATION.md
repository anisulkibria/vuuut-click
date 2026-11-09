# Priority 4 commercial contract

Commercial release requires provider evidence in addition to local tests.
No live charges are part of this task. Use Stripe sandbox/test keys and an
operator-controlled SMTP recipient. Credentials and message links stay in the
ignored private environment; never put them in logs, screenshots or Git.

## Billing policy

- A checkout return URL never grants a plan. Signed snapshot webhook events
  retrieve the current subscription before changing entitlements. Event IDs
  deduplicate under the workspace lock; delayed notifications use current state.
- Active/trialing subscriptions collect under their configured allowance.
  Past-due/unpaid and incomplete/paused subscriptions pause collection immediately;
  reports, exports, privacy deletion and Owner billing recovery remain available.
  Cancellation/incomplete expiry returns to Free. Existing websites/members
  remain; new additions/invitation acceptance obey the new limits. Retention
  becomes the smaller configured website/plan duration and may permanently
  remove old data. Billing never reactivates an operator-suspended workspace.
- Only one unexpired checkout is allowed per workspace. Retries reuse an exact
  persisted request and provider idempotency key. Changing a selection requires
  acknowledging cancellation of the old provider session, or waiting for its
  expiry. Provider subscriptions must be changed through the billing portal.
  A late checkout cannot overwrite another active subscription.
- Integration-managed workspaces cannot override their hosting plan with local
  test billing. Pending invitations reserve seats during manual downgrades.

## Mail, domains and abuse

Account and invitation links have a finite queue lifetime, matching token
expiry. Expired/dead messages never expose their body through the owner console.
Malformed mail and SMTP errors consume bounded retry attempts. Delivered and
expired message bodies are cleared. Requeue is owner-only, exact-origin guarded,
audited, and refuses expired mail. SMTP acceptance is not proof of inbox receipt.

Registration requires a valid address; production website creation requires
email verification. Website control requires the exact TXT challenge at
`_vuuut-click.<host>`; public collector IDs do not authorize reads. TLS SMTP,
SPF/DKIM/DMARC alignment, a working bounce/abuse address, and independently
controlled public domain/TLS must be checked for the deployment. Never probe
arbitrary user-supplied URLs for ownership. Existing per-process request limits
are not a distributed abuse-control release gate.

## Fulfillment monitoring

Owner-only commercial status reports durable success/failure timestamps for
retention, deletion, exports and mail; pending/dead work and old pending jobs
remain visible. Missing/stale success is not healthy, including after process
restart. A successful retention sweep requires synchronous raw and aggregate
cleanup and control-state cleanup. These checks are fulfillment evidence, not
external alerting or production observability (Priority 5).

## Required real-provider evidence

Record exact time, test mode, scenario, observed provider/app state and pass/fail;
never copy a credential or token into evidence. Local/mocked checks must be
identified as such and cannot substitute for the following:

1. Confirm configured recurring Stripe prices, intervals and sandbox mode.
2. Open app checkout, confirm with Stripe's test card through its hosted UI,
   receive the signed webhook, verify the plan and single event/metering effect.
3. Repeat a webhook and deliver an older notification; verify current entitlements.
4. Open the customer portal, inspect invoices, change the plan/interval and cancel
   at period end; verify access remains until authoritative cancellation.
5. Exercise failed initial payment, past-due renewal and recovery with Stripe
   sandbox/test clocks; verify collection refusal and subsequent recovery.
6. Deliver verification, customer reset, owner reset and invitation through real
   SMTP to the operator inbox; consume links, verify expiry/single use and session
   revocation, and inspect inbox/spam plus sender authentication results.
7. Verify a real public TXT challenge, mismatched challenge rejection and exact
   origin collection. Check downgrade quotas/invitations and retention fulfillment.

Stripe's hosted interfaces should be tested interactively; API mock tests cover
error and replay behavior without trying to bypass hosted UI protections.
Sources: [subscription webhooks](https://docs.stripe.com/billing/subscriptions/webhooks),
[webhook delivery](https://docs.stripe.com/webhooks),
[automated testing](https://docs.stripe.com/automated-testing),
[test payments](https://docs.stripe.com/testing).

Implementation and current external evidence are recorded below. A passed
API lifecycle check does not substitute for a manual hosted-UI check or
production deployment evidence.

## Operator commands

Load the trusted private environment and build the current Rust binary first.
Use `APP_ENV=development` and `sk_test_` for these tests; live Stripe keys are
refused by the isolated helper. Do not use test card data in live mode.

```sh
set -a
source .env
set +a
cargo build -p click-server
./target/debug/click-server commercial-check
./target/debug/click-server commercial-mail-test
python3 scripts/validate-commercial.py setup
```

`commercial-check` reads local fulfillment state, checks every configured price
against the real pinned-version Stripe API (active, matching mode, quantity-one
licensed fixed monthly/yearly price), and checks the selected portal configuration (or the default when no
`STRIPE_PORTAL_CONFIGURATION` is set).
It fails if portal invoices/payment methods/cancellation/subscription changes
are not enabled. It performs no payment mutation. Inspect the selected portal's allowed
products/prices, proration and cancellation timing in Stripe before sign-off.
`commercial-mail-test` sends one requested diagnostic to `COMMERCIAL_TEST_EMAIL`;
confirm receipt and authentication headers in that inbox. It is not proof of
account-link or public-domain delivery by itself.

The helper creates **new, isolated PostgreSQL and ClickHouse stores** and starts
an owned app on 8084. The normal 8082 app and its accounts are not changed.
Only the configured operator inbox receives the verification/reset/invitation
messages. A generated password and private PID/store manifest are written under
ignored `.local/commercial-validation/` with private permissions, never stdout.
Open that app and consume the actual links from your inbox. Use the private
credentials file to test customer Checkout/portal with Stripe's hosted test UI.
Hosted payment and subscription controls require the manual test flow;
automated navigation checks stop after reaching the hosted page.

Start Stripe's test listener for
`checkout.session.completed,customer.subscription.created,customer.subscription.updated,customer.subscription.deleted`
and forward to `http://localhost:8084/v1/billing/webhooks/stripe`.
Put that listener's signing secret in private `STRIPE_WEBHOOK_SECRET` **before**
starting the isolated app. Public deployments instead need a registered HTTPS
snapshot webhook with the pinned supported API version, not a thin event format.
No webhook listener/tunnel is installed or started implicitly by the helper.

After customer testing, use the printed private manifest path:

```sh
python3 scripts/validate-commercial.py status .local/commercial-validation/RUN/run.json
python3 scripts/validate-commercial.py owner-reset .local/commercial-validation/RUN/run.json
python3 scripts/validate-commercial.py cleanup .local/commercial-validation/RUN/run.json
```

`owner-reset` intentionally promotes **only the isolated customer** to Owner and
queues its owner reset message to the same operator inbox. Finish customer
billing first; promotion revokes that isolated account's customer sessions.
To exercise owner recovery while another person completes customer billing,
use a separate isolated run with `setup --port 8086 --owner-only`. This queues
only the owner recovery message and leaves the customer run untouched.
The owner link still requires MFA when enabled. Repeated link use and post-reset
session refusal must be verified. Status output includes purpose/state counts,
current plans and accepted event counts, never message bodies or credentials.
SMTP's `delivered` queue state means the SMTP server accepted the message; confirm
actual receipt/inbox/spam and SPF/DKIM/DMARC results separately.

Cleanup validates the owned manifest/database namespace and process start/listener
identity, stops only that test process, then drops **only** its generated stores.
It deletes the generated credentials; private local logs remain for inspection.
It does not cancel Stripe sandbox subscriptions or delete Stripe customers.
Cancel test subscriptions and expire any remaining Checkout sessions in the
Stripe sandbox before cleanup; never reuse this helper for existing accounts.

For real public DNS, create a real HTTPS website in the app after verifying your
email, publish the exact challenge shown in Websites & setup, and set its UUID
as `COMMERCIAL_WEBSITE_ID`. Then run:

```sh
./target/debug/click-server commercial-dns-test
```

This reads that website's own challenge, proves the public TXT match, then
requires rejection of a different challenge. It does not change verification
state; complete the normal website Verify action to exercise the account path.
No arbitrary URL is fetched. A localhost fixture does not count as public DNS/TLS
evidence. DNS sender authentication and an abuse/contact policy remain operator
checks for the actual deployment.

## Queue and evidence lifecycle

Account/owner mail is linked to its hashed token and invitation mail to its
invitation. Consumed, revoked or expired links are retired before delivery;
mail retry also refuses them. Terminal mail metadata is retained at most 30 days
from enqueue, with bodies cleared on delivery/retirement. Expired account tokens
and invitations are removed after 30 days; their linked mail cascades away.
Legacy queued mail has a conservative seven-day expiry because its source
relationship cannot be reconstructed. Existing bearer/session/password values
are not altered by these additive migrations.

The operator-only UI is `/owner/commercial`; JSON is `/owner/v1/commercial`.
`POST /owner/commercial/retry-mail` accepts a form `id` and requires exact Origin,
a current full owner session/grant, a dead nonexpired record and a still-valid
source link. It audits the retry without disclosing its body. Recent failures,
stale/missing checks and old pending queues require investigation, even if
basic `/readyz` succeeds. Prometheus/OTel and external alerting remain Priority 5.


The isolated helper needs the PostgreSQL command-line client and a development
role permitted to create/drop its generated databases, plus ClickHouse database
creation permissions. It refuses missing/test-inappropriate configuration before
creating anything. Its cleanup does not target the normal development stores.
Accepted event retries remain acknowledged after payment pause/downgrade without
new usage; new events retain retry-compatible 429 with a billing-specific reason.


## Verified local and external evidence — 2026-11-10 (Asia/Dhaka)

- Formatting and warnings-denied workspace Clippy passed. All **47 backend
  checks passed** (24 unit and 23 real PostgreSQL/ClickHouse scenarios), including
  every ignored database test, serially. Three helper safety checks passed for
  missing/live configuration, unowned cleanup and private files. The normal and
  owner-only helper paths both reject unsafe configuration before store access.
- All **11 browser scenarios passed together** in 52.8 seconds. The billing
  redirect regression drives real settings forms under the app's CSP: Checkout
  and portal hosts navigate; a lookalike host is blocked before any request.
  Other pages retain self-only form targets. Phone commercial status was
  visually inspected. Earlier analytics failures hit the local Docker
  ClickHouse total-memory limit; a graceful database restart preserved volumes
  and the final full suite passed without weakening assertions. Sustained
  resource/capacity validation remains a Priority 5 gate.
- Real Stripe API preflight authenticated in **test mode**, checked two dedicated
  recurring prices and the selected portal features. Dedicated $1 monthly/$10
  annual fixtures are validation prices, not approved commercial launch prices.
  Shared/default portal settings and live objects were not modified.
- A dedicated real Stripe test-clock subscription was created through supported
  APIs, with a paid test invoice, and mapped only into generated QA stores.
  Actual Stripe events arrived through the authenticated CLI listener; every
  scoped forwarded webhook returned 200. Exact signed-event replay kept one
  ledger row and did not add accepted-event usage. A genuine held older monthly
  notification delivered after an annual change preserved annual entitlements
  by fetching current provider state.
- Advancing the real test clock caused renewal payment failure: the app became
  past-due, rejected new collection with 429 and still acknowledged previously
  accepted IDs with 202. Paying the test invoice restored active collection.
  Period-end cancellation preserved access until clock-driven cancellation,
  which returned the workspace to Free. A separate real incomplete initial
  payment remained paused and rejected collection. These are actual provider
  API/webhook checks; they do not assert that portal controls were clicked.
- The operator manually completed the actual hosted Checkout after navigation
  from the app. One completed Checkout, a paid test invoice and Pro/monthly
  entitlement were independently observed in the QA app/provider. The operator then
  submitted the actual portal changes: both Stripe and the app independently
  showed annual billing, period-end cancellation scheduled and Pro still active.
  Two paid test invoices were observed. Protected hosted controls were confirmed
  manually, not automated.
- Real TLS SMTP delivered verification, customer reset and invitation messages
  to the controlled inbox. Their actual links were consumed and rejected on
  repeat; customer reset revoked the old session. A separate owner-only run
  delivered and consumed its actual owner reset link, rejected replay, revoked
  the old owner session and allowed a fresh owner login. IMAP reads used
  read-only access and BODY.PEEK. All four messages arrived in INBOX.
- Sender and recipient were the same Gmail account. These messages contained
  no Authentication-Results/ARC-Authentication-Results headers; SPF/DKIM/DMARC
  are **unreported**, not failed and not proven. Public sender alignment,
  bounce/abuse contact, independently controlled public TXT/HTTPS and public
  exact-origin checks remain deployment evidence. The operator explicitly deferred
  domain setup; none of the connected Hostinger sites is authorized for this
  test, and no DNS record was changed.

All raw credentials, message links, signatures and private test manifests stay
under ignored `.env`/`.local`; this document contains no bearer links or secrets.
The QA stores are generated and independent of the original user accounts.
Full production readiness, encrypted cross-store restore, distributed abuse
controls, independent security review and load benchmarks are not claimed.


### Completion and cleanup

The operator confirmed both portal submissions. Independent provider/app reads
confirmed annual/yearly interval, `cancel_at_period_end=true`, Pro/active and one
completed Checkout. Cleanup then cancelled only that isolated test subscription;
its real signed deletion event returned the QA workspace to Free before removal.
Both API fixtures were terminal and their clock/customers were deleted. The
manual test customer, QA stores/credentials, temporary browser server, QA
listener/proxy and controlled test Chrome windows were removed/stopped.

The main development app remains on **http://localhost:8082**, with the verified
Stripe **test** settings and SMTP restricted to the controlled inbox. Its own
active listener forwards directly to `/v1/billing/webhooks/stripe` on 8082;
actual signed callback 200 and a fresh real catalog/portal preflight passed.
Dedicated test product/prices/portal are retained for further local use.
Existing customer/owner login, independent sessions and Events returned success;
CMS and hosting responded 200 and all four native mappings retained their prior
states (two active, two suspended). No original account/password or live Stripe
object was changed. No commit or push was performed.

Priority 4 implementation and test-account validation are complete within this
scope. The user explicitly moved public-domain work to later. Deployment domain,
TXT/TLS/public-origin/sender alignment and bounce/abuse contacts remain open;
Priority 5 operational/backup/security/capacity gates remain separate.
