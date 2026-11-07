ALTER TABLE events ADD COLUMN IF NOT EXISTS occurred_ms Int64 DEFAULT toUnixTimestamp(timestamp)*1000;
ALTER TABLE events ADD COLUMN IF NOT EXISTS metric_name LowCardinality(String) DEFAULT '';
ALTER TABLE events ADD COLUMN IF NOT EXISTS metric_value Float64 DEFAULT 0;
ALTER TABLE events ADD COLUMN IF NOT EXISTS amount_minor Int64 DEFAULT 0;
ALTER TABLE events ADD COLUMN IF NOT EXISTS currency LowCardinality(String) DEFAULT '';
ALTER TABLE events ADD COLUMN IF NOT EXISTS metric_id String DEFAULT event_id;
