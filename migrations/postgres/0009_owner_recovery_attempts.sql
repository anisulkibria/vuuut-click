ALTER TABLE account_tokens DROP CONSTRAINT account_tokens_purpose_check;
ALTER TABLE account_tokens ADD CONSTRAINT account_tokens_purpose_check
    CHECK (purpose IN ('verify','reset','owner_reset'));
ALTER TABLE account_tokens ADD COLUMN attempts integer NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN factor_attempts integer NOT NULL DEFAULT 0;
