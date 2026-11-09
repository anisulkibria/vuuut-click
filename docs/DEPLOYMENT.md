# Deployment and operations

## Development / standalone

`./run.sh` is the verified local entrypoint. It generates `.env` with private file mode and distinct random database secrets, starts named Compose volumes and builds the Rust server. There is no dependency on hosted SaaS or Tinybird Cloud. Stop the server with Ctrl+C, databases with `./run.sh --stop`. Re-run after editing Rust. Retain `.env`: changing passwords in it does not rotate credentials inside existing PostgreSQL/ClickHouse volumes.

The tracker bundle is committed, so Rust can compile without Node. Rebuild the browser SDK only after editing its TypeScript source.

The local test-page link is enabled only when ALLOW_LOCALHOST=true and the registered website origin equals PUBLIC_URL. It collects actual visits, is labeled as a development test page, and is disabled in production.

## Production preparation

Dockerfile builds a Rust binary and runs it unprivileged. `deploy/compose.production.yml` contains PostgreSQL, ClickHouse, app and Caddy with persistent volumes, health checks and memory limits. These configuration templates are not a production-readiness certification. A Linux container image build, TLS/domain deployment, backup restore drill and the release gates in SECURITY.md must pass before public launch.

1. Create a private production env file with DOMAIN, random hex POSTGRES_PASSWORD and CLICKHOUSE_PASSWORD. Do not use the development file or default credentials. Use an audited secrets manager for real operations.
2. Point the domain to your Linux host. Install Docker Compose, restrict firewall exposure to SSH/80/443. Database ports are not published by production Compose.
3. Obtain GeoLite2-Country with your own MaxMind account. Place it at `data/geoip/GeoLite2-Country.mmdb`; review commercial/redistribution terms. The application fails closed at startup if missing in production.
4. Set the actual reverse-proxy CIDR in TRUSTED_PROXIES and restrict direct traffic to the edge. Caddy appends the client address to forwarding headers. The application only trusts headers from the configured immediate proxy network and walks the chain right-to-left. Set a fixed private Docker subnet if stable allowlists are required.
5. Review image vulnerability scans and version pins, then `docker compose --env-file /secure/production.env -f deploy/compose.production.yml up -d --build`.
6. Confirm readiness inside the private app container, create an account, register and verify a domain and test a browser event against ClickHouse. Public HTTPS is required for secure cookies.

Use the same deployment/API for standalone Ghost. Native Vuuut/Vuuut-pro provisioning, plan/domain lifecycle and server-side staff exchange are implemented; see GHOST_INTEGRATION.md for opt-in configuration.

## GeoIP updates

Use MaxMind's maintained `geoipupdate` program on the operator's host with their own account and license in a private GeoIP.conf; EditionIDs GeoLite2-Country, DatabaseDirectory `data/geoip`. Schedule daily through the host timer/cron and monitor failures/database age. After atomic update, restart the app to reload the reader. No licensed database belongs in Git or the container image. Local files are never sent to a paid geolocation API.

## Delivery / incidents

PostgreSQL is the durability boundary. ClickHouse outages retain pending payloads with bounded retries; after 20 failures rows become dead letters. Inspect counts/oldest available_at/attempts with a privileged database connection; recover the cause, then run `click-server requeue-dead`. Never delete outbox ledger rows to retry: doing so can inflate usage or permit replay under new acceptance time. A worker drains at most 500 events per transaction; a process restart replays safely. `serve` embeds a worker; `worker` runs independently when extra drain capacity is needed.

Structured tracing, private OTLP export and bounded ClickHouse queries are implemented. Prometheus/Grafana and alert delivery are validated locally; configure production receivers and deployment evidence through OPERATIONS.md.

## Backups, upgrades and rollback

A complete recovery point includes PostgreSQL (control, credentials, dedup ledger and pending outbox), ClickHouse raw plus aggregate tables, secrets and the configured MMDB source. Snapshot consistency across both stores matters: restore the PostgreSQL queue/dedup ledger together with corresponding ClickHouse data. Replaying already-inserted events is safe; losing the ledger and accepting old IDs at new timestamps is not safe.

Encrypted S3-compatible backup and isolated restore tools are implemented in scripts/backup.py. Local validation does not prove off-machine storage or a production recovery schedule; complete those deployment gates before commercial operation. See OPERATIONS.md. For local development, named Docker volumes survive stops; `down -v` destroys them and is intentionally never used by run.sh.

Migrations apply before the listener starts. PostgreSQL SQLx migration history is versioned/checksummed. ClickHouse migrations 0001–0003 are idempotent, serialized with a PostgreSQL advisory lock and recorded with checksums in control_migrations. Future schema changes require new migration versions and backfill tests. Take verified backups before every migration; pin a container tag for rollback. Roll back binaries only across compatible schemas; destructive schema rollback requires the tested restore procedure. Do not drop tables or mutate sibling Ghost schemas.

## Transactional mail and billing configuration

Production registration requires configured SMTP; domain creation requires verified email. Set SMTP_HOST, SMTP_PORT (587 STARTTLS or 465 TLS), SMTP_FROM, and optionally both SMTP_USERNAME/SMTP_PASSWORD. Without SMTP, local development writes 0600 messages under ignored `.local/mail` (0700); it never pretends to send external email. Account-token queue bodies are cleared after delivery. Monitor pending/dead mail and manage local mailbox retention; automatic local-mail deletion is not implemented.

Stripe configuration: STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET, STRIPE_PRICES_JSON such as `{"pro:monthly":"price_...","pro:annual":"price_..."}`. Configure the provider webhook for checkout.session.completed and customer.subscription.created/updated/deleted at `/v1/billing/webhooks/stripe`. Keep test/live modes consistent. Customer owners use Billing & usage in settings; platform manual grants cannot replace a provider-linked subscription. Verify real checkout, portal, cancellation, failed payment and webhook redelivery in the operator's Stripe test account before launch. Real Stripe test-mode and controlled-inbox SMTP checks are recorded in COMMERCIAL_VALIDATION.md. Test validation does not configure live billing or production sender/domain settings.

The privacy worker applies the smaller of website/plan retention to real websites in hourly buckets, discards expired pending payloads before replay, deletes expired exports and sessions, and processes durable data deletion. Demonstration websites are excluded from retention policy changes and live collection. Retention relies on a running worker; monitor failures.

## Owner account recovery

Grant access only through `click-server grant-owner <existing-account-email>`, then sign in at `/owner/signin`. Open **Account & security** to enroll an authenticator and store the one-use recovery codes privately. Maintain a reliable, synchronized system clock. Configure and verify SMTP before depending on `/owner/forgot-password`; local development without SMTP writes the recovery link to `.local/mail` instead of sending email. Email recovery preserves MFA. Losing both the authenticator and all recovery codes requires an operator-assisted identity/recovery process, not a customer reset link. The database contains authenticator shared secrets; secure its access, storage and backups.

Team invitations use the same transactional mail outbox and transport. The UI says when a message is queued or when a link must be shared manually. Monitor delivery failures before assuming that an invite reached an inbox.


Native integration is opt-in. Deploy the matching CMS (0038–0040) and hosting (0004–0005)
builds alongside Click's additive PostgreSQL migrations 0010–0011. Register the
platform via the operator CLI and keep its credential only in private platform
configuration; see GHOST_INTEGRATION.md. Restart hosting after platform key or
plan-map changes to reconcile new revisions. Back up mapping/tombstone state
with the control DB. A credential rotation/disconnect must be acknowledged;
retry uncertain operations using the persisted replacement/version. Public TLS/DNS and independent SSO review remain external validation gates;
real test-mode payment and SMTP evidence is recorded in COMMERCIAL_VALIDATION.md.


Priority 3 upgrades apply PostgreSQL 0011 and ClickHouse 0003 before the
listener starts; deploy Click before CMS 0040 and the updated hosting binary.
Existing rows remain legacy/unclassified for identity and have no invented
engagement or newsletter history. Campaign costs live in PostgreSQL and must
be included in backups; retention/deletion applies to them too. Refresh only
explicitly labeled demo sites with `seed-demo` to see sample reports.

Retention sweeps share a PostgreSQL advisory lock across processes and check
for expired rows before scheduling ClickHouse mutations. Empty sweeps no
longer rewrite shared aggregate parts. This avoids redundant local work;
shared PostgreSQL admission/report budgets now coordinate replicas under Priority 5.

Priority 4 commercial setup and the test-only isolated provider workflow are in
[COMMERCIAL_VALIDATION.md](COMMERCIAL_VALIDATION.md). Deploy additive PostgreSQL
0012–0013 with the corresponding binary. Real Stripe price/portal checks, SMTP
acceptance plus inbox authentication, and public TXT/HTTPS evidence must be
recorded for your deployment; local fixtures never substitute for them. Test
credentials are optional for normal local operation. Live Stripe keys are
refused outside APP_ENV=production, and partial Stripe configuration fails
startup rather than silently selecting local test billing. Keep fixture runs
on explicit local mail/no-provider services; the external helper alone uses
the configured real services and operator inbox in isolated stores.


The operator explicitly deferred deployment-domain setup on 2026-11-10; no
connected Hostinger domain is used for Vuuut Click. Keep public TXT/TLS and
sender SPF/DKIM/DMARC/bounce/abuse checks open until that domain is configured.
For development SMTP, `ALLOW_LOCALHOST=true` requires `COMMERCIAL_TEST_EMAIL`
and restricts delivery to that exact controlled inbox. Fixture browser servers
must omit STRIPE_/SMTP_/COMMERCIAL_ settings; Rust integration fixtures explicitly
inject local services. A real test-mode main server must have a matching active
Stripe listener forwarding to its own webhook endpoint, rather than an expired
isolated QA port.


Priority 5 operational configuration, monitoring, shared budgets, recovery tools
and local validation commands are documented in [OPERATIONS.md](OPERATIONS.md).
Merge compose.observability.yml with production Compose; configure the actual
alert receiver and private scrape mounts. Systemd timer templates are under
deploy/systemd. Build on a separate build worker; the local4GB Docker VM is not
a production capacity qualification. Public-domain work remains deferred.


SDK packaging, signatures and remote publication are described in [RELEASE.md](RELEASE.md).
Follow [MIGRATIONS.md](MIGRATIONS.md) for upgrades/rollback and [ADMIN_GUIDE.md](ADMIN_GUIDE.md)
for customer/owner administration. Current PostgreSQL schema is 0014.
