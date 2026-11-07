CREATE TABLE users (
    id uuid PRIMARY KEY,
    email text NOT NULL UNIQUE CHECK (length(email) <= 254),
    password_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE organizations (
    id uuid PRIMARY KEY,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE memberships (
    organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('owner','admin','analyst','viewer')),
    PRIMARY KEY (organization_id,user_id)
);
CREATE INDEX memberships_user ON memberships(user_id);
CREATE TABLE sessions (
    token_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sessions_expiry ON sessions(expires_at);
CREATE TABLE plans (
    id text PRIMARY KEY,
    monthly_events bigint NOT NULL CHECK (monthly_events > 0),
    website_limit integer NOT NULL CHECK (website_limit > 0),
    retention_days integer NOT NULL CHECK (retention_days > 0)
);
INSERT INTO plans VALUES ('free',100000,3,90), ('pro',1000000,10,90);
CREATE TABLE subscriptions (
    organization_id uuid PRIMARY KEY REFERENCES organizations ON DELETE CASCADE,
    plan_id text NOT NULL REFERENCES plans,
    provider text NOT NULL DEFAULT 'none',
    provider_reference text UNIQUE,
    status text NOT NULL DEFAULT 'free' CHECK (status IN ('free','test','active','past_due','cancelled','suspended'))
);
CREATE TABLE websites (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    origin text NOT NULL CHECK (length(origin) <= 255),
    collection_id text NOT NULL UNIQUE,
    verification_token text NOT NULL,
    verified_at timestamptz,
    active boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (organization_id,origin)
);
CREATE INDEX websites_org ON websites(organization_id);
CREATE TABLE api_keys (
    id uuid PRIMARY KEY,
    website_id uuid NOT NULL REFERENCES websites ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,
    label text NOT NULL CHECK (length(label) BETWEEN 1 AND 100),
    scope text NOT NULL CHECK (scope IN ('read','integration')),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);
CREATE TABLE usage_records (
    organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
    month date NOT NULL,
    accepted_events bigint NOT NULL DEFAULT 0 CHECK (accepted_events >= 0),
    PRIMARY KEY (organization_id,month)
);
CREATE TABLE event_outbox (
    website_id uuid NOT NULL REFERENCES websites ON DELETE CASCADE,
    event_id uuid NOT NULL,
    organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
    payload jsonb,
    accepted_at timestamptz NOT NULL DEFAULT now(),
    delivered_at timestamptz,
    attempts integer NOT NULL DEFAULT 0,
    available_at timestamptz NOT NULL DEFAULT now(),
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','delivered','dead')),
    PRIMARY KEY (website_id,event_id)
);
CREATE INDEX outbox_pending ON event_outbox(available_at,accepted_at) WHERE state='pending';
CREATE TABLE goals (
    id uuid PRIMARY KEY,
    website_id uuid NOT NULL REFERENCES websites ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    event_name text NOT NULL CHECK (length(event_name) BETWEEN 1 AND 64),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(website_id,name)
);
CREATE TABLE audit_logs (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations ON DELETE CASCADE,
    actor_id uuid REFERENCES users ON DELETE SET NULL,
    action text NOT NULL,
    resource_id uuid,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX audit_org_time ON audit_logs(organization_id,created_at DESC);
CREATE TABLE integration_mappings (
    platform_id text NOT NULL,
    tenant_id text NOT NULL,
    website_id uuid NOT NULL REFERENCES websites ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(platform_id,tenant_id)
);
