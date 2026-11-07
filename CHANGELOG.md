# Changelog

## 0.1.0 — unreleased

- Establish independent Rust workspace, durable analytics pipeline and Material 3 dashboard.
- Document architecture, inspected Ghost integration points and production release gates.
- Deliver `run.sh` with generated local secrets and automatic database startup/migrations.
- Add Rust-rendered authentication, setup, overview/audience/acquisition/content/events dashboards, theme switching and basic event-match goals.
- Add verified-origin browser collection, consent-aware 3.8 KiB tracker, MMDB enrichment, transactional outbox/usage and replay-safe ClickHouse aggregates.
- Verify real-stack outage/replay/isolation and browser consent/SPA/mobile workflows.
- Separate platform owner `/owner/signin` from customer `/signin`, with independent cookies, database-bound session audiences, owner grants and logout isolation.
- Add owner console with real platform totals and customer workspace listing; owner access requires an operator CLI grant.
- Share the Material 3 sidebar between customer and owner dashboards, including accessible navigation, profile, theme controls and separate logout actions.
- Add owner directories for workspaces, users, websites, subscriptions, event delivery and audit records with bounded pagination and current-page navigation.
- Add a public Rust-served homepage with responsive editorial design, product illustrations, features, setup steps, privacy information, FAQs and user login/signup links.
- Keep `/` accessible without redirects for anonymous visitors and authenticated user/owner sessions.
- Add isolated labeled demo data and distinct highlights for every customer report, including funnels, journeys, revenue and Web Vitals.
- Add multi-workspace/team/invitation/role and credential lifecycle, account verification/reset, privacy settings, exports and deletion workers.
- Add provider-neutral billing with Stripe signed webhooks and explicit local test mode; optional SMTP/private local mail delivery.
- Add ordered funnels, latest-instance Web Vitals, authoritative backend conversion events and multi-model net revenue attribution with typed Rust/TypeScript clients.
- Add owner workspace status, manual allowance and dead-letter controls with auditing.
- Preserve the full original brief and portable continuation instructions for a deliberate development pause.
