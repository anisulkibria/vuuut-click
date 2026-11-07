CREATE TABLE IF NOT EXISTS events (
    organization_id String,
    website_id String,
    event_id String,
    timestamp DateTime('UTC'),
    session_id String,
    visitor_id String,
    name LowCardinality(String),
    path String,
    country LowCardinality(String),
    referrer String,
    utm_source String,
    utm_medium String,
    utm_campaign String,
    device LowCardinality(String),
    browser LowCardinality(String)
) ENGINE = ReplacingMergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (organization_id,website_id,timestamp,event_id)
TTL timestamp + INTERVAL 90 DAY DELETE;

CREATE TABLE IF NOT EXISTS pageviews_hourly (
    organization_id String,
    website_id String,
    bucket DateTime('UTC'),
    views AggregateFunction(uniqExact,String),
    visitors AggregateFunction(uniqExact,String),
    sessions AggregateFunction(uniqExact,String)
) ENGINE = AggregatingMergeTree
PARTITION BY toYYYYMM(bucket)
ORDER BY (organization_id,website_id,bucket)
TTL bucket + INTERVAL 90 DAY DELETE;

CREATE MATERIALIZED VIEW IF NOT EXISTS pageviews_hourly_mv TO pageviews_hourly AS
SELECT organization_id,website_id,toStartOfHour(timestamp) AS bucket,
    uniqExactState(event_id) AS views,
    uniqExactState(visitor_id) AS visitors,
    uniqExactState(session_id) AS sessions
FROM events WHERE name='pageview'
GROUP BY organization_id,website_id,bucket;
