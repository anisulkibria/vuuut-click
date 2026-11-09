# Security and privacy

Vuuut Click accepts events durably in PostgreSQL and queries replay-safe analytics
in ClickHouse. Customer data, platform administration, native control, collection,
operator metrics and payment webhooks use distinct authorization boundaries.
Production readiness requires the external acceptance gates below.

## Identity and authorization

Passwords use Argon2 hashes. Customer sessions and platform-owner sessions have
independent audiences/cookies; only an explicit operator grant creates an owner.
Cookie mutations enforce exact Origin. Ownership and membership are rechecked
inside locked mutation transactions. Suspended workspaces cannot bypass collection
or management restrictions; billing recovery and erasure have explicit exceptions.
Credentials are hashed in storage, scoped, rotatable and revocable. Public
collection tokens cannot authorize reads.

Owners can enroll TOTP and use one-use recovery codes. Challenges expire in five
minutes, allow five persisted attempts and cannot authorize account data. Replayed
TOTP steps/codes are rejected. Password recovery preserves MFA and revokes sessions;
losing every factor needs an operator identity/recovery process. PostgreSQL contains
authenticator secrets: protect database access and encrypted recovery copies.

Native provisioning uses operator-issued platform credentials, monotonic revisions
and tombstones. CMS JWT exchange pins RS256 keys/issuer/audience and checks nonce,
expiry and replay; it never fetches a URL selected by JWT input. Exchanged tokens
read one website for 60 seconds and immediately honor revocation/suspension.
See [native integration](GHOST_INTEGRATION.md).

## Ingestion, money and tenant isolation

Every accepted event commits payload, dedup identity and usage atomically. Retries
preserve IDs; reports remain correct before background merges. Backend integration
credentials alone can create money facts. Provider webhooks verify raw-body
signatures, timestamp/mode and authoritative subscription state before applying
idempotent entitlements. Checkout returns never grant a plan.

Reports authorize website membership and enforce tenant/global leases, deadlines,
result limits and ClickHouse memory/time bounds. PostgreSQL coordinates request
budgets across replicas. Network-budget keys are minute-scoped HMAC values with a
short lifetime. Public endpoints fail closed when the coordination store fails;
liveness remains available. SQL parameters carry untrusted values.

## Data minimization and recovery

The tracker honors required consent, DNT/GPC and withdrawal. Persistent random
browser identity and engagement are optional. No fingerprinting, form values, raw
IPs, arbitrary queries or full referrer URLs enter analytics storage. Country is
enriched locally from an operator-licensed MMDB. Structured telemetry uses fixed
route groups/method/status, without customer IDs, IPs, request bodies or URLs.

Deletion disables collection first, then removes raw/aggregate data and related
control records. Retention discards expired queued payloads before replay; exports
are bounded, escaped against spreadsheet formulas and expire. Workers must run and
be monitored. SMTP bodies containing one-use links are cleared after delivery;
private development files require operator retention management.

Encrypted restic snapshots coordinate application writers and include both stores,
configuration and the configured MMDB. Restores use fresh isolated namespaces,
verify integrity and replay pending delivery without remetering. Never restore
one store independently or delete dedup state to clear delivery failures.
See [recovery and incidents](OPERATIONS.md).

## Deployment and supply chain

The runtime is unprivileged with a read-only filesystem and dropped capabilities.
The edge terminates TLS, rejects private metrics paths and restricts forwarding
trust to configured proxy networks. Production requires HTTPS, licensed GeoIP,
shared admission keys and a separate metrics token. Secrets belong in private
configuration/secrets management, never source, release bundles or browser code.

Dependency/license/source-secret/runtime scans, real-store adversarial tests,
Linux/private-CA checks and encrypted recovery ran locally under Priority 5.
Release artifacts have checksum manifests, pinned-key local signatures and prepared
GitHub provenance attestations. Tampered bytes, changed manifests and substituted
keys are rejected. Signatures authenticate artifacts; they do not certify runtime
security. Trust signing identities independently of downloads.

## Deployment acceptance still required

- Approved public domain, real DNS ownership/TLS, licensed GeoIP and sender
  SPF/DKIM/DMARC/bounce/abuse configuration.
- Off-machine encrypted backup, separately escrowed password and scheduled restore.
- Sustained full-volume capacity and recovery qualification on the intended host;
  short local arrival profiles do not prove 100M resident rows.
- Independent review of auth/MFA, native exchange, provider/tenant isolation,
  proxy/abuse controls and backup boundaries.
- Successful remote CI for the exact final commit, verified public SDK publication,
  release-signing identity/registry ownership and production acceptance evidence.

No public domain was selected and no live billing is authorized by local testing.
For incidents, restrict access, preserve durable ledgers/volumes, revoke affected
credentials and verify isolated recovery before resuming service. Do not publish
private account data, tokens or exploit details in public logs/issues.
