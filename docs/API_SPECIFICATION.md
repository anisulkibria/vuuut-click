# API contract

[openapi.json](openapi.json) is the versioned OpenAPI 3.1 contract, also served at
`GET /v1/openapi.json`. It covers all 85 JSON/health operations plus documented
owner forms, with named success/error schemas, content types, path parameters,
redirects, empty bodies and retry headers. Validate it and route coverage with:

```sh
.local/ops-venv/bin/pip install -r scripts/requirements-release.txt
.local/ops-venv/bin/python scripts/validate-contract.py
# Optional: real local responses using the existing private development account.
.local/ops-venv/bin/python scripts/validate-contract.py --base http://localhost:8082 --accounts .local/development-accounts.json
```

The live checker is loopback-only, prints counts rather than response data, and
rejects a running binary with a stale embedded specification. It checks actual
account/organization/site/report responses against JSON Schema, including nulls,
UUIDs, dates, exact journey tuple shapes and string-valued monetary totals.

## Authentication and errors

Customer registration/login returns `{id,email}` and an HttpOnly
`click_session` cookie. `GET /v1/me` returns the customer identity. Platform owners
sign in separately at `/owner/signin`; `click_owner_session` is scoped to `/owner`.
MFA's temporary cookie cannot authorize reports. A customer's organization role
does not grant platform-owner access.

Cookie mutations require an exact `Origin` equal to PUBLIC_URL. Read keys are
website-scoped Bearer credentials for reports; integration keys authorize backend
events/native control. Public collection IDs authorize only origin-checked
collection. Native platform credentials and exchanged 60-second read tokens have
separate audiences. Cross-tenant access is hidden with 404 responses.

API clients should send `Accept: application/json`. Errors use
`{"error":{"code":400,"message":"..."}}`; browser navigation can negotiate an
HTML error page. Respect `Retry-After` on 429/503, and retain stable IDs when retrying
accepted facts. 204 and the webhook/reset-request/verification-request success
responses have no body. Do not unconditionally parse every successful response
as JSON. Owner form routes return HTML or 303 redirects.

## Collection and authoritative events

`POST /v1/collect/{token}` accepts a bounded `{events:[...]}` batch. A 202 response
contains `accepted` and `duplicates`; PostgreSQL commits the payload, dedup ledger
and usage before success. Delivery to ClickHouse is asynchronous and replay-safe.
Browser input cannot create authoritative payment/refund facts. URL queries,
raw IPs, full referrer URLs and form values are not stored as analytics data.

`POST /v1/websites/{id}/backend-events` needs an integration credential, stable
`external_id` and `kind`. It returns `{event_id,accepted,duplicates}`. Replay the
same identity/body after uncertain responses. Native signed collection follows
[the native v1 contract](GHOST_INTEGRATION.md).

## Reports and management

Reports share bounded periods and website authorization. Overview/breakdowns,
goals/funnels, journeys, revenue, comparisons, content/acquisition, engagement,
retention, vitals, newsletters and explicit-cost returns have concrete schemas.
See [metric semantics](ANALYTICS_METRICS.md); unavailable values are null rather
than invented zeros, and money stays separated by currency.

`GET /v1/websites/{id}/retention` returns retention/cohort analytics;
`PUT` on the same path configures retention and returns 204. Funnel PATCH also
returns 204. Export/deletion creation returns a job ID, with status exposed
separately; completed exports download as `text/csv`. Goal PATCH returns the
updated goal. Credentials are returned only at issue/rotation, while key listings
contain metadata. Billing checkout/portal actions are owner-only HTML forms;
Stripe webhooks are signature-verified raw-body requests.

SDKs are convenience clients rather than substitutes for the complete contract.
See [SDK/release procedures](RELEASE.md) for installation, signatures and publishing.
Private `/internal/metrics` uses an independent operator token and is blocked by
the public edge; its Prometheus contract is described in [operations](OPERATIONS.md).
