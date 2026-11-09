-- Durable delivery/maintenance evidence without retaining message bodies after expiry.
ALTER TABLE mail_outbox DROP CONSTRAINT mail_outbox_state_check;
ALTER TABLE mail_outbox ADD CONSTRAINT mail_outbox_state_check CHECK(state IN ('pending','delivered','dead','expired'));
ALTER TABLE mail_outbox ADD COLUMN expires_at timestamptz;
ALTER TABLE mail_outbox ADD COLUMN delivered_at timestamptz;
ALTER TABLE mail_outbox ADD COLUMN last_error text;
-- Legacy account/invitation messages cannot have a valid link older than seven days.
UPDATE mail_outbox SET expires_at=created_at+interval '7 days' WHERE state IN ('pending','dead');
CREATE TABLE maintenance_checks (
 name text PRIMARY KEY CHECK(name IN ('deletion','exports','mail','retention')),
 last_started_at timestamptz NOT NULL,
 last_succeeded_at timestamptz,
 last_failed_at timestamptz,
 last_error text,
 failures bigint NOT NULL DEFAULT 0
);
ALTER TABLE billing_checkouts ADD COLUMN request_snapshot jsonb;
ALTER TABLE billing_checkouts ADD COLUMN expires_at timestamptz NOT NULL DEFAULT now()+interval '25 hours';
ALTER TABLE billing_checkouts ADD COLUMN cancelled_at timestamptz;
CREATE INDEX billing_checkout_open ON billing_checkouts(organization_id,expires_at) WHERE completed_at IS NULL AND cancelled_at IS NULL;
CREATE INDEX mail_outbox_expiry ON mail_outbox(expires_at) WHERE state IN ('pending','dead');
ALTER TABLE account_tokens ADD COLUMN created_at timestamptz NOT NULL DEFAULT now();
CREATE INDEX account_token_requests ON account_tokens(user_id,purpose,created_at);
