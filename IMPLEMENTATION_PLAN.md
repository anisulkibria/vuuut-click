# Implementation plan

This is a new application. Production release remains gated on the items below.

## Milestones and acceptance criteria

1. **Inspection and design** — inspected Vuuut, Vuuut-pro and upstream Tinybird. Architecture, schema, security and API decisions documented. Complete.
2. **Real analytics vertical slice — complete for local development** — register account → create website → verify domain → browser tracker → validation and local MMDB enrichment → transactional durable acceptance → retrying worker → ClickHouse → authenticated overview/country dashboard. Verified using real PostgreSQL/ClickHouse and browser tests, including outage/replay correctness, isolation and trusted forwarding. Production geolocation still requires an operator-supplied licensed MMDB.
3. **Basic SaaS foundation — implemented** — registration/login/logout, memberships and RBAC, website credentials, transactional quota metering, event-match goals, scoped APIs, setup, light/dark/system themes. Roles and cross-tenant negative tests pass. Team invitations, organization lifecycle and account/credential/privacy settings are now implemented and tested locally; production hardening remains gated.
4. **Advanced analytics — standalone checkpoint implemented** — ordered session funnels, journeys, first/last/non-direct net revenue attribution, Web Vitals percentiles, exports, per-tenant deletion/retention. Real-database and browser validation completed. Newsletter event ingestion exists; specialized newsletter dashboards, broader engagement/cohort reports and edit flows remain pending.
5. **Ghost integration** — typed adapter client and inspected integration contract; transactional CMS outbox, provisioning callbacks, JWT/JWKS delegated access, tenant lifecycle, native Admin UI. Pending. Sibling apps remain unchanged until adapters and failure recovery are validated.
6. **Commercial operations — adapters implemented, release gates pending** — Stripe checkout/portal/signed webhooks and entitlement dedup, account verification/reset, team invitation links, private local/SMTP mail and owner management controls implemented and locally tested. Real SMTP/payment-account validation, distributed abuse controls, optional NATS scaling, OTel, monitoring, encrypted backups/restore drills, scanning and 1M/10M/100M benchmarks remain pending.

## Unknowns / external configuration

- Licensed production GeoLite Country MMDB requires the operator's MaxMind account/license. Test-only upstream MMDB may be used in automated integration tests; never ship it as production geolocation.
- Production domains/TLS, payment provider, SMTP and S3 credentials are not supplied. No fabricated payment success or benchmark claims.
- Full product scope spans multiple milestones. See release gaps in docs/SECURITY.md and docs/DEPLOYMENT.md.

## Verification

- `cargo check --workspace`: passed.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace -- --include-ignored`: 7 unit tests + 1 real-stack integration test passed, none ignored in this run. Real integration uses isolated PostgreSQL and ClickHouse databases, repeated migrations, real test MMDB IPv4/IPv6 lookups, outage recovery, replay between stores, zero duplicate billing, tenant isolation, public-token/key misuse, viewer permissions, CSRF and key revocation.
- `npm run check` and `npm run build`: passed; tracker IIFE 3,888 bytes uncompressed.
- Playwright: 4 end-to-end tests passed against the running Rust app and real databases. Verified registration, website creation, real browser page views/custom events, dashboard values, goal completions, dark theme, 390px mobile overflow, logout, required consent, DNT and SPA routing/query redaction. Used the existing Chromium headless executable through CHROMIUM_EXECUTABLE_PATH; default browser download remains environment-dependent. Added a same-browser owner/user test proving simultaneous login, logout in both directions, endpoint role restrictions and the `/owner` cookie path.
- `bash -n run.sh` and both Compose configuration validations: passed.
- `./run.sh` launched the database stack and Rust application; `/signup` and `/readyz` returned HTTP 200. Keep this local server running for the user.
- Screenshots: test-results/dashboard-desktop.png, dashboard-dark.png, dashboard-mobile.png (ignored generated artifacts).
- CI workflow and production Docker/TLS templates are prepared; CI remote execution, Linux image build, public deployment and full release/load/security gates have not run.

## Development access

At the user's request, created a local workspace owner and viewer account, with a localhost website and a real tracker-test link. Secrets are stored only in ignored `.local/development-accounts.json` (0600), never committed or used for production. The existing admin account now has an explicit platform owner grant and signs in only at `/owner/signin`; the viewer uses `/signin`. The owner console reports real platform totals and customer workspaces. Wider platform management actions remain pending.

## Owner authentication follow-up

- Migration 0002 separates server-side session audiences and explicit platform-owner grants. Public signup never grants platform access, even to workspace owners.
- Added `/owner/signin`, `/owner/dashboard`, owner-only current identity and independent login/logout. Regular owner credentials are rejected at customer sign-in.
- Added real database tests for cookie swapping, missing/revoked grants, wrong login endpoint, owner CSRF and independent logouts. All 7 unit tests and the expanded real-stack test pass.
- `cargo clippy --workspace --all-targets -- -D warnings` and formatting passed. All 3 browser tests passed against a separately running server on port 8083 before replacing port 8082.
- Fixed incremental SQLx migration discovery with a directory-watching build script; the added migration is included without a clean build.
- Promoted the existing local admin account with the operator CLI and relaunched the verified binary on port 8082. Customer sessions remain valid; the promoted account's prior customer sessions are invalidated. Passwords are unchanged.

## Owner sidebar follow-up

- The customer and owner dashboards now share the Material 3 sidebar renderer, navigation links, brand, theme controls and profile. Logout paths remain separate.
- Added working owner pages for Overview, Workspaces, Users, Websites, Subscriptions, Event delivery and Audit log. All directories use real PostgreSQL queries, bounded pagination and the owner authorization check. They do not expose credentials or event payloads.
- Owner color-scheme changes return to the owner console. Navigation remains accessible when labels collapse on mobile, and logout stays reachable.
- Verified formatting, Clippy, 7 unit tests and the expanded real-stack integration test. All 3 Playwright tests passed, including each owner sidebar destination, current-page highlighting, themes, mobile overflow and independent customer/owner sessions. Visually inspected desktop and mobile screenshots.
- Relaunched the verified binary on port 8082 and confirmed all seven owner destinations return HTTP 200 using the existing owner account. Existing credentials and sessions remain valid.

## Public homepage follow-up

- Replace the root authentication redirect with a public Rust-served landing page. `/` renders for anonymous visitors and signed-in users/owners without querying account or analytics data.
- Add a rose Material 3/editorial hero, original SVG product illustrations explicitly labeled as illustrations, feature sections, privacy information, setup steps, accessible FAQ disclosures, and user login/signup links. Footer also links to the independent owner sign-in.
- Scope landing styles separately from both dashboards. Preserve light/dark themes, reduced-motion preferences and responsive phone/tablet/desktop layouts.
- Verified formatting and Clippy, all 7 unit tests and the real-stack integration test. All 4 browser tests passed, including root HTTP 200 with no redirects in anonymous, user and owner contexts; account sessions preserved; working sign-in/signup, section anchors, FAQ interaction and no horizontal overflow at 320/390/768 pixels.
- Inspected desktop and phone screenshots and corrected illustration/caption layout. Relaunched the verified binary on port 8082 and smoke-tested the public homepage using the actual development user and owner accounts, including both identities in one cookie jar.

## Main SaaS completion — paused checkpoint

User asked to complete the main SaaS, then requested demo data, then explicitly requested a pause after finishing the current checkpoint because of weekly account usage. `CONTINUE.md` is the canonical portable handoff; `docs/ORIGINAL_BRIEF.md` preserves the full original plan. AGENTS.md instructs a new session to read the handoff on **continue**.

- Separate local demo website includes 30 days of varied traffic, audiences, campaigns, goals, ordered funnel, payments/refunds and all five Web Vitals. It is marked, selected first, cannot collect real events, and consumes no metered usage. Real site data is preserved.
- Workspace/team/invitation/role lifecycle, website/key rotation, profile/password/session management, single-use reset/email verification and settings UI are implemented.
- Durable export/deletion/retention workers, provider-neutral billing/Stripe, SMTP/private-file mail, advanced APIs/UI and typed Rust/TypeScript clients are implemented.
- Owner suspension/reactivation, manual allowance grants and dead-letter retry controls are accessible from owner sidebar pages and recorded in platform audit. Provider-linked subscriptions cannot be overridden by manual grants.
- PostgreSQL migrations 0004–0006 and ClickHouse migration 0002 extend the original schema; CH migrations are checksummed under a PG advisory lock. Existing queue rows deserialize with defaults.
- Customer sidebar exposes Advanced analytics and Workspace & settings. Detail pages have distinct highlight cards and omit the repeated overview traffic chart. Separate owner/customer sessions and public landing are preserved.
- Formatting/Clippy passed. Full real-stack run passed **9 unit tests and 5 integration tests**, including durable ingestion/replay/isolation, team/account lifecycle, exact ordered analytics, provider failures/dedup/delayed notifications, retention of backfilled payloads and private mail permissions. The final combined Playwright run passed **5/5 tests** on 8083, including new demo reports and full settings workflows.
- TypeScript check/build passed; base tracker remains dependency-free (~4 KiB), optional vitals addon ~5.7 KiB, separate server API client ~1.1 KiB. License notice included.
- `run.sh` built and replaced the managed main server successfully on 8082 without removing volumes. The refreshed demo is live. Smoke checks with existing user/owner accounts passed for all 5 basic reports, 4 advanced reports, 7 settings tabs and 7 owner destinations; independent sessions and public homepage remain valid. Temporary 8083 server stopped. Existing passwords unchanged.
- `run.sh` builds and replaces its managed server without removing volumes. Git ignore rules cover secrets/local data; no Git initialization, commit or remote push was performed for the user.

## Remaining work after the pause

Follow `CONTINUE.md` priorities. Native Ghost/Vuuut and Vuuut-pro provisioning/SSO/CMS transactional adapters are not implemented. Further analytics breadth/edit UI, concurrency/security hardening, owner recovery/MFA, real external-service validation and operational release gates remain. Do not claim production readiness. Git transfers source/docs only; databases, accounts/passwords, `.env`, sessions and demo rows require a fresh setup or separate secure backup transfer.

## Final checkpoint verification — 2026-11-08

Formatting and Clippy passed, all 9 unit and 5 real-stack integration tests passed with --include-ignored, TypeScript check/build passed, and all 5 browser tests passed in one combined run. Shell syntax, OpenAPI JSON and production Compose configuration validation passed. Source candidates were checked for accidental copies of the private local account passwords: none found. Production image/public deployment/external SMTP/real Stripe/load/security/restore gates were not performed. User-requested pause is now in effect; continuation instructions and full requirements are tracked in the repository.
