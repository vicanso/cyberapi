-- Invitations to make an account (`docs/server-sync.md` §5): the hash of the
-- link's token, never the token. `team` and `role` are the place a team's
-- invitation gives; an administrator's has neither. `expires_at` is unix
-- seconds; `used_by` names the account made with it.
CREATE TABLE IF NOT EXISTS invites (
    id VARCHAR(64) PRIMARY KEY,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    inviter VARCHAR(64) NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    team VARCHAR(64) REFERENCES teams (id) ON DELETE CASCADE,
    role VARCHAR(16),
    created_at VARCHAR(40) NOT NULL,
    expires_at BIGINT NOT NULL,
    used_by VARCHAR(64),
    revoked INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS invites_inviter ON invites (inviter);
