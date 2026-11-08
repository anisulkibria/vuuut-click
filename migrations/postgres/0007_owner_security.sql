-- Platform owner second factor and recovery.
--
-- Owners are users, but their security is kept in its own table rather than
-- as columns on `users`: the customer account flows deliberately exclude
-- owners (see `mail::request_token`), so owner recovery is a separate
-- mechanism and keeping its state separate makes that boundary visible.
CREATE TABLE owner_security (
    user_id uuid PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    -- Base32, as authenticator apps exchange it. Present once enrolment
    -- starts; `confirmed_at` is what makes it required at sign-in, so an
    -- abandoned enrolment never locks anybody out.
    totp_secret text CHECK (totp_secret IS NULL OR length(totp_secret) BETWEEN 16 AND 128),
    confirmed_at timestamptz,
    -- The last step accepted, so a code cannot be replayed inside its own
    -- 30-second window by someone who watched it being typed.
    last_step bigint,
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- One-use recovery codes, stored only as hashes.
CREATE TABLE owner_recovery_codes (
    code_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX owner_recovery_codes_user ON owner_recovery_codes(user_id) WHERE used_at IS NULL;
