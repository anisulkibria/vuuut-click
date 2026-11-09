# Upgrade and migration procedure

Current schema: PostgreSQL 0001–0014 and ClickHouse 0001–0003. SQLx stores PostgreSQL
checksums; ClickHouse uses the checksummed `control_migrations` ledger under a
PostgreSQL advisory lock. Applied files are immutable. New changes get a new
numbered migration; never edit history to clear a checksum error.

## Upgrade

1. Verify the candidate artifact signature, exact-commit CI and its migration
   hashes in release-manifest.json. Read release notes and confirm schema/binary
   compatibility. Rehearse against an isolated restored copy first.
2. Check readiness, outbox/dead-letter counts, disk/memory and backup health. Take
   a consistent encrypted snapshot with backup.py; verify it with an isolated
   restore drill. Retain the previous binary/image and private configuration.
3. For mixed-version upgrades, stop old HTTP/worker processes before snapshot or
   migration if they do not implement the same maintenance barriers. A cooperative
   advisory lock cannot stop an older writer or an operator's direct SQL.
4. Deploy Click before the corresponding CMS/hosting adapters. Starting `serve`
   or `worker` applies migrations before accepting work. Start one candidate first;
   do not run incompatible old workers during the migration. Native integration
   requires CMS 0038–0040 and hosting 0004–0005 as described in GHOST_INTEGRATION.md.
5. Verify `/readyz`, schema ledgers, independent customer/owner login, a consented
   browser event through accepted/delivered counts, native revision synchronization
   and report correctness. Check private metrics, pending/dead work and error rates
   before expanding replicas. Keep the previous snapshot until recovery is proven.

## Rollback and recovery

If readiness fails, stop the candidate and preserve logs/volumes/ledgers. Resolve
configuration or resource failures before retrying an idempotent migration. Never
remove migration rows, skip a failed migration or run `down -v`.

A binary rollback is allowed only after confirming compatibility with the applied
schema. There are no destructive automatic down migrations. When compatibility
fails, use backup.py restore-drill to create fresh databases, verify both stores,
configuration and pending replay, then perform an operator-controlled cutover
while writers are stopped. `--keep-isolated` preserves the verified restore for
inspection; default drills drop their own temporary targets. Restore secrets/MMDB
from the same recovery point, re-establish scoped service credentials and proxy
trust, and verify billing/mail/native workers before restoring public access.

PostgreSQL 0014 adds admission/report/operation coordination; restore clears stale
coordination. Earlier migrations add durable outbox/dedup, native tombstones,
analytics costs and commercial mail/billing state. These are part of recovery,
not disposable caches. Restoring ClickHouse alone can corrupt dedup/metering.
See OPERATIONS.md for exact backup/restore commands and incident boundaries.
