ALTER TABLE event_outbox DROP CONSTRAINT event_outbox_state_check;
ALTER TABLE event_outbox ADD CONSTRAINT event_outbox_state_check CHECK(state IN ('pending','delivered','dead','discarded'));
