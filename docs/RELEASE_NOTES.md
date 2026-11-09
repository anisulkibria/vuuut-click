# Vuuut Click 0.1.0

Independent Rust analytics with durable PostgreSQL acceptance and ClickHouse
reporting, a consent-aware browser tracker, native Vuuut/Vuuut-pro integration,
customer/owner dashboards, advanced analytics, test-validated billing/mail and
local operational recovery/monitoring tooling.

This release packages the typed browser/server JavaScript SDK and Rust SDK,
versioned scripts/types, complete OpenAPI response contracts, operator/migration
runbooks, checksum manifests and prepared signed-provenance publication.

Schema versions are PostgreSQL 0014 and ClickHouse 0003. Take a verified consistent
backup before migration; deploy Click before matching native adapters. See
MIGRATIONS.md and RELEASE.md for compatibility, verification and rollback.

Local validation does not certify public deployment. Public domain/TLS, licensed
GeoIP, production sender/billing setup, off-machine backup/password escrow,
sustained full-volume capacity, independent review and final remote acceptance
remain deployment gates. The operator chose local validation first.
