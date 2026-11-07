-- Platform ownership is an explicit operator grant, never inferred from a customer workspace role.
CREATE TABLE platform_owners (
    user_id uuid PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    granted_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE sessions ADD COLUMN audience text NOT NULL DEFAULT 'user'
    CHECK (audience IN ('user','owner'));
CREATE INDEX sessions_user_audience ON sessions(user_id,audience);
