# Operations

Priority 5 is validated locally first, as requested. Public domains, off-machine
backup storage, production capacity qualification and independent review remain
separate release gates. No live payments or external alert messages are required.

## Local setup

Use Python 3.11+, Docker, Rust and the private development environment:

```sh
python3 -m venv .local/ops-venv
.local/ops-venv/bin/pip install -r scripts/requirements-ops.txt
python3 scripts/install-ops-tools.py
python3 scripts/setup-operations.py
set -a
source .env
set +a
python3 scripts/start-observability.py
```

Monitoring is loopback-only: Prometheus9090, Alertmanager9093, Grafana3001,
local alert inbox9099/alerts, OTLP4318. Grafana's generated password stays in
`.local/ops/secrets/grafana_password`. Non-root container mount files are0444
inside a0700 host directory. Never expose these services publicly. Production
must configure an actual authenticated alert receiver; the local inbox proves
transport only. `start-observability.py --stop` preserves monitoring volumes.

Request metrics use fixed groups/method/status classes, without URLs or customer
labels. Traces contain only service/version, route group, method and status.
The collector rotates sanitized traces after20MB, retaining at most3 backups/1day.
The public edge blocks `/internal/*`; the metrics endpoint also requires its own
bearer token. Health remains available when shared budgets cannot reach PostgreSQL.

All replicas share a32byte ABUSE_HASH_KEY. IP budget subjects are minute-scoped
HMACs and expire after3minutes. Cluster admission bounds their cardinality.
Each workspace has4 report permits, the cluster16; crash leases expire after45s.
Requests have a30s deadline and64 in-flight cap per process. Shared-budget failure
refuses admission. Collection remains quota/idempotency guarded in PostgreSQL.
Native source queues and browser retries must retain event IDs on429/503.

## Backups and recovery

Configure RESTIC_REPOSITORY, an independently escrowed RESTIC_PASSWORD_FILE,
DATABASE_URL, ClickHouse settings and BACKUP_SECRETS_FILE. Use S3/SFTP credentials
restricted to a dedicated encrypted repository. Local repository testing requires
`--allow-local`; it does not prove off-machine durability. Never store the only
password beside the repository or rely on Git/device transfer to preserve it.

```sh
.local/ops-venv/bin/python scripts/backup.py init
.local/ops-venv/bin/python scripts/backup.py snapshot
.local/ops-venv/bin/python scripts/backup.py restore-drill latest
.local/ops-venv/bin/python scripts/backup.py check
.local/ops-venv/bin/python scripts/backup.py retention
```

Snapshots coordinate all application writers/workers through PostgreSQL advisory
lock78422990, then wait for ClickHouse writes/mutations. Private PG dump, raw and
aggregate Native tables, configuration and configured MMDB form one hashed bundle.
Writers resume before encryption/upload. All replicas must run this barrier;
operators must not perform direct SQL/schema/key changes during the snapshot.
Use a matching-major PostgreSQL client or the owned BACKUP_PG_CONTAINER.

Restore always creates fresh `click_restore_<uuid>` stores, never replaces an
existing database. It verifies both stores, logical counts, usage and private
configuration integrity. The materialized view is detached while both tables load.
Volatile budgets/leases are cleared. Default drills remove their stores afterward.
For disaster recovery, `--keep-isolated` retains only the verified new stores and
a private manifest; redirect a stopped application to them after operator review,
restore private configuration/licensed MMDB, rotate credentials and validate
idempotent pending-outbox replay before reopening traffic. Never restore mail or
payment workers against an unreviewed clone. Existing event IDs must not be billed
again. Do not roll binaries back across incompatible schema versions.

Run backups daily and a restore drill weekly; alerts enforce24h/7day freshness.
RPO depends on actual schedule/off-machine success, not a configured timer. Measure
RTO on the deployment data volume. Retention defaults to a dry run preserving
7daily/4weekly/12monthly snapshots; `retention --apply` executes only the tagged
repository policy. Keep credential/key recovery outside that repository.

## Validation and incident response

```sh
.local/ops-venv/bin/python scripts/validate-operations.py
.local/ops-venv/bin/python scripts/validate-linux.py --image vuuut-click:priority5-local
python3 scripts/security-scan.py --image vuuut-click:priority5-local
```

The operational drill creates disposable stores and validates torn-write recovery,
replay, wrong-key/corrupt-pack rejection, OTLP privacy,1M resident synthetic rows
and5x arrival profiles equivalent to1M/10M/100M monthly. These short synthetic
profiles do not qualify100M resident cardinality, long-running production load or
independent security review. Linux validation uses a test MMDB and private CA;
it does not validate public certificates or production geolocation licensing.

On dependency failure, preserve volumes/outbox/dedup state; investigate health,
resource exhaustion and queues before restarting only the affected service.
Dead letters require cause resolution and audited retry. Stale retention/deletion
is an incident even when readiness passes. Never bypass ownership or billing to
clear a queue. Revoke compromised service credentials, update all replicas and
monitoring mounts together, and verify recovery with the isolated tools.

Before release, obtain independent review of authorization, native assertions,
webhook mapping, secrets, proxy trust, abuse budgets and backup/restore boundaries.
Automated tests and scans are evidence for that review, not its replacement.

## Local verification — 2026-11-10

- 51 backend checks passed, including all27 real-store scenarios; strict Clippy
  and formatting passed. All 11 browser scenarios and six Python safety checks passed.
- Actual encrypted recovery verified both stores/configuration, pending replay
  and unchanged usage. The main-data restore completed in13.28seconds.
- 1M synthetic resident rows and5x monthly-arrival profiles for1M/10M/100M passed;
  request p95 values were0.055/0.066/0.305seconds. This is local evidence only.
- Linux release binary built, hardened non-root/read-only runtime exercised
  through trusted private-CA TLS; cookies, CSRF, proxy spoof rejection and hidden
  operational endpoints passed. The final source/license and runtime scan had
  zero high/critical vulnerabilities or secret findings, with no advisory ignores.
- Real Prometheus scrape, rule-to-Alertmanager-to-local-inbox delivery and the
  provisioned eight-panel Grafana dashboard passed.

The4GB local Docker VM encountered resource refusals during overlapping builds
and bulk restore. Recovery now exports bounded Native chunks; read buffers are
bounded through the checked-in ClickHouse profile. Counts and report definitions
are unchanged. Keep build jobs separate from deployment services. Public DNS/TLS,
licensed production GeoIP, off-machine repository/escrow, long-running full-volume
capacity and independent security sign-off remain unpassed deployment gates.
