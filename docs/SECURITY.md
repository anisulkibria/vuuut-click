# Security and privacy

Platform owners use `/owner/signin`; customers use `/signin`. Platform ownership is an explicit `platform_owners` grant from the operator CLI, never a public signup option or a workspace owner role. Owner credentials are rejected by the normal login endpoints. `click_owner_session` is scoped to `/owner`, lasts 8 hours, and resolves only sessions with audience `owner` and a current owner grant. Customer `click_session` is scoped to `/`, lasts 7 days, and resolves only audience `user` for non-platform-owner accounts. Neither portal falls back to the other cookie. Login/logout affects only its own portal; owner logout also revokes its pending MFA challenge. Cookie-name swapping cannot bypass the database audience checks.

Passwords use Argon2id, random opaque sessions/API keys are SHA-256 hashed at rest. HttpOnly, SameSite=Strict cookies use Secure for HTTPS; production startup requires HTTPS. Cookie mutations require an exact configured Origin. Bearer API keys are website scoped and cannot create credentials or organizations. Public collection tokens carry no read privilege.

All analytics routes resolve the website via authenticated membership or a website-scoped read key. Permission failures intentionally return 404. Browser collection requires a verified domain matching both Origin and event URL. This prevents casual cross-site contamination but a public collector cannot prove a human browser or prevent determined forged events. Public tokens are not secrets.

DNS verification rechecks management permissions and workspace status after the lookup, and applies only to the exact origin/challenge looked up. Origin updates compare the current locked row before retaining verification.

Only explicitly configured reverse-proxy CIDRs can contribute X-Forwarded-For. Walk from the immediate peer right to left until the first untrusted hop. Cloudflare country/IP headers are not trusted independently; deployments must configure current Cloudflare CIDRs in the edge and application allowlist. Bad/unknown lookup returns ZZ. No IP persistence, IP fingerprint or arbitrary user-agent storage. URL queries/fragments and referrer paths are removed; only explicit campaign fields survive validation. Site owners must avoid personal identifiers in paths/event names/campaign labels.

Current rate limits are per-process bounded buckets and do not replace edge/distributed abuse controls. Quotas are transactionally enforced. DB/query errors are logged with no payloads/credentials and return generic errors. TLS terminates at Caddy. PostgreSQL/ClickHouse are private to the Docker network in production.

## Release gates

Not yet production ready: MFA review, distributed/per-tenant request budgets, expanded abuse/concurrency testing, audited SSO, live SMTP/payment validation, OTel, dependency and image scanning, encrypted backup/restore drills and independent security review remain required. Email verification/reset, invitation/role/key lifecycle, durable privacy jobs and signature-verified billing workflows are implemented and locally tested. Reports have four concurrent requests per workspace and sixteen per process, plus ClickHouse time/memory/result limits. These request budgets are per process; distributed enforcement remains a release gate. Enforce deployment resource and data-volume budgets before public access.

Hosted operator is responsible for processor agreements, policies, retention/deletion fulfillment and incident response; customers control collection purposes and consent. Self-hosted operators assume both deployment and processing duties. No universal GDPR/ePrivacy compliance claim. Honor browser DNT/GPC and require explicit SDK consent unless an operator deliberately chooses anonymous mode after applicable review.

GeoLite databases are licensed, not bundled assets. Download with the operator's own MaxMind license; review commercial hosted use and any redistribution separately against current terms. Attribute MaxMind and use geoipupdate for updates. Test fixtures from MaxMind-DB are test data, not production GeoLite data.

## Owner account security and recovery

The owner sidebar links to `/owner/account` for display name, password, other-session revocation and authenticator setup. MFA is opt-in; existing owners are not silently enrolled. Setup requires the current password, then a code from the app within 15 minutes. Enabling MFA revokes all other sessions. Authenticator setup uses RFC 6238, SHA1, six digits and 30-second steps with one step of clock skew; the last accepted step cannot be replayed. Keep server clocks synchronized. Ten random one-use recovery codes are displayed only on enrollment or replacement; PostgreSQL stores their hashes. Replacing codes or disabling MFA requires the password and a fresh factor, and revokes other sessions. Pending enrollment secrets are never displayed by the account page.

Once enrolled, password sign-in creates only a five-minute `owner_pending` session with a separate HttpOnly cookie scoped to `/owner`. It cannot read customer or owner data. Completing the challenge consumes the TOTP step or recovery code and the pending session atomically; five failures exhaust that challenge. A full owner session is issued only after successful verification. The current-password check, session/grant recheck, factor consumption and account updates serialize on the user row. Customer password changes invalidate outstanding customer reset links and all older customer sessions. Login, reset and operator promotion take the same account lock.

Owner email recovery starts at `/owner/forgot-password`. The token is separate from customer reset tokens, expires after 30 minutes, permits five factor attempts, and can be used once. Email recovery still requires a fresh authenticator or unused recovery code if MFA is enabled; it never disables MFA. Successful recovery invalidates all account tokens and sessions. If an owner has lost the authenticator and all recovery codes, the email link alone cannot recover access: an authorized database operator must follow a verified identity/recovery procedure. There is no public MFA bypass. External SMTP delivery and independent MFA/security review remain release gates.

Authenticator shared secrets must remain retrievable to verify codes. They are currently stored in the private PostgreSQL database, so encrypted disks/backups and strict database access matter; application-level encryption and managed key rotation are not claimed. Do not capture enrollment/recovery pages in production logs or support screenshots. Generated development screenshots and mail remain ignored artifacts.

## Errors, query budgets and invitation mail

Malformed JSON/form/path/query inputs use the common JSON error envelope. HTML navigations instead receive themed, escaped error pages with a dashboard/sign-in link; API/fetch clients retain JSON. Global and workspace report overload return 503 with a two-second retry hint; the per-IP allowance returns 429 with 60 seconds. Security and no-store headers also cover refusals. Journey queries select the newest 25 sessions first and keep only the earliest 100 steps in each aggregation state, while preserving full event counts and replay deduplication. ClickHouse retains its 10-second, 256 MiB and 10,000-result-row bounds.

Invitations and their email queue records commit in the same transaction. API `emailed` means queued, not confirmed delivery. Email includes workspace, role, expiry and the fragment acceptance link; the returned link remains available for manual sharing. The recipient must sign in with the invited email. SMTP uses the configured account mail transport; development writes private files. Delivery retries may duplicate a message if the SMTP service accepts it before a local commit fails; invitation acceptance is still single-use.

Implementation references: [RFC 6238](https://www.rfc-editor.org/rfc/rfc6238), [ClickHouse groupArraySorted](https://clickhouse.com/docs/sql-reference/aggregate-functions/reference/grouparraysorted).


## Native connector boundaries

`native_platforms` grants operator-selected domain suffix authority; its token
is hashed and never installed in a CMS. A custom origin outside that authority
needs an independent TXT verification. JWKS are pinned through an authenticated
website connector; JWTs never initiate network discovery. RS256, issuer,
dedicated audience, role, website, expiry/issued-time bounds and one-use jti
are checked. Opaque staff access lasts 60 seconds and grants one website's
report reads only; it never creates a customer or owner session. Generation,
credential revocation, platform/workspace/website state and the current origin
are rechecked on native reads. Integration credentials are rechecked under the
same workspace lock as durable event acceptance.

CMS integration state is outside settings/public APIs and only public collector
attributes enter theme context. Hosting exports scrub native credentials from
generated configuration. Automatic conversions do not join member identities
to browser sessions. Transactional source outboxes transmit hashed event IDs
and minimized facts only. See GHOST_INTEGRATION.md for exact retry/dead-letter,
consent and supported provider event behavior. Independent SSO/security review
and production service/restore/load gates remain required.


Priority 3 telemetry remains opt-in and obeys consent, DNT and GPC. Coarse OS
categories are stored without raw user agents. Newsletter edition/recipient
hashes are scoped to the website and edition, cannot join different editions
or browser sessions, and recipient hashes are excluded from CSV exports.
Native staff read tokens remain read-only. Cost writes require an active
website integration credential or a managing customer session with exact
Origin, with authorization rechecked under the workspace lock. Source
references are hashed and revisions reject conflicting/stale updates; supplied
costs are labeled with their provenance, not presented as provider-verified.
