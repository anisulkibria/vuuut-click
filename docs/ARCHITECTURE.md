# Architecture and inspection

Vuuut-click began empty. Sibling `Vuuut` is an Axum workspace using SQLite/MySQL, native HTML Material 3 Admin and local page_views analytics (`ghost-front/src/analytics.rs`). Its current public tracking derives daily visitor fingerprints; Click uses random consent-aware session identifiers instead. `ghost-api/src/identity.rs` issues RS256 five-minute staff identity tokens with public JWKS, but their audience is the site itself; that token cannot safely be accepted as a Click audience without an explicit integration exchange. Member attribution is in `member_events.rs`; newsletter delivery/open/click, subscriptions and payment processing have separate modules. No sibling database migration is needed.

`Vuuut-pro` separates customer/staff sessions, controls limits in generated hostSettings, provisions CMS owners over the actual setup API, and records site lifecycle in `pro-provision/src/lifecycle.rs`. An analytics integration belongs after successful CMS provisioning and needs retryable mapping state, not a direct write into the CMS database.

## Workspace

- click-core: event contracts, sanitization, trusted proxy rules, GeoIP, identity and roles.
- click-store: PostgreSQL control/outbox and ClickHouse migrations, batch adapter and scoped queries.
- click-server: Axum authentication, collection, website/control API, worker process and dashboard assets.
- click-sdk: typed Rust integration API client, independent of CMS schemas.
- dashboard: Rust-rendered dashboard, Material 3 rose tokens from the sibling applications.
- tracker: dependency-free TypeScript browser collector, separate CDN/IIFE and npm builds.

Modular monolith with `serve`, `worker`, and `migrate` executable modes. PostgreSQL durable outbox provides one queue for hosted and self-hosted deployments; no distributed broker is needed initially. This intentionally uses a database queue instead of a separate local file spool, keeping crash/usage/idempotency semantics identical in both modes. PostgreSQL is already required for control state. NATS JetStream is a later scaling adapter, not an initial dependency.

Control commits precede 202 responses. Workers lock batches with SKIP LOCKED, insert into ClickHouse, then mark delivered in PostgreSQL. A crash between the two stores replays a batch. ReplacingMergeTree plus FINAL for raw dimensions and uniqExact(event_id) aggregate states make all published counts replay-safe. Event timestamps are stamped at first acceptance, so replay cannot move a record across partitions. Accepted payloads are immutable.

## Frontend decision and upstream review

Tinybird starter at commit `46f434feab6c98ab5d14d6da8d237b3a89193d2c` is MIT; evaluated its Next 14 / React 18 dashboard, time-range hooks, widgets, query definitions, country table and materializations. Adapt its overview/time-series/dimension-table UX to Rust-rendered HTML and SVG, per the user’s explicit Rust instruction. Dropping its Next server, Tinybird SDK, AI and token/session middleware avoids a second auth backend; Axum renders the dashboard and owns all authorization. This is a documented framework deviation to fit the user’s existing Rust applications and chosen Material 3 design. No proprietary DataFast assets copied. Full Tinybird MIT notice retained in TINYBIRD-LICENSE.

## Sources

- https://github.com/tinybirdco/web-analytics-starter-kit (MIT; local inspected checkout)
- https://github.com/ClickHouse/ClickHouse (Apache-2.0)
- https://clickhouse.com/docs/integrations/rust (official client; query bindings)
- https://clickhouse.com/docs/engines/table-engines/mergetree-family/replacingmergetree (replacement is eventual; read correctness needs FINAL or unique aggregation)
- https://github.com/TryGhost/Ghost (MIT) and https://ghost.org
- https://datafa.st (feature reference only)
- https://dev.maxmind.com/geoip/geolite2-free-geolocation-data/ and https://dev.maxmind.com/geoip/updating-databases/

## Scaling

Start with one PostgreSQL and one ClickHouse, bounded API bodies, batches and query limits. Split worker/serve, then partition queues and scale ingestion, then replicate ClickHouse. Keep authoritative membership/control queries in PostgreSQL. No unsupported capacity claims; benchmarks remain a release gate.

The public homepage is Rust-served from `landing.rs` and `dashboard/landing/`. It performs no authentication or database lookup and remains available at `/` for every session state. Its SVG artwork is labeled as a product illustration; private customer analytics are never used in marketing content. Public assets are separate from the dashboard stylesheet additions.
