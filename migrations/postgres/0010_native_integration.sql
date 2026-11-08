-- Platform credentials are operator-issued and never installed in tenant CMSes.
CREATE TABLE native_platforms (
 id text PRIMARY KEY CHECK(length(id) BETWEEN 1 AND 100),
 token_hash text NOT NULL UNIQUE,
 domain_suffix text NOT NULL CHECK(length(domain_suffix) BETWEEN 1 AND 253),
 active boolean NOT NULL DEFAULT true,
 created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE integration_mappings DROP CONSTRAINT integration_mappings_website_id_fkey;
ALTER TABLE integration_mappings ALTER COLUMN website_id DROP NOT NULL;
ALTER TABLE integration_mappings ADD FOREIGN KEY (website_id) REFERENCES websites ON DELETE SET NULL;
ALTER TABLE integration_mappings ADD COLUMN revision bigint NOT NULL DEFAULT 0;
ALTER TABLE integration_mappings ADD COLUMN request_hash text NOT NULL DEFAULT '';
ALTER TABLE integration_mappings ADD COLUMN state text NOT NULL DEFAULT 'active' CHECK(state IN ('active','suspended','deleted'));
CREATE TABLE native_connections (
 website_id uuid PRIMARY KEY REFERENCES websites ON DELETE CASCADE,
 key_id uuid NOT NULL REFERENCES api_keys ON DELETE CASCADE,
 issuer text NOT NULL,
 jwks jsonb NOT NULL,
 generation bigint NOT NULL DEFAULT 1,
 connected boolean NOT NULL DEFAULT true,
 updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE native_sessions (
 token_hash text PRIMARY KEY,
 website_id uuid NOT NULL REFERENCES native_connections ON DELETE CASCADE,
 generation bigint NOT NULL,
 expires_at timestamptz NOT NULL
);
CREATE INDEX native_sessions_expiry ON native_sessions(expires_at);
CREATE TABLE native_assertions (
 website_id uuid NOT NULL REFERENCES native_connections ON DELETE CASCADE,
 jti text NOT NULL CHECK(length(jti) BETWEEN 1 AND 100),
 expires_at timestamptz NOT NULL,
 PRIMARY KEY(website_id,jti)
);
