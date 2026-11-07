CREATE TABLE billing_checkouts (
 id uuid PRIMARY KEY,organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
 plan_id text NOT NULL REFERENCES plans,billing_period text NOT NULL CHECK(billing_period IN ('monthly','annual')),
 provider_reference text UNIQUE,url text,created_at timestamptz NOT NULL DEFAULT now(),completed_at timestamptz
);
ALTER TABLE plans ADD COLUMN goal_limit integer NOT NULL DEFAULT 10 CHECK(goal_limit>0);
ALTER TABLE plans ADD COLUMN funnel_limit integer NOT NULL DEFAULT 1 CHECK(funnel_limit>0);
UPDATE plans SET goal_limit=100,funnel_limit=20 WHERE id='pro';
