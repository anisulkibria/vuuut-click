ALTER TABLE events ADD COLUMN IF NOT EXISTS visitor_mode LowCardinality(String) DEFAULT 'unknown';
ALTER TABLE events ADD COLUMN IF NOT EXISTS os LowCardinality(String) DEFAULT 'Unknown';
ALTER TABLE events ADD COLUMN IF NOT EXISTS engagement_ms UInt32 DEFAULT 0;
ALTER TABLE events ADD COLUMN IF NOT EXISTS scroll_percent UInt8 DEFAULT 0;
ALTER TABLE events ADD COLUMN IF NOT EXISTS newsletter_id String DEFAULT '';
ALTER TABLE events ADD COLUMN IF NOT EXISTS recipient_id String DEFAULT '';
