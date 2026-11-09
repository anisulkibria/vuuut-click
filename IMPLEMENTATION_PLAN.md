# Implementation plan

This is a new application. Production release remains gated on the items below.

## Milestones and acceptance criteria

1. **Inspection and design** — inspected Vuuut, Vuuut-pro and upstream Tinybird. Architecture, schema, security and API decisions documented. Complete.
2. **Real analytics vertical slice — complete for local development** — register account → create website → verify domain → browser tracker → validation and local MMDB enrichment → transactional durable acceptance → retrying worker → ClickHouse → authenticated overview/country dashboard. Verified using real PostgreSQL/ClickHouse and browser tests, including outage/replay correctness, isolation and trusted forwarding. Production geolocation still requires an operator-supplied licensed MMDB.
3. **Basic SaaS foundation — implemented** — registration/login/logout, memberships and RBAC, website credentials, transactional quota metering, event-match goals, scoped APIs, setup, light/dark/system themes. Roles and cross-tenant negative tests pass. Team invitations, organization lifecycle and account/credential/privacy settings are now implemented and tested locally; production hardening remains gated.
4. **Advanced analytics — Priority 3 implemented** — ordered session funnels, journeys, first/last/non-direct net revenue attribution, Web Vitals percentiles, exports, per-tenant deletion/retention. Real-database and browser validation completed. Period comparisons, richer content/acquisition/event reports, optional visible-time/scroll tracking, provided-identity retention/cohorts, visitor/prefix funnels, newsletter editions and currency-separated campaign-cost/ROI reporting are implemented; goal/funnel editing is complete. See docs/ANALYTICS_METRICS.md for coverage semantics.
5. **Ghost integration — Priority 2 complete and verified** — versioned idempotent provisioning, domain/lifecycle/entitlement sync, pinned RS256/JWKS staff exchange, signed transactional CMS outboxes, retries/rotation/disconnect, consent-aware tracker injection and native Admin reports are implemented in all three repositories. Terminal erasure and fresh-mapping restoration are covered. See docs/GHOST_INTEGRATION.md and CONTINUE.md for final verification evidence.
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
- `npm run check` and `npm run build`: passed; tracker IIFE was 3,888 bytes at this historical checkpoint; the current Priority 3 bundle is 5,519 bytes uncompressed.
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

Historical pause checkpoint: native adapters were unfinished here. They have since been implemented under Priority 2; follow the latest `CONTINUE.md` status. Analytics breadth is now implemented under Priority 3. Distributed security budgets, independent MFA/security review, real external-service validation and operational release gates remain. Do not claim production readiness. Git transfers source/docs only; databases, accounts/passwords, `.env`, sessions and demo rows require a fresh setup or separate secure backup transfer.

## Final checkpoint verification — 2026-11-08

Formatting and Clippy passed, all 9 unit and 5 real-stack integration tests passed with --include-ignored, TypeScript check/build passed, and all 5 browser tests passed in one combined run. Shell syntax, OpenAPI JSON and production Compose configuration validation passed. Source candidates were checked for accidental copies of the private local account passwords: none found. Production image/public deployment/external SMTP/real Stripe/load/security/restore gates were not performed. User-requested pause is now in effect; continuation instructions and full requirements are tracked in the repository.

## Goal and funnel editing, and in-transaction privilege rechecks (2026-11-08)

Resumed at the user's request, against priority 1 of the handoff.

- `PATCH /v1/websites/{id}/goals/{goal}` and `PATCH
  /v1/websites/{id}/funnels/{funnel}` edit a goal's name and event, and a
  funnel's name, steps and conversion window. Both were previously
  create-and-delete only, so correcting either meant losing it and rebuilding.
- Events & goals gained a per-goal edit disclosure and a Remove button;
  deletion had existed as an API only. Advanced analytics → Funnels gained an
  edit disclosure pre-filled with the funnel's current steps. HTML forms post
  to `/goals/action` and `/insights/action`, because a browser form cannot send
  PATCH.
- `workspace::lock_editor` now guards every goal and funnel mutation: it holds
  the organization row, refuses a workspace that is not active, and re-reads
  the actor's role through `click_core::Role::can_edit_goals`. This replaces a
  bare `SELECT … FOR UPDATE` plus a hardcoded `role IN (…)` list in the two
  create paths, and adds the check to the two delete paths, which had none.
  It closes a hole where a suspended workspace could still create and delete
  goals and funnels, since `authorize_site` never consulted
  `organizations.status`.
- Verified: fmt and Clippy clean with warnings denied; 9 unit + 5 real-database
  integration tests passed; 5/5 Playwright tests, with the analytics spec
  driving the new forms in a real browser. The race and suspension assertions
  were confirmed to fail when `lock_editor` is removed.
- Not done, and still priority 1: the same recheck review across website,
  credential, team, billing and account mutations; invitation email; owner
  account profile/password/recovery/MFA UI.

## In-transaction privilege rechecks across the remaining mutations (2026-11-08)

Generalized the goal/funnel guard into `workspace::lock_member(tx, org, actor,
allow)`, which holds the organization row, refuses a workspace that is not
active, and re-reads the actor's role through the same `Role` predicate the
pre-transaction check used. `lock_editor` is the goal/funnel wrapper;
`lock_member_even_if_suspended` is the explicit exception.

Closed:

- `retention` reported success while changing nothing — its membership test was
  an `EXISTS` inside the `UPDATE`, so a role lost in the race window changed no
  rows yet returned `Ok(())` and wrote an audit entry claiming the change.
- `issue_key` and `update_website` worked on a suspended workspace, so a
  suspension could be worked around by issuing a credential.
- `queue_export` never re-read the membership, so a removed member could queue
  an export of the whole workspace's data.
- `checkout` never re-checked ownership inside its transaction.

Deliberately exempt from the active-workspace requirement: `checkout` (paying
is how a past-due workspace recovers) and `delete_site`/`delete_workspace`
(erasure must remain possible). Deliberately untouched: the background workers
and the Stripe webhook, which have no actor, and `owner::action`, which is how
suspension and reactivation happen.

Verified: fmt and Clippy clean with warnings denied; 9 unit + 5 real-database
integration tests passed; 5/5 Playwright. The retention no-op reproduces only
as a lock-contended race, so its test holds the organization row while the
request waits; both it and the suspension refusals were confirmed to fail when
the fix is reverted.

Paused at the user's request after this checkpoint.

## Priority 1 complete locally — 2026-11-08

The user resumed and asked to finish Priority 1. Completed invitation email queueing, locked account/password/session/reset/promotion authorization, common HTML/API errors (including extractor rejections), retry hints, four-report-request workspace budgets and bounded 100-step journey aggregation after selecting 25 sessions. Added owner profile/password/session/MFA/recovery forms at `/owner/account` and separate owner recovery under `/owner/account/reset`. MFA challenges have a separate five-minute audience and five persisted attempts; TOTP steps/recovery codes are single use. Recovery preserves MFA and revokes sessions/tokens. Existing accounts remain unchanged. Additive migrations 0007–0009; contracts/security/deployment/handoff updated.

Verification: fmt/Clippy, 17 unit tests + 11 real-database integration tests with no ignored tests skipped, npm ci/check/build, and 6/6 Playwright scenarios. New coverage includes password contention, transactional invite mail, per-workspace budgets, ordered/deduplicated 1,200-event journeys capped at 100 steps, malformed/overload errors, contended website origin/DNS verification and role changes, MFA enrollment/replay/recovery/attempt limits/audience isolation and UI lifecycle alongside an independent customer session. Owner security screenshots inspected; local app remains usable on 8082. See CONTINUE.md for details and actual limits.

Earlier entries about unfinished Priority 1 items and pauses are historical. Priority 2 native integration and Priority 3 analytics breadth are implemented; see the latest verification sections in CONTINUE.md. SMTP delivery against a real provider, distributed budgets, independent security/MFA review, protected database/backups, production deployment/load/restore and commercial release gates remain required. No production-readiness claim.

## Owner panel text sizing — 2026-11-08

Raised the seven owner overview/directory pages from 9px headers and 11px rows to 14px headers and 16px rows, with 16px form controls/body text, 22px panel headings, readable supporting labels and more row spacing. Scoped to `owner-console`; phone text stays readable and tables scroll within their panels. All seven pages passed computed-size and overflow checks at desktop/390px; screenshots inspected; existing owner browser scenario passed. Formatting/Clippy, 17 unit tests and build passed. Database integration tests were not rerun for this presentation change. Updated local server is running on 8082.

Owner typography adjustment at the user’s request: reduced the first increase to 14px body/rows/controls, 13px labels/headers/buttons and 20px panel headings, with slightly tighter row padding.

Further owner typography reduction requested by the user: 13px body/rows/controls, 12px labels/headers/buttons, 19px panel headings and 12px vertical row padding.


### User-requested pause — 2026-11-09

Priority 3 implementation is applied across all three local applications.
Final checks: Click 39 tests (all real-store integrations included), CMS 73
headless tests, real MySQL edition/outbox tests, three native bridge scenarios,
strict Click Clippy and tracker check/build pass. The latest full browser suite
is **8/9**, with a ClickHouse total-memory refusal on the engagement scenario's
last comparison request; preceding measurement assertions pass. Rerun the full
suite with heavy database/build work idle before declaring final verification
complete. The user will commit/push and resume later; no further work is
authorized during this pause. CONTINUE.md records restart details and evidence.
