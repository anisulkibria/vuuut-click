ALTER TABLE users ADD COLUMN display_name text NOT NULL DEFAULT '' CHECK(length(display_name)<=100);
ALTER TABLE users ADD COLUMN email_verified_at timestamptz;
ALTER TABLE organizations ADD COLUMN status text NOT NULL DEFAULT 'active' CHECK(status IN ('active','suspended','deleting'));
ALTER TABLE plans ADD COLUMN seat_limit integer NOT NULL DEFAULT 3 CHECK(seat_limit>0);
UPDATE plans SET seat_limit=10 WHERE id='pro';
ALTER TABLE websites ADD COLUMN retention_days integer NOT NULL DEFAULT 90 CHECK(retention_days BETWEEN 7 AND 90);
ALTER TABLE websites ADD COLUMN deletion_requested_at timestamptz;
ALTER TABLE subscriptions ADD COLUMN provider_customer text;
ALTER TABLE subscriptions ADD COLUMN billing_period text NOT NULL DEFAULT 'monthly' CHECK(billing_period IN ('monthly','annual'));
ALTER TABLE subscriptions ADD COLUMN provider_event_created bigint NOT NULL DEFAULT 0;
CREATE TABLE invitations (
 id uuid PRIMARY KEY,organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
 email text NOT NULL,role text NOT NULL CHECK(role IN ('admin','analyst','viewer')),
 token_hash text NOT NULL UNIQUE,invited_by uuid REFERENCES users ON DELETE SET NULL,
 expires_at timestamptz NOT NULL,accepted_at timestamptz,revoked_at timestamptz,created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX invitations_org ON invitations(organization_id,created_at DESC);
CREATE TABLE account_tokens (
 token_hash text PRIMARY KEY,user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
 purpose text NOT NULL CHECK(purpose IN ('verify','reset')),expires_at timestamptz NOT NULL,used_at timestamptz
);
CREATE TABLE mail_outbox (
 id uuid PRIMARY KEY,recipient text NOT NULL,subject text NOT NULL,body text,
 state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','delivered','dead')),
 attempts integer NOT NULL DEFAULT 0,available_at timestamptz NOT NULL DEFAULT now(),created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX mail_outbox_pending ON mail_outbox(available_at) WHERE state='pending';
CREATE TABLE funnels (
 id uuid PRIMARY KEY,website_id uuid NOT NULL REFERENCES websites ON DELETE CASCADE,
 name text NOT NULL CHECK(length(name) BETWEEN 1 AND 100),steps jsonb NOT NULL,
 window_seconds integer NOT NULL DEFAULT 86400 CHECK(window_seconds BETWEEN 60 AND 604800),created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(website_id,name)
);
CREATE TABLE export_jobs (
 id uuid PRIMARY KEY,organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
 website_id uuid NOT NULL REFERENCES websites ON DELETE CASCADE,requested_by uuid REFERENCES users ON DELETE SET NULL,
 start_at bigint NOT NULL,end_at bigint NOT NULL,state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','complete','failed')),
 payload bytea,error text,created_at timestamptz NOT NULL DEFAULT now(),expires_at timestamptz NOT NULL DEFAULT now()+interval '1 day'
);
CREATE TABLE deletion_jobs (
 id uuid PRIMARY KEY,organization_id uuid NOT NULL,website_id uuid,requested_by uuid REFERENCES users ON DELETE SET NULL,
 state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','complete','failed')),error text,created_at timestamptz NOT NULL DEFAULT now(),completed_at timestamptz
);
CREATE UNIQUE INDEX deletion_jobs_site_pending ON deletion_jobs(website_id) WHERE state='pending';
CREATE TABLE billing_events (
 provider text NOT NULL,event_id text NOT NULL,event_type text NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(provider,event_id)
);
CREATE TABLE control_migrations (version text PRIMARY KEY,checksum text NOT NULL,applied_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE platform_audit (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,actor_id uuid REFERENCES users ON DELETE SET NULL,
 action text NOT NULL,resource_id uuid,created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX outbox_recent ON event_outbox(accepted_at DESC);
