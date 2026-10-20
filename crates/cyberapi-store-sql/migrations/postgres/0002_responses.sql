-- The responses each API keeps (F7.9): every account its own, the newest
-- by `seq`. They go with their API, project or account.
CREATE TABLE IF NOT EXISTS responses (
    seq BIGSERIAL PRIMARY KEY,
    id VARCHAR(64) NOT NULL UNIQUE,
    account VARCHAR(64) NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    collection VARCHAR(64) NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    api VARCHAR(64) NOT NULL REFERENCES apis (id) ON DELETE CASCADE,
    doc TEXT NOT NULL,
    created_at VARCHAR(40) NOT NULL
);
CREATE INDEX IF NOT EXISTS responses_api ON responses (account, api, seq);
CREATE INDEX IF NOT EXISTS responses_collection ON responses (collection);
