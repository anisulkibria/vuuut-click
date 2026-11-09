-- Operator credentials are authenticated ciphertext; the key is external to PG.
CREATE SEQUENCE platform_connection_revision;
CREATE TABLE platform_connections (
    service text PRIMARY KEY CHECK (service IN ('maxmind','smtp','stripe','telemetry')),
    ciphertext bytea NOT NULL,
    revision bigint NOT NULL DEFAULT nextval('platform_connection_revision'),
    updated_at timestamptz NOT NULL DEFAULT now()
);
