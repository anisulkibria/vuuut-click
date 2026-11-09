# Database design

Versioned PostgreSQL migrations own users, opaque hashed sessions, organizations, memberships (owner/admin/analyst/viewer), websites with independent random collection IDs, DNS verification challenges, API credentials, plans, subscriptions, usage, durable queue, audit logs, goals and integration mappings. Website IDs and organization IDs are UUIDs. Foreign keys prevent orphan control records. Membership is the only path from user to website; never authorize using a request organization ID alone.

The outbox primary key `(website_id,event_id)` also provides the deduplication ledger. Deduplication and one monthly accepted-events usage increment happen in the same transaction under an organization row lock. Duplicate submissions return success without billing again, even after delivery. Queue rows retain a compact ledger after delivery; payloads are cleared. Queue deletion and ledger retention must be designed together before release.

ClickHouse events use ReplacingMergeTree ordered by organization, website, timestamp, event ID and month partitioned, with an initial 90-day TTL. Timestamps are server assigned and fixed in PostgreSQL before insertion. Country is ISO code or ZZ; IP never enters either database. A materialized hourly AggregatingMergeTree stores uniqExactState(event_id), uniqExactState(visitor_id), uniqExactState(session_id). These sets merge without inflating retries. Daily/monthly queries roll hourly states forward. Raw dimension queries use FINAL. Approximate uniqueness can be introduced explicitly as uniqCombined64 with labeled error semantics; initial unique counts are exact within the chosen identity lifetime, not unique humans.

Metrics: page views are unique accepted pageview event IDs. Visitors are distinct random identity IDs in pageview events in range; session-only default means this cannot measure returning humans. Sessions are distinct session IDs with a page view in range. Bounce is sessions with one page view divided by sessions, scoped to the selected interval. Engagement duration is not inferred from wall-clock gaps; requires future explicit engagement events. Real-time is visitors with a page view within five minutes. All API time intervals are UTC and half-open [start,end).

Per-tenant TTL changes and deletion workflows are pending. Do not promise plan-specific retention until applied and verified in raw and aggregate storage.

Migration 0002 adds explicit platform_owners grants and a constrained session audience (`user` or `owner`). Customer workspace membership roles never imply platform ownership. Both cookie names and database session types are independent; moving a token between cookies cannot change its audience. Granting platform ownership revokes that account's former customer sessions, and owner authentication checks the current grant on every request. SQLx migration directories are watched by click-store/build.rs so incremental builds embed newly added migrations.


Native migration 0010 adds operator `native_platforms`, version/fingerprint/state
columns on `integration_mappings`, pinned `native_connections`, opaque hashed
`native_sessions` and one-use `native_assertions`. Mapping website references
become nullable with ON DELETE SET NULL so terminal tombstones survive erasure.
Native tenants have a separate organization/subscription/website and no implicit
Click user membership. CMS 0038–0039 and platform 0004–0005 (both SQLite/MySQL) are in the
sibling repositories; they hold private connection/source outbox and durable
lifecycle revisions respectively. No sibling DB schema is imported into Click.

Priority 3 adds PostgreSQL **0011** (funnel participant scope and versioned,
website-bound campaign costs with hashed source references) and ClickHouse
**0003** (provided/session/unknown visitor mode, coarse OS, bounded engagement
and opaque edition/recipient context). Historical/pending old events retain
unknown identity coverage. Migration directories invalidate the compiled
migration bundle through click-store/build.rs. Retention removes cost dates in
UTC, skips empty ClickHouse rewrites and uses a database-scoped advisory sweep
lock; workspace locks still protect each site's acceptance/retention race.
