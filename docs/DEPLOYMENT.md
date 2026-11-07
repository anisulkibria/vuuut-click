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

Use the same deployment/API for standalone Ghost. Hosted Ghost plan lifecycle and SSO wiring are pending; see GHOST_INTEGRATION.md.

## GeoIP updates

Use MaxMind's maintained `geoipupdate` program on the operator's host with their own account and license in a private GeoIP.conf; EditionIDs GeoLite2-Country, DatabaseDirectory `data/geoip`. Schedule daily through the host timer/cron and monitor failures/database age. After atomic update, restart the app to reload the reader. No licensed database belongs in Git or the container image. Local files are never sent to a paid geolocation API.

## Delivery / incidents

PostgreSQL is the durability boundary. ClickHouse outages retain pending payloads with bounded retries; after 20 failures rows become dead letters. Inspect counts/oldest available_at/attempts with a privileged database connection; recover the cause, then run `click-server requeue-dead`. Never delete outbox ledger rows to retry: doing so can inflate usage or permit replay under new acceptance time. A worker drains at most 500 events per transaction; a process restart replays safely. `serve` embeds a worker; `worker` runs independently when extra drain capacity is needed.

Logs use structured tracing and query limits are enforced in the ClickHouse client. Prometheus/Grafana dashboards and OpenTelemetry export are remaining operations milestones; no unsupported monitoring guarantees are claimed.

## Backups, upgrades and rollback

A complete recovery point includes PostgreSQL (control, credentials, dedup ledger and pending outbox), ClickHouse raw plus aggregate tables, secrets and the configured MMDB source. Snapshot consistency across both stores matters: restore the PostgreSQL queue/dedup ledger together with corresponding ClickHouse data. Replaying already-inserted events is safe; losing the ledger and accepting old IDs at new timestamps is not safe.

Encrypted off-site S3 backup scripts and automated restore verification are not yet implemented; do not operate commercially before this gate is complete. For local development, named Docker volumes survive stops; `down -v` destroys them and is intentionally never used by run.sh.

Migrations apply before the listener starts. PostgreSQL SQLx migration history is versioned/checksummed. ClickHouse migrations 0001/0002 are idempotent, serialized with a PostgreSQL advisory lock and recorded with checksums in control_migrations. Future schema changes require new migration versions and backfill tests. Take verified backups before every migration; pin a container tag for rollback. Roll back binaries only across compatible schemas; destructive schema rollback requires the tested restore procedure. Do not drop tables or mutate sibling Ghost schemas.

## Transactional mail and billing configuration

Production registration requires configured SMTP; domain creation requires verified email. Set SMTP_HOST, SMTP_PORT (587 STARTTLS or 465 TLS), SMTP_FROM, and optionally both SMTP_USERNAME/SMTP_PASSWORD. Local development instead writes 0600 messages under ignored `.local/mail` (0700); it never pretends to send external email. Account-token queue bodies are cleared after delivery. Monitor pending/dead mail and manage local mailbox retention; automatic local-mail deletion is not implemented.

Stripe configuration: STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET, STRIPE_PRICES_JSON such as `{"pro:monthly":"price_...","pro:annual":"price_..."}`. Configure the provider webhook for checkout.session.completed and customer.subscription.created/updated/deleted at `/v1/billing/webhooks/stripe`. Keep test/live modes consistent. Customer owners use Billing & usage in settings; platform manual grants cannot replace a provider-linked subscription. Verify real checkout, portal, cancellation, failed payment and webhook redelivery in the operator's Stripe test account before launch. No provider credentials were supplied or real payments exercised here.

The privacy worker applies the smaller of website/plan retention to real websites in hourly buckets, discards expired pending payloads before replay, deletes expired exports and sessions, and processes durable data deletion. Demonstration websites are excluded from retention policy changes and live collection. Retention relies on a running worker; monitor failures.
