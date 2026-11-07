# Vuuut Click

Independent analytics SaaS; never modify sibling Vuuut or Vuuut-pro without inspecting the affected code. Follow docs/ARCHITECTURE.md and keep IMPLEMENTATION_PLAN.md honest. No production readiness claims without all release gates.

## Session handoff

Read `CONTINUE.md` at the start of a new session, including when the user says only **continue**. It contains the accepted product decisions, current implementation, verification, restart instructions and prioritized unfinished work. The original detailed user requirements are preserved in `docs/ORIGINAL_BRIEF.md`; implementation status in `CONTINUE.md` takes precedence over stale milestone notes. Work was intentionally paused at the user's request for account-usage and device migration. Resume only when the user asks. Do not recreate completed features or treat the original production brief as already delivered. Never commit `.env`, `.local`, local account credentials, mail, databases, licensed MMDBs or generated test artifacts.

- Rust stable, Axum 0.8, SQLx/PostgreSQL, official ClickHouse client. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` after backend changes.
- `npm ci && npm run check && npm run build` verifies the browser tracker. The dashboard is Rust-rendered. `npm run test:e2e` uses the real development databases.
- Run database integration tests with `cargo test --workspace -- --include-ignored`; never call skipped tests passed.
- Authorize website access through the authenticated membership. Public collection IDs never authorize reads. SQL must be parameterized. Never retain raw IPs, full referrer URLs, arbitrary URL queries, form values, or secrets in events/logs.
- An event is accepted only after PostgreSQL commits it and its usage increment atomically. Retry idempotency is `(website_id,event_id)`. Analytics must be correct before ClickHouse background merges.
- Material 3 tokens match the existing rose palette. Accessible tables accompany charts. Render real API values and explicit empty/error states.
- Keep API contracts, migrations, license notices, verification evidence and remaining work current. No agents or delegation are required by this file.
