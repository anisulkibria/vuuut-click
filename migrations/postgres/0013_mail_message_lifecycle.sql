ALTER TABLE mail_outbox ADD COLUMN account_token_hash text REFERENCES account_tokens(token_hash) ON DELETE CASCADE;
ALTER TABLE mail_outbox ADD COLUMN invitation_id uuid REFERENCES invitations(id) ON DELETE CASCADE;
CREATE INDEX mail_outbox_account_token ON mail_outbox(account_token_hash) WHERE account_token_hash IS NOT NULL;
CREATE INDEX mail_outbox_invitation ON mail_outbox(invitation_id) WHERE invitation_id IS NOT NULL;
