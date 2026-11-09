# Priority 3 metric contract

This contract defines the implemented Priority 3 analytics and their measurement limits.
All report access remains website-scoped and holds the existing report budget.
Queries use deduplicated raw `events FINAL`; auxiliary telemetry is never a page
view, a bounce-reducing interaction, a funnel page or a new visitor. Report
periods retain the existing UTC acceptance-time semantics. Ordered paths and
conversion windows use the validated original occurrence time. Historical
coverage is limited by the website/plan retention and the available dataset.

## Period comparison and performance

Compare the selected period with the immediately preceding equal-duration
window. Return both exact UTC boundaries. Relative change is unavailable when
the previous value is zero; bounce change is percentage points. Vital comparisons
use the last value per metric instance within each window, with p50/p75/p95 and
sample sizes. Empty periods remain empty, never invented zero measurements.

## Content, acquisition and engagement

Entry/exit pages are the first/last pageview per session inside the selected
window, ordered by `(occurred_ms,event_id)`. Content reports count pageviews,
visitors and sessions per path, plus goal-converting sessions which viewed that
path; this is association, not causal attribution. Acquisition groups sessions
by their first pageview's source, medium, campaign, channel and landing page.
Channels follow a documented deterministic source/medium classification.
Operating system is a coarse user-agent category; raw agents are not retained.

Optional `data-engagement="true"` telemetry records visible-document time in
bounded increments and maximum scroll depth, with no text, DOM or form values.
It follows consent and DNT/GPC, excludes hidden time, attributes SPA transitions
to the page being left and preserves event IDs on retries. It is opt-in because
heartbeats consume event allowance. Reports label observed visible seconds and
coverage; sessions/pages without telemetry have no engagement measurement.

## Returning visitors and cohorts

Only events with an explicitly supplied visitor UUID qualify for cross-session
visitor reports. Session-only and legacy/unclassified records are counted
separately. No identity stitching or fingerprinting is introduced. New means
first observed in the retained history during this window; returning means
observed before its start. These are browser identifiers, not people.
Weekly cohorts use the first observed UTC Monday week and pageview activity in
weeks 0–4. Incomplete future/ongoing follow-up weeks return unavailable cells,
not zero retention. Both lookback boundaries and identity coverage are returned.

## Funnels

Existing ordered session funnels remain the default. Add literal page-prefix
steps and an explicit provided-visitor scope for consented cross-session paths.
The same configured occurrence-time conversion window applies. Counts identify
their participant unit; page steps exclude telemetry/custom events. Date/window,
role/isolation and strict increasing-time guarantees remain in force.

## Newsletter editions

Authoritative backend facts may include website/edition-scoped opaque hashes for
newsletter edition and recipient. Public/browser events cannot supply these
facts. CMS captures those fields in the source transaction; edition recipient
hashes cannot stitch recipients across newsletters or to browser sessions.
A report's cohort is recipients whose send was captured in the selected period.
Opens/clicks are matched to that cohort through the range end and after send
occurrence. Count distinct opened/clicked recipients and unique link-recipient
facts separately. Missing captured sends/identifiers never become invented
rates. Titles may be resolved locally in CMS Admin without sending them to Click.

## Campaign costs and ROI

Costs are explicit backend or manager-supplied records with source provenance,
UTC date, campaign, currency, nonnegative integer minor units and a stable
source reference. Persist revisions/idempotency; adjustments replace the same
fact. They are records supplied by the operator/integration, not a claim that
Click independently verified an advertising provider. Public tracking cannot supply costs; authenticated managers can record their advertising ledger. Each website permits up to 50,000 retained cost records, with corrections updating the original source reference.
Currency-separated net revenue uses the selected attribution model. ROI is
`(net revenue - cost) / cost * 100`, ROAS is `net revenue / cost`. Require positive
cost and explicit daily cost coverage across the selected UTC dates. Missing
campaign attribution or coverage, absent cost, zero cost and unmatched currency return unavailable ROI.
Explicit zero-cost daily records may establish coverage but not a denominator.
Never invent ad spend, currency conversion or provider verification. Retention,
authorization, auditing and website deletion also cover cost records.

## Required evidence

Real-store tests must cover equal windows/zero baselines, entry/exit ties,
telemetry exclusion/dedup, anonymous/provided identity separation, left-censored
cohorts/incomplete cells, cross-session/prefix funnels, newsletter send-cohort
matching and replay, provenance/revision/role isolation and incomplete/zero/
currency-separated cost coverage. Browser tests cover new report controls,
engagement consent/visibility/SPA behavior and desktop/phone layouts. Existing
Priority 1/2 suites remain regression gates. Update this contract if implementation
forces a deliberate semantic change; do not silently loosen the assertions.
