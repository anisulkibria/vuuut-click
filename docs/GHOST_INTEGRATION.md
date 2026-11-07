# Ghost integration contract

No changes to sibling repositories in this milestone. The SDK is an API client, not an assertion of completed native integration.

Inspected real interfaces:

- Vuuut: `ghost-api/src/permissions.rs`, `identity.rs` (RS256 staff tokens; JWKS; site audience), `member_events.rs`, `newsletter_events.rs`, `member_subscriptions.rs`; `ghost-front/src/analytics.rs`; Admin `/analytics/` and post `/posts/{id}/analytics/`.
- Vuuut-pro: `pro-provision/src/lifecycle.rs::create_site`, generated config in `config_file.rs`, hostSettings plan limits, separate staff/customer sessions in pro-web auth.

Planned versioned adapter: platform service credentials invoke idempotent provisioning keyed by `(platform_id,tenant_id)`; the analytics organization and website map to this key. Replays must return the original website and support credential rotation without returning platform secrets. CMS must save a website-scoped integration credential server-side. Tracker injection uses only public collection ID.

Before CMS write-side integration, add transactional outbox records to member/subscription/payment/newsletter mutation transactions; worker sends signed backend-only events with original event IDs. Never derive revenue from browser events. No schema compatibility with upstream Ghost is assumed. Domain changes need independently verified mappings; suspension and deletion are explicit lifecycle commands with audit logs and retry state.

Staff dashboard exchange must verify JWKS against a pre-registered issuer, verify signature/expiry/audience and mapped staff permissions, then issue a short-lived Click website-limited session. Current Vuuut identity audience cannot simply be repurposed. Use server-to-server exchange or a dedicated Click audience. No arbitrary issuer discovery/SSRF and no platform credentials in embedded content.

Standalone connector uses the same hosted or local base URL. Connect/test/register/disconnect/rotate APIs and native Admin embedding remain pending. Hosted mode must not be required in self-hosted deployments.
