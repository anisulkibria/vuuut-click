# Vuuut Click — continue from here

Last handoff: 2026-11-08 (Asia/Dhaka). Work is intentionally **paused at the user's request** because weekly Codex/ChatGPT usage is nearly exhausted. Finish the checkpoint, keep the local app running, and do no further feature development until the user asks to resume. On another device, open this repository in Codex and say **continue**: AGENTS.md tells the agent to read this file first.

## What the user is building

An independent commercial web analytics SaaS called **Vuuut Click**, with Rust application/server/dashboard, PostgreSQL control/durable acceptance, and self-hosted ClickHouse analytics. It must work for arbitrary websites, standalone Rust Ghost/Vuuut installations, and managed Rust Ghost(Pro)/Vuuut-pro tenants. Native CMS/hosting integration is a later unfinished milestone. The exact original 1,109-line requirements are portable in [docs/ORIGINAL_BRIEF.md](docs/ORIGINAL_BRIEF.md), not in an inaccessible old attachment.

Accepted decisions and follow-up requirements:

- Rust stable, Axum/Tokio/SQLx, official ClickHouse client, Rustls; dashboard is native Rust-rendered HTML/SVG. TypeScript is used for the browser tracker/API client only; no Node runtime or React server.
- Rose Material 3 style matches existing Vuuut and Vuuut-pro; accessible responsive tables, light/dark/system themes.
- Keep this service independent. Sibling repositories were inspected but not modified. Do not assume their schemas/interfaces without reading the actual code again.
- `run.sh` starts the complete local stack on **http://localhost:8082** and supports iterative development.
- Public homepage `/` always stays accessible, including while logged into either portal.
- Customer `/signin` and platform-owner `/owner/signin` must be different, with independent same-browser sessions. Workspace owner is not a platform owner.
- Owner console uses the same left-sidebar style and real platform directories/actions.
- Populate analytics with visibly labeled dummy/demo data so each report can be experienced. Keep demo records separate from real websites and billable usage.
- User requested completion of the main SaaS, then asked to pause after finishing the current planned checkpoint. Do not describe the entire original production brief as completed.

## Checkpoint implemented

**Customer dashboard** (`/dashboard`): overview chart, real-time visitors, unique visitors/pageviews/sessions/bounce, audience/countries/devices/browsers, acquisition/referrers/campaigns, content/pages, events and event-match goals. Detail pages have their own highlights; the traffic chart is on Overview only.

**Advanced analytics** (sidebar → `/insights`): ordered page/event funnels, session journeys, first-touch/last-touch/last-non-direct net revenue attribution with a 30-day acquisition lookback, separate currencies/refunds, and latest-instance Web Vitals p50/p75/p95. Optional `vitals.js` uses official MIT web-vitals. Page funnel steps count pageview events only; tests caught and fixed a performance-event overcount.

**Workspace & settings** (sidebar → `/settings`): multiple organizations, rename/switch, team invitations with one-time fragment links and seat reservations, role updates/removal with last-owner protection, website name/origin/collection changes, read/integration credential creation/revocation/atomic rotation, usage and plan allowances, retention, durable CSV exports/download, website/workspace deletion, profile, password change, other-session revocation, email verification and password reset. Invites currently return a shareable link; automatic invitation email is not implemented. Viewer permissions intentionally hide management controls.

**Billing/account services**: provider-neutral interface plus Stripe HTTP adapter, checkout attempts/idempotency, signed raw-body webhooks with timestamp/mode checks, authoritative current-subscription lookup, transactional dedup/entitlements, billing portal. No entitlement upgrade on a checkout return URL. Without credentials, local test plans record no payment. SMTP supports STARTTLS/implicit TLS; development writes private `.local/mail/*.eml`. Hashed one-use reset/verify tokens, expiry and session revocation. Production registration needs SMTP, and real website creation requires verified email.

**Platform owner** (`/owner/dashboard`): separate explicit owner grant and 8-hour `/owner` cookie, real paginated Workspaces/Users/Websites/Subscriptions/Event delivery/Audit directories, suspension/reactivation, manual plan grants (cannot override provider-linked subscriptions), dead-letter retry preserving IDs and retention, platform audit. Customer and owner cookies cannot substitute for one another.

**Data correctness/privacy**: PostgreSQL transaction commits accepted payload+metering; replay-safe raw `FINAL` queries and unique-state hourly aggregates; outbox retry/dead letters; organization/site isolation; exact Origin and verified domain; public collection IDs never authorize reads. Browser timestamps bounded to 7 days past/60 seconds future, reserved server names rejected, integration-key-only monetary conversions. Deletion stops collection before storage removal. Retention uses the smaller website/plan limit, purges hourly buckets and discards expired queued payloads to prevent replay resurrection. Exports wait for ingestion and fail explicitly on dead letters/over 100k rows; CSV formula escaping; 24-hour expiry. Process-wide report concurrency and ClickHouse time/memory/result bounds.

**Demo**: CLI `seed-demo <registered-customer-email>` creates/refreshes only a marked separate demo website, 30 days of varied traffic, sources/campaigns/countries/devices, custom events, four goals, a funnel, net revenue/refunds and five Web Vitals. No collector or metering for demo. Default first site on the dashboard; banner clearly says Demo data.

**Developer assets**: Rust `click-sdk`, server-side TypeScript `tracker/src/api.ts`, checked-in bundles in `tracker/dist`, OpenAPI, deployment templates, CI, architecture/license/security docs, real-stack and browser tests. `run.sh` now builds and replaces this project's managed process while retaining volumes.

## Current local access and state

Original machine project: `/Users/anisulkibria/Documents/Rust/Ghost/Vuuut-click`. This absolute path is not required on another device.

- Homepage: http://localhost:8082/
- Customer: http://localhost:8082/signin → `/dashboard`
- Owner: http://localhost:8082/owner/signin → `/owner/dashboard`
- Existing local customer email: `user@vuuut.local` (viewer in its shared workspace).
- Existing local platform owner email: `admin@vuuut.local`.
- Passwords are unchanged and stored only in ignored **`.local/development-accounts.json`**, mode 0600. Never put those passwords in tracked docs, commits, output or production defaults.
- Existing demo website: `Vuuut Journal · Demo`, id `2220b39d-ac4c-4830-9ed4-c242718129c0`. Existing real website stays separate.
- Local private `.env`; Docker PostgreSQL on 55432, ClickHouse HTTP on 58123. Persistent named Docker volumes. Health `/healthz`, dependencies `/readyz`.
- Main server log/PID: `.local/server.log`, `.local/server.pid`. Temporary staging 8083 was stopped; the verified build is running on 8082. Verify readiness; do not assume a process survived a device restart.
- Migrations 0001–0006 PostgreSQL and 0001–0002 ClickHouse. SQLx checksums and ClickHouse `control_migrations` ledger. Never edit an applied migration; add a new one.

## Clone and run on another device

Git transfers source/docs, **not local databases, users/passwords, sessions, demo rows, `.env`, mail or licensed GeoIP data**. To retain the exact existing accounts/data, separately transfer encrypted consistent backups of both databases and private configuration through your own secure channel. Automated encrypted cross-store backup/restore remains unfinished; never push Docker volumes or `.env` to Git.

For a clean development install:

```sh
# Inside the cloned Vuuut-click repository; install Rust stable and Docker first.
./run.sh
```

This generates private local secrets, starts databases, migrates, builds Rust, and serves 8082. Node is needed only to change/build the tracker or run browser tests; committed bundles let Rust start without it.

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

Edit Rust, then rerun `./run.sh` to rebuild/restart this project's managed app. Ctrl+C stops the foreground app; `./run.sh --stop` stops managed app/databases and preserves volumes. Optional already-built local restart helper: `python3 scripts/restart-local.py` after sourcing `.env`; it uses `lsof` and checks the listener before replacing it.

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

Reproduce checks (with Docker services running):

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

On the original Mac, the working installed Chromium override is `/Users/anisulkibria/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell`. This path is machine-specific; on other devices install Playwright Chromium normally. Source `.env` for browser tests: owner/demo tests use the local operator CLI. Browser tests create their own test workspaces in the local development DB. Database integration tests create/drop isolated `click_it_*` databases. Do not delete arbitrary databases to clean up a failed test.

## Resume priorities — do not restart from scratch

1. **Finish product hardening around existing workflows**: review all mutations for membership/role rechecks after row locks; account/password and suspension races; consistent HTML/API validation errors; handle report concurrency/rate limits cleanly; correct privilege-lost/no-op errors. Add targeted race/negative tests. Goal deletion exists as API only; goal/funnel edit UI/API remains missing. Invitations currently require manually sharing the link. Owner account profile/password/recovery/MFA UI is unfinished. Evaluate per-tenant query limits and bounded journey aggregation for very large sessions.
2. **Native Ghost/Vuuut and Vuuut-pro integration**: read docs/GHOST_INTEGRATION.md and re-inspect actual sibling code (not included in this standalone repo). Implement versioned idempotent provisioning, domain/lifecycle/entitlement synchronization, staff JWT/JWKS exchange with pinned issuers/audiences, server transactional CMS outboxes, retries/rotation/disconnect, tracker injection and native Admin reporting. Existing generic backend conversion endpoint/SDK is a foundation, not completed native integration. Never expose platform/integration credentials in browser content.
3. **Remaining analytics breadth** from original brief: richer content/engagement/returning/cohort and period comparisons, expanded funnel options, newsletter-specific reports, additional attribution dimensions, ROI only when authoritative ad-spend data exists. Define metric semantics and exact tests before UI claims.
4. **Production commercial validation**: operator-configured Stripe test-account checkout/portal/cancel/past-due/webhook scenarios, real SMTP verification/reset delivery and invitation mail, email/DNS/abuse policies, plan/downgrade edge cases, retention fulfillment monitoring. No live charges are authorized by this local project task.
5. **Operational release gates**: OTel and metrics/dashboards/alerts, distributed edge/request budgets, encrypted off-site backups with cross-store consistency and verified restore drills, dependency/image/license scanning, Linux image/TLS deployment, capacity/load benchmarks at 1M/10M/100M monthly events, independent security review. NATS JetStream is optional only when distributed queue requirements justify it; current PostgreSQL outbox is durable. Keep readiness, retry and dedup behavior through migrations.
6. **Release/package work**: version/publish signed browser and Rust SDKs, OpenAPI response-schema completeness, operator/admin and migration documentation, CI execution on actual remote and production acceptance evidence. Maintain license notices. Do not assert production-ready until all gates genuinely pass.

External inputs eventually needed: actual deployment domain/TLS, MaxMind account/licensed Country MMDB, SMTP credentials, Stripe keys/price IDs/webhook secret, backup storage/secrets. Ask only when an actual next step requires them; continue independent code/test work first.

## Source map

- `crates/click-core`: validation, identity hashing, trusted proxy/GeoIP, roles.
- `crates/click-store`: storage, versioned migrations, acceptance/metering/outbox, base reports.
- `crates/click-server/src/analytics.rs`, `insights.rs`: advanced query API and Rust report UI.
- `workspace.rs`, `manage.rs`, `settings.rs`: organizations/team/credentials/account and UI.
- `billing.rs`, `mail.rs`, `jobs.rs`: providers/recovery, exports/deletion/retention workers.
- `auth.rs`, `owner.rs`, `dashboard.rs`, `landing.rs`, `lib.rs`, `main.rs`: auth, portals, routes/middleware/startup.
- `demo.rs`: isolated local sample seeder.
- `crates/click-sdk`, `tracker/src/{index,api,vitals}.ts`, `tracker/dist`: SDKs and bundles.
- `migrations/{postgres,clickhouse}`: additive schemas. `dashboard`: scoped CSS/landing templates.
- `crates/click-server/tests`, `tests/browser`: verification. `deploy`, `scripts`, `.github`: operations.

The next agent should state which unfinished priority it is tackling, preserve all user decisions above, update this handoff with actual evidence, and keep localhost:8082 usable during development. Do not repeat permission requests for already authorized reversible local work. Stop when the user asks to pause.
