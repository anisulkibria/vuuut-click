-- Volatile, bounded shared budgets. Subjects are window-scoped HMACs, never IPs.
CREATE TABLE request_budgets (
 bucket_minute bigint NOT NULL,
 scope text NOT NULL CHECK(scope IN ('edge','auth','ip','auth_ip','collection','reports')),
 subject text NOT NULL CHECK(length(subject)<=64),
 hits bigint NOT NULL CHECK(hits>0),
 PRIMARY KEY(bucket_minute,scope,subject)
);
CREATE TABLE report_leases (
 id uuid PRIMARY KEY,
 organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
 expires_at timestamptz NOT NULL
);
CREATE INDEX report_leases_org ON report_leases(organization_id,expires_at);
CREATE TABLE operational_runs (
 id uuid PRIMARY KEY,
 kind text NOT NULL CHECK(kind IN ('backup','restore','benchmark','scan')),
 started_at timestamptz NOT NULL DEFAULT now(),
 completed_at timestamptz,
 status text NOT NULL CHECK(status IN ('running','succeeded','failed')),
 details jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX operational_runs_recent ON operational_runs(kind,started_at DESC);
