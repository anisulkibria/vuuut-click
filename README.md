# Vuuut Click

Independent Rust analytics SaaS in the same Material 3 style as Vuuut and Vuuut-pro. Rust renders the dashboard directly; there is no React/Next/Node server. PostgreSQL durably accepts events and meters usage; a retrying worker delivers to ClickHouse; reports query actual analytics data.

The main standalone SaaS workflows and advanced reports are implemented for local development. Native Vuuut/Vuuut-pro integration is complete and verified locally; Priority 3 analytics breadth is complete and verified locally, including engagement, retention/cohorts, newsletter editions and campaign costs. Priority 4 commercial safeguards are implemented and verified with 47 backend checks, 11 browser scenarios, real Stripe test-mode lifecycle events and controlled-inbox account/owner email. Manual Checkout, portal interval change and scheduled cancellation passed; public-domain setup is deferred by the operator. Verification is recorded in the handoff and [commercial validation runbook](docs/COMMERCIAL_VALIDATION.md). Priority 5 adds shared admission limits, private monitoring/OTLP, encrypted consistent backup/restore and operational validation tools; see [operations](docs/OPERATIONS.md). Priority 6 provides versioned SDK packages, signature/provenance tooling, complete API response contracts and operator/release guides. **Remote final-revision CI, public publication and external production release gates remain unfinished.** Start with [CONTINUE.md](CONTINUE.md) to resume on any device; the full original requirements are preserved in [docs/ORIGINAL_BRIEF.md](docs/ORIGINAL_BRIEF.md).

## Run and experience the app

Requirements: Rust stable, Python 3.11+ and Homebrew on macOS. Local PostgreSQL
and ClickHouse run natively; Docker is not required for normal local operation.

```sh
./run.sh setup
./run.sh start
./run.sh status
# Later:
./run.sh restart
./run.sh stop
```

Open **http://localhost:8082/** for the homepage, **/signup** to register, or
**/signin** for customer login. Add a website and install its tracking code.
`./run.sh` defaults to detached background startup; `./run.sh log -f` follows
its log. The launcher manages only this project's native databases and retains
all data on stop/restart. Ports remain app 8082, PostgreSQL 55432 and ClickHouse
HTTP 58123, bound to loopback. Rust binaries are installed outside build caches.

For an existing Docker installation, use **`./run.sh migrate-docker` once** to
preserve accounts, credentials and analytics before switching. Source Docker
volumes remain intact as rollback copies. Fresh installations need no Docker.
See [native setup, command reference and data locations](docs/NATIVE_LOCAL.md).

The tracker bundle is committed; Node is needed only to build/change the tracker
or run browser checks. Separate labeled demo data can be created with
`.local/bin/click-server seed-demo <registered-email>` after loading trusted
`.env`. Real websites continue to report actual collected events. Configure
licensed country data through **Owner → Connections → MaxMind**.

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
- [SDK packaging, signatures and release](docs/RELEASE.md)
- [External service connections](docs/CONNECTIONS.md)
- [Customer/platform administration](docs/ADMIN_GUIDE.md)
- [Migrations, upgrade and rollback](docs/MIGRATIONS.md)
- [Tracker and consent](docs/TRACKER.md)
- [Security/privacy and release gates](docs/SECURITY.md)
- [Commercial policies and actual provider evidence](docs/COMMERCIAL_VALIDATION.md)
- [Native local setup and lifecycle commands](docs/NATIVE_LOCAL.md)
- [Deployment and operations](docs/DEPLOYMENT.md)
- [Native Vuuut/Vuuut-pro integration and setup](docs/GHOST_INTEGRATION.md)

GeoLite data attribution: This product includes support for GeoLite data created by MaxMind, available from https://www.maxmind.com. Licensed databases must be supplied by the operator; none are redistributed. Tinybird MIT attribution is in docs/TINYBIRD-LICENSE.

Platform owners can open **Account & security** at `/owner/account` to change their display name/password, sign out other sessions, and enable authenticator MFA with one-use recovery codes. Owner recovery starts at `/owner/forgot-password`; it preserves MFA. Team invitations queue email through the configured transport (private files locally). Report overload and validation errors provide readable browser pages and structured API errors.
