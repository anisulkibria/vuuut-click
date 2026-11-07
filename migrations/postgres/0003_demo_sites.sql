ALTER TABLE websites ADD COLUMN is_demo boolean NOT NULL DEFAULT false;
CREATE UNIQUE INDEX websites_one_demo_per_org ON websites(organization_id) WHERE is_demo;
