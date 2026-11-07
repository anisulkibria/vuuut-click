# MASTER CODEX PROMPT
## Build a Production-Ready, Rust-Powered Web Analytics SaaS with ClickHouse, Inspired by DataFast and Tinybird

You are a principal software architect, senior Rust engineer, ClickHouse specialist, SaaS platform engineer, DevOps engineer, security engineer, and frontend developer.

Your mission is to **design, implement, test, document, and prepare for production deployment a complete, independent, commercial web analytics SaaS platform**.

The product should provide capabilities comparable to DataFast, with a high-performance analytics backend powered by ClickHouse and a modern dashboard adapted from the Tinybird Web Analytics Starter Kit.

The platform must integrate seamlessly with my existing Rust-powered Ghost CMS port and my Rust-powered Ghost(Pro)-style managed hosting platform, while remaining completely independent and usable by any website.

This is a real commercial product, not a prototype or demonstration.

**Primary references:**

- ClickHouse: https://github.com/ClickHouse/ClickHouse
- Tinybird Web Analytics Starter Kit: https://github.com/tinybirdco/web-analytics-starter-kit
- DataFast: https://datafa.st
- Ghost CMS: https://github.com/TryGhost/Ghost
- Ghost official website: https://ghost.org
- MaxMind GeoLite2: https://dev.maxmind.com/geoip/geolite2-free-geolocation-data/

Use these references to understand the architecture, features, and user experience. Do not blindly copy their implementation or proprietary assets.

Inspect upstream repositories, current documentation, dependencies, and licenses before making architectural decisions.

---

# 1. Project Objectives

Develop a standalone analytics SaaS that provides:

1. Privacy-conscious website analytics.
2. Real-time traffic monitoring.
3. Visitor and session analytics.
4. Country-level geolocation.
5. Traffic source and campaign attribution.
6. Conversion tracking.
7. Custom events and goals.
8. Conversion funnels.
9. Visitor journeys.
10. Revenue attribution.
11. Content performance analytics.
12. Device and browser analytics.
13. Core Web Vitals.
14. Organizations, teams, and permissions.
15. Multi-tenant isolation.
16. Public and private APIs.
17. SaaS subscription billing.
18. Usage metering and quotas.
19. Data exports and retention controls.
20. Native integration with my Rust Ghost CMS and Ghost(Pro)-style hosting platform.
21. Optional self-hosted deployment.

The product must support three operating models.

### Model A: Independent Analytics SaaS

Customers can:

- Create an account.
- Create an organization.
- Add multiple websites.
- Verify domain ownership.
- Install a lightweight tracking script.
- View analytics dashboards.
- Create goals and funnels.
- Track conversions and revenue.
- Invite team members.
- Generate API keys.
- Manage subscription plans.
- Configure privacy and retention settings.

The SaaS must support websites built with any technology, including WordPress, Shopify, Webflow, Ghost, Next.js, React, Vue, Nuxt, Laravel, and static HTML.

### Model B: Managed Rust Ghost(Pro) Integration

My existing Rust-powered Ghost(Pro)-style platform hosts multiple independent Ghost publications.

Each hosted tenant must automatically receive analytics.

When a new Ghost site is provisioned:

1. The hosting platform calls the analytics provisioning API.
2. The analytics platform creates or links the corresponding organization and website.
3. The hosting platform receives the website ID and integration configuration.
4. The CMS automatically enables the tracking script.
5. Ghost Admin displays the analytics dashboard.
6. Member, subscription, newsletter, and revenue events are synchronized.
7. Analytics access is authenticated through the existing Ghost Admin session.
8. Analytics entitlements are determined by the Ghost hosting plan.

Tenants must not need to register separately for analytics or manually configure API keys.

### Model C: Standalone Rust Ghost CMS Integration

Standalone Ghost installations must support:

- Connecting to the hosted analytics SaaS.
- Secure API key or OAuth-based connection.
- Automatic website registration.
- Native analytics inside Ghost Admin.
- Server-side CMS event synchronization.
- Optional self-hosted analytics deployment.
- Disconnecting or migrating analytics providers.

The hosted and self-hosted modes must use compatible APIs.

---

# 2. Required Technology Stack

Use the following technologies unless there is a compelling, documented reason to change them.

## Backend

- Rust, stable toolchain.
- Axum.
- Tokio.
- Serde.
- SQLx for the relational control database.
- ClickHouse Rust client.
- Reqwest.
- Tracing and OpenTelemetry.
- Tower middleware.
- Rustls.

Use PostgreSQL for the independent SaaS control database unless integration with my existing Ghost infrastructure makes another relational database more appropriate.

Do not migrate my existing Ghost database unnecessarily.

## Analytics Storage

- Self-hosted ClickHouse.
- MergeTree tables.
- AggregatingMergeTree tables.
- Materialized views.
- Partitioning and TTL policies.
- Efficient batch insertion.
- Approximate distinct counting where appropriate.
- Query optimization and resource limits.

Do not rewrite ClickHouse or implement a new database engine.

## Event Processing

- Rust ingestion service.
- Durable event processing.
- NATS JetStream when a distributed queue is required.
- Lightweight durable local spool for standalone installations.
- Batching and retries.
- Idempotency and deduplication.
- Dead-letter handling.

## Geolocation

- MaxMind GeoLite2 Country database.
- Local MMDB lookup.
- Automatic database updates.
- IPv4 and IPv6 support.
- Trusted reverse-proxy handling.
- Unknown-country fallback.

Avoid depending on paid IP geolocation APIs.

## Frontend

Start by evaluating and adapting:

https://github.com/tinybirdco/web-analytics-starter-kit

Preserve useful dashboard components and UX patterns.

Replace Tinybird-specific APIs, data sources, and backend dependencies with our Rust API and ClickHouse.

Use TypeScript and the framework already present in the starter kit when practical.

Use Tailwind CSS if appropriate for the existing architecture.

Build a responsive, accessible, polished SaaS dashboard supporting light and dark themes.

## Deployment

- Docker.
- Docker Compose.
- Nginx, Caddy, or Traefik.
- Linux VPS.
- S3-compatible backup storage.
- Prometheus.
- Grafana.
- OpenTelemetry.

Kubernetes must remain optional.

---

# 3. Architecture Requirements

Design the platform using a modular Rust workspace.

Separate these logical components:

1. Analytics API gateway.
2. Event ingestion service.
3. Event processing workers.
4. ClickHouse storage adapter.
5. Analytics query service.
6. GeoIP service.
7. Visitor identity service.
8. Attribution engine.
9. SaaS authentication and authorization.
10. Organization and website management.
11. Subscription and entitlement service.
12. Usage metering.
13. Ghost CMS integration adapter.
14. Ghost(Pro) provisioning adapter.
15. Standalone Ghost connector.
16. Dashboard application.
17. Tracking SDK.
18. Administrative CLI.
19. Database migrations.
20. Deployment tooling.

Prefer a modular monolith with independently scalable ingestion and worker processes initially.

Do not introduce unnecessary microservices.

The analytics platform must remain independent of Ghost CMS.

Ghost integration must be implemented through versioned APIs and adapters.

---

# 4. Multi-Tenant SaaS Architecture

Implement:

- Organizations.
- Organization members.
- Roles and permissions.
- Websites.
- Verified domains.
- API credentials.
- Subscription plans.
- Entitlements.
- Usage quotas.
- Audit logs.
- Data retention policies.
- Tenant lifecycle management.

Each website belongs to an organization.

One organization can own multiple websites.

A user may belong to multiple organizations.

Support these initial roles:

- Owner.
- Administrator.
- Analyst.
- Viewer.

All data access must be scoped to the authenticated organization and authorized website.

Never trust a client-supplied tenant ID for authorization.

Implement tenant isolation at the application, query, credential, and operational layers.

Add automated tests proving that one tenant cannot access another tenant's data.

---

# 5. Database Design

Design and implement production-ready schemas for the relational control database and ClickHouse.

## Control Database

Include tables for:

- Users.
- Organizations.
- Organization memberships.
- Websites.
- Verified domains.
- API keys.
- Integration credentials.
- Plans.
- Subscriptions.
- Entitlements.
- Usage records.
- Goals.
- Funnels.
- Attribution settings.
- Data retention settings.
- Export jobs.
- Audit logs.
- Ghost integration mappings.

Use proper indexes, foreign keys, unique constraints, and migration versioning.

## ClickHouse

Design event schemas supporting:

- Page views.
- Sessions.
- Unique visitors.
- Referrers.
- UTM parameters.
- Country codes.
- Device types.
- Browsers.
- Operating systems.
- Custom events.
- Goal completions.
- Funnel steps.
- Content views.
- Member signups.
- Subscription events.
- Payments.
- Newsletter events.
- Core Web Vitals.

Create optimized aggregation tables for hourly, daily, and monthly analytics.

Implement retention policies and efficient queries.

Ensure replayed or duplicate events do not inflate published metrics.

Document the difference between exact and approximate unique-visitor counts.

---

# 6. Lightweight JavaScript Tracking SDK

Develop a lightweight, dependency-free browser tracker.

Requirements:

- TypeScript source.
- Small production bundle.
- Asynchronous loading.
- Minimal effect on page performance.
- Automatic page-view tracking.
- SPA route-change tracking.
- Referrer tracking.
- UTM attribution.
- Session handling.
- Custom events.
- Goal events.
- Core Web Vitals.
- Consent-aware tracking.
- Configurable visitor identity.
- First-party collection endpoint.
- Retry and batching.
- `sendBeacon` support where appropriate.

Never collect passwords, form values, authentication tokens, or sensitive query parameters.

Do not use invasive fingerprinting.

Provide integration instructions for:

- Plain HTML.
- React.
- Next.js.
- Vue.
- Nuxt.
- WordPress.
- Shopify.
- Webflow.
- Ghost CMS.

Provide npm distribution and CDN-compatible builds.

---

# 7. Visitor Tracking and Country Detection

Implement a privacy-conscious visitor identification system.

Support:

- Session-only identifiers.
- Optional first-party persistent identifiers.
- Configurable retention.
- Consent-aware modes.
- Anonymous event collection.
- Authenticated server-side conversion association.

Use GeoLite2 for country detection.

Country tracking must:

- Work with IPv4 and IPv6.
- Support trusted reverse proxies.
- Handle Cloudflare correctly.
- Prevent spoofed forwarding headers.
- Avoid storing raw IP addresses unnecessarily.
- Support automatic GeoLite2 updates.
- Provide unknown-country fallback.

Review the current MaxMind licensing requirements for commercial SaaS and self-hosted distribution.

Do not redistribute licensed databases in violation of their terms.

---

# 8. Analytics Features

Implement the following dashboard modules.

## Overview

- Unique visitors.
- Page views.
- Sessions.
- Bounce rate.
- Average engagement.
- Traffic trends.
- Period comparisons.
- Real-time activity.

Define each metric precisely and consistently.

## Audience

- Countries.
- Devices.
- Browsers.
- Operating systems.
- New and returning visitors where supported by the identity mode.

## Acquisition

- Referrers.
- Traffic channels.
- UTM campaigns.
- Source and medium.
- Landing pages.
- Campaign performance.

## Content

- Top pages.
- Entry pages.
- Exit pages.
- Post-level analytics.
- Content engagement.
- Conversion performance by page.

## Events and Goals

- Custom events.
- Goal definitions.
- Goal completions.
- Conversion rates.
- Event properties.
- Event filtering.

## Funnels

- Configurable funnel steps.
- Conversion rates.
- Drop-off analysis.
- Date filtering.
- Attribution windows.

## Visitor Journeys

- Session-based journeys.
- Page navigation sequences.
- Conversion paths.
- Privacy-conscious identity handling.

## Revenue Attribution

- Revenue events.
- Conversion source attribution.
- First-touch attribution.
- Last-touch attribution.
- Last non-direct attribution.
- Campaign ROI where reliable cost data exists.
- Revenue by source.
- Revenue by campaign.
- Revenue by landing page.

Do not fabricate advertising cost data or ROI.

Use authoritative backend payment events and deduplicate webhooks.

## Core Web Vitals

Track supported web performance metrics through browser APIs and appropriate libraries.

Provide percentile-based reporting and historical comparisons.

---

# 9. Public and Private API

Design a versioned REST API.

Use `/v1/` as the initial API version.

Provide endpoints for:

- Authentication.
- Organizations.
- Websites.
- Domain verification.
- Event ingestion.
- Overview analytics.
- Traffic timeseries.
- Countries.
- Pages.
- Referrers.
- Campaigns.
- Events.
- Goals.
- Funnels.
- Journeys.
- Revenue.
- Members.
- Newsletters.
- Real-time analytics.
- API keys.
- Subscription plans.
- Usage.
- Exports.
- Privacy settings.
- Ghost provisioning.
- Ghost integration lifecycle.

Generate an OpenAPI 3.1 specification.

Provide typed TypeScript and Rust SDKs.

Use appropriate rate limits, pagination, validation, error handling, and authentication.

Public browser collection identifiers must not grant analytics read access.

Never expose arbitrary ClickHouse SQL execution to untrusted users.

---

# 10. Ghost CMS Integration

Develop a reusable Ghost analytics integration package.

It must support my Rust Ghost CMS port without assuming that my internal database schema is identical to upstream Ghost.

Before integration, inspect my existing Rust Ghost repository and identify:

- Authentication implementation.
- Admin routing.
- Member model.
- Post model.
- Newsletter model.
- Subscription model.
- Payment integrations.
- Event system.
- Existing analytics features.
- Existing tenant architecture.

Do not invent internal interfaces.

If the repository is unavailable, define a documented adapter contract and implement mock integrations for testing.

The integration must support:

- Automatic tracker installation.
- Native Ghost Admin analytics dashboard.
- Post-level analytics.
- Member growth analytics.
- Newsletter performance.
- Subscription analytics.
- Revenue reporting.
- Analytics configuration.
- Secure API communication.

Use transactional outbox processing for important CMS lifecycle events.

---

# 11. Ghost(Pro)-Style Managed Hosting Integration

Develop an integration specifically for my Rust Ghost(Pro)-style hosting platform.

When a hosted tenant is created:

1. Provision the Ghost publication.
2. Provision or associate an analytics organization.
3. Register the website.
4. Generate scoped integration credentials.
5. Configure the tracking script.
6. Enable the dashboard in Ghost Admin.
7. Apply plan-based analytics entitlements.
8. Start collecting analytics.

When a tenant changes domains, update verified domain mappings safely.

When a tenant upgrades or downgrades, update analytics entitlements.

When a tenant is suspended, apply the configured analytics access and ingestion policy.

When a tenant is deleted, execute the documented analytics data lifecycle workflow.

All provisioning operations must be idempotent and retry-safe.

Implement secure single sign-on or short-lived delegated access for embedded analytics.

Never expose platform-level API credentials to tenants.

---

# 12. Standalone Ghost CMS Integration

Develop a standalone Ghost connector supporting two modes.

## Hosted Analytics

The Ghost administrator can connect the CMS to the analytics SaaS.

Support:

- Website registration.
- Secure credential exchange.
- Connection testing.
- Tracking configuration.
- Native analytics dashboard.
- CMS event synchronization.
- Disconnect and reconnect.
- Credential rotation.

## Self-Hosted Analytics

Allow the administrator to deploy the analytics stack locally using Docker Compose.

Support:

- Local ClickHouse.
- Local Rust analytics API.
- Local GeoLite2 database.
- Local dashboard.
- Local credentials.
- Backup and restore.
- Versioned migrations.

Use the same API contracts for both modes.

Do not require standalone customers to depend on the hosted SaaS when using the self-hosted mode.

---

# 13. SaaS Billing and Entitlements

Implement a provider-agnostic billing architecture.

Do not hard-code one payment processor into the analytics core.

Support:

- Free plans.
- Paid plans.
- Monthly and annual billing.
- Event-volume allowances.
- Website limits.
- Team seat limits.
- Retention limits.
- API limits.
- Feature entitlements.
- Plan upgrades and downgrades.
- Usage metering.
- Subscription cancellation.
- Billing webhooks.

Provide a payment-provider adapter.

If no provider credentials are configured, keep billing in a safe test mode without pretending that real payments succeeded.

Usage metering must count accepted billable events consistently and avoid duplicate charges caused by retries.

---

# 14. Security and Privacy

Implement production security controls.

Include:

- Secure authentication.
- RBAC.
- Tenant isolation.
- API key hashing and rotation.
- Rate limiting.
- Request validation.
- TLS.
- CSRF protection where applicable.
- Strict CORS policies.
- Secure cookies.
- Secrets management.
- Audit logging.
- Domain verification.
- Abuse prevention.
- Query resource limits.
- Dependency scanning.
- Backup encryption.
- Secure data exports.

Provide configurable data retention, export, and deletion workflows.

Consider GDPR, UK GDPR, ePrivacy, and other applicable regulations.

Do not claim universal legal compliance without review.

Document data-processing responsibilities for hosted and self-hosted installations.

---

# 15. Performance and Scalability

Optimize for efficient operation on affordable VPS infrastructure.

Start with:

- One ClickHouse instance.
- Rust API services.
- Durable ingestion.
- Batched inserts.
- Materialized aggregates.
- Optional Valkey caching.

Do not require Kubernetes, Kafka, or a distributed ClickHouse cluster for the initial deployment.

Design clear upgrade paths for:

- Horizontal ingestion scaling.
- Independent query scaling.
- ClickHouse replication.
- Storage expansion.
- Queue scaling.
- Dedicated enterprise tenants.

Benchmark performance rather than making unsupported capacity claims.

Test at multiple event volumes, including:

- 1 million events per month.
- 10 million events per month.
- 100 million events per month.

Measure ingestion throughput, query latency, memory usage, storage consumption, queue lag, and recovery behavior.

---

# 16. Deployment and Operations

Provide complete deployment configurations for:

1. Local development.
2. Production SaaS.
3. Managed Ghost hosting integration.
4. Standalone Ghost self-hosted analytics.

Use Docker Compose as the default deployment system.

Include:

- Dockerfiles.
- Compose files.
- Environment templates.
- Health checks.
- Persistent volumes.
- Resource limits.
- TLS configuration.
- Secrets handling.
- ClickHouse migrations.
- Control database migrations.
- GeoLite2 updater.
- Backup scripts.
- Restore scripts.
- Monitoring configuration.
- Upgrade instructions.
- Rollback procedures.

Use pinned dependency and container versions.

Never commit production secrets.

---

# 17. Testing Requirements

Create comprehensive automated tests.

## Unit Tests

Test:

- Event validation.
- Visitor identity.
- Country lookup.
- Attribution calculations.
- Entitlement rules.
- Metric calculations.
- API authorization.

## Integration Tests

Test:

- Rust API with ClickHouse.
- Queue delivery.
- Database migrations.
- Materialized aggregations.
- Ghost event synchronization.
- SaaS provisioning.
- Billing webhook processing.

## Security Tests

Test:

- Cross-tenant access.
- Credential misuse.
- Unauthorized API requests.
- Forged events.
- Request flooding.
- SQL injection prevention.
- Sensitive data leakage.

## End-to-End Tests

Test:

- Customer registration.
- Website creation.
- Tracking installation.
- Analytics collection.
- Dashboard rendering.
- Goal creation.
- Conversion tracking.
- Ghost(Pro) provisioning.
- Standalone Ghost connection.
- Subscription lifecycle.

## Reliability Tests

Test:

- ClickHouse outages.
- Queue outages.
- Worker restarts.
- Duplicate events.
- Delayed events.
- Database restore.
- Tenant deletion.
- Upgrade and rollback.

All critical tests must pass before declaring the product production-ready.

---

# 18. Documentation

Create comprehensive documentation covering:

- Architecture.
- Repository structure.
- Development setup.
- Environment configuration.
- Database schemas.
- API reference.
- Tracking SDK.
- Ghost integration.
- Ghost(Pro) integration.
- Standalone deployment.
- Billing.
- Security.
- Privacy.
- Backup and recovery.
- Monitoring.
- Scaling.
- Troubleshooting.
- Release procedures.

Create an `AGENTS.md` file containing project conventions and instructions for future Codex sessions.

Maintain a `CHANGELOG.md`.

Maintain an `IMPLEMENTATION_PLAN.md` with milestones, completed tasks, remaining tasks, blockers, and verification results.

---

# 19. Development Workflow for Codex

Follow this workflow strictly.

## Step 1 — Repository Inspection

Before writing code:

1. Inspect the current working directory.
2. Identify existing repositories and applications.
3. Check for existing architecture documents and AGENTS.md instructions.
4. Inspect the Tinybird starter kit.
5. Review the ClickHouse integration options.
6. Identify reusable code.
7. Identify licensing obligations.
8. Document assumptions and unknowns.

Do not overwrite existing work.

Do not modify unrelated projects.

## Step 2 — Architecture and Planning

Create:

- `docs/ARCHITECTURE.md`
- `docs/DATABASE_DESIGN.md`
- `docs/API_SPECIFICATION.md`
- `docs/SECURITY.md`
- `docs/DEPLOYMENT.md`
- `docs/GHOST_INTEGRATION.md`
- `IMPLEMENTATION_PLAN.md`
- `AGENTS.md`

Explain major architectural decisions and trade-offs.

## Step 3 — Foundation

Implement:

- Rust workspace.
- Shared configuration.
- Error handling.
- Logging.
- Control database.
- ClickHouse connection.
- Migrations.
- Docker development environment.
- CI pipeline.

## Step 4 — Analytics Engine

Implement:

- Browser tracker.
- Event ingestion.
- GeoLite2 enrichment.
- Durable queue.
- Batch processing.
- ClickHouse event storage.
- Aggregate queries.
- Traffic analytics.

## Step 5 — SaaS Platform

Implement:

- Authentication.
- Organizations.
- Websites.
- API credentials.
- Permissions.
- Subscription entitlements.
- Usage metering.

## Step 6 — Dashboard

Adapt the Tinybird starter kit.

Replace Tinybird APIs with the Rust analytics API.

Implement responsive analytics views and website management.

## Step 7 — Advanced Analytics

Implement:

- Goals.
- Funnels.
- Journeys.
- Revenue attribution.
- Campaign analytics.
- Core Web Vitals.

## Step 8 — Ghost Integration

Implement:

- Ghost CMS connector.
- Managed Ghost(Pro) provisioning.
- Native Admin dashboard.
- Member analytics.
- Newsletter analytics.
- Subscription analytics.
- Revenue analytics.

## Step 9 — Production Hardening

Implement:

- Security testing.
- Load testing.
- Monitoring.
- Backups.
- Restore verification.
- Privacy controls.
- Release automation.

---

# 20. Important Engineering Rules

Follow these rules throughout development.

1. Write maintainable, idiomatic Rust.
2. Prefer proven libraries over custom infrastructure.
3. Keep the system modular without overengineering.
4. Avoid unnecessary microservices.
5. Avoid unnecessary dependencies.
6. Do not create placeholder implementations and present them as complete.
7. Do not use mock data in production analytics.
8. Do not silently swallow errors.
9. Do not expose sensitive credentials.
10. Do not assume APIs or database schemas that have not been inspected.
11. Do not modify my existing Ghost CMS without first understanding its architecture.
12. Do not implement features that violate third-party licenses.
13. Do not claim tests passed unless they actually ran.
14. Do not claim production readiness without verification.
15. Document incomplete work and blockers.
16. Keep database migrations reproducible.
17. Preserve backward compatibility where practical.
18. Implement security and tenant isolation from the beginning.
19. Keep hosted and standalone deployment contracts compatible.
20. Prioritize correctness, reliability, and maintainability over unnecessary complexity.

---

# 21. Expected Deliverables

The final project must include:

- Complete Rust backend source code.
- ClickHouse database migrations.
- Relational database migrations.
- Event ingestion pipeline.
- Tracking SDK.
- GeoLite2 integration.
- Analytics query engine.
- Adapted Tinybird dashboard.
- SaaS account and organization management.
- Billing and entitlement infrastructure.
- Ghost CMS integration package.
- Ghost(Pro) integration package.
- Standalone Ghost connector.
- REST API and OpenAPI specification.
- Docker deployment configurations.
- Automated tests.
- Monitoring.
- Backup and recovery tooling.
- Complete documentation.

The application should be deployable, testable, and maintainable without relying on Tinybird Cloud.

---

# 22. Start Now

Begin by inspecting the current repository and available project files.

Do not immediately generate the entire application without understanding the existing codebase.

First:

1. Summarize what already exists.
2. Identify missing components.
3. Evaluate the Tinybird starter kit and its license.
4. Propose the final architecture.
5. Define the Rust workspace structure.
6. Design the control database and ClickHouse schemas.
7. Define the API contracts.
8. Create a phased implementation plan with acceptance criteria.
9. Identify the smallest independently testable vertical slice.

Then begin implementing that first vertical slice.

The first vertical slice must demonstrate:

**A browser sends a real page-view event → Rust validates and enriches it with GeoLite2 → the event is durably accepted → ClickHouse stores it → Rust queries the result → the dashboard displays the correct page views and country breakdown.**

Use real database queries and integration tests, not hard-coded dashboard values.

After completing each milestone:

- Run formatting and linting.
- Run relevant tests.
- Report what changed.
- Report verification results.
- Update `IMPLEMENTATION_PLAN.md`.
- Continue to the next milestone when safe.

If a decision requires unavailable credentials, paid services, or access to another repository, implement a safe interface or test adapter and document exactly what is needed.

**The ultimate objective is to build a commercially viable, production-ready, Rust-powered analytics SaaS comparable in purpose to DataFast, with ClickHouse as the analytics engine and seamless integration into my Rust Ghost CMS and Ghost(Pro)-style managed hosting platform.**