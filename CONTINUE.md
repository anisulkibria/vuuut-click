# Vuuut Click — continue from here

Last handoff: 2026-11-10 (Asia/Dhaka), updated the same day after the native cutover below. Priorities 1–6 are implemented in the authorized local scope. Priority 6 release packages/contracts/runbooks and local verification are complete; remote final-revision CI, registry publication and production acceptance remain external gates. The user handles commits/pushes. Public-domain work is deferred and no Hostinger site is authorized. Main remains on http://localhost:8082 with real TEST Stripe/scoped SMTP and Priority 5 operations. Local databases now run natively; see the final native macOS checkpoint. No commit or push.

## What the user is building

An independent commercial web analytics SaaS called **Vuuut Click**, with Rust application/server/dashboard, PostgreSQL control/durable acceptance, and self-hosted ClickHouse analytics. It must work for arbitrary websites, standalone Rust Ghost/Vuuut installations, and managed Rust Ghost(Pro)/Vuuut-pro tenants. Native CMS/hosting integration is now implemented under Priority 2; see its current verification section. The exact original 1,109-line requirements are portable in [docs/ORIGINAL_BRIEF.md](docs/ORIGINAL_BRIEF.md), not in an inaccessible old attachment.

Accepted decisions and follow-up requirements:

- Rust stable, Axum/Tokio/SQLx, official ClickHouse client, Rustls; dashboard is native Rust-rendered HTML/SVG. TypeScript is used for the browser tracker/API client only; no Node runtime or React server.
- Rose Material 3 style matches existing Vuuut and Vuuut-pro; accessible responsive tables, light/dark/system themes.
- Keep this service independent. The standalone service builds without sibling repositories. Native adapters were implemented in the inspected Vuuut and Vuuut-pro checkouts under Priority 2; inspect their current schemas/interfaces before further changes.
- `run.sh` starts the complete local stack on **http://localhost:8082** and supports iterative development.
- Public homepage `/` always stays accessible, including while logged into either portal.
- Customer `/signin` and platform-owner `/owner/signin` must be different, with independent same-browser sessions. Workspace owner is not a platform owner.
- Owner console uses the same left-sidebar style and real platform directories/actions.
- Populate analytics with visibly labeled dummy/demo data so each report can be experienced. Keep demo records separate from real websites and billable usage.
- User requested completion of the main SaaS, then asked to pause after finishing the current planned checkpoint. Do not describe the entire original production brief as completed.

## Checkpoint implemented

**Customer dashboard** (`/dashboard`): overview chart, real-time visitors, unique visitors/pageviews/sessions/bounce, audience/countries/devices/browsers, acquisition/referrers/campaigns, content/pages, events and event-match goals. Detail pages have their own highlights; the traffic chart is on Overview only.

**Advanced analytics** (sidebar → `/insights`): ordered page/event funnels, session journeys, first-touch/last-touch/last-non-direct net revenue attribution with a 30-day acquisition lookback, separate currencies/refunds, and latest-instance Web Vitals p50/p75/p95. Optional `vitals.js` uses official MIT web-vitals. Page funnel steps count pageview events only; tests caught and fixed a performance-event overcount.

**Workspace & settings** (sidebar → `/settings`): multiple organizations, rename/switch, team invitations with one-time fragment links and seat reservations, role updates/removal with last-owner protection, website name/origin/collection changes, read/integration credential creation/revocation/atomic rotation, usage and plan allowances, retention, durable CSV exports/download, website/workspace deletion, profile, password change, other-session revocation, email verification and password reset. Invites queue email transactionally when configured and also return a shareable link; development uses private local mail files. Viewer permissions intentionally hide management controls.

**Billing/account services**: provider-neutral interface plus Stripe HTTP adapter, checkout attempts/idempotency, signed raw-body webhooks with timestamp/mode checks, authoritative current-subscription lookup, transactional dedup/entitlements, billing portal. No entitlement upgrade on a checkout return URL. Without credentials, local test plans record no payment. SMTP supports STARTTLS/implicit TLS; development writes private `.local/mail/*.eml`. Hashed one-use reset/verify tokens, expiry and session revocation. Production registration needs SMTP, and real website creation requires verified email.

**Platform owner** (`/owner/dashboard`): separate explicit owner grant and 8-hour `/owner` cookie, real paginated Workspaces/Users/Websites/Subscriptions/Event delivery/Audit directories, suspension/reactivation, manual plan grants (cannot override provider-linked subscriptions), dead-letter retry preserving IDs and retention, platform audit. Customer and owner cookies cannot substitute for one another.

**Data correctness/privacy**: PostgreSQL transaction commits accepted payload+metering; replay-safe raw `FINAL` queries and unique-state hourly aggregates; outbox retry/dead letters; organization/site isolation; exact Origin and verified domain; public collection IDs never authorize reads. Browser timestamps bounded to 7 days past/60 seconds future, reserved server names rejected, integration-key-only monetary conversions. Deletion stops collection before storage removal. Retention uses the smaller website/plan limit, purges hourly buckets and discards expired queued payloads to prevent replay resurrection. Exports wait for ingestion and fail explicitly on dead letters/over 100k rows; CSV formula escaping; 24-hour expiry. Process-wide report concurrency and ClickHouse time/memory/result bounds.

**Demo**: CLI `seed-demo <registered-customer-email>` creates/refreshes only a marked separate demo website, 30 days of varied traffic, sources/campaigns/countries/devices, custom events, four goals, a funnel, net revenue/refunds and five Web Vitals. No collector or metering for demo. Default first site on the dashboard; banner clearly says Demo data.

**Developer assets**: Rust `click-sdk`, server-side TypeScript `tracker/src/api.ts`, checked-in bundles in `tracker/dist`, OpenAPI, deployment templates, CI, architecture/license/security docs, real-stack and browser tests. `run.sh` now manages detached native app/database services and installs builds outside the cache; see docs/NATIVE_LOCAL.md.

## Native cutover and location independence (2026-11-10)

- **Docker is no longer used locally.** Both sign-in pages had returned 500 for
  two days because the app's PostgreSQL/ClickHouse were still the
  `compose.dev.yml` containers and Docker Desktop was not running. The data
  was moved with `./run.sh migrate-docker` (verified: 37 control tables and
  the analytics fingerprints matched). The app now runs from `.local/bin` on
  the project-owned native PostgreSQL 17 (Homebrew `postgresql@17`, keg-only,
  `LOCAL_PG_BINDIR` in `.env`) and the pinned native ClickHouse in
  `.local/native`. `LOCAL_DB_MODE=native` is set in `.env`. The two Docker
  containers are stopped and their volumes retained for rollback; the
  snapshot is under `.local/native-migration/`. Homebrew's `postgresql@14`
  remains the linked version on `PATH`.
- **No absolute paths anywhere.** The three checkouts find each other only
  through URLs at runtime, plus `.env`: here `VUUUT_DIR` / `VUUUT_PRO_DIR`
  (default `../vuuut`, `../vuuut-pro`) feed `./run.sh link-siblings`, which
  writes `.local/siblings/*` symlinks for the optional `tests/native-bridge`
  suite; `./run.sh native-test [-- cargo-test-args]` links and runs it. The
  main `Cargo.toml` excludes `.local` so Cargo does not adopt the linked
  crates into this workspace through its cached root. Vuuut and Vuuut-pro
  each gained a gitignored `.env` loaded by their `run.sh` (`GHOST_*` /
  `PRO_*` config overrides, `VUUUT_DIR` for the hosting platform's CMS
  checkout, `GHOST_REFERENCE_DIR` for the CMS gscan parity tests); see each
  `.env.example`. The user handles commits; none were made.

## Current local access and state

Original machine project: `/Users/anisulkibria/Documents/Rust/Ghost/Vuuut-click`. This absolute path is not required on another device.

- Homepage: http://localhost:8082/
- Customer: http://localhost:8082/signin → `/dashboard`
- Owner: http://localhost:8082/owner/signin → `/owner/dashboard`
- The sign-in pages are `/signin` and `/owner/signin`; `/auth/login` and
  `/owner/auth/login` are POST form targets, not pages. `/login`, `/register`
  and `/owner/login` are now 303 aliases to the real pages — `/owner/login`
  previously returned 404, because the `/owner/{section}` catch-all rejects an
  unknown section before considering who is asking, so a wrong address looked
  like a broken portal.
- Existing local customer email: `user@vuuut.local` (viewer in its shared workspace).
- Existing local platform owner email: `admin@vuuut.local`, workspace `Vuuut
  Platform`. Recreated 2026-11-08 at the user's request: the account named in
  the earlier handoff was no longer in the database, and
  `.local/development-accounts.json` was missing, so neither survived the
  device move. It was registered through `/v1/auth/register` (so the password
  is Argon2-hashed by the app's own path) and then granted with
  `grant-owner`. Email is unverified, which is fine for the owner console but
  blocks creating a real website from that account.
- Passwords are unchanged and stored only in ignored **`.local/development-accounts.json`**, mode 0600. Never put those passwords in tracked docs, commits, output or production defaults.
- Existing demo website: `Vuuut Journal · Demo`, id `2220b39d-ac4c-4830-9ed4-c242718129c0`. Existing real website stays separate.
- Local private `.env`; native PostgreSQL 17 on 55432 and ClickHouse HTTP on 58123. Project-owned durable data in `.local/native`; original stopped Docker volumes retained as rollback evidence. Health `/healthz`, dependencies `/readyz`.
- Main server log/PID: `.local/server.log`, `.local/server.pid`. Temporary staging 8083 was stopped; the verified build is running on 8082. Verify readiness; do not assume a process survived a device restart.
- Migrations 0001–0015 PostgreSQL and 0001–0003 ClickHouse. SQLx checksums and ClickHouse `control_migrations` ledger. Never edit an applied migration; add a new one.

## Clone and run on another device

Git transfers source/docs, **not local databases, users/passwords, sessions, demo rows, `.env`, mail or licensed GeoIP data**. To retain the exact existing accounts/data, separately transfer encrypted consistent backups of both databases and private configuration through your own secure channel. Use the encrypted cross-store backup/restore workflow in docs/OPERATIONS.md; never push database directories or `.env` to Git.

For a clean development install:

```sh
# Inside the cloned Vuuut-click repository on macOS; install Rust, Python 3.11+ and Homebrew.
./run.sh setup
./run.sh start
```

This generates private local secrets, installs native database prerequisites, starts project-owned databases, migrates, builds Rust, and serves 8082 without Docker. Existing Docker installations use the explicit verified `./run.sh migrate-docker` first; do not initialize replacement data over them. Node is needed only to change/build the tracker or run browser tests; committed bundles let Rust start without it.

1. Register a customer at `/signup`; add a localhost website for actual tracker testing.
2. In a second terminal, create labeled sample data for your registered customer:

```sh
set -a
source .env # Load only your trusted local file.
set +a
./target/debug/click-server seed-demo your-customer@example.com
```

3. For a separate owner account, register a second email once, then run `./target/debug/click-server grant-owner your-owner@example.com`. It becomes owner-only and its old customer sessions are invalidated. Sign in at `/owner/signin`. There are no universal default credentials on a fresh installation.
4. Save your new local credentials privately. Configure your own licensed `GEOIP_PATH` for real country lookups; development otherwise shows Unknown. Never enable the test MMDB as production data.
5. Visit sidebar **Advanced analytics** and **Workspace & settings**. A viewer can explore reports but needs its own workspace/appropriate role to manage settings.

Edit Rust, then run `./run.sh restart` to build and restart the detached app/databases. `./run.sh stop` (or legacy `--stop`) gracefully stops only this project's managed services and preserves data. Use `status`, `doctor`, `log -f`, `build`, `rebuild`, `db-start`, `db-stop` and `help` for local operations. See docs/NATIVE_LOCAL.md; runtime and database directories are not caches.

Before initializing Git, work in **Vuuut-click**, not the parent Ghost directory containing sibling projects. `.gitignore` excludes `.env`, `.local`, `target`, `node_modules`, test artifacts and `data`. Review staged files with `git status`/`git diff --cached` before pushing. Include Cargo.lock, package-lock.json, tracker/dist, docs and test fixtures with their license notices. The user will choose the remote and initialize/push Git; no remote publication was performed by the agent.

## Verification checkpoint

Completed locally against real PostgreSQL/ClickHouse:

- Rust formatting and Clippy with warnings denied.
- Full `cargo test --workspace -- --include-ignored`: **9 unit tests + 5 real-database integration tests passed**, no ignored tests left skipped in that run.
- Coverage includes durable acceptance/outage/replay/idempotency, tenant/key/role/audience isolation, exact ordered funnel sequences, revenue attribution/refunds/currency separation, latest vital sample dedup, team/seat/last-owner lifecycle, exports/deletion/retention, private mail file permissions, reset/verification, provider-failure retry/dedup/delayed notifications and owner suspension/actions.
- Browser tracker TypeScript check and bundle build passed. Browser tests use real local databases; current suite has five tests, including the new demo/settings workflow. The final full suite passed: **5/5 Playwright tests**, including the new demo/settings workflow.
- `run.sh` launched the final build successfully on 8082. Existing-account smoke checks covered all 5 dashboard reports, 4 advanced reports, 7 settings tabs and 7 owner destinations, plus simultaneous owner/user sessions and the public homepage. Existing passwords unchanged.
- Browser screenshots of demo reports and phone settings inspected. Full tables scroll inside their panel on small screens.
- No external SMTP, real Stripe account, public domain/TLS, production MaxMind, Linux image deployment or production load gates verified. Configurable adapters are implemented; external service validation is still required.

## Resumed 2026-11-08 — priority 1, goal and funnel editing

Work resumed at the user's request. Two items from priority 1 below are now
done; the rest of that list stands unchanged.

**Goals and funnels are editable.** Both were create-and-delete only, so a goal
whose event had been renamed in the site's own code, or a funnel with one wrong
step, could not be corrected — only destroyed and rebuilt under a new name,
losing the original. Added `PATCH /v1/websites/{id}/goals/{goal}` and `PATCH
/v1/websites/{id}/funnels/{funnel}`, plus the UI for both: a per-goal edit
disclosure and a Remove button on Events & goals (goal deletion had been API
only), and an "Edit this funnel" disclosure pre-filled with the funnel's own
steps on Advanced analytics → Funnels. Goal rows stay readable text cells with
editing folded into a disclosure; putting the fields in the cells was tried
first and reverted, because it takes the values out of the table's text and the
browser suite caught it at once. HTML forms reach these through
`POST /goals/action` and the existing `/insights/action`, since a browser form
cannot send PATCH; the JSON API keeps PATCH.

**Every goal/funnel mutation now re-checks privileges inside its own
transaction**, through one new `workspace::lock_editor`. Previously `add_goal`
and `create_funnel` locked the organization row with a bare `SELECT … FOR
UPDATE` and re-checked membership with a hardcoded `role IN (…)` list, while
both delete paths did neither. Two consequences, both fixed: a role revoked
after a request's own check could still complete a write, and **a suspended
workspace could still create or delete goals and funnels**, because
`auth::authorize_site` re-reads membership per request but never looks at
`organizations.status`. `lock_editor` reuses the existing `lock_org`, which
refuses a workspace that is not active, and decides the role through
`click_core::Role::can_edit_goals` so who may edit is defined once instead of
being respelled per query.

Verified: `cargo fmt --all --check` clean; Clippy clean with warnings denied;
`cargo test --workspace -- --include-ignored` at **9 unit + 5 real-database
integration tests passed**; **5/5 Playwright tests**, with the analytics spec
extended to drive the new goal edit, goal remove and funnel edit forms in a
real browser. `real_stack.rs` gained the edit paths, duplicate-name conflict,
invalid event name, unknown id, cross-website isolation, viewer refusal, a
lock-contended race that revokes a role while the request waits on the
organization row, and the suspended-workspace refusal. The race and suspension
assertions were checked by removing `lock_editor` and confirming the suite
fails, so they are not passing vacuously.

### The same recheck across the remaining workspace mutations

Audited every transactional mutation for whether it re-reads the actor's role,
and the workspace's status, inside the transaction that writes. The guard is
now `workspace::lock_member(tx, org, actor, allow)`, taking the same `Role`
predicate the pre-transaction check used, with `lock_editor` as the
goal/funnel wrapper. Three real holes were closed:

- **`retention` reported success while changing nothing.** Its membership test
  was an `EXISTS` inside the `UPDATE`, so a role lost in the race window meant
  zero rows changed, `Ok(())` returned, and an audit entry written saying the
  retention *had* been changed. Now the role is checked under the lock and a
  zero-row update is an error.
- **A suspended workspace could issue API credentials** (`issue_key`) and
  **change where collection comes from** (`update_website`). A credential
  grants access to the data the suspension withdrew. Both now refuse.
- **`queue_export` never re-read the membership at all** — a member removed
  after their request began could still queue an export of the whole
  workspace's data.

Two paths deliberately keep working while a workspace is suspended, through
`lock_member_even_if_suspended`, which re-checks the role but waives the
active-workspace requirement. The name is the documentation at the call site:

- **`checkout`**, because paying is how a past-due workspace recovers, and
  refusing it would trap the customer in the state they are trying to leave.
  It did not re-check ownership inside the transaction; now it does.
- **`delete_site` / `delete_workspace`**, because erasing your own data has to
  remain possible while suspended. These already re-checked the role
  explicitly; the change only removes a second hardcoded role list.

Left alone on purpose, and *not* to be "fixed" by applying the guard
mechanically: the background workers (`process_export`, `process_deletion`,
`enforce_retention`) and the Stripe webhook (`apply_event`) have no actor and
must run against a suspended workspace; `owner::action` is how a platform
owner suspends and reactivates, so gating it on an active workspace would make
reactivation impossible. `delete_workspace` keeps its own explicit owner check
because it needs the workspace name from the locked row for its confirmation
prompt.

Behaviour change worth your review: a suspended workspace can no longer change
retention, rename a website, change its origin, issue credentials, or queue an
export. That follows the stance the code already took for workspace management
(`rename_workspace`, `invite_member`, `change_member` all refused when
suspended), but it is a policy decision, not only a bug fix. Export is the one
I am least sure of — data portability is an argument for allowing it while
suspended. Say so and it is a one-line change to
`lock_member_even_if_suspended`.

Verified: fmt and Clippy clean with warnings denied; **9 unit + 5
real-database integration tests passed**; **5/5 Playwright**. The retention
no-op and the suspended-credential refusals were each confirmed by reverting
the fix and watching the suite fail — the retention one only reproduces as a
lock-contended race, because the per-request check catches a role that is
already gone, so the test holds the organization row while the request waits.

Note for the next session: the local server applies a per-IP allowance of 20
sensitive requests a minute (`lib.rs`), so running the browser suite twice in
quick succession, or probing sign-up by hand alongside it, makes a signup fail
with "Request allowance reached". That is the limiter working, not a
regression; wait a minute and re-run.

**Paused again at the user's request, 2026-11-08.** Do no further feature
development until the user asks to resume. localhost:8082 is running the
verified build.

Earlier in the same session, before resuming: sign-in path aliases (above) in
`crates/click-server/src/lib.rs`, covered by `real_stack.rs` which also asserts
the alias does not shadow a real `/owner/{section}`.

Reproduce checks (with databases running; use `./run.sh start` for native local services):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
set -a
source .env
set +a
cargo test --workspace --locked -- --include-ignored
npm ci
npm run check
npm run build
npx playwright install chromium
npm run test:e2e
```

On this Mac, use `CHROMIUM_EXECUTABLE_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"` for Playwright. The older cached chromium-1194 executable is incomplete (its framework is missing). On other devices install Playwright Chromium normally. Source `.env` for browser tests: owner/demo tests use the local operator CLI. Browser tests create their own test workspaces in the local development DB. Database integration tests create/drop isolated `click_it_*` databases. Do not delete arbitrary databases to clean up a failed test.

## Resume priorities — do not restart from scratch

1. **Finish product hardening around existing workflows — complete locally, 2026-11-08.** Goal/funnel editing/deletion, transactional workspace privilege rechecks, account/password/reset/promotion races, consistent HTML/API validation and overload errors, automatic invitation email queueing, owner profile/password/recovery/MFA UI, per-workspace report budgets and bounded journey aggregation are implemented and verified. Details and limits are in the completion section below. Production SMTP, distributed edge budgets and independent security/MFA review remain release gates under priorities 4–5.
2. **Native Ghost/Vuuut and Vuuut-pro integration — complete and verified.** All requested components are present in the sibling repositories; see the new native integration section and docs/GHOST_INTEGRATION.md. Never expose platform/integration credentials in browser content.
3. **Analytics breadth — complete and verified under Priority 3.** Richer content/acquisition/event reports, opt-in engagement, returning/cohorts and period comparisons, visitor/prefix funnels, newsletter editions, extra attribution dimensions and explicit-cost ROI are implemented. See docs/ANALYTICS_METRICS.md and the completion evidence below.
4. **Production commercial validation — complete in agreed test scope.** Test-mode Checkout, real API/webhook replay/renewal failure/recovery/cancellation and controlled-inbox customer/owner mail passed, with safe checkout, mail and fulfillment policies implemented. See the latest evidence below. Public-domain/TLS/sender alignment is explicitly deferred by the user; none of the connected Hostinger sites may be used. No live charges are authorized.
5. **Operational release gates — implemented and locally validated under Priority 5**: OTel and metrics/dashboards/alerts, distributed edge/request budgets, encrypted off-site backups with cross-store consistency and verified restore drills, dependency/image/license scanning, Linux image/TLS deployment, capacity/load benchmarks at 1M/10M/100M monthly events, independent security review. NATS JetStream is optional only when distributed queue requirements justify it; current PostgreSQL outbox is durable. Keep readiness, retry and dedup behavior through migrations.
6. **Release/package work — implementation complete locally**: versioned SDK packages/signatures, OpenAPI response schemas, operator/admin/migration documentation and gated release automation. Remote final-revision CI/publication and production acceptance are external completion gates. Maintain license notices. Do not assert production-ready until all gates genuinely pass.

External inputs eventually needed: actual deployment domain/TLS, MaxMind account/licensed Country MMDB, SMTP credentials, Stripe keys/price IDs/webhook secret, backup storage/secrets. Ask only when an actual next step requires them; continue independent code/test work first.

## Priority 1 completed — 2026-11-08

The user explicitly resumed and asked to complete Priority 1. Completed the partial work found in the working tree without discarding the earlier goal/funnel and workspace fixes.

- **Account races:** customer password changes prove the current password after locking the user, then recheck that the session is still live and the account is not an owner. They invalidate older sessions and outstanding reset links. Profile updates and other-session revocation use the same guard. Both customer and owner login verify the locked password before creating sessions. Reset completion already serialized on the user; token requests now recheck owner status after the lock. Operator owner promotion takes that same lock and invalidates existing account tokens/customer sessions. Owner account and platform actions recheck session/grant under that lock too.
- **Website verification races:** DNS lookup completion rechecks the current role/workspace status under the organization lock and commits only if both origin and challenge are unchanged. Origin updates compare the locked row, so a concurrent change cannot carry another domain’s verification or challenge across. Tests wait until the request actually reaches the contended lock before rotating the origin/challenge or downgrading the actor.
- **Errors and request budgets:** malformed JSON/forms/UUIDs/query strings now use the JSON envelope. HTML navigations receive escaped, themed errors with dashboard/sign-in links. API/fetch clients keep JSON. Report overload uses 503 with Retry-After 2; the per-IP allowance uses 429 with Retry-After 60. Security and private/no-store headers cover early refusals and converted pages as well.
- **Invitations:** invitation creation and email enqueueing commit together, using configured SMTP or the development private-file mailbox. API `emailed` means queued, not confirmed delivered. The UI reports queued delivery or the manual-sharing fallback. The original shareable fragment link remains available. No real external email delivery was performed.
- **Owner security:** `/owner/account` is in the sidebar. Display name, password, sign-out-other-sessions, authenticator enrollment/confirmation, replacement recovery codes and disabling MFA work through real forms. Enrollment needs the password and a TOTP code within 15 minutes; ten random one-use recovery codes are shown once and stored hashed. Security changes need the current password and, when enabled, a fresh factor. MFA enrollment, replacement codes and disabling MFA revoke other sessions. Existing development accounts were not enrolled or changed.
- **MFA sign-in:** after enrollment the password creates only a five-minute `owner_pending` audience/cookie, with five persisted attempts. It grants no owner or customer data access. A successful code consumes the step or recovery code and pending session in one transaction before issuing the full owner cookie. Code steps cannot be replayed. Owner logout clears and revokes pending/full owner sessions and preserves the customer portal.
- **Owner recovery:** `/owner/forgot-password` queues a separate one-use 30-minute owner token. Reset still requires a fresh authenticator/recovery code when MFA is enabled, limits attempts to five, and invalidates all sessions/account tokens on success. Customer reset cannot recover owner accounts or consume owner tokens. Losing the authenticator and every recovery code requires an operator-assisted identity/recovery procedure; there is no email-only MFA bypass. Authenticator secrets are stored in private PostgreSQL (not application-encrypted); independent review, protected storage/backups and managed encryption remain production concerns documented in SECURITY.md.
- **Report fairness and journeys:** four concurrent report requests per organization, sixteen per process, including read-key access. Permits are released on completion/error/cancellation and idle organizations leave the map. Journeys select the newest 25 sessions before aggregating, with `groupArraySorted(100)` retaining the earliest 100 ordered steps while preserving full event counts. Existing ClickHouse time/memory/result bounds remain. These budgets are per process, not distributed promises.
- **Contracts and migrations:** additive PostgreSQL 0007–0009 introduce owner factors/recovery codes, the pending session audience, token purpose and attempt counters. OpenAPI, API specification, security/deployment docs and this handoff are updated. Goal/funnel PATCH contracts are now documented too.

Verification: formatting, Clippy with warnings denied, **17 unit tests + 11 real-database integration tests** with `--include-ignored`, npm clean install/check/build, and **6/6 Playwright scenarios**. The added integration suite covers concurrent password writes (only one succeeds), transactional invitation mail, report budget release, a reverse-ordered 1,200-event session with duplicate replay (count 1,200; exactly the earliest 100 steps), malformed/overload HTML/JSON, MFA replay/single-use recovery codes, challenge attempt exhaustion, reset audience isolation, preserved MFA and session revocation. The new browser scenario runs a temporary local server with the same databases and its own per-IP limiter, stops it afterwards, and exercises profile/enrollment/login/code replacement/password/disable while a customer stays signed in. Desktop/390px owner-security screenshots were inspected. Build restarted on 8082; persistent databases and existing local credentials are preserved.

Production external-service/load/distributed-budget/security/recovery-drill gates are not claimed complete. At that checkpoint, the next feature priority was **2: native Ghost/Vuuut and Vuuut-pro integration**. It is now implemented below; the next feature work is Priority 3.

## Owner panel typography — 2026-11-08

At the user's request, increased text across `/owner/dashboard`, `/owner/users`, `/owner/workspaces`, `/owner/websites`, `/owner/subscriptions`, `/owner/delivery` and `/owner/audit`. Their shared page wrapper now has `owner-console`; scoped CSS uses 16px row/body/control text, 14px table headers/supporting labels/buttons and 22px panel headings, with more row spacing and 44px minimum button height. These sizes also apply on phones. Customer pages retain their own styles.

Verified computed text sizes and no document overflow on all seven pages at 1440px and 390px; inspected desktop/phone dashboard screenshots. The existing owner navigation/session Playwright scenario passed. Formatting, Clippy with warnings denied, all 17 unit tests and the Rust build passed; 11 database integration tests were intentionally skipped in this presentation-only run and are not claimed as newly passed. Restarted the updated app on 8082, preserving accounts and data.

Owner typography follow-up: the user found the first increase too large. Reduced owner body/table/control text to 14px, supporting labels/table headers/buttons to 13px, panel headings to 20px and row padding to 14px. This supersedes the larger sizes above.

Further owner typography adjustment: the user requested another small reduction. Current sizes are 13px body/table/control text, 12px supporting labels/table headers/buttons, 19px panel headings and 12px vertical table-cell padding. These supersede the earlier typography sizes.

## Customer sign-in outage recovered — 2026-11-08

The user reported `/signin` failing. The page returned 200, but `/readyz` returned 500 and logs showed PostgreSQL pool timeouts. Docker Desktop was stopped, leaving both development databases unavailable. Started Docker Desktop and brought up the existing development Compose stack without removing volumes. Both containers are healthy, readiness is 200, and the database retains all 57 accounts including the original customer/owner. Verified the actual customer form in Chromium with an existing browser-test customer: sign-in, redirect, authenticated `/v1/me` and logout passed. No account/password changes or authentication-code changes were needed. This was the historical Docker setup; the later native macOS checkpoint supersedes it. Use `./run.sh start` for local services now.

## Source map

- `crates/click-core`: validation, identity hashing, trusted proxy/GeoIP, roles.
- `crates/click-store`: storage, versioned migrations, acceptance/metering/outbox, base reports.
- `crates/click-server/src/analytics.rs`, `insights.rs`: advanced query API and Rust report UI.
- `workspace.rs`, `manage.rs`, `settings.rs`: organizations/team/credentials/account and UI.
- `billing.rs`, `mail.rs`, `jobs.rs`: providers/recovery, exports/deletion/retention workers.
- `auth.rs`, `owner.rs`, `owner_security.rs`, `dashboard.rs`, `landing.rs`, `lib.rs`, `main.rs`: auth, portals, routes/middleware/startup.
- `demo.rs`: isolated local sample seeder.
- `crates/click-sdk`, `tracker/src/{index,api,vitals}.ts`, `tracker/dist`: SDKs and bundles.
- `migrations/{postgres,clickhouse}`: additive schemas. `dashboard`: scoped CSS/landing templates.
- `crates/click-server/tests`, `tests/browser`: verification. `deploy`, `scripts`, `.github`: operations.

The next agent should state which unfinished priority it is tackling, preserve all user decisions above, update this handoff with actual evidence, and keep localhost:8082 usable during development. Do not repeat permission requests for already authorized reversible local work. Stop when the user asks to pause.

## Priority 2 — native Vuuut/Vuuut-pro integration (2026-11-09)

The user explicitly requested the **full Priority 2**, including the sibling
applications. The implementation, functional verification and local optimized-binary
rebuild/restart checks are complete. All changes are uncommitted in **three separate Git
repositories**. Do not transfer only the Click changes and expect an older CMS
or hosting binary to support the connector.

Implemented:

- **Click control plane:** `/native/v1` versioned tenant snapshots keyed by
  authenticated platform and tenant. Persisted revision/fingerprint conflicts,
  stable lost-response replay, separate native workspace/website/entitlement,
  operator-issued hashed platform credentials and registered DNS suffix trust.
  Custom domains require independent TXT verification. Suspension, disable and
  deletion stop access/collection; erasure uses the existing durable privacy
  worker, retaining terminal mapping tombstones.
- **Dedicated staff exchange:** website integration key plus a signed RS256
  assertion with pinned public JWKS, exact issuer/dedicated audience, website,
  internal staff subject, Owner/Administrator role, time bounds and one-use jti.
  No issuer/JWKS URL discovery. Opaque hashed native read tokens last 60 seconds,
  authorize one website and cannot create customer/owner sessions or submit
  revenue. Generation, origin and key/workspace/platform state revoke access.
- **CMS:** private connector state outside settings/public APIs; native Admin
  reports at `/ghost/analytics/click/`, Owner connection/test/rotation/disconnect
  and delivery retries; Administrator read access. Material 3 tables/forms,
  empty/error states, phone overflow checks, no credentials in report HTML.
  Consent-aware `ghost_foot` injection includes only public attributes. Exact
  member/subscription/newsletter summaries, current CMS membership and selected
  post traffic use authoritative counts, separate from top-list truncation.
  Post analytics use the real permalink resolver; period tabs retain the post.
- **Transactional delivery:** SQLite/MySQL source triggers capture member,
  subscription, payment/refund and newsletter facts in their source transaction.
  Signed raw-body HMAC-SHA256 native deliveries carry stable hashed source IDs,
  original times and minimized facts only. Persistent retries/dead letters and
  duplicate acceptance do not inflate usage. Integration keys are rechecked
  under the same acceptance lock, including a proven revocation race.
- **Hosting:** transactional tenant/plan/domain dirty state, exact persisted
  request snapshots and revisions, an independent reconciliation worker,
  per-site status/TXT instructions/retry/rotate/disconnect/reconnect controls.
  Only website keys enter private tenant config; exports scrub them and retain
  a disabled hosting marker. Hosted policy always overrides old standalone
  connector state, including during suspension/disconnect/restore. Local
  file/restart errors preserve retry state. Restoring a terminally erased site
  creates a new incarnation/mapping; the CMS clears old-target queued records.
- **Provider facts:** paid invoices (including first payment) are distinct from
  one-off checkout receipts; stable source IDs dedupe invoice aliases/replays.
  Individual successful refunds dedupe cumulative charge notifications; large
  refund lists use bounded pagination. New Stripe webhook registrations include
  invoices/refunds; native Admin can explicitly upgrade an existing endpoint's
  event list without rotating its signing secret. No real provider call/charge
  was used for verification.

Additive migrations: **Click PostgreSQL 0010**, **CMS SQLite/MySQL 0038–0039**,
**hosting SQLite/MySQL 0004–0005**. Applied older migrations were not edited.
Hosting MySQL migration startup now aligns future-table database collation with
the existing unicode_ci tables, fixing the old 0003 foreign-key failure on a
fresh MySQL 8+ default; existing columns and migration checksums are untouched.

Verified in this session:

- Click formatting, warnings-denied Clippy and full workspace tests with
  `--include-ignored`: **17 unit + 15 real PostgreSQL/ClickHouse integration
  tests passed**, none ignored in that run. Native tests include provisioning
  concurrency/replay, cross-platform/site isolation, staff role/audience/time
  rejection, one-use assertions, signed-body tamper/time rejection, credential
  rotation/suspension/erasure and lock-contended revocation before acceptance.
  Final exact-report tests prove top-100 independence, slash aliases, pageview
  filtering, read isolation and duplicate replay before ClickHouse merges.
- **2/2 real three-repository bridge scenarios**, including actual CMS worker,
  identity signer, Admin/theme renderers, hosting reconciler and Click stores.
  Tested consent-gated real browser collection, staff restrictions, lost
  rotation/disconnect acknowledgements, forced local config failure recovery,
  terminal restore with a fresh mapping, and old-queue isolation. Desktop and
  390px native Admin screenshots were inspected. The final run also checks
  post selection and retaining that selection while changing the report period,
  hosted backup/disable policy and existing-post links; both scenarios passed
  together in 59.7 seconds.
- **6/6 existing Playwright scenarios** passed together against a fresh 8083
  server using the same real development stores. The first run while competing
  builds were paging hit two 60-second timeouts; the final run used a 180-second
  harness limit and finished the entire suite in 49.6 seconds. No functional
  assertions were loosened. Temporary 8083 server was stopped; main 8082 remains.
- `npm ci`, TypeScript check and all tracker bundles passed. No private existing
  development-account password was found in source candidates. OpenAPI JSON,
  shell syntax and whitespace checks passed.
- CMS **88 database unit tests**, **72 headless API tests** and transactional
  outbox test passed on SQLite. The source outbox test and native signed-provider
  payment/refund/idempotency/forced-rollback test also passed on **real MySQL**.
  Test DDL uses raw execution where MySQL does not support prepared CREATE/DROP
  TRIGGER; source application persistence still uses parameterized queries.
- Hosting full workspace suite passed (one pre-existing ignored documentation
  example, not counted as verified), including 153 database unit tests, 44
  provisioning tests and existing web/billing/proxy/supervisor suites. The 44
  provisioning unit tests were rerun successfully after the backup-policy fix. Its
  separate **15/15 real MySQL tests** and four schema-parity checks passed.
  MySQL tests which return early without a server in the normal workspace run
  were explicitly rerun against the actual server, not treated as verified by
  their skip path.
- Sibling Clippy retains unrelated existing lint warnings under this newer Rust
  toolchain; warnings-denied runs stop on those older warnings. New native
  hosting and CMS lines have no diagnostics. Click's warnings-denied Clippy is clean.

Limitations that remain intentional: automatic CMS events do not stitch member
identities to browser sessions, so their revenue attribution is Direct unless
an application explicitly supplies consented session IDs through the backend
API. At the Priority 2 checkpoint, native reports covered basic traffic/activity/net revenue; Priority 3 extends them as documented below. Real public DNS/TLS, SMTP/Stripe account delivery,
independent SSO/security review, distributed budgets, backups/restore drills and
production/load/scanning gates remain priorities 4–6. No production readiness
claim, live payment, remote publication or Git commit was performed.

The complete setup/contract is [docs/GHOST_INTEGRATION.md](docs/GHOST_INTEGRATION.md),
with sibling [CMS notes](../vuuut/docs/native-click.md) and
[hosting notes](../vuuut-pro/NATIVE_CLICK.md). The optional bridge suite lives at
`tests/native-bridge`; `./run.sh native-test` links both sibling checkouts from
`VUUUT_DIR`/`VUUUT_PRO_DIR` in `.env` (default: next to this checkout) and uses
the trusted local Click environment. Set `NATIVE_TEST_BROWSER=1` and a working
`CHROMIUM_EXECUTABLE_PATH` to include its real browser phases.

Local runtime verified: Click is rebuilt and ready on **8082**, standalone CMS
is running its new optimized binary on **1971**, and hosting is running its new
optimized binary on **8080** (`http://pro.localhost:8080`). CMS migration 39,
hosting migration 5 and Click migration 10 are applied. All four existing hosting
tenants synchronized at their current revisions with no errors: two active and
two suspended. Read-only Click checks confirm matching lifecycle, verified
domains and valid mapped entitlements. Active configurations contain scoped
keys; suspended configurations contain only the disabled marker. Both active
public tenant pages returned 200 with the consent-required public tracker,
without their private integration keys, and migrated their CMS stores to 39.
The standalone native Admin route correctly returns 401 without staff login.

Private local platform/configuration files and the previous config backup remain
under ignored `.local` / hosting `run/`, mode 0600. Consistent pre-migration
SQLite snapshots are in ignored `.local/native-before-migration/`. Existing
account passwords are unchanged. Inactive Rust incremental caches were cleared
when the disk filled; compiled binaries and all data/source were retained.
CMS was started with a detached process session (automation's shell otherwise
terminated the nohup child); its normal pidfile/control script tracks it.

## Priority 3 — implemented 2026-11-09

After completing Priority 2, the user explicitly asked to continue on Priority 3.
Implement the remaining analytics breadth from the original brief: richer
content/engagement/returning/cohort reports and period comparisons, expanded
funnel options, newsletter-specific reports, additional attribution dimensions,
and ROI only with authoritative ad-spend inputs. Inspect current event semantics,
queries and UI before choosing additive schema/API changes. Define exact metric
semantics and verification before claiming a metric. Priority 2 is completed;
do not repeat it or stop to ask whether to proceed with Priority 3.


Implemented across Click, CMS and hosting:

- Exact current/previous UTC period comparisons, latest-instance vital
  comparisons and samples, richer content entry/exit/goal association,
  acquisition source/medium/channel/campaign/landing groups, event context
  filters and coarse operating-system reports.
- Optional consent/DNT/GPC-aware visible-time and scroll telemetry. Hidden time
  is excluded; SPA time stays with the page being left. In-flight batches and
  navigation beacons have distinct ownership; bounded retries preserve IDs and
  occurrence time. UTF-8 campaign limits and sub-60 KB batching preserve the
  collector contract. Telemetry defaults off and consumes the normal allowance.
- Provided-visitor new/returning coverage and weekly cohorts with incomplete
  follow-up cells unavailable. Session-only and legacy coverage remain distinct;
  there is no historical identity reconstruction. Funnels support literal page
  prefixes and provided-visitor scope across sessions, preserving configured
  conversion windows and strict event ordering.
- Edition-scoped newsletter send/open/click cohorts, unique recipients and link
  facts, with explicit missing-send diagnostics. CMS 0040 captures the facts
  transactionally and signs new edition links; legacy links remain supported.
  The worker hashes recipient identity separately per website/edition. CMS
  resolves titles locally; no titles, emails or raw member IDs go to Click.
- Currency-separated campaign costs and returns, extra revenue dimensions,
  stable source references/revisions, provenance, manager/integration-only
  writes and retention/deletion. ROI/ROAS require positive cost, complete daily
  coverage and observed matching attributed revenue. Missing data stays
  unavailable; no FX or advertising-provider verification is invented.
- Native CMS content, retention, newsletter, campaigns and performance views,
  Owner cost entry and optional measurement preferences. Hosted policy remains
  authoritative; staff exchange tokens still only read reports. Engagement and
  persistent identity default off; persistent identity requires consent.
- Updated typed Rust/TypeScript SDKs, OpenAPI and metric/deployment/privacy
  documentation. The marked demo includes sample editions, engagement, cohorts
  and explicit campaign-cost coverage, separated from real billable traffic.

Additive schemas: PostgreSQL **0011**, ClickHouse **0003**, CMS SQLite/MySQL
**0040**; hosting stays at **0005**. Applied migrations were not edited.
Old queued payloads retain compatible defaults for the new fields.

Final backend verification: formatting and warnings-denied Click Clippy pass;
**21 unit tests + 18 real PostgreSQL/ClickHouse integration tests pass** with
`--include-ignored` (39 total, none ignored). Tests cover period boundaries,
identity/cohort censoring, engagement dedup, cross-session/prefix funnels,
newsletter cohort matching, revisions/currency/coverage/roles and privacy.
TypeScript checks and all tracker bundles pass; base IIFE is 5,519 bytes.
CMS **73/73 headless API tests** pass on SQLite. The newsletter edition/open/
signed-link test and source-outbox test also pass on **real MySQL**; the open
rate update now uses portable ROUND and computes the rate before incrementing
its count. Hosting **44 provisioning unit tests** pass; its five MySQL skip
paths in that ordinary run are not counted as new database verification.
Priority 2's separate real-MySQL evidence remains recorded above.
Sibling Clippy retains existing unrelated toolchain warnings; changed CMS lines
have no diagnostics. Click strict Clippy is clean.

Two regression fixes were required during final checks. Retention sweeps now
share a PostgreSQL advisory lock and avoid scheduling ClickHouse mutations
when no expired rows exist, preventing empty sweeps from rewriting shared
aggregate parts under local memory pressure. Real-store tests cover both the
empty-sweep and contended-lock cases. Export form submission now redirects to
the GET job list, so refreshing while waiting cannot enqueue duplicate exports.
The browser regression also asserts exactly one job was created.

The metric definitions and deliberate measurement limits are in
[docs/ANALYTICS_METRICS.md](docs/ANALYTICS_METRICS.md). Automatic CMS revenue
still does not join member identities to browser sessions; use the backend API
with explicitly consented identifiers when appropriate. Without attribution,
campaign ROI is unavailable. Production commercial/operational/package gates
remain priorities **4–6**; they are not included in this completion claim.
All source changes remain uncommitted in the three separate repositories.


## Historical user-requested pause — 2026-11-09 (resumed below)

The user said: "Implement what you have done/think and pause, I will commit and
push and work later." Work is paused. No commit or push was made. Do not start
Priority 4 or further changes until the user resumes. Priority 2 is complete;
Priority 3 source changes are applied and running, with the final full browser
rerun below still required before calling its verification fully complete.

Latest verification and runtime:

- **3/3 real three-repository bridge scenarios passed**, including native
  desktop/phone reports, all new native views, cost entry, measurement
  preferences, consented public attributes and newsletter worker privacy.
  Screenshots now live in ignored `.local/native-screenshots`, so Root
  Playwright does not delete them when resetting `test-results`.
- Latest full Playwright run: **8/9 passed**. The engagement test's final
  comparison request hit ClickHouse's **total** 3.44 GiB memory limit during
  concurrent heavy builds/test activity. Its preceding exact 10,000 ms visible
  time, 8,000/2,000 ms SPA split, max scroll, consent/DNT and identity assertions
  passed. No assertion was relaxed; the request now asserts its status explicitly
  before decoding so future failures report the actual API error. The full
  nine-scenario suite must be rerun after heavy work is idle. The earlier
  export-refresh failure is fixed and that scenario now passes, asserting
  exactly one queued job. Do not describe the latest browser suite as 9/9.
- Backend final **39/39**, CMS API **73/73**, real MySQL edition/outbox tests,
  strict Click Clippy and tracker checks/build all passed as documented above.
  CMS and hosting ordinary Clippy runs completed; no diagnostics on new/changed
  lines. Existing unrelated warnings remain in those sibling repositories.
- Rebuilt/restarted optimized CMS and hosting, with final Click debug build
  running. HTTP 200 verified on **8082**, **1971** and **8080**. Standalone CMS
  schema **40**, hosting schema **5**, Click PostgreSQL **11**/ClickHouse **3**.
  All four existing hosted mappings are synchronized at their current revision,
  with no sync errors: two active, two suspended. Active pages return 200 with
  consent-required public trackers, no private key in HTML and CMS schema 40.
  Engagement/persistent identity remain off in hosting's default policy.
  Suspended configurations retain only the disabled marker and no credential.
  Unauthenticated standalone native Admin returns 401. Existing passwords and
  user data are preserved. The existing explicitly labeled demo was refreshed.
- Temporary **8083** test server was stopped for this pause. Main **8082**,
  standalone **1971** and hosting **8080** remain running. Services must be
  checked again after a device restart. Inactive Click incremental compilation
  caches were cleared to free disk; compiled binaries/source/data were retained.

Resume with readiness checks, then source the trusted Click `.env` and rerun
`npm run test:e2e -- --timeout=180000` with the working Chrome executable.
Use a dedicated temporary 8083 server as documented above to keep test limiter
state separate from the user's main app. Run database-intensive suites
sequentially on this 8 GB Mac; concurrent ClickHouse work can exceed its global
memory cap even with per-query limits. Inspect errors before deciding whether
resource configuration or query changes are warranted; do not retry silently or
weaken correctness assertions. Update this handoff after the actual final result.

Review all **three Git repositories** for the user's commit: Click, Vuuut and
Vuuut-pro. Their native adapters/migrations must move together. Never stage
`.env`, `.local`, `run/` private configs, SQLite databases, credentials or test
screenshots. No remote publication or production readiness claim was made.


## Priority 3 — final verification after resume, 2026-11-09

The user resumed with "Continue". Priority 3 implementation and local
verification are now **complete**. The earlier pause and 8/9 browser result are
historical; this section supersedes their pending-work status.

- The entire **9/9 Playwright suite passed together in 50.0 seconds** against
  the temporary 8083 server and the real development databases. No heavy builds
  or separate database suites ran concurrently. The previously failing final
  comparison request passed, along with the exact engagement timing/SPA split,
  scroll maximum, identity coverage, consent/DNT, retries, report controls,
  newsletter/cohort/cost UI, responsive layouts, owner security/session
  isolation and export-refresh single-job regression. No correctness assertion
  was relaxed. No new implementation change was needed.
- Main Click and the temporary test server logged **no errors since their latest
  restarts**. The earlier total-memory refusal did not recur. This verifies the
  local workflow under the tested conditions, not production load capacity.
- Runtime checks passed again: Click **8082** readiness, CMS **1971** and hosting
  **8080** return 200. All four hosted mappings remain synchronized with no
  errors. The two active public sites expose consent-required public tracker
  attributes, omit private credentials and use CMS schema 40. The two suspended
  sites retain only disabled markers. Hosted engagement/persistent identity
  remain off. Unauthenticated standalone native Admin returns 401. The main
  server serves the committed 5,519-byte tracker bundle.
- Existing backend **39/39**, native bridge **3/3**, CMS headless **73/73**,
  real MySQL edition/outbox, strict Click Clippy and tracker check/build evidence
  remains recorded above. No backend/SDK source changed during this resume,
  so those expensive suites were not repeated unnecessarily.
- Temporary **8083** was stopped after verification. Main **8082**, standalone
  CMS **1971** and hosting **8080** remain running with existing accounts/data.
  The new browser log is ignored `.local/priority3-browser-resumed.log`.

The user committed the Click implementation as `1133ae8` ("analytics breadth done, test pending") before this resume. CMS and hosting adapter/migration
changes are still uncommitted in their separate repositories; transfer all
three applications together. This resume updates only Click documentation with
completion evidence. No commit/push was performed by the agent.

The next planned priority is **4: production commercial validation**, followed
by operations and release/package gates. Those require their own work and,
where applicable, actual provider/deployment configuration; they are not part
of this Priority 3 completion. No production-readiness claim is made.


## Customer Events layout and typography — 2026-11-09

At the user's request, fixed `/dashboard?view=events` and slightly increased
text throughout customer report, advanced analytics, setup and settings panels.
The customer shell now has a scoped `customer-console` wrapper. Table rows are
13px (previously 11px), headers 12px (previously 9px), supporting text 12px,
controls 13px and panel headings 20px. Existing owner typography is preserved.

The Events filter form had been a direct child of the two-column summary grid,
stretching to the summary card's height and pushing the detailed table into a
half-width column. It now sits inside a full-width event detail panel. The
summary uses the available width, goals appear before the detailed results,
and the long result table scrolls within a 560px region with sticky headers
and keyboard access. Goal action cells retain proper table-cell layout, and
forms adapt to tablet/phone widths without document overflow.

Verified computed typography and overflow on 20 customer screens at 1440px,
768px and 390px, including the actual requested demo page. Keyboard scrolling,
filter alignment, full-width panels and goal ordering pass; desktop, phone and
dark screenshots were inspected. The existing nine-scenario Playwright suite
passes together, including goal editing/removal and event filtering. Formatting,
warnings-denied Clippy and all 39 backend checks pass across the workspace run
and focused reruns. The initial full database run hit ClickHouse's total-memory
limit during demo seeding; that single scenario passed on its focused rerun.
The workspace library run also passes all 21 unit tests. No assertions changed.
The earlier Priority 3 unit/integration split was corrected above: 21 unit and
18 real-store scenarios, still 39 total.

The rebuilt application is running on **8082** with the new stylesheet. Temporary
8083 was stopped after browser verification. No database migration, tracker
change, credential change, commit or push was needed for this design fix.
Private visual-check scripts/screenshots/logs remain under ignored `.local`.


## Priority 4 — implementation in progress, 2026-11-09

The user requested completion of Priority 4 (production commercial validation).
Work is authorized and active; no commit/push is requested. Preserve the Events
layout/customer typography improvements above. The operator said they will add
real Stripe test/SMTP configuration to ignored `.env`; the last presence check
still found those settings absent. Never print secrets or treat local mocks as
real-provider evidence. The required real scenarios and exact policies are in
[docs/COMMERCIAL_VALIDATION.md](docs/COMMERCIAL_VALIDATION.md).

Implemented locally so far:

- Additive PG 0012/0013: exact pending checkout snapshots/expiry/cancellation,
  mail expiry/delivery/error/source metadata and durable maintenance checks.
- One pending checkout per workspace, exact request/idempotency replay,
  acknowledged cancellation and later-event/current-customer mapping protection.
  Portal ownership is checked again after network I/O. Unknown/duplicate/invalid
  price mappings fail configuration; live keys are refused outside production.
  Real catalog preflight checks pinned Stripe price/portal configuration.
- Test downgrades include pending invitation seat reservations and refuse native
  or paid-provider overrides. Active/trialing, past-due/unpaid, paused/incomplete
  and cancellation policies are explicit. Cancelled retired prices still reduce
  entitlements. Billing never clears an operator suspension.
- Accepted-ID-only retries remain acknowledged after payment pause/downgrade,
  without new usage; newly inserted events remain atomically quota/entitlement
  guarded. Inactive billing keeps retry-compatible 429 with a distinct reason.
- Mail construction/SMTP/file errors all consume bounded attempts. Queue lifetime
  matches source tokens/invitations; consumed/revoked/expired links clear bodies
  and cannot be retried. Account/owner request cooldown includes consumed tokens.
  Terminal mail and expired token/invitation metadata have bounded cleanup.
- Owner Commercial status page/API show worker success/failure/staleness and
  queues, plus audited retry of valid dead mail. No bodies or credentials enter
  its responses. Skipped retention locks are not successful sweeps.
- CLI `commercial-check`, `commercial-mail-test`, `commercial-dns-test`, and
  configuration-free `--help`. Isolated Python helper prepares real-service QA
  stores/app on 8084, queues only to COMMERCIAL_TEST_EMAIL, supports separate
  owner recovery and checks its owned process/database namespace on cleanup.
  Tests are explicitly constructed with local mail/no provider so operator
  SMTP/Stripe credentials cannot accidentally affect fixture accounts.

Verification before runtime deployment: **46/46 Rust tests passed**, including
all ignored real-store tests with `--test-threads=1` (23 unit + 23 PostgreSQL/
ClickHouse integration scenarios). Strict Clippy passed. The new five commercial
integration scenarios cover exact retries/cancellation/late mapping, billing
status/quota replay, portal revocation race, mail expiry/dead/retry/cooldown,
actual retention outage/recovery/skipped-lock evidence, invitation reservations
and native plan protection. Two adapter unit tests exercise a real local HTTP
fixture for Stripe wire parameters, pinned version, signatures/modes and catalog
checks. Three Python safety checks pass. Earlier overlapping build/test attempts
hit a ClickHouse query timeout; use the clean `.local/priority4-backend-verified.log`
for the serial successful evidence. No assertions were weakened.

Private pre-migration PG archive `.local/priority4-before-control.dump` (0600)
was created with the matching PostgreSQL 17 container client; its archive listing
is readable. The host's pg_dump 14 cannot dump this server. This is a local
control-store snapshot, **not** the encrypted cross-store backup/restore gate.
Runtime rebuild/restart and the ten browser scenarios (including owner commercial
status) are still in progress at this checkpoint. The normal local servers must
use local mail/no payment provider during fixture verification. Actual provider
configuration is used only by the explicit isolated commercial helper/commands.
Real Stripe Checkout/portal/past-due and SMTP inbox/public DNS/TLS evidence are
still pending and cannot be called complete until actually exercised.


### Priority 4 continuation: verified credentials and external QA

The user responded "add it yourself" to the configuration request. Existing
private sibling configuration was inspected without printing secret values.
A CMS Stripe **test** key authenticated against Stripe; configured Gmail SMTP
credentials authenticated with TLS, and read-only IMAP inbox access succeeded.
These were used to create a dedicated Stripe **test-only** Vuuut Click product,
$1 monthly/$10 annual validation prices and a separate portal configuration.
These are validation fixtures, not approved commercial launch prices. Original
shared Stripe default portal settings were not changed.

Verified values were added atomically to ignored `.env` (0600), with a private
backup `.local/env-before-priority4`. `STRIPE_PORTAL_CONFIGURATION` selects the
dedicated portal. Secret discovery caches are private `.local/discovered-*.json`.
A Stripe test listener runs detached, forwarding the four configured snapshot
event types to **8084**; its identity is in `.local/priority4-stripe-listener.json`
and private log contains its signing secret. Never print that log or `.env`.
The optional Stripe plugin was discovered/suggested before local credentials
were found; it was not connected and is no longer needed for these HTTP tests.

The external helper has a live isolated app on **8084**, with manifest
`.local/commercial-validation/click_commercial_f41a8cfaccf94498a9fc7cb28e1c9833/run.json`.
Generated customer credentials are in that directory's private credentials.json.
**Real SMTP inbox receipt + link consumption passed** for verification, customer
reset and invitation. Links were single-use and reset revoked the prior session.
Evidence is `.local/priority4-real-mail-evidence.json`; private driver is
`.local/verify-priority4-mail-flows.py`. Do not re-run it blindly: links are now
consumed. Sender-auth flag extraction in that driver uses over-escaped regexes;
re-read only the scoped messages' headers and update evidence without reusing
links. IMAP access must remain read-only and BODY.PEEK; never log unrelated mail,
message bodies, raw headers/IPs or tokens. Owner reset is still pending **after**
customer billing validation (its helper promotes only the isolated QA account).

Real Stripe price/portal preflight passed (2 prices, test mode, features ready).
A real pending Checkout was created/retried idempotently and mapped in the QA DB,
but hosted browser navigation is blocked. Latest diagnosis: browser logs show
net::ERR_ABORTED, remain on the app Billing page; the app CSP uses **form-action
'self'**, which blocks the cross-origin 303 from its billing forms. Next fix:
allow ONLY checkout.stripe.com and billing.stripe.com as form-action targets on
/settings and /settings/action documents, preserve all other CSP directives and
other pages. Add a browser regression using actual settings CSP + intercepted
303/allowed-host responses, and prove a lookalike/unapproved host stays blocked.
Then resume real hosted Checkout UI (normal test-card flow; no CAPTCHA bypass).
Private inspector `.local/inspect-priority4-checkout.mjs` and logs are available;
its early over-escaped hostname regex was removed during diagnostics. Do not
print full external browser logs/URLs; mask the controlled inbox in screenshots.
No test payment has completed yet. Checkout session URL/state is private; prefer
retrieving its stored `billing_checkouts.url` from the isolated QA DB when resuming.

Query resource investigation: last Rust suite passed **47/47** (24 unit + 23
real-store scenarios), strict Clippy passed, build current. Local SMTP now limits
delivery to COMMERCIAL_TEST_EMAIL when ALLOW_LOCALHOST=true, preventing fixtures
from sending to third-party addresses. The store client also caps query/FINAL
reader threads at 2. Initial full browsers still hit transient total-memory
refusals despite small (<40 MiB) query peaks. ClickHouse allocator stats reported
~10 GiB resident on this 4 GiB Docker VM with heavy retained allocation. The
persisted ClickHouse container was gracefully restarted (no volume/table reset),
and main/test apps restarted. The final full browser suite then passed
**10/10 in 49.1 seconds**, log `.local/priority4-browser-final.log`. Keep these
verified results distinct from older 8/10 failures. Production memory/capacity
benchmarks remain Priority 5. The user data and original passwords are unchanged.
Main 8082 and temporary 8083 currently run local-file/no-provider configurations
through `.local/restart-priority4-local.py`; it strips real-service variables
from child environments only. QA 8084 uses the verified real test settings.
At final handoff, either switch the real Stripe listener to 8082 and restart the
main app with matching test settings after QA cleanup, or clearly keep/configure
separate forwarding targets. Do not leave main billing enabled with events
forwarded solely to an absent 8084 server. Original CMS 1971/hosting 8080 and all
four mappings were verified healthy after the schema upgrade; CMS schema 40,
hosting 5, Click PG 13/CH 3. Remaining work is the real billing/owner-mail scenarios,
CSP fix, final checks/docs and external-QA cleanup/reviewable runtime.


### Priority 4 final verification and active manual test — 2026-11-10

User resumed with "Continue". Implemented the settings-only CSP correction:
form-action permits self plus exact HTTPS checkout.stripe.com/billing.stripe.com;
all other pages remain self-only. Real Checkout and portal navigation now work.
Browser regression intercepts only provider destinations at Chromium Fetch,
drives actual settings forms, and proves a lookalike host is CSP-blocked before
any request. Ordinary Playwright routing did not intercept the post-303 target;
that fixture error was corrected without relaxing the security assertions.

Current verification: formatting/strict Clippy; **47/47 backend tests** (24 unit,
23 real-store, none skipped); TypeScript check; **11/11 browser scenarios together
in 52.8 seconds**; three Python guard tests, including owner-only config refusal.
Logs: `.local/priority4-csp-{clippy,backend,build}.log`,
`.local/priority4-browser-complete.log`, `.local/priority4-helper-final.log`.
Commercial phone screenshot inspected. The first browser attempt hit a real
ClickHouse total-memory refusal plus the redirect-fixture issue. Graceful CH
restart retained all stores; final suite passed unchanged. Retention later
recorded a transient memory refusal and automatically recovered on the next
sweep; all four maintenance checks were healthy afterward. Keep production
resource sizing/soak/load checks under Priority 5, not an asserted pass here.

Actual provider evidence (all private `.local/priority4-*.json`):
- Real Stripe catalog: two valid test recurring prices; selected custom portal
  features ready. Real test-clock paid subscription, exact signed-event replay
  (one ledger row, no usage increase), genuine held older event after annual
  change, failed renewal (new events429, accepted-ID retries202), paid recovery,
  scheduled cancellation retaining access, clock-driven Free downgrade and
  incomplete initial-payment refusal all passed. Dedicated generated QA mapping
  for API lifecycle is explicit; it is not fabricated Checkout evidence.
- Hosted Checkout **was completed manually by the user**. One completed Checkout,
  real paid test invoice and Pro/monthly were independently observed. Real
  portal navigation from the app passed; Update subscription and Cancel
  subscription controls were observed. User needed help finding the portal;
  opened it directly in a new foreground Chrome window. User said "I have seen
  those 2"; do not interpret that as submitted mutations. Last provider read
  still monthly, cancel_at_period_end=false. An async question asks them to
  submit annual change + period-end cancellation or explicitly defer them.
- Actual inbox verification/customer reset/invitation passed link consumption,
  replay refusal and customer session revocation. Separate owner-only helper
  (`setup --port 8086 --owner-only`) passed owner reset receipt/consumption/replay,
  old-session revocation and fresh owner login without touching manual QA user.
  Its app/stores/credentials were cleaned up. All messages INBOX; read-only IMAP
  and BODY.PEEK. Same-inbox Gmail headers contain no Authentication-Results or
  ARC results: SPF/DKIM/DMARC are unreported, not failed or proven. Evidence was
  corrected from false booleans to null/unreported values.
- User explicitly: "We will add the domain later, none of these" regarding the
  three connected Hostinger sites. Do not use any of them or ask again now.
  Public TXT/TLS/exact-origin/sender-alignment/abuse deployment checks deferred.

Runtime now: **main8082 uses real Stripe TEST + SMTP**, `.env`0600 matches its
separate active listener in `.local/priority4-main-stripe-listener.json` forwarding
DIRECTLY to8082. Main real preflight passed and actual signed test callback200
was observed. Temporary browser8083 stopped. Current public helper fixture tests
must strip Stripe/SMTP/COMMERCIAL variables before starting fixture servers.
The independent **manual QA8084 remains active**, original manifest:
`.local/commercial-validation/click_commercial_f41a8cfaccf94498a9fc7cb28e1c9833/run.json`.
Its original listener `.local/priority4-stripe-listener.json` forwards to the
scoped evidence proxy8085 (`.local/priority4-webhook-proxy.json`), then QA8084.
Do not stop them or remove its credentials until manual testing finishes or
is explicitly deferred. Two listeners deliberately handle isolated/main apps;
unknown subscription mappings are ignored after signature verification.

The 2 API subscriptions are terminal, their clock and customers were removed
with exact ownership/mode checks (`.local/priority4-api-cleanup-evidence.json`).
DELETE of an incomplete Stripe subscription returns incomplete_expired, which
is correctly treated as terminal/Free; the first private cleanup assertion was
corrected for that provider state. Primary manual Checkout subscription is NOT
part of that API scope and remains untouched. Dedicated test product/prices/
portal are retained for main app use, not approved commercial launch pricing.
Original shared/default portal and live objects remain unchanged.

Manual helpers: `.local/inspect-priority4-manual-state.py` safely prints only
plan/period/cancel flag/invoice count. `.local/priority4-manual-portal.mjs` keeps
user's current Chrome portal window open; manifest has its node PID, check
command identity before stopping. Old completed Checkout window was closed to
reduce memory. Read-only portal inspector `.local/inspect-priority4-portal.mjs`
never submits protected controls. Do not automate protected Stripe payment/
subscription confirmation UI. Await user submissions or explicit deferral.

Next: verify portal annual/cancellation against real app/provider, then cancel
ONLY the manual QA test subscription and expire its remaining open Checkouts;
cleanup QA8084 using the exact helper manifest, stop its owned listener/proxy,
retain main listener8082. Archive QA listener identity and make the main listener
canonical if useful. Close only our remaining test Chrome window. Update final
README/plan/runbook/handoff to exact observed evidence and cleanup. No commit,
push, live charge or public deployment. A credential scan found zero known key/
password values in reviewable changes; .env/.local remain ignored. Removed only
regenerable Rust incremental cache after confirming no compiler active to free
~2GB; sources/binaries/stores preserved, available disk about2.5GB afterward.


### Priority 4 COMPLETE in agreed test scope — final handoff, 2026-11-10

The user submitted both portal changes and said "I have done both". Independent
reads confirmed **Pro/active, annual/yearly interval, cancel_at_period_end=true,
one completed Checkout and two paid TEST invoices**. Evidence:
`.local/priority4-manual-stripe-evidence.json` and
`.local/priority4-manual-completion-evidence.json`. The earlier pending notes are
historical and superseded by this completion record. Protected payment/portal
confirmation controls were operated by the user, never automated.

Cleanup cancelled ONLY the isolated manual QA subscription, waited for its real
signed deletion webhook to return the QA workspace to Free, and deleted its
Stripe test customer. The separate two API fixtures were already terminal and
their clock/customers deleted. Original/shared/live objects were untouched.
Both generated commercial QA stores and their private credentials were removed;
QA8084, proxy8085, owner-only8086 and browser test8083 are stopped. Only controlled
test Chrome windows were closed. Private evidence/logs remain ignored.

**Main8082 remains running with actual Stripe TEST + controlled-inbox SMTP**.
The canonical `.local/priority4-stripe-listener.json` and
`.local/priority4-main-stripe-listener.json` refer to the SAME active process,
forwarding DIRECTLY to http://localhost:8082/v1/billing/webhooks/stripe. The old
QA listener identity is archived in
`.local/priority4-qa-stripe-listener-completed.json` and is stopped. Never print
listener logs: they contain the signing secret. Main private `.env`0600 matches
its listener. Dedicated test product/prices/custom portal remain configured for
main local use; no shared default portal or live object was modified.

Final verification remains **47/47 backend** (all real-store scenarios included),
**11/11 browser**, strict Clippy/formatting, TS check, three helper guards and
clean Git diff checks. Main real preflight: mode=test, two valid prices, selected
portal/features ready; actual main signed callback200. Original customer and
owner credentials still sign in, independent sessions remain valid and the
existing Events page200. CMS1971 and hosting with Host pro.localhost:8080 return
200; integration mappings remain2active/2suspended. All four main fulfillment
checks had recent successful runs at handoff; a transient CH memory refusal
recovered automatically. Sustained Docker/production sizing remains P5.

User deferred domain setup explicitly; none of the connected Hostinger sites
may be used. Public TXT/TLS/exact-origin/sender SPF/DKIM/DMARC and bounce/abuse
contact checks remain for the eventual deployment. Same-inbox Gmail receipt is
real evidence of the four account mail flows, but headers did not report sender
authentication. No public domain change, live charge, commit or push occurred.

Next priority is **5: operational release gates**, when the user requests it.
Do not reopen completed P4 code or call the entire original project production
ready. See docs/COMMERCIAL_VALIDATION.md for exact evidence and limitations.
For future fixture browser runs, strip STRIPE_/SMTP_/COMMERCIAL_ variables from
that server's child environment and use a separate port; Rust integration tests
explicitly construct local mail/no-provider services. Current main must retain
its matching active test listener when exercising real billing. A machine
restart requires restarting that listener (or removing all Stripe configuration
until it is available); run.sh does not create an external webhook endpoint.
No remaining P4 implementation work is pending after these accepted deferrals.

## Priority 5 complete locally — paused, 2026-11-10

Implemented distributed admission/report leases, cooperative backup barriers,
private metrics and OTLP, provisioned monitoring/alerts, encrypted restic recovery,
Linux hardening and repeatable capacity/security validation. Main8082 runs the
validated candidate with original accounts/data and real TEST Stripe/scoped SMTP.
No commit or push. Do not start Priority 6 until the user resumes.

Final checks passed: 51 backend tests (27 real-store), all 11 browser scenarios,
six Python safety checks, strict Clippy and formatting. Actual main-data encrypted
restore verified both stores/configuration in 13.28 seconds. Local recovery replay,
1M resident synthetic rows and 5x arrival profiles for 1M/10M/100M monthly passed
(p95 0.055/0.066/0.305 seconds). Private-CA Linux TLS/hardening, real Prometheus
scrape/alert delivery/Grafana and source/license/secret/runtime scans passed;
zero high/critical or secret findings, no advisory ignores.

Evidence: `.local/priority5-backend.log`, `priority5-browser-bounded.log`,
`priority5-python-final.log`, `priority5-clippy-final.log`,
`priority5-main-restore-final.log`, `priority5-security-verified.log`, and curated
`.local/ops/{validation,linux,monitoring,security}-evidence.json`.
The temporary browser server is stopped. Main8082, its matching Stripe listener
and private local monitoring remain available. Local encrypted repository/password
are private under `.local/ops`; this is not an off-machine or escrowed backup.

External deployment gates remain: the user's later domain/public TLS and licensed
GeoIP, off-machine backup/escrow, full-volume sustained capacity qualification and
independent security review. Local arrival profiles do not qualify 100M resident
rows. See `docs/OPERATIONS.md` for commands, limitations and recovery procedures.
The checked-in ClickHouse read-buffer profile and chunked Native backups address
the local 4GB VM memory refusals without weakening counts/report assertions.

## Priority 6 — local release implementation complete, 2026-11-10

Added versioned @vuuut/click and click-sdk packaging, declarations, actual installed
npm consumer/type/wire checks, Cargo package verification, browser/operator bundles,
source/migration SHA256 manifests, SHA384 SRI and explicit pinned-key signatures.
The validation private key is ignored and 0600; it is not a production identity.
Signature guards reject changed bytes/manifests, substituted keys and path escapes.

OpenAPI covers all 85 API/health/form operations with 46 response/data schemas,
shared error/content/retry definitions and exact route/method coverage. Both
retention methods and empty funnel PATCH are correct. The live checker validates
28 real responses, including existing-account reports, viewer export denial and
independent owner authorization. Restored overwritten API/security documents and
added ADMIN_GUIDE.md, MIGRATIONS.md, RELEASE.md and RELEASE_NOTES.md.

Local checks: strict Clippy/all targets and formatting, 11 Python safety checks,
three SDK wire checks against source and the installed tarball, strict TypeScript
consumer/check/build, packaged Rust compilation, actionlint and source/license/
secret scan passed. No high/critical or secret findings. The unchanged tracker
JavaScript bundles retain the previous 11-browser/51-backend evidence; these full
suites were not rerun for documentation/packaging changes. Main rebuilt to embed
the updated contract; original customer/owner/Events and TEST Stripe/SMTP smoke
passed. No account/data/password changes, migration or sibling modification.

The actual remote is https://github.com/anisulkibria/vuuut-click. Inspected run
42689087362 for committed P5 revision4fca5c3: security/image passed; verify failed
at Playwright Chromium installation after ubuntu-latest moved. CI now pins
Ubuntu24.04, fixes the components input, updates Node-based actions and adds
contract/package validation. These new corrections are not remotely verified.
Release workflow requires a matching tag ref and green push CI on its exact SHA,
attests each artifact and creates a draft release. Optional npm/Rust publication
uses separate sdk-release environment jobs after provenance verification.

Candidate artifacts: `.local/releases/0.1.0-priority6-final/` (four artifacts,
manifest/checksums/signature/public key; local validation identity, dirty candidate).
Evidence: `.local/priority6-{package-final,live-contract,clippy,python,workflows,
main-smoke,security,signature-final}.log`, `priority6-security-evidence.json` and
`priority6-remote-ci.json`. Production/registry signing identities and package-name
ownership/configuration have not been supplied. The user must commit/push, obtain
green remote CI, configure registry trust/secrets and dispatch the tagged release.
Do not call public publication or production acceptance complete. External P5
public-domain, GeoIP, off-machine escrow/soak/independent-review gates still apply.

## Native CMS analytics, connections and cache cleanup — 2026-11-10

Resumed at the user's request to finish Click-backed CMS analytics, automatic Pro
connections and easy external provider setup. No commit, push or public deployment.

- Connected CMS `/ghost/analytics/` now uses Click for traffic, comparisons,
  content/engagement, returning visitors, newsletter editions, campaign returns
  and performance. Growth/editorial email facts remain local. Post analytics
  displays scoped traffic, daily views, sources, countries and campaigns inline.
  Connected traffic APIs use exact Click date projections and canonical slash
  aliases, rather than silently reading historical local page_views.
- Background CMS setup registers pinned signing keys before any Admin visit.
  Pro's existing revisioned reconciliation still provisions active tenants
  automatically after one operator platform setup. The legacy server collector
  is disabled under native/managed policy so it cannot bypass browser consent.
  Staff report requests run at most three remote requests concurrently.
- Owner console **Connections**, `/owner/connections`, manages MaxMind, SMTP,
  Stripe, OTLP and hosting platform registration. Credential generation and a
  copyable private Pro configuration are provided. PostgreSQL migration0015
  stores service-bound AES-GCM ciphertext with global monotonic revisions.
  SMTP/Stripe/GeoIP snapshots reload live, across processes within five seconds;
  exporter changes require restart. Secret fields never return saved values.
- MaxMind downloads use the fixed official licensed Country archive, bounded
  HTTPS redirects/bytes/time, MMDB validation and atomic replacement. Failed
  downloads retain the prior reader. Workers check hourly for a daily update.
  Production owner setup can start without GeoIP, while readiness/browser
  collection remain503 until configured. Licensed real downloads still need
  operator credentials; none were invented or redistributed.
- Encrypted backup bundles include the matching connection encryption key and
  managed MMDB, with integrity verification and safe restore member checks.
  Production mount/key/replica/recovery instructions: docs/CONNECTIONS.md.

Verification: **55 distinct Click backend checks** passed (54-test full workspace
run with all ignored database tests included, then both connection scenarios
including the added production-readiness/bootstrap case); **3/3 native bridge
scenarios**, including real CMS/platform adapters, Ghost-compatible stats,
slash-alias unique counts, consent, rotation and actual Chrome browser checks;
**11/11 dashboard Playwright scenarios**, including Connections credential
masking/generation and mobile layout; strict Clippy/all-targets, formatting,
tracker TypeScript/build, JavaScript syntax, OpenAPI exact85-operation coverage,
and **4 backup guard tests** pass. An initial browser run hit ClickHouse's
process memory limit after the database suite; a graceful ClickHouse restart
preserved volumes and the unchanged entire browser suite passed in53.0s.

At the user's request, cleared old Rust build trees, writable npm caches, package
manager/Python/Composer/node-gyp/tool caches and Docker build cache. Disk free
space increased from1.8GiB to about29GiB after rebuilding. A small root-owned
npm cache remains because noninteractive sudo required an administrator password.
A shared `.dev-build` with debug info/incremental compilation disabled replaces
separate large build trees for this work. Source, private configuration, licensed
files, local accounts, mail, databases and backup evidence were preserved.

Cleanup removed the CMS executable that idle Pro tenants depended on. Restored
`hello.localhost:8080`, verified its homepage200 and Admin sign-in200 in a real
browser, and installed runtime copies outside build caches:
`Vuuut-pro/run/bin/ghost`, `Vuuut-pro/run/bin/ghost-pro`, and
`Vuuut-click/.local/bin/click-server`. Compatibility symlinks remain under the
old target paths. Private Pro config now uses `paths.ghostBinary=bin/ghost`, so
waking tenants no longer depends on the CMS build directory. Pro8080, standalone
CMS1971 and Click8082 run the final binaries. Temporary browser8083 and its
isolated PostgreSQL/ClickHouse stores were stopped/dropped after verification.
The CMS launcher now starts a detached process session like Pro's launcher,
fixing a standalone process that otherwise died when its automation group ended.

Runtime copies are installed artifacts, not caches. Preserve their directories,
`CONNECTIONS_DATA_DIR` and encryption key when cleaning/recovering the device.
The local runtime copies should be atomically updated after future builds.
Public deployment, real licensed MaxMind acceptance and earlier external release
gates remain operator work; no universal Tinybird wire compatibility or
production-readiness claim is made.


## Native macOS databases and lifecycle commands — 2026-11-10

User requested native local databases without Docker and a full `run.sh` command
interface. Implemented `scripts/local.py`, pinned/checksummed official macOS
ClickHouse installers and matching Homebrew PostgreSQL 17 tools. `run.sh` now
supports detached `start` (default), `stop`, `restart`, `status`, `setup`, `doctor`,
`build`, `rebuild`, `test`, `log`/`logs` with service selection/follow, `db-start`,
`db-stop`, and explicit one-time `migrate-docker`; legacy `--stop` remains valid.
`LOCAL_DB_MODE=external` leaves database lifecycle unmanaged. Normal commands
never invoke Docker. Build/install preserves runtime copies outside caches;
restart builds before stopping the live services. Private control locking,
listener/PID ownership checks and graceful shutdown preserve unrelated services
and durable data. Native setup persists matching PostgreSQL backup client paths.

Migrated the active installation from Docker to project-owned native services:
PostgreSQL 17 on55432, ClickHouse25.8.4.13 HTTP58123/TCP59000, app8082. Existing
credentials, accounts, provider encryption key/licensed files, usage, pending
outboxes and native CMS/Pro mappings are retained. Migration compared counts and
sorted row fingerprints for all37 PostgreSQL tables,595,745 distinct immutable
analytics rows and20,960 logical hourly aggregates before cutover. The verified
private snapshot and receipt remain in `.local/native-migration` and
`.local/native/migration-complete.json`. Original Docker volumes remain intact;
source database/monitoring containers and Docker Desktop's engine are stopped.
Never start those source containers on the native ports. No Docker data was
pruned as part of this conversion.

All active database data is in `.local/native`, not a cache. Keep it with `.env`,
`.local/bin`, `.local/connections` and the matching encrypted backups during
cleanup/device moves. Failed migration snapshots/targets also remain private;
do not mistake them for the live native directory. An interrupted target stop
refuses to move live database paths. Native ClickHouse uses bounded caches,
small background pools and1.5GiB/256MiB server/query limits. A restart following
the full database suite exposed global-worker starvation while loading dropped
test metadata; increasing global headroom and limiting loader pools fixed it.
The stalled startup was recovered without changing data, and subsequent graceful
stop/restart after both integration/browser/operations suites passed. The pin
matches the source engine; native macOS support and production release limits
are documented in docs/NATIVE_LOCAL.md.

Verification with the Docker engine stopped: **55/55 backend checks** (all ignored
real-store tests included), **3/3 native CMS/Pro bridge scenarios** including real
Chrome, **11/11 dashboard browser scenarios** in46.5s, **21/21 Python guard checks**
including9 native lifecycle/migration guards, formatting, shell/Python syntax,
85-operation exact contract coverage and28 authenticated live response checks.
A bounded native operational run (`--duration 5 --volume-rows 0`) verified encrypted
snapshots and isolated restore/replay, matching-major clients, wrong-key/pack
corruption rejection, maintenance admission, exact metering/delivery and actual
OTLP privacy. It is compatibility/recovery evidence, not a capacity qualification.
The previous larger operational evidence was retained separately. Current private
evidence: `.local/native-*-tests.log` and `.local/ops/validation-evidence.json`.

Actual setup/start/repeated-start/stop/restart/status/doctor and legacy stop were
exercised; the final setup/restart/start/doctor passed with a PATH containing no
Docker executable. Stop released the owned service ports. Main Click readyz,
hello.localhost:8080 homepage/Admin and standalone1971 all returned200. No commit,
push, public deployment, licensed-provider acceptance or production release
claim. Documentation: docs/NATIVE_LOCAL.md and docs/OPERATIONS.md.
