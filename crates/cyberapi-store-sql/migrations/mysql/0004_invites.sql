-- Invitations to make an account (`docs/server-sync.md` §5): the hash of the
-- link's token, never the token. `team` and `role` are the place a team's
-- invitation gives; an administrator's has neither. `expires_at` is unix
-- seconds; `used_by` names the account made with it.
CREATE TABLE IF NOT EXISTS invites (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    inviter VARCHAR(64) NOT NULL,
    team VARCHAR(64) NULL,
    role VARCHAR(16) NULL,
    created_at VARCHAR(40) NOT NULL,
    expires_at BIGINT NOT NULL,
    used_by VARCHAR(64) NULL,
    revoked INT NOT NULL DEFAULT 0,
    KEY invites_inviter (inviter),
    FOREIGN KEY (inviter) REFERENCES accounts (id) ON DELETE CASCADE,
    FOREIGN KEY (team) REFERENCES teams (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
