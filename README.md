# Vuuut Click

Independent Rust analytics SaaS in the same Material 3 style as Vuuut and Vuuut-pro. Rust renders the dashboard directly; there is no React/Next/Node server. PostgreSQL durably accepts events and meters usage; a retrying worker delivers to ClickHouse; reports query actual analytics data.

The main standalone SaaS workflows and advanced reports are implemented for local development. Native Vuuut/Vuuut-pro integration is complete and verified locally; Priority 3 analytics breadth is complete and verified locally, including engagement, retention/cohorts, newsletter editions and campaign costs. Priority 4 commercial safeguards are implemented and verified with 47 backend checks, 11 browser scenarios, real Stripe test-mode lifecycle events and controlled-inbox account/owner email. Manual Checkout, portal interval change and scheduled cancellation passed; public-domain setup is deferred by the operator. Verification is recorded in the handoff and [commercial validation runbook](docs/COMMERCIAL_VALIDATION.md). **Production release gates remain unfinished.** Start with [CONTINUE.md](CONTINUE.md) to resume on any device; the full original requirements are preserved in [docs/ORIGINAL_BRIEF.md](docs/ORIGINAL_BRIEF.md).

## Run and experience the app

Requirements: Rust stable, Docker Desktop/Engine with Compose, OpenSSL. The first run downloads database images and compiles Rust.

```sh
./run.sh
```

The public homepage is **http://localhost:8082/** and stays accessible for signed-in users and owners. It links to user login and workspace signup.

Open **http://localhost:8082/signup** to register. Register a workspace, add a website, and install its tracking code. To try locally, add `http://localhost:8082` as your website origin. The **Open real tracker test page** link sends real browser events; wait a few seconds, then view your dashboard. A separate, labeled demo website provides sample data without consuming billable usage. To create it after registering, source your trusted `.env` and run `./target/debug/click-server seed-demo your-account@example.com`. Real websites continue to show only real collected events. Local visits have Unknown country; set `GEOIP_PATH` in `.env` to your licensed GeoLite2-Country.mmdb for public-IP country enrichment. Test fixtures are never enabled by run.sh.

`run.sh` creates a private `.env`, starts only this app's PostgreSQL and ClickHouse containers, applies migrations, and launches Rust. Ctrl+C stops a foreground Rust process gracefully; `./run.sh --stop` stops the managed background server (when present) and databases without removing volumes. Edit Rust and rerun `./run.sh` to compile and replace this project's managed server. Ports: app 8082, PostgreSQL 55432, ClickHouse HTTP 58123; all bound to loopback. Existing Vuuut/Vuuut-pro projects are untouched.

Owner sign in: **http://localhost:8082/owner/signin**. User sign in: **http://localhost:8082/signin**. The portals have independent session cookies and can stay signed in with different accounts in the same browser. The owner console shows actual platform totals and customer directories, with workspace suspension/reactivation, manual allowance grants, dead-letter retry and auditing. Ownership is explicitly granted by an operator using `click-server grant-owner <existing-account-email>` after loading the trusted environment. Customer workspace owners do not automatically gain platform access.

From the dashboard, open **Advanced analytics** for funnels, journeys, net revenue attribution and Web Vitals. Open **Workspace & settings** for multiple workspaces, team invitations/roles, websites, credential rotation, billing/usage, retention, durable CSV exports/deletion, profile, password and verification. Local billing and mail are explicitly labeled development modes; real Stripe and SMTP require operator configuration. The default demo website populates all analytics report views.

The tracker supports consent, DNT/GPC, SPA navigation and an optional Web Vitals addon. Authoritative backend events require separate integration credentials; browser events cannot create revenue. Rust and TypeScript API clients and OpenAPI contracts are included.

## Verify

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Source only your trusted local configuration; the ignored integration test creates/drops isolated test databases.
set -a
source .env
set +a
cargo test --workspace -- --include-ignored
# Browser SDK and real end-to-end checks, with ./run.sh running in another terminal:
npm ci
npm run check
npm run build
npx playwright install chromium
npm run test:e2e
```

The normal unit command explicitly skips the real-stack integration test. `--include-ignored` is required to verify it. Browser tests register distinct test workspaces, exercise real collection and explicitly seed separate demo websites for demo-report checks. They do not replace real website data.

## Documentation

- [Resume, device setup and remaining work](CONTINUE.md)
- [Original detailed product brief](docs/ORIGINAL_BRIEF.md)
- [Architecture and source/license review](docs/ARCHITECTURE.md)
- [Database design and metrics](docs/DATABASE_DESIGN.md)
- [API contracts](docs/API_SPECIFICATION.md) / [OpenAPI](docs/openapi.json)
- [Tracker and consent](docs/TRACKER.md)
- [Security/privacy and release gates](docs/SECURITY.md)
- [Commercial policies and actual provider evidence](docs/COMMERCIAL_VALIDATION.md)
- [Deployment and operations](docs/DEPLOYMENT.md)
- [Native Vuuut/Vuuut-pro integration and setup](docs/GHOST_INTEGRATION.md)

GeoLite data attribution: This product includes support for GeoLite data created by MaxMind, available from https://www.maxmind.com. Licensed databases must be supplied by the operator; none are redistributed. Tinybird MIT attribution is in docs/TINYBIRD-LICENSE.

Platform owners can open **Account & security** at `/owner/account` to change their display name/password, sign out other sessions, and enable authenticator MFA with one-use recovery codes. Owner recovery starts at `/owner/forgot-password`; it preserves MFA. Team invitations queue email through the configured transport (private files locally). Report overload and validation errors provide readable browser pages and structured API errors.
