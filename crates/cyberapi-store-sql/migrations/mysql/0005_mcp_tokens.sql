-- MCP tokens: how an AI assistant signs in as an account at the bridge's
-- /v1/mcp. The hash of the token, never the token. `can_send` (0 / 1) lets
-- it send the account's saved requests; `expires_at` is unix seconds, or
-- null for a token that lasts until it is revoked, which deletes its row.
CREATE TABLE IF NOT EXISTS mcp_tokens (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    account VARCHAR(64) NOT NULL,
    name VARCHAR(64) NOT NULL,
    can_send INT NOT NULL DEFAULT 0,
    created_at VARCHAR(40) NOT NULL,
    expires_at BIGINT NULL,
    UNIQUE KEY mcp_tokens_account_name (account, name),
    FOREIGN KEY (account) REFERENCES accounts (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
