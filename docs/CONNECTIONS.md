# External connections

Platform owners open **Owner console → Connections** (`/owner/connections`).
Customer accounts and workspace owners cannot change platform connections.
Every mutation requires the independent owner session and exact-origin CSRF
check. Provider passwords, platform credentials and read tokens are never
returned in settings HTML, logs or audit records.

## MaxMind

Enter your numeric MaxMind account ID and GeoLite license key, save, then choose
**Download / update database**. Only the official GeoLite2-Country archive URL
is fetched, with bounded HTTPS redirects, compressed/uncompressed sizes and
request duration. The Country MMDB is validated before atomic replacement.
Failed downloads retain the previous database. The connection worker checks
hourly and downloads when the installed file is at least a day old. Country
lookups reload without restarting; old Unknown event countries are unchanged.

No licensed data is redistributed. GeoLite data is created by MaxMind, available
at https://www.maxmind.com. The operator must maintain licensed download access.
Official download/authentication reference:
https://dev.maxmind.com/geoip/updating-databases/.

Production can start for owner setup without a database: `/readyz` and browser
collection return 503 until a Country reader is available. An existing
`GEOIP_PATH` remains supported; invalid configured files still fail startup.

## SMTP and Stripe

SMTP requires sender, host and TLS port 465/587. Username/password are optional
as a pair. Development requires a controlled delivery inbox and retains its
recipient restriction. **Test connection** tests the TLS/authentication handshake
without sending a message. Saving applies to new deliveries/account requests;
queued messages continue through the same durable mail worker.

Stripe requires matching secret key, webhook signing secret and recurring price
mapping JSON, for example `{"pro:monthly":"price_..."}`. An optional portal
configuration ID is supported. Live keys require production. **Test connection**
validates the remote account/catalog/portal. Register
`https://YOUR-CLICK-HOST/v1/billing/webhooks/stripe` in Stripe for
`checkout.session.completed` and `customer.subscription.created`, `.updated`,
`.deleted`. Normal signature verification and authoritative subscription lookup
remain in force. Settings changes alone do not create a subscription or charge.

Saved SMTP/billing settings apply immediately to the serving process and within
five seconds to other serving/worker processes. Existing in-flight operations
retain their service snapshot. Blank fields keep saved values. **Restore
environment settings** deletes the dashboard override and restores the private
deployment configuration.

## Monitoring and storage

The monitoring form stores an OTLP HTTPS endpoint and optional authorization
headers. Restart serving and worker processes to replace the process-wide
exporter. Private metrics, reverse-proxy trust, PostgreSQL/ClickHouse connections
and backup destinations remain deployment settings; reconnecting databases from
a web form would change the authoritative store underneath active requests.

## Vuuut Pro

Register a platform ID and exact hosting domain suffix once. Generate a private
credential, then **Copy Vuuut Pro configuration before registering**. Paste the
copied `click` block into private `pro.json` and restart hosting. Existing and new
active tenants reconcile automatically; their CMS worker registers its signing
key without anyone opening Admin. Tenant users do not connect manually.
Custom domains outside the delegated suffix require their independent TXT proof.
Registering the same platform again revokes old tenant credentials/read sessions;
restart hosting to reconcile fresh revisions. Never give the platform credential
to a tenant. See [native lifecycle contract](GHOST_INTEGRATION.md).

## Encryption, replicas and recovery

PostgreSQL migration 0015 stores AES-256-GCM authenticated ciphertext, bound to
its service name. Service revisions use a global monotonic sequence so removing
and re-adding a connection cannot leave replicas on stale settings.

Set `CONNECTIONS_KEY` to a shared 32-byte hex key, or retain the automatically
created private `CONNECTIONS_DATA_DIR/key` file (0600). The default directory is
`.local/connections`; the production Compose mount is `data/connections`, mounted
as `/connections`. Prepare the directory for container UID/GID 10001 with mode
0700. Every replica/worker must share the key and the managed GeoIP directory.
Neither belongs in Git. Keep the key after cache cleanup and device migration.
When running backups on the host, set `CONNECTIONS_DATA_DIR` to the host-side
mount path (for example `data/connections`). The encrypted backup bundle now
includes `connections.key` and the managed MMDB
when present. Restore that key/directory alongside the matching PostgreSQL dump;
restore refuses a tampered or incomplete manifest. Loss of the matching key
requires re-entering dashboard credentials.
