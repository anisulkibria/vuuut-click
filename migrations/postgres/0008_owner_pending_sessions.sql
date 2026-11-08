-- A sign-in that has proved the password but not yet the second factor.
--
-- Added to the existing audience set rather than invented as a new table, so
-- expiry, revocation and the cookie plumbing are the ones already in use. It
-- is deliberately distinct from 'owner': `authenticated_owner` accepts only
-- 'owner', so a half-finished sign-in can reach nothing.
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_audience_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_audience_check
    CHECK (audience IN ('user','owner','owner_pending'));
