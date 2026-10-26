-- H1: the MCP tokens that may make and change APIs where their account
-- edits. Beside `mcp_tokens`, a row for each, so the tokens issued before
-- read as they were: allowed nothing new. Revoking a token takes its row.
CREATE TABLE IF NOT EXISTS mcp_token_writes (
    token VARCHAR(64) PRIMARY KEY REFERENCES mcp_tokens (id) ON DELETE CASCADE
);
