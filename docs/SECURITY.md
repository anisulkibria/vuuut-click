# Security and privacy

Platform owners use `/owner/signin`; customers use `/signin`. Platform ownership is an explicit `platform_owners` grant from the operator CLI, never a public signup option or a workspace owner role. Owner credentials are rejected by the normal login endpoints. `click_owner_session` is scoped to `/owner`, lasts 8 hours, and resolves only sessions with audience `owner` and a current owner grant. Customer `click_session` is scoped to `/`, lasts 7 days, and resolves only audience `user` for non-platform-owner accounts. Neither portal falls back to the other cookie. Login/logout affects only its own audience. Cookie-name swapping cannot bypass the database audience checks.

Passwords use Argon2id, random opaque sessions/API keys are SHA-256 hashed at rest. HttpOnly, SameSite=Strict cookies use Secure for HTTPS; production startup requires HTTPS. Cookie mutations require an exact configured Origin. Bearer API keys are website scoped and cannot create credentials or organizations. Public collection tokens carry no read privilege.

All analytics routes resolve the website via authenticated membership or a website-scoped read key. Permission failures intentionally return 404. Browser collection requires a verified domain matching both Origin and event URL. This prevents casual cross-site contamination but a public collector cannot prove a human browser or prevent determined forged events. Public tokens are not secrets.

Only explicitly configured reverse-proxy CIDRs can contribute X-Forwarded-For. Walk from the immediate peer right to left until the first untrusted hop. Cloudflare country/IP headers are not trusted independently; deployments must configure current Cloudflare CIDRs in the edge and application allowlist. Bad/unknown lookup returns ZZ. No IP persistence, IP fingerprint or arbitrary user-agent storage. URL queries/fragments and referrer paths are removed; only explicit campaign fields survive validation. Site owners must avoid personal identifiers in paths/event names/campaign labels.

Current rate limits are per-process bounded buckets and do not replace edge/distributed abuse controls. Quotas are transactionally enforced. DB/query errors are logged with no payloads/credentials and return generic errors. TLS terminates at Caddy. PostgreSQL/ClickHouse are private to the Docker network in production.

## Release gates

Not yet production ready: MFA review, distributed/per-tenant request budgets, expanded abuse/concurrency testing, audited SSO, live SMTP/payment validation, OTel, dependency and image scanning, encrypted backup/restore drills and independent security review remain required. Email verification/reset, invitation/role/key lifecycle, durable privacy jobs and signature-verified billing workflows are implemented and locally tested. Reports have a process-wide concurrency cap and ClickHouse time/memory/result limits. Enforce deployment resource and data-volume budgets before public access.

Hosted operator is responsible for processor agreements, policies, retention/deletion fulfillment and incident response; customers control collection purposes and consent. Self-hosted operators assume both deployment and processing duties. No universal GDPR/ePrivacy compliance claim. Honor browser DNT/GPC and require explicit SDK consent unless an operator deliberately chooses anonymous mode after applicable review.

GeoLite databases are licensed, not bundled assets. Download with the operator's own MaxMind license; review commercial hosted use and any redistribution separately against current terms. Attribute MaxMind and use geoipupdate for updates. Test fixtures from MaxMind-DB are test data, not production GeoLite data.
