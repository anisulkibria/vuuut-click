ALTER TABLE funnels ADD COLUMN scope text NOT NULL DEFAULT 'session' CHECK(scope IN ('session','visitor'));
CREATE TABLE campaign_costs (
 id uuid PRIMARY KEY,
 website_id uuid NOT NULL REFERENCES websites ON DELETE CASCADE,
 reference_hash text NOT NULL,
 revision bigint NOT NULL CHECK(revision>0),
 request_hash text NOT NULL,
 day date NOT NULL,
 campaign text NOT NULL CHECK(length(campaign) BETWEEN 1 AND 120),
 currency text NOT NULL CHECK(currency IN ('USD','EUR','GBP','CAD','AUD','NZD','SGD','INR','BDT','JPY')),
 amount_minor bigint NOT NULL CHECK(amount_minor BETWEEN 0 AND 1000000000000),
 source text NOT NULL CHECK(length(source) BETWEEN 1 AND 32),
 updated_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(website_id,reference_hash)
);
CREATE INDEX campaign_costs_period ON campaign_costs(website_id,day);
